use any_cal_app::AppConfig;
use std::fs;
use std::path::Path;

slint::slint! {
    import { LineEdit, Button } from "std-widgets.slint";
    export component ConfigWindow inherits Window {
        in property <string> status_text;
        in-out property <string> endpoint;
        in-out property <string> api_version;
        in-out property <string> space_id;
        in-out property <string> contacts_collection;
        in-out property <string> tasks_collection;
        in-out property <string> listen_address;
        in-out property <string> token;
        in-out property <string> local_auth;
        callback health_clicked();
        width: 520px;
        height: 360px;
        VerticalLayout {
            padding: 20px;
            spacing: 8px;
            Text { text: "Any-Cal service"; font-size: 22px; }
            LineEdit { text <=> endpoint; accessible-label: "Anytype endpoint"; }
            LineEdit { text <=> api_version; accessible-label: "API version"; }
            LineEdit { text <=> space_id; accessible-label: "Space ID"; }
            LineEdit { text <=> contacts_collection; accessible-label: "Contacts collection"; }
            LineEdit { text <=> tasks_collection; accessible-label: "Tasks collection"; }
            LineEdit { text <=> listen_address; accessible-label: "Listen address"; }
            LineEdit { text <=> token; input-type: password; accessible-label: "Anytype token"; }
            LineEdit { text <=> local_auth; input-type: password; accessible-label: "Local health credential (runtime only)"; }
            Button { text: "Check health"; clicked => { root.health_clicked(); } }
            Text { text: status_text; }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigViewModel {
    pub config: AppConfig,
    pub token_masked: bool,
    pub status: String,
    pub service_status: String,
    pub transport_status: String,
    pub cache_status: String,
}

pub trait HealthClient {
    fn check(&self, endpoint: &str) -> Result<String, String>;

    fn check_with_auth(&self, endpoint: &str, _credential: Option<&str>) -> Result<String, String> {
        self.check(endpoint)
    }
}

pub struct LocalHealthClient;
impl HealthClient for LocalHealthClient {
    fn check(&self, endpoint: &str) -> Result<String, String> {
        self.check_with_auth(endpoint, None)
    }

    fn check_with_auth(&self, endpoint: &str, credential: Option<&str>) -> Result<String, String> {
        let address = health_address(endpoint)?;
        let mut stream = std::net::TcpStream::connect(address).map_err(|e| e.to_string())?;
        use std::io::{Read, Write};
        let authorization = credential
            .filter(|value| !value.is_empty())
            .map(|value| format!("Authorization: Bearer {value}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "GET /health HTTP/1.1\r\nHost: {}\r\n{}Connection: close\r\n\r\n",
            address, authorization
        )
        .map_err(|e| e.to_string())?;
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|e| e.to_string())?;
        if !response.starts_with("HTTP/1.1 200") {
            return Err(format!(
                "health request failed: {}",
                response.lines().next().unwrap_or("unknown response")
            ));
        }
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .ok_or("malformed health response".into())
    }
}

fn health_address(endpoint: &str) -> Result<&str, &'static str> {
    if let Some(address) = endpoint.strip_prefix("http://") {
        return Ok(address);
    }
    if endpoint.contains("://") {
        return Err("only http local health endpoints are supported");
    }
    if endpoint.is_empty() {
        return Err("health listen address is empty");
    }
    Ok(endpoint)
}

impl ConfigViewModel {
    pub fn set_field(&mut self, name: &str, value: String) {
        match name {
            "endpoint" => self.config.endpoint = value,
            "api_version" => self.config.api_version = value,
            "space_id" => self.config.space_id = value,
            "contacts_collection" => self.config.contacts_collection = value,
            "tasks_collection" => self.config.tasks_collection = value,
            "listen_address" => self.config.listen_address = value,
            "token" => {
                self.config.token = Some(value);
                self.token_masked = true;
            }
            "local_auth" => {
                if !value.is_empty() {
                    self.config.local_auth_credential = Some(value);
                }
            }
            _ => self.status = format!("Unknown field: {name}"),
        }
    }
}

impl ConfigViewModel {
    pub fn new(config: AppConfig) -> Self {
        Self {
            token_masked: config.token.is_some(),
            config,
            status: "Not checked".into(),
            service_status: "unknown".into(),
            transport_status: "unknown".into(),
            cache_status: "unknown".into(),
        }
    }
    pub fn validate(&mut self) -> bool {
        match self.config.validate() {
            Ok(()) => {
                self.status = "Configuration is valid".into();
                true
            }
            Err(error) => {
                self.status = format!("Configuration error: {error:?}");
                false
            }
        }
    }
    pub fn health_summary(&mut self) -> String {
        self.status = format!(
            "Configured for {} (token {})",
            self.config.space_id,
            if self.token_masked { "set" } else { "not set" }
        );
        self.status.clone()
    }
    pub fn health<C: HealthClient>(&mut self, client: &C) -> String {
        self.status = match client.check_with_auth(
            &self.config.listen_address,
            self.config.local_auth_credential.as_deref(),
        ) {
            Ok(body) => {
                self.service_status =
                    json_value(&body, "status").unwrap_or_else(|| "unknown".into());
                self.transport_status =
                    json_value(&body, "transport").unwrap_or_else(|| "unknown".into());
                self.cache_status = json_value(&body, "cache").unwrap_or_else(|| "unknown".into());
                let label = match self.service_status.as_str() {
                    "healthy" | "ready" | "ok" => "Health OK",
                    "not_tested" => "Health pending upstream probe",
                    "unavailable" => "Health unavailable",
                    _ => "Health reported",
                };
                format!(
                    "{label}: status={}, transport={}, cache={}",
                    self.service_status, self.transport_status, self.cache_status
                )
            }
            Err(error) => {
                self.service_status = "error".into();
                self.transport_status = "error".into();
                format!("Health error: {error}")
            }
        };
        self.status.clone()
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if self.config.token.is_some() {
            return Err(
                "refusing to persist Anytype token: GUI secret-store support is unavailable; use ANY_CAL_ANYTYPE_TOKEN or the CLI"
                    .into(),
            );
        }
        let data = format!(
            "endpoint={}\napi_version={}\nspace_id={}\ncontacts_collection={}\ntasks_collection={}\nlisten_address={}\n",
            self.config.endpoint,
            self.config.api_version,
            self.config.space_id,
            self.config.contacts_collection,
            self.config.tasks_collection,
            self.config.listen_address,
        );
        if path.exists()
            && fs::symlink_metadata(path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("refusing to overwrite symlink".into());
        }
        let parent = path.parent().ok_or("config path has no parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = parent.join(format!(".any-cal-config-{}.tmp", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        use std::io::Write;
        file.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| {
            let _ = fs::remove_file(&temp);
            e.to_string()
        })?;
        Ok(())
    }
}

fn json_value(body: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = body.find(&marker)? + marker.len();
    Some(body[start..].split('"').next()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_and_status_never_expose_token() {
        let mut config = AppConfig::defaults();
        config.space_id = "space".into();
        config.token = Some("secret-value".into());
        let mut model = ConfigViewModel::new(config);
        assert!(model.validate());
        let status = model.health_summary();
        assert!(status.contains("token set"));
        assert!(!status.contains("secret-value"));
    }
    #[test]
    fn invalid_config_is_presented_as_status() {
        let mut model = ConfigViewModel::new(AppConfig::defaults());
        assert!(!model.validate());
        assert!(model.status.contains("space_id"));
    }

    struct FakeHealth(Result<String, String>);
    impl HealthClient for FakeHealth {
        fn check(&self, _: &str) -> Result<String, String> {
            self.0.clone()
        }
    }
    #[test]
    fn injected_health_success_and_failure_are_visible() {
        let mut config = AppConfig::defaults();
        config.space_id = "space".into();
        let mut model = ConfigViewModel::new(config);
        assert!(model
            .health(&FakeHealth(Ok("{\"status\":\"ok\"}".into())))
            .contains("Health OK"));
        assert!(model
            .health(&FakeHealth(Ok("{\"status\":\"not_tested\"}".into())))
            .contains("Health pending upstream probe"));
        assert!(model
            .health(&FakeHealth(Err("offline".into())))
            .contains("Health error: offline"));
    }
    #[test]
    fn health_address_accepts_bare_and_bracketed_socket_addresses() {
        assert_eq!(health_address("127.0.0.1:8080"), Ok("127.0.0.1:8080"));
        assert_eq!(health_address("[::1]:8080"), Ok("[::1]:8080"));
        assert_eq!(
            health_address("http://localhost:8080"),
            Ok("localhost:8080")
        );
        assert!(health_address("https://localhost:8080").is_err());
    }

    #[test]
    fn local_health_auth_is_sent_as_bearer_without_persisting_it() {
        let mut config = AppConfig::defaults();
        config.space_id = "space".into();
        config.local_auth_credential = Some("health-secret".into());
        let model = ConfigViewModel::new(config);
        let path =
            std::env::temp_dir().join(format!("any-cal-ui-local-auth-{}.conf", std::process::id()));
        model.save(&path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("health-secret"));
        let _ = fs::remove_file(path);
    }
    #[test]
    fn save_refuses_plaintext_token_persistence() {
        let mut config = AppConfig::defaults();
        config.space_id = "space".into();
        config.token = Some("secret".into());
        let model = ConfigViewModel::new(config);
        let path = std::env::temp_dir().join(format!("any-cal-ui-{}.conf", std::process::id()));
        let error = model.save(&path).unwrap_err();
        assert!(error.contains("secret-store support is unavailable"));
        assert!(!path.exists());
    }
}
