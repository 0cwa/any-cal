use any_cal_anytype_adapter::{AnytypeTransport, FakeAnytypeTransport};
use any_cal_core::{
    BridgeCheckpoint, BridgeDecision, BridgeError, BridgeErrorCode, BridgeRequest, BridgeResponse,
    BridgeTombstone, Collection, CollectionId, DavKind, DavRoute, DomainBinding, DomainCollection,
    MemoryRepository, Repository, RepositoryError, ResourceEnvelope, SyncDecision, WriteCondition,
    BRIDGE_SCHEMA_VERSION,
};
use any_cal_observability::{
    AuditEventWriter, AuditHealthState, Correlation, ErrorCategory, Event, EventBuffer, Health,
    ReconciliationReport,
};
use any_cal_sync::{CommitFault, ObservedResource, SyncState, SyncStore};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

mod admin;
pub mod composition;
pub mod composition_orchestration;
pub mod person_context;
mod config;
pub mod discovery;
mod domain_registry;
mod http;
pub mod identity;

pub use config::{parse_cli, AppConfig, ConfigError};

use crate::domain_registry::{BindingSuspension, DomainRepositoryRegistry, UpstreamCapability};
use crate::http::{connection_requests_close, read_http_request, write_http};
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

fn binding_collection_ids(
    binding: &DomainBinding,
) -> Result<(CollectionId, CollectionId), ConfigError> {
    let mut contacts = CollectionId::try_from("contacts").expect("default contacts id is valid");
    let mut tasks = CollectionId::try_from("tasks").expect("default tasks id is valid");
    for route in &binding.routes {
        let collection = route
            .path
            .rsplit('/')
            .next()
            .ok_or_else(|| ConfigError::Invalid("DAV route has no collection id".into()))?;
        match route.collection {
            DomainCollection::Contacts => {
                contacts = CollectionId::try_from(collection)
                    .map_err(|_| ConfigError::Invalid("invalid contacts route".into()))?;
            }
            DomainCollection::Tasks => {
                tasks = CollectionId::try_from(collection)
                    .map_err(|_| ConfigError::Invalid("invalid tasks route".into()))?;
            }
            DomainCollection::Events => {
                return Err(ConfigError::Invalid(
                    "event routes are not supported by the current DAV runtime".into(),
                ))
            }
        }
    }
    Ok((contacts, tasks))
}

fn ensure_collection(
    repository: &mut MemoryRepository,
    id: CollectionId,
    name: &str,
) -> Result<(), ConfigError> {
    match repository.create_collection(Collection {
        id,
        name: name.into(),
    }) {
        Ok(()) | Err(RepositoryError::CollectionAlreadyExists(_)) => Ok(()),
        Err(error) => Err(ConfigError::Repository(error.to_string())),
    }
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
    transport_mode: &'static str,
    registry: DomainRepositoryRegistry<T>,
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
    discovery_snapshots:
        BTreeMap<crate::discovery::DiscoverySnapshotKey, crate::discovery::SpaceDiscoverySnapshot>,
}
impl<T: AnytypeTransport> AppGeneric<T> {
    /// Dispatch directly to the configured primary DAV context. This is a
    /// protocol-test seam; normal application routing should use [`Self::handle`].
    pub fn handle_primary_dav(
        &mut self,
        request: any_cal_dav_server::Request,
    ) -> any_cal_dav_server::Response {
        self.registry.primary_mut().server.handle(request)
    }

    /// Return a snapshot of the underlying transport for deterministic tests
    /// and diagnostics without exposing a mutable process-global active domain.
    pub fn transport_snapshot(&self) -> T
    where
        T: Clone,
    {
        self.registry.transport_snapshot()
    }

    /// Execute a bounded operation against the shared transport. Domain
    /// selection still happens through repository contexts; this seam exists
    /// for transport fault injection and diagnostics.
    pub fn with_transport_mut<R>(&self, operation: impl FnOnce(&mut T) -> R) -> R {
        self.registry.with_transport_mut(operation)
    }

    /// Inspect repository metrics for one configured domain.
    pub fn repository_metrics(
        &self,
        domain_id: &str,
    ) -> Option<&any_cal_anytype_adapter::OperationMetrics> {
        self.registry
            .context(domain_id)
            .map(|context| &context.server.repository.metrics)
    }

    /// Explicit domain-qualified repository mutation seam used by recovery
    /// and isolation tests. The domain must already exist in configuration.
    pub fn delete_resource_in_domain(
        &mut self,
        domain_id: &str,
        resource_id: &any_cal_core::ResourceId,
        condition: WriteCondition,
    ) -> Option<Result<any_cal_core::StoredResource, RepositoryError>> {
        self.registry.context_mut(domain_id).map(|context| {
            context
                .server
                .repository
                .delete_resource(resource_id, condition)
        })
    }

    /// Return the immutable identity that capability discovery and
    /// reauthorization must match before changing a domain's upstream state.
    pub fn upstream_binding_identity(&self, domain_id: &str) -> Option<(String, String, String)> {
        self.registry
            .binding_identity(domain_id)
            .map(|(binding, account, space)| {
                (binding.to_owned(), account.to_owned(), space.to_owned())
            })
    }

    /// Install capability discovery results only when they belong to the
    /// exact configured binding fingerprint.
    pub fn configure_upstream_capabilities(
        &mut self,
        domain_id: &str,
        binding_fingerprint: &str,
        read: bool,
        write: bool,
    ) -> bool {
        self.registry
            .configure_upstream_capabilities(domain_id, binding_fingerprint, read, write)
    }

    /// Clear an authorization suspension only after the exact configured
    /// upstream account context and Space have been revalidated.
    pub fn reauthorize_upstream_domain(
        &mut self,
        domain_id: &str,
        upstream_account_fingerprint: &str,
        space_id: &str,
    ) -> bool {
        self.registry
            .reauthorize(domain_id, upstream_account_fingerprint, space_id)
    }

    /// Bounded diagnostic view; no token, endpoint, or raw credential data is exposed.
    pub fn upstream_binding_status(
        &self,
        domain_id: &str,
    ) -> Option<(&'static str, &'static str, &'static str)> {
        let state = self.registry.upstream_state(domain_id)?;
        let capability = |value| match value {
            UpstreamCapability::Unknown => "unknown",
            UpstreamCapability::Allowed => "allowed",
            UpstreamCapability::Denied => "denied",
        };
        let suspension = match state.suspension {
            BindingSuspension::Active => "active",
            BindingSuspension::Auth => "auth",
            BindingSuspension::Forbidden => "forbidden",
        };
        Some((capability(state.read), capability(state.write), suspension))
    }

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
            if self.registry.len() > 1 {
                let mut headers = vec![("Content-Type".into(), "application/json".into())];
                no_store(&mut headers);
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 409,
                    headers,
                    body: b"{\"status\":\"error\",\"error\":\"domain_required\"}".to_vec(),
                };
            }
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
            let schema_json = self.schema_health_json();
            let space_configured = !self.registry.is_empty();
            let contacts_configured = self.has_collection_route(DomainCollection::Contacts);
            let tasks_configured = self.has_collection_route(DomainCollection::Tasks);
            let body = format!("{{\"status\":\"{service_state}\",\"ready\":{service_ready},\"space_configured\":{},\"contacts_collection_configured\":{},\"tasks_collection_configured\":{},\"transport\":\"{}\",\"upstream\":{},\"schema\":{},\"cache\":\"rebuildable\",\"events\":{},\"failures\":{},\"last_error\":{},\"recovery\":\"sync-checkpoint\"{}{}{} }}", space_configured, contacts_configured, tasks_configured, self.transport_mode, upstream_json, schema_json, self.health.counters.events, self.health.counters.failures, last_error, sync_export_json, separator, audit_json);
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
        let mut options_write_allowed = options_request
            .then(|| self.options_write_allowed(&request))
            .flatten();
        let durable = matches!(method.as_str(), "PUT" | "DELETE");
        let route_domain = self
            .resolve_request_route(&request.path)
            .map(|(domain_id, _)| domain_id.to_owned());
        if options_request
            && route_domain.as_deref().is_some_and(|domain_id| {
                self.registry.upstream_preflight(domain_id, true).is_some()
            })
        {
            options_write_allowed = Some(false);
        }
        let collection_home = matches!(request.path.as_str(), "/carddav/" | "/caldav/");
        if (request.path.starts_with("/carddav/") || request.path.starts_with("/caldav/"))
            && !collection_home
            && route_domain.is_none()
        {
            return any_cal_dav_server::Response {
                status: 404,
                headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
                body: b"not found".to_vec(),
            };
        }
        let mut upstream_observation = None;
        let mut response = if let Some(domain_id) = route_domain.as_deref() {
            if let Some(status) = self.registry.upstream_preflight(domain_id, durable) {
                any_cal_dav_server::Response {
                    status,
                    headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
                    body: if status == 401 {
                        b"upstream authorization required".to_vec()
                    } else {
                        b"upstream access denied".to_vec()
                    },
                }
            } else {
                let Some(context) = self.registry.context_mut(domain_id) else {
                    return any_cal_dav_server::Response {
                        status: 500,
                        headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
                        body: b"binding context unavailable".to_vec(),
                    };
                };
                let response = context.server.handle(request);
                upstream_observation = Some((domain_id.to_owned(), durable, response.status));
                response
            }
        } else {
            self.registry.primary_mut().server.handle(request)
        };
        if let Some((domain_id, write, status)) = upstream_observation {
            self.registry
                .observe_upstream_response(&domain_id, write, status);
        }
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
            "com.android.contacts" => Some((
                self.registry.primary().server.contacts.clone(),
                DavKind::Contact,
            )),
            // CalendarContract has no separate Anytype collection in the
            // current configuration. Events share the configured CalDAV
            // collection with VTODOs and are filtered by kind here.
            "com.android.calendar" => {
                Some((self.registry.primary().server.tasks.clone(), DavKind::Event))
            }
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
        let rows = match self
            .registry
            .primary_mut()
            .server
            .repository
            .list_resources(&collection, true)
        {
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
        let rows = match self
            .registry
            .primary_mut()
            .server
            .repository
            .list_resources(&collection, true)
        {
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
                    .registry
                    .primary_mut()
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
                    .registry
                    .primary_mut()
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
                let stored = match self
                    .registry
                    .primary_mut()
                    .server
                    .repository
                    .archive_resource(
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
        let (domain_id, collection, resource_id, _) = self.request_scope(request)?;
        Some(
            identity.policy.authorize_domain(
                &principal,
                &domain_id,
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
        let bindings = config.domain_bindings()?;
        let active_binding = bindings
            .bindings
            .first()
            .cloned()
            .ok_or_else(|| ConfigError::Invalid("no domain bindings configured".into()))?;
        let registry =
            DomainRepositoryRegistry::new(&config, &bindings, transport, transport_mode)?;

        let sync = if let Some(path) = config.sync_checkpoint.as_deref() {
            let scope = config.sync_scope_for(&active_binding, transport_mode)?;
            Some(
                SyncStore::open_scoped(path, scope)
                    .map_err(|error| ConfigError::Io(error.to_string()))?,
            )
        } else {
            None
        };
        let audit = config
            .audit_directory
            .as_deref()
            .map(|directory| {
                AuditEventWriter::open(directory, config.audit_queue_capacity)
                    .map_err(|error| ConfigError::Io(classify_audit_error(error)))
            })
            .transpose()?;
        Ok(Self {
            config,
            transport_mode,
            registry,
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
            discovery_snapshots: BTreeMap::new(),
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
        let capabilities = identity.capabilities_for_domain(
            token,
            self.identity_now,
            self.registry.primary_domain_id(),
        );
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
        if matches!(
            request.path.as_str(),
            "/" | "/principals/users/default" | "/.well-known/caldav" | "/.well-known/carddav"
        ) {
            self.record_auth_event(request, "authenticated", Some(&principal));
            return None;
        }
        if matches!(request.path.as_str(), "/carddav/" | "/caldav/") && self.registry.len() > 1 {
            self.record_auth_event(request, "forbidden", Some(&principal));
            return Some(403);
        }
        let Some((domain_id, collection, resource_id, operation)) = self.request_scope(request)
        else {
            self.record_auth_event(request, "not_found", Some(&principal));
            return Some(404);
        };
        let decision = identity.policy.authorize_domain(
            &principal,
            &domain_id,
            collection,
            resource_id.as_deref(),
            operation,
        );
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
    ) -> Option<(String, CollectionKind, Option<String>, Operation)> {
        let operation = match request.method.as_str() {
            "PUT" | "DELETE" => Operation::Write,
            _ => Operation::Read,
        };
        if matches!(request.path.as_str(), "/carddav/" | "/caldav/") {
            if self.registry.len() != 1 {
                return None;
            }
            let collection = if request.path == "/carddav/" {
                DomainCollection::Contacts
            } else {
                DomainCollection::Tasks
            };
            let context = self.registry.primary();
            if !context
                .binding
                .routes
                .iter()
                .any(|route| route.collection == collection)
            {
                return None;
            }
            return Some((
                context.binding.domain_id.clone(),
                match collection {
                    DomainCollection::Contacts => CollectionKind::Contacts,
                    DomainCollection::Tasks => CollectionKind::Tasks,
                    DomainCollection::Events => return None,
                },
                None,
                operation,
            ));
        }
        let (domain_id, route) = self.resolve_request_route(&request.path)?;
        let collection = match route.collection {
            DomainCollection::Contacts => CollectionKind::Contacts,
            DomainCollection::Tasks => CollectionKind::Tasks,
            DomainCollection::Events => return None,
        };
        let remainder = request.path.strip_prefix(&route.path)?;
        let resource_id = remainder
            .strip_prefix('/')
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        Some((domain_id.to_owned(), collection, resource_id, operation))
    }

    fn resolve_request_route(&self, path: &str) -> Option<(&str, &DavRoute)> {
        self.registry
            .resolve_route(path)
            .map(|(context, route)| (context.binding.domain_id.as_str(), route))
    }

    fn has_collection_route(&self, collection: DomainCollection) -> bool {
        self.registry.has_collection_route(collection)
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
        let primary = self.registry.primary_mut();
        for collection in [
            primary.server.contacts.clone(),
            primary.server.tasks.clone(),
        ] {
            rows.extend(
                primary
                    .server
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
        let spaces = self.registry.space_ids();
        for space_id in spaces {
            let result = self
                .registry
                .with_transport_mut(|transport| transport.list_objects(&space_id, None));
            if let Err(error) = result {
                self.upstream = UpstreamState::Unavailable(error.category());
                return;
            }
        }
        self.upstream = UpstreamState::Ready;
    }

    fn upstream_json(&self) -> String {
        let configured = !self.config.endpoint.trim().is_empty() && !self.registry.is_empty();
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
            !self.registry.is_empty(),
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
                // BSD/macOS accept inherits the nonblocking listener flag;
                // our per-connection workers require blocking I/O with timeouts.
                stream.set_nonblocking(false)?;
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
        (CollectionKind::Composition, Operation::Read) => Capability::ReadComposition,
        (CollectionKind::Composition, Operation::Write) => Capability::WriteComposition,
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

#[cfg(test)]
mod framing_tests {
    use super::*;
    use crate::http::parse_http;
    use std::io::{Cursor, Read, Write};
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

    fn read_response_headers(stream: &mut TcpStream) -> String {
        let mut output = Vec::new();
        let mut chunk = [0u8; 512];
        while !output.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "connection closed before response headers");
            output.extend_from_slice(&chunk[..read]);
        }
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
            app.serve_listener_with_limits(listener, 1, Some(3))
        });

        // Keep the admitted connection alive while the two excess clients
        // are accepted.  Sending requests on connections that are rejected
        // before request parsing leaves unread bytes in their receive buffers;
        // macOS resets those sockets when the server closes them, so this
        // sequencing tests the 503 response without depending on that
        // platform-specific close behavior.
        let mut admitted = TcpStream::connect(address).unwrap();
        admitted
            .write_all(
                b"GET /health HTTP/1.1\r\nHost: limit-test\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n",
            )
            .unwrap();
        admitted
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let admitted_response = read_response_headers(&mut admitted);
        assert!(admitted_response.starts_with("HTTP/1.1 200 OK\r\n"));

        let mut rejected_responses = Vec::new();
        for _ in 0..2 {
            let mut rejected = TcpStream::connect(address).unwrap();
            rejected
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            rejected_responses.push(read_response_headers(&mut rejected));
        }
        assert_eq!(
            rejected_responses
                .iter()
                .filter(|response| { response.starts_with("HTTP/1.1 503 Service Unavailable\r\n") })
                .count(),
            2,
            "unexpected overload responses: {rejected_responses:?}"
        );

        drop(admitted);
        assert!(server.join().unwrap().is_ok());
    }
}
