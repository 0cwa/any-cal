use any_cal_anytype_adapter::{AnytypeRepository, AnytypeTransport, FakeAnytypeTransport};
use any_cal_core::{
    BridgeCheckpoint, BridgeDecision, BridgeError, BridgeErrorCode, BridgeRequest, BridgeResponse,
    BridgeTombstone, Collection, CollectionId, DavKind, Repository, RepositoryError,
    ResourceEnvelope, SyncDecision, WriteCondition, BRIDGE_SCHEMA_VERSION,
};
use any_cal_dav_server::DavServer;
use any_cal_observability::{
    AuditEventWriter, AuditHealthState, AuditReadbackQuery, AuditSummary, Correlation,
    ErrorCategory, Event, EventBuffer, Health, ReconciliationReport, MAX_AUDIT_READBACK,
};
use any_cal_sync::{CommitFault, ObservedResource, SyncState, SyncStore};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::net::ToSocketAddrs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

pub mod identity;

use crate::identity::{
    AccessDecision, AuthOutcome, Capability, CollectionKind, IdentityStore, Operation,
};

#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn request_shutdown(_: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    pub endpoint: String,
    pub api_version: String,
    pub space_id: String,
    pub contacts_collection: String,
    pub tasks_collection: String,
    pub listen_address: String,
    pub token: Option<String>,
    /// Runtime-only credential accepted for local health/status checks.
    /// This is intentionally environment-only and is never written by the GUI.
    pub local_auth_credential: Option<String>,
    pub transport_mode: String,
    pub sync_checkpoint: Option<String>,
    pub allow_lan: bool,
    pub reverse_proxy_tls: bool,
    pub auth_credential: Option<String>,
    /// Emit bounded authentication lifecycle events into the in-memory
    /// diagnostic buffer. Disabled by default because even principal labels
    /// can be sensitive in deployments that expose diagnostics.
    pub emit_auth_events: bool,
    pub rate_limit_per_minute: u32,
    pub max_connections: usize,
    /// Optional private directory for the durable, redacted audit journal.
    pub audit_directory: Option<String>,
    pub audit_queue_capacity: usize,
    /// Include bounded audit state in health/status responses.
    pub expose_audit_health: bool,
    /// Refuse durable DAV writes and readiness when the audit journal is not healthy.
    pub audit_required: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    Missing(&'static str),
    Invalid(String),
    Io(String),
    Repository(String),
}
impl AppConfig {
    pub fn defaults() -> Self {
        Self {
            // Headless Anytype API endpoint. The desktop/MCP service uses
            // 31009 and is not assumed to expose this HTTP API.
            endpoint: "http://127.0.0.1:31012".into(),
            api_version: "2025-11-08".into(),
            space_id: String::new(),
            contacts_collection: "contacts".into(),
            tasks_collection: "tasks".into(),
            listen_address: "127.0.0.1:8080".into(),
            token: None,
            local_auth_credential: None,
            // Live HTTP is the service default. The deterministic fake is
            // available only through an explicit test/development override.
            transport_mode: "http".into(),
            sync_checkpoint: None,
            allow_lan: false,
            reverse_proxy_tls: false,
            auth_credential: None,
            emit_auth_events: false,
            rate_limit_per_minute: 120,
            max_connections: 64,
            audit_directory: None,
            audit_queue_capacity: 64,
            expose_audit_health: false,
            audit_required: false,
        }
    }
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path)
            .map_err(|_| ConfigError::Io("configuration file could not be read".into()))?;
        let mut c = Self::defaults();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| ConfigError::Invalid("invalid configuration line".into()))?;
            c.set(k.trim(), v.trim())?;
        }
        Ok(c)
    }
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        match key {
            "endpoint" => self.endpoint = value.into(),
            "api_version" => self.api_version = value.into(),
            "space_id" => self.space_id = value.into(),
            "contacts_collection" => self.contacts_collection = value.into(),
            "tasks_collection" => self.tasks_collection = value.into(),
            "listen_address" => self.listen_address = value.into(),
            "token" => self.token = Some(value.into()),
            "transport_mode" => self.transport_mode = value.into(),
            "sync_checkpoint" => self.sync_checkpoint = Some(value.into()),
            "allow_lan" => self.allow_lan = parse_bool(value)?,
            "reverse_proxy_tls" => self.reverse_proxy_tls = parse_bool(value)?,
            "auth_credential" => self.auth_credential = Some(value.into()),
            "emit_auth_events" => self.emit_auth_events = parse_bool(value)?,
            "rate_limit_per_minute" => {
                self.rate_limit_per_minute = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("invalid rate_limit_per_minute".into()))?
            }
            "max_connections" => {
                self.max_connections = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("invalid max_connections".into()))?
            }
            "audit_directory" => self.audit_directory = Some(value.into()),
            "audit_queue_capacity" => {
                self.audit_queue_capacity = value
                    .parse()
                    .map_err(|_| ConfigError::Invalid("invalid audit_queue_capacity".into()))?
            }
            "expose_audit_health" => self.expose_audit_health = parse_bool(value)?,
            "audit_required" => self.audit_required = parse_bool(value)?,
            // Configuration keys are operator-controlled input.  Do not echo
            // an unknown key: a malformed key may itself contain a pasted
            // credential or a sensitive path.
            _ => return Err(ConfigError::Invalid("unknown configuration key".into())),
        };
        Ok(())
    }
    pub fn apply_env(
        &mut self,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<(), ConfigError> {
        for (k, v) in env {
            if k == "ANY_CAL_LOCAL_AUTH" {
                self.local_auth_credential = Some(v);
                continue;
            }
            let key = match k.as_str() {
                "ANY_CAL_ANYTYPE_ENDPOINT" => Some("endpoint"),
                "ANY_CAL_ANYTYPE_API_VERSION" => Some("api_version"),
                "ANY_CAL_SPACE_ID" => Some("space_id"),
                "ANY_CAL_CONTACTS_COLLECTION" => Some("contacts_collection"),
                "ANY_CAL_TASKS_COLLECTION" => Some("tasks_collection"),
                "ANY_CAL_LISTEN_ADDRESS" => Some("listen_address"),
                "ANY_CAL_ANYTYPE_TOKEN" => Some("token"),
                "ANY_CAL_TRANSPORT_MODE" => Some("transport_mode"),
                "ANY_CAL_SYNC_CHECKPOINT" => Some("sync_checkpoint"),
                "ANY_CAL_ALLOW_LAN" => Some("allow_lan"),
                "ANY_CAL_REVERSE_PROXY_TLS" => Some("reverse_proxy_tls"),
                "ANY_CAL_AUTH_CREDENTIAL" => Some("auth_credential"),
                "ANY_CAL_EMIT_AUTH_EVENTS" => Some("emit_auth_events"),
                "ANY_CAL_RATE_LIMIT_PER_MINUTE" => Some("rate_limit_per_minute"),
                "ANY_CAL_MAX_CONNECTIONS" => Some("max_connections"),
                "ANY_CAL_AUDIT_DIRECTORY" => Some("audit_directory"),
                "ANY_CAL_AUDIT_QUEUE_CAPACITY" => Some("audit_queue_capacity"),
                "ANY_CAL_EXPOSE_AUDIT_HEALTH" => Some("expose_audit_health"),
                "ANY_CAL_AUDIT_REQUIRED" => Some("audit_required"),
                _ => None,
            };
            if let Some(key) = key {
                self.set(key, &v)?;
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.endpoint.trim().is_empty() {
            return Err(ConfigError::Missing("endpoint"));
        }
        if self
            .auth_credential
            .as_deref()
            .is_some_and(|value| value.chars().any(char::is_control))
            || self
                .local_auth_credential
                .as_deref()
                .is_some_and(|value| value.chars().any(char::is_control))
        {
            return Err(ConfigError::Invalid(
                "authentication credentials must not contain control characters".into(),
            ));
        }
        if self.auth_credential.as_deref().is_some_and(str::is_empty)
            || self
                .local_auth_credential
                .as_deref()
                .is_some_and(str::is_empty)
            || self.token.as_deref().is_some_and(str::is_empty)
        {
            return Err(ConfigError::Invalid(
                "credentials must not be empty when configured".into(),
            ));
        }
        if self.transport_mode != "fake" && self.transport_mode != "http" {
            return Err(ConfigError::Invalid(
                "transport_mode must be fake or http".into(),
            ));
        }
        if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
            return Err(ConfigError::Invalid(
                "endpoint must use http:// or https://".into(),
            ));
        }
        if self.api_version != any_cal_anytype_adapter::API_VERSION {
            return Err(ConfigError::Invalid("unsupported api_version".into()));
        }
        if self.space_id.trim().is_empty() {
            return Err(ConfigError::Missing("space_id"));
        }
        if self.listen_address.trim().is_empty() {
            return Err(ConfigError::Missing("listen_address"));
        }
        if self.listen_address.to_socket_addrs().is_err() {
            return Err(ConfigError::Invalid(
                "listen_address is not a valid socket address".into(),
            ));
        }
        let loopback = self
            .listen_address
            .to_socket_addrs()
            .map_err(|_| ConfigError::Invalid("invalid listen address".into()))?
            .all(|address| address.ip().is_loopback());
        if self.reverse_proxy_tls {
            if !loopback {
                return Err(ConfigError::Invalid(
                    "reverse_proxy_tls requires a loopback listen_address; expose the HTTPS reverse proxy, not this plaintext listener".into(),
                ));
            }
            if self.auth_credential.as_deref().is_none_or(str::is_empty) {
                return Err(ConfigError::Missing("auth_credential"));
            }
        }
        if !loopback {
            if !self.allow_lan {
                return Err(ConfigError::Invalid(
                    "non-loopback bind requires allow_lan=true".into(),
                ));
            }
            if !self.reverse_proxy_tls {
                return Err(ConfigError::Invalid(
                    "LAN HTTP requires reverse_proxy_tls=true; direct plaintext is refused".into(),
                ));
            }
        }
        if self.rate_limit_per_minute == 0 {
            return Err(ConfigError::Invalid(
                "rate_limit_per_minute must be positive".into(),
            ));
        }
        if self.max_connections == 0 {
            return Err(ConfigError::Invalid(
                "max_connections must be positive".into(),
            ));
        }
        if self.audit_queue_capacity == 0 {
            return Err(ConfigError::Invalid(
                "audit_queue_capacity must be positive".into(),
            ));
        }
        if self.audit_required && self.audit_directory.is_none() {
            return Err(ConfigError::Invalid(
                "audit_required requires audit_directory".into(),
            ));
        }
        if self.audit_directory.as_deref().is_some_and(str::is_empty) {
            return Err(ConfigError::Invalid(
                "audit_directory must not be empty".into(),
            ));
        }
        if self.contacts_collection.trim().is_empty() {
            return Err(ConfigError::Missing("contacts_collection"));
        }
        if self.tasks_collection.trim().is_empty() {
            return Err(ConfigError::Missing("tasks_collection"));
        }
        CollectionId::try_from(self.contacts_collection.as_str())
            .map_err(|_| ConfigError::Invalid("invalid contacts_collection".into()))?;
        CollectionId::try_from(self.tasks_collection.as_str())
            .map_err(|_| ConfigError::Invalid("invalid tasks_collection".into()))?;
        Ok(())
    }
}

impl AppGeneric<any_cal_anytype_adapter::HttpAnytypeTransport> {
    pub fn http(config: AppConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let transport = any_cal_anytype_adapter::HttpAnytypeTransport::from_config(
            config.endpoint.clone(),
            config.api_version.clone(),
            config.token.clone(),
        )
        .map_err(|e| ConfigError::Invalid(format!("Anytype HTTP transport: {e:?}")))?;
        Self::with_transport_mode(config, transport, "http")
    }

    /// Test seam for exercising HTTP-mode construction without requiring a
    /// loopback listener. Production callers should use [`Self::http`].
    pub fn http_with_exchange(
        config: AppConfig,
        exchange: Box<dyn any_cal_anytype_adapter::HttpExchange>,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        let transport = any_cal_anytype_adapter::HttpAnytypeTransport::from_config(
            config.endpoint.clone(),
            config.api_version.clone(),
            config.token.clone(),
        )
        .map_err(|e| ConfigError::Invalid(format!("Anytype HTTP transport: {e:?}")))?
        .with_exchange(exchange);
        Self::with_transport_mode(config, transport, "http")
    }
}

pub type App = AppGeneric<FakeAnytypeTransport>;
impl AppGeneric<FakeAnytypeTransport> {
    /// Build the complete application seam using the deterministic transport.
    /// This is an explicit test/development seam; the command-line service
    /// defaults to the live HTTP transport instead.
    pub fn fake(config: AppConfig) -> Result<Self, ConfigError> {
        Self::with_transport_mode(config, FakeAnytypeTransport::new(100), "fake")
    }
}

pub type AppWithTransport<T> = AppGeneric<T>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UpstreamState {
    /// The service has configuration, but has not performed a read against
    /// the configured Anytype endpoint yet.
    NotTested,
    Ready,
    Unavailable(&'static str),
}

impl UpstreamState {
    fn tested(self) -> bool {
        !matches!(self, Self::NotTested)
    }

    fn ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    fn error(self) -> Option<&'static str> {
        match self {
            Self::Unavailable(category) => Some(category),
            Self::NotTested | Self::Ready => None,
        }
    }

    fn status(self) -> &'static str {
        match self {
            Self::NotTested => "not_tested",
            Self::Ready => "ready",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

struct AuditStatus {
    ready: bool,
    state: &'static str,
    json: String,
}

/// The core bridge response deliberately contains only decisions and a
/// checkpoint.  Android needs the canonical payloads in the same response so
/// it can project Anytype changes into the account-owned provider rows.  Keep
/// this extension private to the app boundary; the core contract remains
/// usable by other adapters.
#[derive(Serialize)]
struct AndroidSyncTombstone {
    #[serde(flatten)]
    tombstone: BridgeTombstone,
    collection_id: String,
}

#[derive(Serialize)]
struct AndroidSyncResponse {
    #[serde(flatten)]
    bridge: BridgeResponse,
    resources: Vec<ResourceEnvelope>,
    tombstones: Vec<AndroidSyncTombstone>,
}

fn classify_audit_error(error: io::Error) -> String {
    let class = match error.kind() {
        io::ErrorKind::PermissionDenied => "permission",
        io::ErrorKind::InvalidData => "corrupt",
        io::ErrorKind::AlreadyExists | io::ErrorKind::WouldBlock => "locked",
        io::ErrorKind::NotFound => "missing",
        _ => "io",
    };
    format!("audit initialization failed ({class})")
}

pub struct AppGeneric<T: AnytypeTransport> {
    pub config: AppConfig,
    pub server: DavServer<AnytypeRepository<T>>,
    transport_mode: &'static str,
    upstream: UpstreamState,
    sync: Option<SyncStore>,
    pub events: EventBuffer,
    pub health: Health,
    audit: Option<AuditEventWriter>,
    audit_previous_state: Option<AuditHealthState>,
    audit_recovering: bool,
    next_request: u64,
    rate_window: Instant,
    rate_count: u32,
    identity: Option<IdentityStore>,
    identity_now: i64,
}
impl<T: AnytypeTransport> AppGeneric<T> {
    /// Attach an in-memory identity/ACL policy for a bounded service instance.
    /// Persistent credential lifecycle and clock selection remain outside this
    /// constructor; callers must provide the synthetic/validated clock value.
    pub fn with_identity(mut self, identity: IdentityStore, now: i64) -> Self {
        self.identity = Some(identity);
        self.identity_now = now;
        self
    }

    /// Load a digest-only identity snapshot and attach it to the DAV
    /// authorization boundary. Loading is explicit so callers can choose and
    /// audit the service-owned clock value.
    pub fn with_identity_file(self, path: &Path, now: i64) -> Result<Self, ConfigError> {
        let identity = IdentityStore::load_from(path)
            .map_err(|error| ConfigError::Io(format!("identity snapshot: {error:?}")))?;
        Ok(self.with_identity(identity, now))
    }

    /// Advance the identity clock in tests or in a service-owned clock adapter.
    pub fn set_identity_now(&mut self, now: i64) {
        self.identity_now = now;
    }

    pub fn handle(&mut self, request: any_cal_dav_server::Request) -> any_cal_dav_server::Response {
        self.next_request = self.next_request.saturating_add(1);
        let request_id = request
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("x-request-id"))
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| format!("req-{}", self.next_request));
        let sync_id = request
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("x-sync-id"))
            .map(|(_, value)| value.clone());
        let correlation = Correlation {
            request_id: Some(request_id),
            sync_id,
        };
        if let Some(status) = self.authorization_status(&request) {
            if request.path.starts_with("/admin/sync") {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Auth),
                    "operation=denied",
                ));
            }
            let mut headers = vec![("Content-Type".into(), "text/plain; charset=utf-8".into())];
            if request.path.starts_with("/admin/sync") {
                no_store(&mut headers);
                add_correlation(&mut headers, &correlation);
            }
            if status == 401 {
                headers.push((
                    "WWW-Authenticate".into(),
                    "Bearer realm=any-cal, Basic realm=any-cal".into(),
                ));
            }
            return any_cal_dav_server::Response {
                status,
                headers,
                // Keep all denial bodies generic: resource and policy details
                // are intentionally never reflected to an unauthorised caller.
                body: match status {
                    401 => b"unauthorized".to_vec(),
                    404 => b"not found".to_vec(),
                    _ => b"forbidden".to_vec(),
                },
            };
        }
        if self.rate_limited() {
            return any_cal_dav_server::Response {
                status: 429,
                headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
                body: b"rate limited".to_vec(),
            };
        }
        if request.path.starts_with("/admin/sync") {
            return self.handle_sync_admin(request, correlation);
        }
        if request.path == "/android/sync" {
            if !request.method.eq_ignore_ascii_case("POST") {
                let mut headers = vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("Allow".into(), "POST".into()),
                ];
                no_store(&mut headers);
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 405,
                    headers,
                    body: b"{\"status\":\"error\",\"error\":\"method_not_allowed\"}".to_vec(),
                };
            }
            return self.handle_android_sync(request, correlation);
        }
        if matches!(request.path.as_str(), "/health" | "/status" | "/ready") {
            if request.method != "GET" {
                let mut headers = health_headers();
                headers.push(("Allow".into(), "GET".into()));
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 405,
                    headers,
                    body: b"method not allowed".to_vec(),
                };
            }
            let readiness = request.path == "/ready";
            // A live service is not ready merely because its local listener
            // exists. Probe the configured Anytype Space on /ready and keep
            // the result separate from local audit/configuration health.
            if readiness {
                self.probe_upstream();
            }
            let audit = self.audit_snapshot();
            // An audit journal is an optional diagnostic sink unless the
            // operator explicitly makes it required.  Its degraded state is
            // still reported through health/status, but must not gate DAV
            // readiness in optional mode.
            let audit_ready = if self.config.audit_required {
                audit.as_ref().is_some_and(|status| status.ready)
            } else {
                true
            };
            let upstream_ready = self.upstream.ready();
            if readiness && !audit_ready {
                let mut headers = health_headers();
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 503,
                    headers,
                    body: b"{\"status\":\"unavailable\",\"ready\":false}".to_vec(),
                };
            }
            if readiness && !upstream_ready {
                let mut headers = health_headers();
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 503,
                    headers,
                    body: b"{\"status\":\"unavailable\",\"ready\":false,\"reason\":\"upstream\"}"
                        .to_vec(),
                };
            }
            let service_state = if self.upstream.status() != "ready" {
                self.upstream.status()
            } else {
                audit.as_ref().map_or("healthy", |status| status.state)
            };
            let service_ready = audit_ready && upstream_ready;
            let last_error = self
                .health
                .last_error
                .as_ref()
                .map_or("null".into(), |error| format!("\"{}\"", error_name(error)));
            // Health is an operational diagnostic, not a configuration dump.
            // Space and collection values may be Anytype identifiers, so only
            // expose their configured/not-configured state here.
            let audit_json = if self.config.expose_audit_health {
                audit.map_or_else(
                    || "\"audit\":{\"enabled\":false}".into(),
                    |status| format!("\"audit\":{}", status.json),
                )
            } else {
                String::new()
            };
            let separator = if audit_json.is_empty() { "" } else { "," };
            let sync_export_json = if self.sync.is_some() {
                ",\"sync_export_ready\":true"
            } else {
                ""
            };
            let upstream_json = self.upstream_json();
            let body = format!("{{\"status\":\"{service_state}\",\"ready\":{service_ready},\"space_configured\":{},\"contacts_collection_configured\":{},\"tasks_collection_configured\":{},\"transport\":\"{}\",\"upstream\":{},\"cache\":\"rebuildable\",\"events\":{},\"failures\":{},\"last_error\":{},\"recovery\":\"sync-checkpoint\"{}{}{} }}", !self.config.space_id.trim().is_empty(), !self.config.contacts_collection.trim().is_empty(), !self.config.tasks_collection.trim().is_empty(), self.transport_mode, upstream_json, self.health.counters.events, self.health.counters.failures, last_error, sync_export_json, separator, audit_json);
            self.health.record(true, 0, None);
            self.events
                .push(correlation.event("health", None, "health check"));
            let mut headers = health_headers();
            add_correlation(&mut headers, &correlation);
            return any_cal_dav_server::Response {
                status: 200,
                headers,
                body: body.into_bytes(),
            };
        }
        let method = request.method.clone();
        let options_request = method.eq_ignore_ascii_case("OPTIONS");
        let options_write_allowed = options_request
            .then(|| self.options_write_allowed(&request))
            .flatten();
        let durable = matches!(method.as_str(), "PUT" | "DELETE");
        let mut response = self.server.handle(request);
        if options_write_allowed == Some(false) {
            filter_write_methods(&mut response);
        }
        let mut checkpoint_failed = false;
        if durable && (200..300).contains(&response.status) {
            if let Err(error) = self.checkpoint_visible_state() {
                checkpoint_failed = true;
                response.status = 500;
                response.body = format!("sync checkpoint failed: {error}").into_bytes();
                response
                    .headers
                    .retain(|(key, _)| !key.eq_ignore_ascii_case("ETag"));
            }
        }
        let success = response.status < 400;
        let category = (!success).then_some(if checkpoint_failed {
            ErrorCategory::Recovery
        } else {
            match response.status {
                401 | 403 => ErrorCategory::Auth,
                408 | 504 => ErrorCategory::Timeout,
                409 | 412 => ErrorCategory::Conflict,
                500..=599 => ErrorCategory::Anytype,
                _ => ErrorCategory::Protocol,
            }
        });
        self.health.record(success, 0, category.clone());
        let audit_event = correlation.event(
            "dav.request",
            category,
            &format!("status={}", response.status),
        );
        self.events.push(audit_event.clone());
        if self.config.audit_required {
            if let Some(audit) = self.audit.as_ref() {
                if audit.append(audit_event).is_err() {
                    response.status = 503;
                    response.body = b"audit unavailable".to_vec();
                    response
                        .headers
                        .retain(|(key, _)| !key.eq_ignore_ascii_case("ETag"));
                }
            }
        } else if let Some(audit) = self.audit.as_ref() {
            let _ = audit.append(audit_event);
        }
        response
    }

    /// Handle the direct Android provider bridge.  The Android account
    /// adapter sends a pull with no payloads, or a bounded batch of provider
    /// edits with canonical envelopes.  The repository remains the only
    /// source of truth: this route never exposes provider row IDs or treats a
    /// failed Anytype read as an empty collection.
    fn handle_android_sync(
        &mut self,
        request: any_cal_dav_server::Request,
        correlation: Correlation,
    ) -> any_cal_dav_server::Response {
        const MAX_BODY: usize = 4 * 1024 * 1024;
        if request.body.len() > MAX_BODY {
            return android_sync_error(
                413,
                None,
                BridgeErrorCode::InvalidRequest,
                "bridge request is too large",
                &correlation,
            );
        }
        let body = match std::str::from_utf8(&request.body) {
            Ok(body) => body,
            Err(_) => {
                return android_sync_error(
                    400,
                    None,
                    BridgeErrorCode::InvalidRequest,
                    "bridge request is not valid UTF-8",
                    &correlation,
                )
            }
        };
        let bridge_request = match BridgeRequest::from_json(body) {
            Ok(request) => request,
            Err(_) => {
                return android_sync_error(
                    400,
                    None,
                    BridgeErrorCode::InvalidRequest,
                    "bridge request is invalid",
                    &correlation,
                )
            }
        };
        let Some((collection, expected_kind)) = self.android_collection(&bridge_request.authority)
        else {
            return android_sync_error(
                200,
                bridge_request.checkpoint,
                BridgeErrorCode::InvalidRequest,
                "Android authority is unsupported",
                &correlation,
            );
        };

        if bridge_request.resources.is_empty() && bridge_request.tombstones.is_empty() {
            self.android_pull(bridge_request, collection, expected_kind, correlation)
        } else {
            self.android_push(bridge_request, collection, expected_kind, correlation)
        }
    }

    fn android_collection(&self, authority: &str) -> Option<(CollectionId, DavKind)> {
        match authority {
            "com.android.contacts" => Some((self.server.contacts.clone(), DavKind::Contact)),
            // CalendarContract has no separate Anytype collection in the
            // current configuration. Events share the configured CalDAV
            // collection with VTODOs and are filtered by kind here.
            "com.android.calendar" => Some((self.server.tasks.clone(), DavKind::Event)),
            _ => None,
        }
    }

    fn android_pull(
        &mut self,
        request: BridgeRequest,
        collection: CollectionId,
        expected_kind: DavKind,
        correlation: Correlation,
    ) -> any_cal_dav_server::Response {
        let rows = match self.server.repository.list_resources(&collection, true) {
            Ok(rows) => rows,
            Err(error) => {
                let (status, code, message) = android_repository_error(&error);
                return android_sync_error(status, request.checkpoint, code, message, &correlation);
            }
        };
        let mut resources = Vec::new();
        let mut tombstones = Vec::new();
        let mut decisions = Vec::new();
        let mut revision = request
            .checkpoint
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.revision);
        for row in rows {
            if row.envelope.collection_id != collection || row.envelope.kind != expected_kind {
                continue;
            }
            revision = revision.max(row.envelope.revision);
            if row.archived {
                let tombstone_revision = row.envelope.revision.max(1);
                revision = revision.max(tombstone_revision);
                let resource_id = row.envelope.resource_id.clone();
                tombstones.push(AndroidSyncTombstone {
                    tombstone: BridgeTombstone {
                        resource_id: resource_id.clone(),
                        canonical_id: row.envelope.anytype_object_id.to_string(),
                        revision: tombstone_revision,
                    },
                    collection_id: collection.to_string(),
                });
                decisions.push(BridgeDecision {
                    resource_id,
                    decision: SyncDecision::Archive,
                    reason: None,
                });
            } else {
                let resource_id = row.envelope.resource_id.clone();
                resources.push(row.envelope);
                decisions.push(BridgeDecision {
                    resource_id,
                    decision: SyncDecision::Upsert,
                    reason: None,
                });
            }
        }
        resources.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        tombstones
            .sort_by(|left, right| left.tombstone.resource_id.cmp(&right.tombstone.resource_id));
        decisions.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        let response = BridgeResponse {
            schema_version: BRIDGE_SCHEMA_VERSION,
            checkpoint: Some(android_checkpoint(&request.authority, revision)),
            decisions,
            error: None,
        };
        android_sync_response(200, response, resources, tombstones, &correlation)
    }

    fn android_push(
        &mut self,
        request: BridgeRequest,
        collection: CollectionId,
        expected_kind: DavKind,
        correlation: Correlation,
    ) -> any_cal_dav_server::Response {
        if let Err(message) = validate_android_batch(&request, &collection, &expected_kind) {
            return android_sync_error(
                200,
                request.checkpoint,
                BridgeErrorCode::InvalidRequest,
                message,
                &correlation,
            );
        }
        // Resolve identities before any write. This both makes retries
        // deterministic and ensures an unavailable/read-failed repository
        // cannot be mistaken for an empty state that authorizes tombstones.
        let rows = match self.server.repository.list_resources(&collection, true) {
            Ok(rows) => rows,
            Err(error) => {
                let (status, code, message) = android_repository_error(&error);
                return android_sync_error(status, request.checkpoint, code, message, &correlation);
            }
        };
        let mut decisions = Vec::new();
        let mut resources = Vec::new();
        let mut tombstones = Vec::new();
        let mut revision = request
            .checkpoint
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.revision);
        for incoming in &request.resources {
            revision = revision.max(incoming.revision);
            let existing = rows.iter().find(|row| {
                row.envelope.resource_id == incoming.resource_id
                    || row.envelope.anytype_object_id == incoming.anytype_object_id
                    || row.envelope.dav_uid == incoming.dav_uid
            });
            let mut candidate = incoming.clone();
            candidate.collection_id = collection.clone();
            let stored = if let Some(existing) = existing {
                if existing.archived {
                    return android_sync_error(
                        200,
                        request.checkpoint.clone(),
                        BridgeErrorCode::Conflict,
                        "provider edit targets an archived resource",
                        &correlation,
                    );
                }
                // The server's stable IDs win over Android's provider-side
                // source ID. This prevents a second Anytype object when a
                // provider replays an edit with a provisional resource ID.
                candidate.resource_id = existing.envelope.resource_id.clone();
                candidate.anytype_object_id = existing.envelope.anytype_object_id.clone();
                candidate.dav_uid = existing.envelope.dav_uid.clone();
                match self
                    .server
                    .repository
                    .update_resource(candidate, WriteCondition::Unconditional)
                {
                    Ok(stored) => stored,
                    Err(error) => {
                        let (status, code, message) = android_repository_error(&error);
                        return android_sync_error(
                            status,
                            request.checkpoint.clone(),
                            code,
                            message,
                            &correlation,
                        );
                    }
                }
            } else {
                match self
                    .server
                    .repository
                    .create_resource(candidate, WriteCondition::IfNoneMatch)
                {
                    Ok(stored) => stored,
                    Err(error) => {
                        let (status, code, message) = android_repository_error(&error);
                        return android_sync_error(
                            status,
                            request.checkpoint.clone(),
                            code,
                            message,
                            &correlation,
                        );
                    }
                }
            };
            revision = revision.max(stored.envelope.revision);
            resources.push(stored.envelope);
            decisions.push(BridgeDecision {
                resource_id: incoming.resource_id.clone(),
                decision: SyncDecision::Upsert,
                reason: None,
            });
        }
        for incoming in &request.tombstones {
            revision = revision.max(incoming.revision);
            let existing = rows.iter().find(|row| {
                row.envelope.resource_id == incoming.resource_id
                    || row.envelope.anytype_object_id.as_str() == incoming.canonical_id
                    || row.envelope.dav_uid.as_str() == incoming.canonical_id
            });
            if let Some(existing) = existing {
                if existing.archived {
                    tombstones.push(AndroidSyncTombstone {
                        tombstone: incoming.clone(),
                        collection_id: collection.to_string(),
                    });
                    decisions.push(BridgeDecision {
                        resource_id: incoming.resource_id.clone(),
                        decision: SyncDecision::Noop,
                        reason: None,
                    });
                    continue;
                }
                let stored = match self.server.repository.archive_resource(
                    &existing.envelope.resource_id,
                    WriteCondition::Unconditional,
                ) {
                    Ok(stored) => stored,
                    Err(error) => {
                        let (status, code, message) = android_repository_error(&error);
                        return android_sync_error(
                            status,
                            request.checkpoint.clone(),
                            code,
                            message,
                            &correlation,
                        );
                    }
                };
                let tombstone_revision = incoming.revision.max(stored.envelope.revision).max(1);
                revision = revision.max(tombstone_revision);
                tombstones.push(AndroidSyncTombstone {
                    tombstone: BridgeTombstone {
                        resource_id: incoming.resource_id.clone(),
                        canonical_id: stored.envelope.anytype_object_id.to_string(),
                        revision: tombstone_revision,
                    },
                    collection_id: collection.to_string(),
                });
                decisions.push(BridgeDecision {
                    resource_id: incoming.resource_id.clone(),
                    decision: SyncDecision::Archive,
                    reason: None,
                });
            } else {
                tombstones.push(AndroidSyncTombstone {
                    tombstone: incoming.clone(),
                    collection_id: collection.to_string(),
                });
                decisions.push(BridgeDecision {
                    resource_id: incoming.resource_id.clone(),
                    decision: SyncDecision::Noop,
                    reason: None,
                });
            }
        }
        resources.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        tombstones
            .sort_by(|left, right| left.tombstone.resource_id.cmp(&right.tombstone.resource_id));
        decisions.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        android_sync_response(
            200,
            BridgeResponse {
                schema_version: BRIDGE_SCHEMA_VERSION,
                checkpoint: Some(android_checkpoint(&request.authority, revision)),
                decisions,
                error: None,
            },
            resources,
            tombstones,
            &correlation,
        )
    }

    /// OPTIONS is authorized as a read operation, but its Allow header must
    /// not promise writes that this principal cannot perform.  Apply the
    /// same collection/resource ACL and credential capability intersection
    /// used by mutation requests to the advertised methods.
    fn options_write_allowed(&self, request: &any_cal_dav_server::Request) -> Option<bool> {
        let identity = self.identity.as_ref()?;
        let token = bearer_or_basic_credential(request)?;
        let Some((principal, AuthOutcome::Authenticated)) =
            identity.authenticate(token, self.identity_now)
        else {
            return None;
        };
        let (collection, resource_id, _) = self.request_scope(request)?;
        Some(
            identity.policy.authorize(
                &principal,
                collection,
                resource_id.as_deref(),
                Operation::Write,
            ) == AccessDecision::Allowed
                && identity.credential_allows(
                    token,
                    self.identity_now,
                    capability_for(collection, Operation::Write),
                ),
        )
    }
    pub fn with_transport(config: AppConfig, transport: T) -> Result<Self, ConfigError> {
        Self::with_transport_mode(config, transport, "custom")
    }
    fn with_transport_mode(
        config: AppConfig,
        transport: T,
        transport_mode: &'static str,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        let contacts = CollectionId::try_from(config.contacts_collection.as_str()).unwrap();
        let tasks = CollectionId::try_from(config.tasks_collection.as_str()).unwrap();
        let mut server = DavServer::new(AnytypeRepository::new(transport, config.space_id.clone()));
        server.contacts = contacts.clone();
        server.tasks = tasks.clone();
        match server.repository.create_collection(Collection {
            id: contacts,
            name: "Contacts".into(),
        }) {
            Ok(()) | Err(any_cal_core::RepositoryError::CollectionAlreadyExists(_)) => {}
            Err(error) => return Err(ConfigError::Repository(error.to_string())),
        }
        match server.repository.create_collection(Collection {
            id: tasks,
            name: "Tasks".into(),
        }) {
            Ok(()) | Err(any_cal_core::RepositoryError::CollectionAlreadyExists(_)) => {}
            Err(error) => return Err(ConfigError::Repository(error.to_string())),
        }
        let sync = config
            .sync_checkpoint
            .as_deref()
            .map(SyncStore::open)
            .transpose()
            .map_err(|error| ConfigError::Io(error.to_string()))?;
        let audit = config
            .audit_directory
            .as_deref()
            .map(|directory| {
                AuditEventWriter::open(directory, config.audit_queue_capacity)
                    .map_err(|error| ConfigError::Io(classify_audit_error(error)))
            })
            .transpose()?;
        Ok(Self {
            server,
            config,
            transport_mode,
            upstream: if transport_mode == "fake" {
                UpstreamState::Ready
            } else {
                UpstreamState::NotTested
            },
            sync,
            events: EventBuffer::default(),
            health: Health::default(),
            audit,
            audit_previous_state: None,
            audit_recovering: false,
            next_request: 0,
            rate_window: Instant::now(),
            rate_count: 0,
            identity: None,
            identity_now: 0,
        })
    }
    fn authorization_status(&mut self, request: &any_cal_dav_server::Request) -> Option<u16> {
        if request.path == "/android/sync" {
            if self.identity.is_some() {
                return self.android_identity_authorization_status(request);
            }
            return (!self.authorized(request)).then_some(401);
        }
        if self.identity.is_some()
            && !matches!(request.path.as_str(), "/health" | "/status" | "/ready")
        {
            return self.identity_authorization_status(request);
        }
        (!self.authorized(request)).then_some(401)
    }

    fn android_identity_authorization_status(
        &mut self,
        request: &any_cal_dav_server::Request,
    ) -> Option<u16> {
        let identity = self.identity.as_ref().expect("identity checked above");
        let Some(token) = bearer_or_basic_credential(request) else {
            self.record_auth_event(request, "missing", None);
            return Some(401);
        };
        let Some((principal, AuthOutcome::Authenticated)) =
            identity.authenticate(token, self.identity_now)
        else {
            self.record_auth_event(request, "invalid", None);
            return Some(401);
        };
        let capabilities = identity.capabilities(token, self.identity_now);
        let account_capable = capabilities.contains(&Capability::ReadContacts)
            || capabilities.contains(&Capability::WriteContacts)
            || capabilities.contains(&Capability::ReadTasks)
            || capabilities.contains(&Capability::WriteTasks);
        if account_capable {
            self.record_auth_event(request, "authenticated", Some(&principal));
            None
        } else {
            self.record_auth_event(request, "forbidden", Some(&principal));
            Some(403)
        }
    }

    fn sync_export_path(&self) -> Option<PathBuf> {
        self.config
            .sync_checkpoint
            .as_deref()
            .map(Path::new)
            .map(|path| path.with_extension("export"))
    }

    fn admin_authorized(&self, request: &any_cal_dav_server::Request) -> bool {
        self.config
            .auth_credential
            .as_deref()
            .is_some_and(|expected| self.authorized_credential(request, expected))
    }

    fn sync_admin_error(
        status: u16,
        message: &'static str,
        correlation: Option<&Correlation>,
    ) -> any_cal_dav_server::Response {
        let mut headers = vec![("Content-Type".into(), "application/json".into())];
        no_store(&mut headers);
        if let Some(correlation) = correlation {
            add_correlation(&mut headers, correlation);
        }
        if status == 405 {
            headers.push(("Allow".into(), "GET, POST".into()));
        }
        any_cal_dav_server::Response {
            status,
            headers,
            body: format!("{{\"status\":\"error\",\"error\":\"{message}\"}}").into_bytes(),
        }
    }

    fn sync_admin_receipt(
        operation: &'static str,
        receipt: &any_cal_sync::ExportReceipt,
        correlation: &Correlation,
    ) -> any_cal_dav_server::Response {
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Cache-Control".into(), "no-store".into()),
        ];
        add_correlation(&mut headers, correlation);
        any_cal_dav_server::Response {
            status: 200,
            headers,
            body: format!(
                "{{\"status\":\"ok\",\"operation\":\"{operation}\",\"format\":\"{}\",\"export_version\":{},\"state_generation\":{},\"observed\":{},\"pending\":{},\"tombstones\":{}}}",
                receipt.format,
                receipt.export_version,
                receipt.state_generation,
                receipt.observed,
                receipt.pending,
                receipt.tombstones,
            )
            .into_bytes(),
        }
    }

    fn sync_admin_audit_readback(
        summaries: Vec<AuditSummary>,
        correlation: &Correlation,
    ) -> any_cal_dav_server::Response {
        let next_after = summaries.last().map(|summary| summary.sequence);
        let body = serde_json::json!({
            "status": "ok",
            "count": summaries.len(),
            "next_after": next_after,
            "events": summaries,
        });
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Cache-Control".into(), "no-store".into()),
        ];
        add_correlation(&mut headers, correlation);
        any_cal_dav_server::Response {
            status: 200,
            headers,
            body: serde_json::to_vec(&body).expect("audit readback response serializes"),
        }
    }

    fn parse_audit_readback_query(path: &str) -> Result<AuditReadbackQuery, &'static str> {
        let Some((_, query)) = path.split_once('?') else {
            return Ok(AuditReadbackQuery {
                after_sequence: None,
                limit: MAX_AUDIT_READBACK,
            });
        };
        let mut after_sequence = None;
        let mut limit = None;
        for component in query.split('&') {
            let Some((key, value)) = component.split_once('=') else {
                return Err("malformed_query");
            };
            if value.is_empty() {
                return Err("malformed_query");
            }
            match key {
                "after" if after_sequence.is_none() => {
                    after_sequence = Some(value.parse().map_err(|_| "malformed_query")?);
                }
                "limit" if limit.is_none() => {
                    limit = Some(value.parse().map_err(|_| "malformed_query")?);
                }
                _ => return Err("unsupported_query"),
            }
        }
        let query = AuditReadbackQuery {
            after_sequence,
            limit: limit.unwrap_or(MAX_AUDIT_READBACK),
        };
        query.validate().map_err(|_| "limit_out_of_bounds")?;
        Ok(query)
    }

    fn handle_sync_admin(
        &mut self,
        request: any_cal_dav_server::Request,
        correlation: Correlation,
    ) -> any_cal_dav_server::Response {
        if !self.admin_authorized(&request) {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Auth),
                "operation=denied",
            ));
            return Self::sync_admin_error(401, "unauthorized", Some(&correlation));
        }
        let (route, _) = request.path.split_once('?').unwrap_or((&request.path, ""));
        if route == "/admin/sync/audit" {
            if request.method != "GET" {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=audit_readback status=method_not_allowed",
                ));
                return Self::sync_admin_error(405, "method_not_allowed", Some(&correlation));
            }
            if !request.body.is_empty() {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=audit_readback status=request_body_not_allowed",
                ));
                return Self::sync_admin_error(400, "request_body_not_allowed", Some(&correlation));
            }
            let query = match Self::parse_audit_readback_query(&request.path) {
                Ok(query) => query,
                Err(error) => {
                    self.record_admin_audit(correlation.event(
                        "recovery.admin",
                        Some(ErrorCategory::Protocol),
                        &format!("operation=audit_readback status={error}"),
                    ));
                    return Self::sync_admin_error(400, error, Some(&correlation));
                }
            };
            let Some(audit) = self.audit.as_ref() else {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Recovery),
                    "operation=audit_readback status=audit_unavailable",
                ));
                return Self::sync_admin_error(503, "audit_unavailable", Some(&correlation));
            };
            match audit.readback(query) {
                Ok(summaries) => {
                    self.record_admin_audit(correlation.event(
                        "recovery.admin",
                        None,
                        "operation=audit_readback status=ok",
                    ));
                    return Self::sync_admin_audit_readback(summaries, &correlation);
                }
                Err(_) => {
                    self.record_admin_audit(correlation.event(
                        "recovery.admin",
                        Some(ErrorCategory::Recovery),
                        "operation=audit_readback status=failed",
                    ));
                    return Self::sync_admin_error(503, "audit_unavailable", Some(&correlation));
                }
            }
        }
        let operation = match request.path.as_str() {
            "/admin/sync" | "/admin/sync/capabilities" if request.method == "GET" => {
                let configured = self.sync.is_some();
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    None,
                    &format!("operation=capabilities status=ok configured={configured}"),
                ));
                let mut headers = vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("Cache-Control".into(), "no-store".into()),
                ];
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 200,
                    headers,
                    body: format!(
                        "{{\"status\":\"ok\",\"export\":{},\"restore\":{},\"format\":\"any-cal.sync-export\",\"export_version\":1}}",
                        configured, configured
                    )
                    .into_bytes(),
                };
            }
            "/admin/sync/export" if request.method == "POST" => "export",
            "/admin/sync/restore" if request.method == "POST" => "restore",
            "/admin/sync"
            | "/admin/sync/capabilities"
            | "/admin/sync/export"
            | "/admin/sync/restore" => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=admin status=method_not_allowed",
                ));
                return Self::sync_admin_error(405, "method_not_allowed", Some(&correlation));
            }
            _ => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=admin status=not_found",
                ));
                return Self::sync_admin_error(404, "not_found", Some(&correlation));
            }
        };
        if !request.body.is_empty() {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Protocol),
                &format!("operation={operation} status=request_body_not_allowed"),
            ));
            return Self::sync_admin_error(400, "request_body_not_allowed", Some(&correlation));
        }
        let Some(path) = self.sync_export_path() else {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Recovery),
                &format!("operation={operation} status=sync_checkpoint_unavailable"),
            ));
            return Self::sync_admin_error(503, "sync_checkpoint_unavailable", Some(&correlation));
        };
        let Some(sync) = self.sync.as_mut() else {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Recovery),
                &format!("operation={operation} status=sync_checkpoint_unavailable"),
            ));
            return Self::sync_admin_error(503, "sync_checkpoint_unavailable", Some(&correlation));
        };
        let result = if operation == "export" {
            sync.export_to(&path)
        } else {
            sync.restore_export(&path)
        };
        match result {
            Ok(receipt) => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    None,
                    &format!("operation={operation} status=ok"),
                ));
                Self::sync_admin_receipt(operation, &receipt, &correlation)
            }
            Err(_) => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Recovery),
                    &format!("operation={operation} status=failed"),
                ));
                Self::sync_admin_error(500, "sync_operation_failed", Some(&correlation))
            }
        }
    }

    /// Persist admin-operation events when the optional durable audit journal
    /// is configured. Admin responses retain their operation result even if
    /// the diagnostic sink is degraded; the writer health surface reports
    /// that failure separately.
    fn record_admin_audit(&mut self, event: Event) {
        self.events.push(event.clone());
        if let Some(audit) = self.audit.as_ref() {
            let _ = audit.append(event);
        }
    }

    fn identity_authorization_status(
        &mut self,
        request: &any_cal_dav_server::Request,
    ) -> Option<u16> {
        let identity = self.identity.as_ref().expect("identity checked above");
        let Some(token) = bearer_or_basic_credential(request) else {
            self.record_auth_event(request, "missing", None);
            return Some(401);
        };
        let Some((principal, outcome)) = identity.authenticate(token, self.identity_now) else {
            self.record_auth_event(request, "invalid", None);
            return Some(401);
        };
        if outcome != AuthOutcome::Authenticated {
            self.record_auth_event(request, auth_outcome_name(outcome), None);
            return Some(401);
        }
        let (collection, resource_id, operation) = self.request_scope(request)?;
        let decision =
            identity
                .policy
                .authorize(&principal, collection, resource_id.as_deref(), operation);
        let capability = capability_for(collection, operation);
        let capability_allowed = identity.credential_allows(token, self.identity_now, capability);
        match (decision, capability_allowed) {
            (AccessDecision::Allowed, true) => {
                self.record_auth_event(request, "authenticated", Some(&principal));
                None
            }
            (AccessDecision::Forbidden, _) | (AccessDecision::Allowed, false) => {
                self.record_auth_event(request, "forbidden", Some(&principal));
                Some(403)
            }
            // A resource-level denial is deliberately indistinguishable from
            // a missing resource, so resource existence cannot be enumerated.
            (AccessDecision::NotFound, _) => {
                self.record_auth_event(request, "not_found", Some(&principal));
                Some(404)
            }
        }
    }

    fn record_auth_event(
        &mut self,
        request: &any_cal_dav_server::Request,
        outcome: &str,
        principal: Option<&crate::identity::PrincipalId>,
    ) {
        if !self.config.emit_auth_events {
            return;
        }
        let request_id = request
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("x-request-id"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("auth");
        let scope = if request.path.starts_with("/carddav/") {
            "contacts"
        } else if request.path.starts_with("/caldav/") {
            "tasks"
        } else {
            "other"
        };
        // The principal is represented only as a bounded class. Resource
        // paths, credential IDs, and tokens never enter the event message.
        let principal_class = principal.map_or("unknown", |_| "known");
        self.events.push(
            Event::new(
                "dav.auth",
                (outcome != "authenticated").then_some(ErrorCategory::Auth),
                format!("outcome={outcome} scope={scope} principal_class={principal_class}"),
            )
            .request(request_id.to_owned()),
        );
    }

    fn request_scope(
        &self,
        request: &any_cal_dav_server::Request,
    ) -> Option<(CollectionKind, Option<String>, Operation)> {
        let (collection, base) = if request.path.starts_with("/carddav/") {
            (CollectionKind::Contacts, "/carddav/")
        } else if request.path.starts_with("/caldav/") {
            (CollectionKind::Tasks, "/caldav/")
        } else {
            return None;
        };
        let configured = match collection {
            CollectionKind::Contacts => self.config.contacts_collection.as_str(),
            CollectionKind::Tasks => self.config.tasks_collection.as_str(),
        };
        let remainder = request.path.strip_prefix(base)?;
        let remainder = remainder.strip_prefix(configured)?;
        let resource_id = remainder
            .strip_prefix('/')
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let operation = match request.method.as_str() {
            "PUT" | "DELETE" => Operation::Write,
            _ => Operation::Read,
        };
        Some((collection, resource_id, operation))
    }

    fn authorized(&self, request: &any_cal_dav_server::Request) -> bool {
        let is_health = matches!(request.path.as_str(), "/health" | "/status" | "/ready");
        if is_health {
            if let Some(local) = self.config.local_auth_credential.as_deref() {
                if self.authorized_credential(request, local) {
                    return true;
                }
                if self
                    .config
                    .auth_credential
                    .as_deref()
                    .is_none_or(|global| !self.authorized_credential(request, global))
                {
                    return false;
                }
            }
        }
        let Some(expected) = self.config.auth_credential.as_deref() else {
            return true;
        };
        self.authorized_credential(request, expected)
    }
    fn authorized_credential(&self, request: &any_cal_dav_server::Request, expected: &str) -> bool {
        let authorization: Vec<_> = request
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.as_str())
            .collect();
        let [value] = authorization.as_slice() else {
            return false;
        };
        let Some((scheme, supplied)) = value.trim_start().split_once(char::is_whitespace) else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("bearer") && !scheme.eq_ignore_ascii_case("basic") {
            return false;
        }
        let supplied = supplied.trim_start();
        constant_time_equal(supplied.as_bytes(), expected.as_bytes())
    }
    fn rate_limited(&mut self) -> bool {
        if self.rate_window.elapsed() >= Duration::from_secs(60) {
            self.rate_window = Instant::now();
            self.rate_count = 0;
        }
        self.rate_count = self.rate_count.saturating_add(1);
        self.rate_count > self.config.rate_limit_per_minute
    }
    fn checkpoint_visible_state(&mut self) -> Result<(), String> {
        let Some(_) = self.sync.as_ref() else {
            return Ok(());
        };
        let mut rows = Vec::new();
        for collection in [self.server.contacts.clone(), self.server.tasks.clone()] {
            rows.extend(
                self.server
                    .repository
                    .list_resources(&collection, false)
                    .or_else(|error| match error {
                        any_cal_core::RepositoryError::CollectionNotFound(_) => Ok(Vec::new()),
                        other => Err(other),
                    })
                    .map_err(|error| error.to_string())?,
            );
        }
        let items = rows
            .into_iter()
            .map(|row| ObservedResource {
                resource_id: row.envelope.resource_id.clone(),
                anytype_object_id: row.envelope.anytype_object_id.to_string(),
                dav_uid: row.envelope.dav_uid.to_string(),
                revision: row.envelope.revision,
                etag: row.etag.as_str().to_owned(),
                modified_at: row.modified_at.unix_seconds(),
                archived: row.archived,
            })
            .collect::<Vec<_>>();
        self.sync
            .as_mut()
            .expect("checked above")
            .replace_observed(items)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    pub fn record_reconciliation(&mut self, report: ReconciliationReport) {
        for event in report.events {
            self.events.push(event);
        }
        self.events.push(
            Event::new(
                "sync.reconciliation",
                None,
                format!(
                    "sync_id={} scanned={} changed={} omitted={} conflicts={}",
                    report.sync_id,
                    report.scanned,
                    report.changed,
                    report.omitted,
                    report.conflicts
                ),
            )
            .sync(report.sync_id),
        );
    }
    pub fn backup_artifact(&mut self, source: &Path, backup: &Path) -> io::Result<()> {
        let result = any_cal_observability::atomic_backup(source, backup);
        self.events.push(
            Event::new(
                "recovery.backup",
                result.as_ref().err().map(|_| ErrorCategory::Recovery),
                if result.is_ok() {
                    "backup complete"
                } else {
                    "backup failed"
                },
            )
            .error_code(any_cal_observability::ErrorCode::Recovery)
            .correlation_id("recovery-backup"),
        );
        result
    }
    pub fn restore_artifact(&mut self, backup: &Path, destination: &Path) -> io::Result<()> {
        let result = any_cal_observability::atomic_restore(backup, destination);
        self.events.push(
            Event::new(
                "recovery.restore",
                result.as_ref().err().map(|_| ErrorCategory::Recovery),
                if result.is_ok() {
                    "restore complete"
                } else {
                    "restore failed"
                },
            )
            .error_code(any_cal_observability::ErrorCode::Recovery)
            .correlation_id("recovery-restore"),
        );
        result
    }
    pub fn sync_state(&self) -> Option<&SyncState> {
        self.sync.as_ref().map(SyncStore::state)
    }
    pub fn inject_checkpoint_fault(&mut self, fault: CommitFault) {
        if let Some(sync) = self.sync.as_mut() {
            sync.inject_commit_fault(fault);
        }
    }

    fn probe_upstream(&mut self) {
        // The fake transport is deliberately local and deterministic. Marking
        // it ready makes the distinction explicit in diagnostics without
        // pretending that a network endpoint was contacted.
        if self.transport_mode == "fake" {
            self.upstream = UpstreamState::Ready;
            return;
        }
        let space_id = self.config.space_id.clone();
        self.upstream = match self
            .server
            .repository
            .transport
            .list_objects(&space_id, None)
        {
            Ok(_) => UpstreamState::Ready,
            Err(error) => UpstreamState::Unavailable(error.category()),
        };
    }

    fn upstream_json(&self) -> String {
        let configured =
            !self.config.endpoint.trim().is_empty() && !self.config.space_id.trim().is_empty();
        let error = self
            .upstream
            .error()
            .map_or_else(|| "null".into(), |category| format!("\"{category}\""));
        format!(
            "{{\"mode\":\"{}\",\"configured\":{},\"tested\":{},\"ready\":{},\"status\":\"{}\",\"error\":{}}}",
            self.transport_mode,
            configured,
            self.upstream.tested(),
            self.upstream.ready(),
            self.upstream.status(),
            error,
        )
    }

    fn audit_snapshot(&mut self) -> Option<AuditStatus> {
        let writer = self.audit.as_ref()?;
        let health = writer.health();
        let previous = self.audit_previous_state.replace(health.state.clone());
        if matches!(
            previous,
            Some(AuditHealthState::Degraded | AuditHealthState::Unavailable)
        ) && health.state == AuditHealthState::Healthy
        {
            self.audit_recovering = true;
        }
        let recovering = self.audit_recovering;
        self.audit_recovering = false;
        let state = if recovering {
            "recovering"
        } else {
            match health.state {
                AuditHealthState::Healthy => "healthy",
                AuditHealthState::Degraded => "degraded",
                AuditHealthState::Unavailable => "unavailable",
            }
        };
        let ready = matches!(health.state, AuditHealthState::Healthy)
            && health.running
            && health.queued <= health.queue_capacity;
        Some(AuditStatus {
            ready,
            state,
            json: format!(
                "{{\"enabled\":true,\"state\":\"{state}\",\"running\":{},\"queued\":{},\"queue_capacity\":{},\"accepted\":{},\"persisted\":{},\"failed\":{},\"checkpoint_ready\":{},\"export_ready\":{},\"last_failure\":{}}}",
                health.running,
                health.queued,
                health.queue_capacity,
                health.accepted,
                health.persisted,
                health.failed,
                ready,
                ready,
                health
                    .last_failure
                    .map_or_else(|| "null".into(), |failure| format!("\"{failure:?}\"")),
            ),
        })
    }
    pub fn check(&self) -> String {
        format!(
            "ok endpoint={} api_version={} space_configured={} listen_address={} token_configured={} transport={} upstream_tested={} auth_audit_enabled={} max_connections={}",
            redact_endpoint(&self.config.endpoint),
            self.config.api_version,
            !self.config.space_id.trim().is_empty(),
            self.config.listen_address,
            self.config.token.is_some(),
            self.transport_mode,
            self.upstream.tested(),
            self.config.emit_auth_events,
            self.config.max_connections
        )
    }
    /// Run the local-only HTTP listener. Each connection is parsed in a
    /// bounded worker so slow or malformed clients cannot block new clients.
    pub fn serve(&mut self) -> io::Result<()>
    where
        T: Send,
    {
        let listener = TcpListener::bind(&self.config.listen_address)?;
        self.serve_listener(listener)
    }

    fn serve_listener(&mut self, listener: TcpListener) -> io::Result<()>
    where
        T: Send,
    {
        self.serve_listener_with_limits(listener, self.config.max_connections, None)
    }

    #[cfg(test)]
    fn serve_listener_limit(
        &mut self,
        listener: TcpListener,
        connection_limit: Option<usize>,
    ) -> io::Result<()>
    where
        T: Send,
    {
        self.serve_listener_with_limits(listener, usize::MAX, connection_limit)
    }

    fn serve_listener_with_limits(
        &mut self,
        listener: TcpListener,
        max_connections: usize,
        accept_limit: Option<usize>,
    ) -> io::Result<()>
    where
        T: Send,
    {
        if max_connections == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "max_connections must be positive",
            ));
        }
        #[cfg(unix)]
        {
            listener.set_nonblocking(true)?;
            SHUTDOWN_REQUESTED.store(false, Ordering::Relaxed);
            // The handler only flips an atomic flag; all cleanup remains in
            // ordinary Rust control flow so SyncStore::Drop releases its lock.
            unsafe {
                libc::signal(
                    libc::SIGINT,
                    request_shutdown as *const () as libc::sighandler_t,
                );
                libc::signal(
                    libc::SIGTERM,
                    request_shutdown as *const () as libc::sighandler_t,
                );
            }
        }
        let state = std::sync::Mutex::new(self);
        let active = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = std::thread::scope(|scope| {
            let mut accepted = 0usize;
            loop {
                #[cfg(unix)]
                if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
                    break;
                }
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                accepted = accepted.saturating_add(1);
                let admitted = active
                    .fetch_update(
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Acquire,
                        |current| (current < max_connections).then_some(current + 1),
                    )
                    .is_ok();
                if !admitted {
                    let _ = write_http(
                        &mut stream,
                        protocol_error(503, "connection limit reached"),
                        false,
                    );
                    if accept_limit.is_some_and(|limit| accepted >= limit) {
                        break;
                    }
                    continue;
                }
                let state = &state;
                let active = std::sync::Arc::clone(&active);
                scope.spawn(move || {
                    let _ = serve_connection(&mut stream, state);
                    active.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                });
                if accept_limit.is_some_and(|limit| accepted >= limit) {
                    break;
                }
            }
            Ok(())
        });
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
        }
        result
    }
}

fn serve_connection<T: AnytypeTransport + Send>(
    stream: &mut TcpStream,
    state: &std::sync::Mutex<&mut AppGeneric<T>>,
) -> io::Result<()> {
    loop {
        let request = match read_http_request(stream) {
            Ok(request) => request,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                write_http(stream, protocol_error(408, "request timeout"), false)?;
                break;
            }
            Err(_) => {
                write_http(stream, protocol_error(400, "bad request"), false)?;
                break;
            }
        };
        let close = connection_requests_close(&request.headers);
        let response = {
            let mut app = state
                .lock()
                .map_err(|_| io::Error::other("application state lock poisoned"))?;
            app.handle(request)
        };
        write_http(stream, response, !close)?;
        if close {
            break;
        }
    }
    Ok(())
}

fn protocol_error(status: u16, message: &str) -> any_cal_dav_server::Response {
    any_cal_dav_server::Response {
        status,
        headers: vec![("Content-Type".into(), "text/plain".into())],
        body: message.as_bytes().to_vec(),
    }
}

fn health_headers() -> Vec<(String, String)> {
    vec![
        (
            "Content-Type".into(),
            "application/json; charset=utf-8".into(),
        ),
        (
            "Cache-Control".into(),
            "no-store, no-cache, max-age=0".into(),
        ),
        ("Pragma".into(), "no-cache".into()),
        ("X-Content-Type-Options".into(), "nosniff".into()),
    ]
}

fn no_store(headers: &mut Vec<(String, String)>) {
    if !headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("cache-control"))
    {
        headers.push(("Cache-Control".into(), "no-store".into()));
    }
}

fn add_correlation(headers: &mut Vec<(String, String)>, correlation: &Correlation) {
    let Some(request_id) = correlation.request_id.as_deref().and_then(safe_request_id) else {
        return;
    };
    headers.push(("X-Request-ID".into(), request_id));
}

fn safe_request_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 128 {
        return None;
    }
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
        .then(|| value.to_owned())
}

fn validate_android_batch(
    request: &BridgeRequest,
    collection: &CollectionId,
    expected_kind: &DavKind,
) -> Result<(), &'static str> {
    let mut resource_ids = BTreeSet::new();
    let mut anytype_ids = BTreeSet::<String>::new();
    let mut dav_uids = BTreeSet::<String>::new();
    for resource in &request.resources {
        if &resource.kind != expected_kind {
            return Err("bridge resource kind does not match the Android authority");
        }
        let logical_collection = match expected_kind {
            DavKind::Contact => resource.collection_id.as_str() == "contacts",
            DavKind::Event => {
                resource.collection_id.as_str() == "tasks"
                    || resource.collection_id.as_str() == "calendar"
            }
            _ => false,
        };
        if resource.collection_id != *collection && !logical_collection {
            return Err("bridge resource is outside the configured collection");
        }
        if !resource_ids.insert(resource.resource_id.clone())
            || !anytype_ids.insert(resource.anytype_object_id.to_string())
            || !dav_uids.insert(resource.dav_uid.to_string())
        {
            return Err("bridge resource identities must be unique");
        }
    }
    let mut tombstone_ids = BTreeSet::new();
    let mut tombstone_canonical_ids = BTreeSet::new();
    for tombstone in &request.tombstones {
        if tombstone.revision == 0 {
            return Err("bridge tombstone revision must be positive");
        }
        if !tombstone_ids.insert(tombstone.resource_id.clone())
            || !tombstone_canonical_ids.insert(tombstone.canonical_id.clone())
        {
            return Err("bridge tombstone identities must be unique");
        }
        if resource_ids.contains(&tombstone.resource_id)
            || anytype_ids.contains(&tombstone.canonical_id)
            || dav_uids.contains(&tombstone.canonical_id)
        {
            return Err("bridge resource and tombstone identities overlap");
        }
    }
    Ok(())
}

fn android_checkpoint(authority: &str, revision: u64) -> BridgeCheckpoint {
    BridgeCheckpoint {
        cursor: format!("android:{authority}:{revision}"),
        revision,
    }
}

fn android_repository_error(error: &RepositoryError) -> (u16, BridgeErrorCode, &'static str) {
    match error {
        RepositoryError::Auth => (401, BridgeErrorCode::PermissionDenied, "sync access denied"),
        RepositoryError::Forbidden => {
            (403, BridgeErrorCode::PermissionDenied, "sync access denied")
        }
        RepositoryError::IdentityAlreadyExists(_)
        | RepositoryError::ResourceAlreadyExists(_)
        | RepositoryError::PreconditionFailed { .. } => {
            (409, BridgeErrorCode::Conflict, "Anytype resource conflict")
        }
        RepositoryError::CollectionNotFound(_) => (
            503,
            BridgeErrorCode::NotLinked,
            "configured Anytype collection is unavailable",
        ),
        RepositoryError::ResourceNotFound(_) => (
            409,
            BridgeErrorCode::Conflict,
            "Anytype resource is unavailable",
        ),
        RepositoryError::InvalidEnvelope(_) => (
            400,
            BridgeErrorCode::InvalidRequest,
            "Anytype resource is invalid",
        ),
        RepositoryError::Timeout
        | RepositoryError::RateLimited
        | RepositoryError::Unavailable
        | RepositoryError::MalformedState
        | RepositoryError::ArchiveFailure
        | RepositoryError::ReadAfterWriteDelay
        | RepositoryError::CollectionAlreadyExists(_) => (
            503,
            BridgeErrorCode::TransportUnavailable,
            "Anytype service is unavailable",
        ),
    }
}

fn android_sync_response(
    status: u16,
    bridge: BridgeResponse,
    resources: Vec<ResourceEnvelope>,
    tombstones: Vec<AndroidSyncTombstone>,
    correlation: &Correlation,
) -> any_cal_dav_server::Response {
    let wire = AndroidSyncResponse {
        bridge,
        resources,
        tombstones,
    };
    let body = serde_json::to_vec(&wire).unwrap_or_else(|_| {
        b"{\"schema_version\":1,\"checkpoint\":null,\"decisions\":[],\"error\":{\"code\":\"transport_unavailable\",\"message\":\"bridge response failed\"},\"resources\":[],\"tombstones\":[]}".to_vec()
    });
    let mut headers = vec![("Content-Type".into(), "application/json".into())];
    no_store(&mut headers);
    add_correlation(&mut headers, correlation);
    any_cal_dav_server::Response {
        status,
        headers,
        body,
    }
}

fn android_sync_error(
    status: u16,
    checkpoint: Option<BridgeCheckpoint>,
    code: BridgeErrorCode,
    message: &'static str,
    correlation: &Correlation,
) -> any_cal_dav_server::Response {
    android_sync_response(
        status,
        BridgeResponse {
            schema_version: BRIDGE_SCHEMA_VERSION,
            checkpoint,
            decisions: Vec::new(),
            error: Some(BridgeError {
                code,
                message: message.into(),
            }),
        },
        Vec::new(),
        Vec::new(),
        correlation,
    )
}

fn connection_requests_close(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"))
    })
}

/// Keep operational diagnostics useful without echoing endpoint userinfo or
/// query/fragment material, which commonly contains credentials or signed
/// URLs.  Configuration still retains the original endpoint for transport.
fn redact_endpoint(endpoint: &str) -> String {
    let Some(scheme_end) = endpoint.find("://") else {
        return "[REDACTED_ENDPOINT]".into();
    };
    let prefix_end = scheme_end + 3;
    let rest = &endpoint[prefix_end..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let suffix = &rest[authority_end..];
    let path_end = suffix.find(['?', '#']).unwrap_or(suffix.len());
    format!(
        "{}{}{}",
        &endpoint[..prefix_end],
        authority,
        &suffix[..path_end]
    )
}
fn parse_bool(value: &str) -> Result<bool, ConfigError> {
    match value {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::Invalid("expected boolean".into())),
    }
}

fn bearer_or_basic_credential(request: &any_cal_dav_server::Request) -> Option<&str> {
    let mut values = request
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str());
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, supplied) = value.trim_start().split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") && !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let supplied = supplied.trim_start();
    (!supplied.is_empty()).then_some(supplied)
}

fn capability_for(collection: CollectionKind, operation: Operation) -> Capability {
    match (collection, operation) {
        (CollectionKind::Contacts, Operation::Read) => Capability::ReadContacts,
        (CollectionKind::Contacts, Operation::Write) => Capability::WriteContacts,
        (CollectionKind::Tasks, Operation::Read) => Capability::ReadTasks,
        (CollectionKind::Tasks, Operation::Write) => Capability::WriteTasks,
    }
}

fn filter_write_methods(response: &mut any_cal_dav_server::Response) {
    if let Some((_, allow)) = response
        .headers
        .iter_mut()
        .find(|(key, _)| key.eq_ignore_ascii_case("allow"))
    {
        let filtered = allow
            .split(',')
            .map(str::trim)
            .filter(|method| !matches!(*method, "PUT" | "DELETE"))
            .collect::<Vec<_>>()
            .join(", ");
        *allow = filtered;
    }
}

fn auth_outcome_name(outcome: AuthOutcome) -> &'static str {
    match outcome {
        AuthOutcome::Authenticated => "authenticated",
        AuthOutcome::Invalid => "invalid",
        AuthOutcome::NotYetValid => "not_yet_valid",
        AuthOutcome::Expired => "expired",
        AuthOutcome::Revoked => "revoked",
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut diff = (left.len() ^ right.len()) as u64;
    for index in 0..left.len().max(right.len()) {
        diff |= u64::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    diff == 0
}
fn error_name(error: &ErrorCategory) -> &'static str {
    match error {
        ErrorCategory::Config => "config",
        ErrorCategory::Transport => "transport",
        ErrorCategory::Auth => "auth",
        ErrorCategory::Protocol => "protocol",
        ErrorCategory::Anytype => "anytype",
        ErrorCategory::Timeout => "timeout",
        ErrorCategory::Conflict => "conflict",
        ErrorCategory::Recovery => "recovery",
        ErrorCategory::Unknown => "unknown",
    }
}
fn parse_http(raw: &[u8]) -> io::Result<any_cal_dav_server::Request> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed HTTP headers"))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP"))?;
    let mut lines = head.lines();
    let first = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut parts = first.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?;
    let path = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?;
    let mut headers = Vec::new();
    let mut length = None;
    for line in lines {
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed header"))?;
        if key.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate content length",
                ));
            }
            length = Some(value.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
            })?);
        }
        headers.push((key.into(), value.trim().into()));
    }
    let body = &raw[split + 4..];
    let length = length.unwrap_or(0);
    if body.len() != length {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated body",
        ));
    }
    Ok(any_cal_dav_server::Request {
        method: method.into(),
        path: path.into(),
        headers,
        body: body[..length].to_vec(),
    })
}
fn read_http_request(stream: &mut impl Read) -> io::Result<any_cal_dav_server::Request> {
    let mut bytes = Vec::new();
    let mut one = [0_u8; 1];
    let header_end = loop {
        if stream.read(&mut one)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated HTTP headers",
            ));
        }
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP headers too large",
            ));
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP headers"))?;
    let mut length = 0usize;
    let mut saw_length = false;
    for line in head.lines().skip(1) {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed header"))?;
        if key.eq_ignore_ascii_case("content-length") {
            if saw_length {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate content length",
                ));
            }
            saw_length = true;
            length = value.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
            })?;
        }
    }
    if length > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP body too large",
        ));
    }
    bytes.resize(header_end + length, 0);
    stream.read_exact(&mut bytes[header_end..])?;
    parse_http(&bytes)
}
fn write_http(
    stream: &mut impl Write,
    response: any_cal_dav_server::Response,
    keep_alive: bool,
) -> io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        302 => "Found",
        204 => "No Content",
        207 => "Multi-Status",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        412 => "Precondition Failed",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: {}\r\n",
        response.status,
        reason,
        response.body.len(),
        if keep_alive { "keep-alive" } else { "close" }
    )?;
    for (key, value) in response.headers {
        write!(stream, "{}: {}\r\n", key, value)?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&response.body)
}

pub fn parse_cli(
    args: impl IntoIterator<Item = String>,
    mut config: AppConfig,
) -> Result<(String, AppConfig), ConfigError> {
    let mut command = "check".into();
    let mut it = args.into_iter();
    let _ = it.next();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "serve" | "check" => command = arg,
            "--config" => {
                return Err(ConfigError::Invalid(
                    "--config must be loaded before CLI parsing".into(),
                ))
            }
            "--endpoint" => config.endpoint = it.next().ok_or(ConfigError::Missing("endpoint"))?,
            "--space-id" => config.space_id = it.next().ok_or(ConfigError::Missing("space_id"))?,
            "--listen" => {
                config.listen_address = it.next().ok_or(ConfigError::Missing("listen_address"))?
            }
            "--api-version" => {
                config.api_version = it.next().ok_or(ConfigError::Missing("api_version"))?
            }
            "--token" => config.token = Some(it.next().ok_or(ConfigError::Missing("token"))?),
            "--transport-mode" => {
                config.transport_mode = it.next().ok_or(ConfigError::Missing("transport_mode"))?
            }
            "--sync-checkpoint" => {
                config.sync_checkpoint =
                    Some(it.next().ok_or(ConfigError::Missing("sync_checkpoint"))?)
            }
            "--allow-lan" => config.allow_lan = true,
            "--reverse-proxy-tls" => config.reverse_proxy_tls = true,
            "--emit-auth-events" => config.emit_auth_events = true,
            "--no-emit-auth-events" => config.emit_auth_events = false,
            "--rate-limit-per-minute" => {
                config.rate_limit_per_minute = it
                    .next()
                    .ok_or(ConfigError::Missing("rate_limit_per_minute"))?
                    .parse()
                    .map_err(|_| ConfigError::Invalid("invalid rate_limit_per_minute".into()))?
            }
            "--max-connections" => {
                config.max_connections = it
                    .next()
                    .ok_or(ConfigError::Missing("max_connections"))?
                    .parse()
                    .map_err(|_| ConfigError::Invalid("invalid max_connections".into()))?
            }
            "--audit-directory" => {
                config.audit_directory =
                    Some(it.next().ok_or(ConfigError::Missing("audit_directory"))?)
            }
            "--audit-queue-capacity" => {
                config.audit_queue_capacity = it
                    .next()
                    .ok_or(ConfigError::Missing("audit_queue_capacity"))?
                    .parse()
                    .map_err(|_| ConfigError::Invalid("invalid audit_queue_capacity".into()))?
            }
            "--expose-audit-health" => config.expose_audit_health = true,
            "--no-expose-audit-health" => config.expose_audit_health = false,
            "--audit-required" => config.audit_required = true,
            "--no-audit-required" => config.audit_required = false,
            // Do not echo arbitrary command-line input in startup errors.
            // The unknown argument may contain a credential-like value.
            _ => return Err(ConfigError::Invalid("unknown command-line option".into())),
        }
    }
    Ok((command, config))
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpStream;
    use std::thread;

    fn test_app() -> App {
        App::fake({
            let mut config = AppConfig::defaults();
            config.space_id = "test-space".into();
            config
        })
        .unwrap()
    }

    fn read_response(stream: &mut TcpStream) -> String {
        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn app_reader_handles_two_framed_requests_without_eof() {
        let bytes = b"GET /health HTTP/1.1\r\nContent-Length: 0\r\n\r\nGET /status HTTP/1.1\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc";
        let mut stream = Cursor::new(bytes.as_slice());
        let mut app = App::fake({
            let mut config = AppConfig::defaults();
            config.space_id = "test-space".into();
            config
        })
        .unwrap();
        let first = read_http_request(&mut stream).unwrap();
        assert_eq!(first.path, "/health");
        let first_response = app.handle(first);
        let second = read_http_request(&mut stream).unwrap();
        assert_eq!(second.path, "/status");
        assert_eq!(second.body, b"abc");
        assert!(second
            .headers
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("connection")
                && value.eq_ignore_ascii_case("close")));
        let second_response = app.handle(second);
        let mut output = Vec::new();
        write_http(&mut output, first_response, true).unwrap();
        write_http(&mut output, second_response, false).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("HTTP/1.1 200 OK").count(), 2);
        assert!(output.contains("Connection: keep-alive"));
        assert!(output.contains("Connection: close"));
    }

    #[test]
    fn app_parser_rejects_duplicate_and_extra_body_bytes() {
        assert!(
            parse_http(b"GET / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n")
                .is_err()
        );
        assert!(parse_http(b"PUT / HTTP/1.1\r\nContent-Length: 1\r\n\r\nxy").is_err());
    }

    #[test]
    fn app_connection_tokens_honor_close_in_a_comma_list() {
        let headers = vec![("Connection".into(), "keep-alive, close".into())];
        assert!(connection_requests_close(&headers));
    }

    #[test]
    fn app_reason_phrases_cover_service_unavailable() {
        let mut output = Vec::new();
        write_http(
            &mut output,
            any_cal_dav_server::Response {
                status: 503,
                headers: vec![],
                body: vec![],
            },
            false,
        )
        .unwrap();
        assert!(String::from_utf8(output)
            .unwrap()
            .starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    }

    #[test]
    fn app_well_known_redirect_is_framed_as_found() {
        let mut app = test_app();
        let response = app.handle(any_cal_dav_server::Request {
            method: "GET".into(),
            path: "/.well-known/caldav".into(),
            headers: vec![
                ("Host".into(), "dav.example.test".into()),
                ("X-Forwarded-Proto".into(), "https".into()),
                ("Connection".into(), "close".into()),
            ],
            body: vec![],
        });
        assert_eq!(response.status, 302);
        assert_eq!(response.body.len(), 0);
        assert!(response
            .headers
            .iter()
            .any(|(key, value)| key == "Location" && value == "https://dav.example.test/caldav/"));

        let mut output = Vec::new();
        write_http(&mut output, response, false).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(output.contains("Content-Length: 0\r\n"));
        assert!(output.contains("Connection: close\r\n"));
        assert!(!output.contains("Internal Server Error"));

        let response = app.handle(any_cal_dav_server::Request {
            method: "PROPFIND".into(),
            path: "/.well-known/carddav".into(),
            headers: vec![
                ("Host".into(), "dav.example.test".into()),
                ("X-Forwarded-Proto".into(), "https".into()),
                ("Connection".into(), "close".into()),
            ],
            body: vec![],
        });
        assert_eq!(response.status, 302);
        assert!(response
            .headers
            .iter()
            .any(|(key, value)| key == "Location" && value == "https://dav.example.test/carddav/"));
    }

    #[test]
    fn malformed_request_gets_400_and_next_connection_is_served() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut app = test_app();
            app.serve_listener_limit(listener, Some(2))
        });

        let mut malformed = TcpStream::connect(address).unwrap();
        malformed
            .write_all(b"GET /health HTTP/1.1\r\nBroken\r\n\r\n")
            .unwrap();
        malformed
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let bad_response = read_response(&mut malformed);
        assert!(bad_response.starts_with("HTTP/1.1 400 Bad Request\r\n"));

        let mut valid = TcpStream::connect(address).unwrap();
        valid
            .write_all(b"GET /health HTTP/1.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        valid
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let good_response = read_response(&mut valid);
        assert!(good_response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(server.join().unwrap().is_ok());
    }

    #[test]
    fn slow_partial_client_does_not_block_concurrent_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut app = test_app();
            app.serve_listener_limit(listener, Some(2))
        });

        let mut slow = TcpStream::connect(address).unwrap();
        slow.write_all(b"GET /health HTTP/1.1\r\nHost: slow\r\n")
            .unwrap();

        let mut valid = TcpStream::connect(address).unwrap();
        valid
            .write_all(b"GET /health HTTP/1.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        valid
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let response = read_response(&mut valid);
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));

        drop(slow);
        assert!(server.join().unwrap().is_ok());
    }

    #[test]
    fn production_connection_limit_rejects_excess_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut app = test_app();
            app.serve_listener_with_limits(listener, 1, Some(2))
        });

        let mut slow = TcpStream::connect(address).unwrap();
        slow.write_all(b"GET /health HTTP/1.1\r\nHost: slow\r\n")
            .unwrap();

        let mut rejected = TcpStream::connect(address).unwrap();
        rejected
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let response = read_response(&mut rejected);
        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));

        drop(slow);
        assert!(server.join().unwrap().is_ok());
    }
}
