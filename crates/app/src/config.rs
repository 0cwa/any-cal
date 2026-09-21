use any_cal_core::{CollectionId, DomainBindings};
use any_cal_sync::SyncScope;
use sha2::{Digest, Sha256};
use std::fs;
use std::net::ToSocketAddrs;
use std::path::Path;

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
    pub fn domain_bindings(&self) -> Result<DomainBindings, ConfigError> {
        self.validate()?;
        DomainBindings::legacy_single_space(
            self.space_id.clone(),
            &self.contacts_collection,
            &self.tasks_collection,
        )
        .map_err(|_| ConfigError::Invalid("invalid domain binding configuration".into()))
    }

    pub fn sync_scope(&self, transport_mode: &str) -> Result<SyncScope, ConfigError> {
        if transport_mode.trim().is_empty() || transport_mode.chars().any(char::is_control) {
            return Err(ConfigError::Invalid("invalid transport identity".into()));
        }
        let bindings = self.domain_bindings()?;
        let [binding] = bindings.bindings.as_slice() else {
            return Err(ConfigError::Invalid(
                "runtime requires exactly one domain binding".into(),
            ));
        };
        let account_fingerprint = self.account_context_fingerprint(transport_mode);
        SyncScope::for_binding(binding, &self.endpoint, &account_fingerprint)
            .map_err(|_| ConfigError::Invalid("invalid sync binding scope".into()))
    }

    fn account_context_fingerprint(&self, transport_mode: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"any-cal-upstream-account-context-v1");
        if let Some(token) = self.token.as_deref() {
            hasher.update(b"\0token\0");
            hasher.update(token.as_bytes());
        } else {
            hasher.update(b"\0transport\0");
            hasher.update(transport_mode.as_bytes());
        }
        let digest = hasher.finalize();
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut fingerprint = String::with_capacity(digest.len() * 2);
        for byte in digest {
            fingerprint.push(HEX[(byte >> 4) as usize] as char);
            fingerprint.push(HEX[(byte & 0x0f) as usize] as char);
        }
        fingerprint
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

fn parse_bool(value: &str) -> Result<bool, ConfigError> {
    match value {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::Invalid("expected boolean".into())),
    }
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
