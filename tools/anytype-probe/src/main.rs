//! Guarded, read-only-by-default Anytype probe launcher.
//!
//! This tool deliberately does not know the Anytype schema. It isolates a
//! supplied headless binary, captures only redacted bounded diagnostics, and
//! emits a plan/report when no explicitly authorized disposable environment is
//! available.
use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const DEFAULT_API: &str = "http://127.0.0.1:31012";
const MAX_CAPTURE: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Options {
    binary: Option<PathBuf>,
    api_url: String,
    state_root: Option<PathBuf>,
    allow_write: bool,
    confirm_write: bool,
    cleanup: bool,
    run: bool,
    space_id: Option<String>,
    api_key_stdin: bool,
    approval_artifact: Option<PathBuf>,
    app_link: Option<PathBuf>,
    revoke_capability: Option<PathBuf>,
}

fn main() {
    match run(env::args().skip(1)) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("probe error: {error}");
            std::process::exit(2);
        }
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<String, String> {
    let options = parse_args(args)?;
    validate_options(&options)?;
    let root = options
        .state_root
        .clone()
        .unwrap_or_else(default_state_root);
    if !options.cleanup {
        fs::create_dir_all(&root).map_err(|e| format!("create isolated state root: {e}"))?;
        restrict_directory(&root)?;
    }
    let sentinel = root.join(".any-cal-probe-root");
    if !sentinel.exists() {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sentinel)
            .map_err(|e| format!("initialize probe root sentinel: {e}"))?;
        restrict_file(&sentinel)?;
    }
    let port = if options.run { ephemeral_port()? } else { 0 };
    let mut report = format!(
        "mode=read-only\napi_url={}\nstate_root={}\nprobe_port={}\n",
        redact(&options.api_url),
        root.display(),
        port
    );
    if options.allow_write {
        report.push_str("write_mode=authorized-disposable-only\n");
    } else {
        report.push_str(
            "write_mode=deferred (requires --allow-write and ANY_CAL_CONFIRM_WRITE=YES)\n",
        );
    }
    report.push_str("stages=version,capability,schema,list,get; writes=deferred\n");

    if options.run && options.binary.is_none() {
        let space = options
            .space_id
            .as_deref()
            .ok_or_else(|| "--run requires --space-id".to_owned())?;
        let key = if options.api_key_stdin {
            read_key_stdin()?
        } else {
            env::var("ANY_CAL_API_KEY")
                .map_err(|_| "set ANY_CAL_API_KEY or pass --api-key-stdin".to_owned())?
        };
        let evidence = probe_api(&options.api_url, space, &key)?;
        report.push_str(&evidence);
    }
    if options.run && options.binary.is_some() {
        let binary = options
            .binary
            .as_ref()
            .ok_or_else(|| "--run requires --binary".to_owned())?;
        let output = Command::new(binary)
            .arg("--version")
            .env("ANY_CAL_PROBE_STATE_ROOT", &root)
            .env("ANY_CAL_PROBE_LISTEN_ADDRESS", format!("127.0.0.1:{port}"))
            .env("ANYTYPE_API_URL", &options.api_url)
            .output()
            .map_err(|e| format!("launch read-only version probe: {e}"))?;
        let stdout = secret_aware_capture(&output.stdout, &[]);
        let stderr = secret_aware_capture(&output.stderr, &[]);
        let captured = format!(
            "status={} stdout={} stderr={} stdout_sha256={} stderr_sha256={}\n",
            output.status,
            stdout,
            stderr,
            sha256_hex(&output.stdout),
            sha256_hex(&output.stderr)
        );
        report.push_str(&captured);
    }
    if options.cleanup {
        if !sentinel.is_file() {
            return Err("refusing cleanup: probe root sentinel is missing".into());
        }
        fs::remove_dir_all(&root).map_err(|e| format!("cleanup isolated state root: {e}"))?;
        if root.exists() {
            return Err("cleanup verification failed: probe root still exists".into());
        }
        report.push_str("cleanup=performed verified=absent\n");
    } else {
        report.push_str("cleanup=preserved (pass --cleanup explicitly)\n");
    }
    Ok(report)
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut binary = None;
    let mut api_url = DEFAULT_API.to_owned();
    let mut state_root = None;
    let mut allow_write = false;
    let mut cleanup = false;
    let mut run = false;
    let mut space_id = None;
    let mut api_key_stdin = false;
    let mut approval_artifact = None;
    let mut app_link = None;
    let mut revoke_capability = None;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--binary" => binary = Some(PathBuf::from(next(&mut it, "binary")?)),
            "--api-url" => api_url = next(&mut it, "api-url")?,
            "--state-root" => state_root = Some(PathBuf::from(next(&mut it, "state-root")?)),
            "--allow-write" => allow_write = true,
            "--cleanup" => cleanup = true,
            "--run" => run = true,
            "--space-id" => space_id = Some(next(&mut it, "space-id")?),
            "--api-key-stdin" => api_key_stdin = true,
            "--approval-artifact" => {
                approval_artifact = Some(PathBuf::from(next(&mut it, "approval-artifact")?))
            }
            "--app-link" => app_link = Some(PathBuf::from(next(&mut it, "app-link")?)),
            "--revoke-capability" => {
                revoke_capability = Some(PathBuf::from(next(&mut it, "revoke-capability")?))
            }
            "--help" => return Err(usage()),
            _ => return Err(format!("unknown argument {arg}\n{}", usage())),
        }
    }
    Ok(Options {
        binary,
        api_url,
        state_root,
        allow_write,
        confirm_write: env::var("ANY_CAL_CONFIRM_WRITE").ok().as_deref() == Some("YES"),
        cleanup,
        run,
        space_id,
        api_key_stdin,
        approval_artifact,
        app_link,
        revoke_capability,
    })
}

fn next(it: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    it.next()
        .ok_or_else(|| format!("--{name} requires a value"))
}

fn validate_options(options: &Options) -> Result<(), String> {
    if !options.api_url.starts_with("http://") {
        return Err("--api-url must use loopback http:// (HTTPS is deferred)".into());
    }
    let authority = options
        .api_url
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if authority
        .to_socket_addrs()
        .map_err(|_| "--api-url has an invalid host/port")?
        .any(|a| !a.ip().is_loopback())
    {
        return Err("--api-url must resolve to a loopback address".into());
    }
    if options.run
        && options
            .space_id
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err("--run requires a non-empty --space-id".into());
    }
    if options.allow_write && !options.confirm_write {
        return Err("writes require ANY_CAL_CONFIRM_WRITE=YES; no write was attempted".into());
    }
    if options.allow_write {
        let artifact = options
            .approval_artifact
            .as_ref()
            .ok_or("write preflight requires --approval-artifact")?;
        validate_approval_artifact(
            artifact,
            &options.api_url,
            options.space_id.as_deref().unwrap_or(""),
        )?;
        validate_credential_prerequisites(
            options.app_link.as_deref(),
            options.revoke_capability.as_deref(),
        )?;
        return Err("write probe is not implemented; read-only evidence only".into());
    }
    if let Some(binary) = &options.binary {
        let text = binary.to_string_lossy().to_ascii_lowercase();
        if text.contains("flatpak") || text.contains("/var/lib/flatpak") {
            return Err("Flatpak paths are refused; supply an external headless binary".into());
        }
        if options.run && !binary.is_file() {
            return Err("--binary must name an existing file when --run is used".into());
        }
        if binary.is_file() {
            let canonical = binary
                .canonicalize()
                .map_err(|_| "--binary cannot be resolved")?;
            let text = canonical.to_string_lossy().to_ascii_lowercase();
            if text.contains("flatpak") || text.contains("/var/lib/flatpak") {
                return Err("Flatpak paths are refused; supply an external headless binary".into());
            }
        }
    }
    if options.cleanup && options.state_root.is_none() {
        return Err("--cleanup requires an explicit --state-root".into());
    }
    if let Some(root) = &options.state_root {
        let parent = root.parent().ok_or("state root must have a parent")?;
        let temp = env::temp_dir()
            .canonicalize()
            .map_err(|_| "temporary directory unavailable")?;
        let parent = parent
            .canonicalize()
            .map_err(|_| "state root parent must exist")?;
        if !parent.starts_with(&temp)
            || root.is_symlink()
            || (!options.cleanup && root.exists())
            || (options.cleanup && !root.exists())
        {
            return Err(
                "state root must be a new child under the system temporary directory".into(),
            );
        }
        if options.cleanup && !root.join(".any-cal-probe-root").is_file() {
            return Err("cleanup target is not a probe-owned root".into());
        }
    }
    Ok(())
}

fn default_state_root() -> PathBuf {
    env::temp_dir().join(format!(
        "any-cal-anytype-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn validate_approval_artifact(path: &PathBuf, endpoint: &str, space: &str) -> Result<(), String> {
    if path.is_symlink() || !path.is_file() {
        return Err("approval artifact must be an existing regular file".into());
    }
    let text = fs::read_to_string(path).map_err(|_| "approval artifact is unreadable")?;
    let root: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "approval artifact is not JSON")?;
    if root.get("contract_version").is_none()
        || root.get("status").is_none()
        || root
            .get("target")
            .and_then(serde_json::Value::as_object)
            .is_none()
        || root
            .get("scope")
            .and_then(serde_json::Value::as_object)
            .is_none()
        || root
            .get("authority")
            .and_then(serde_json::Value::as_object)
            .is_none()
    {
        return Err("approval artifact lacks required structure".into());
    }
    let authority = root["authority"].as_object().unwrap();
    for dimension in ["read", "write", "external", "live", "destructive"] {
        if authority.get(dimension).and_then(serde_json::Value::as_str) != Some("approval-required")
        {
            return Err(format!(
                "approval artifact authority is not fail-closed for {dimension}"
            ));
        }
    }
    if ["api_key", "token", "authorization", "bearer"]
        .iter()
        .any(|secret| text.to_ascii_lowercase().contains(secret))
    {
        return Err("approval artifact contains a forbidden secret field".into());
    }
    let locator = root["target"]
        .get("locator")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !locator.contains(endpoint) || !locator.contains(space) {
        return Err("approval artifact target does not match endpoint and Space".into());
    }
    Ok(())
}

/// A future guest-only provisioning runner must prove both prerequisites
/// before creating or printing an account key. The app-link is checked only by
/// metadata; its contents are never read. The capability receipt is a
/// nonsecret artifact produced by a prior `auth apikey revoke --help` check.
fn validate_credential_prerequisites(
    app_link: Option<&std::path::Path>,
    revoke_capability: Option<&std::path::Path>,
) -> Result<(), String> {
    let app_link = app_link.ok_or(
        "credential provisioning refused: account app-link is required before key creation",
    )?;
    let revoke_capability = revoke_capability
        .ok_or("credential provisioning refused: revocation capability receipt is required")?;
    validate_private_regular_file(app_link, "account app-link")?;
    validate_private_regular_file(revoke_capability, "revocation capability receipt")?;
    let receipt = fs::read_to_string(revoke_capability)
        .map_err(|_| "revocation capability receipt is unreadable".to_owned())?;
    if receipt.trim() != "revoke=available" {
        return Err(
            "credential provisioning refused: revocation capability receipt is invalid".into(),
        );
    }
    Ok(())
}

fn validate_private_regular_file(path: &std::path::Path, label: &str) -> Result<(), String> {
    if path.is_symlink() || !path.is_file() {
        return Err(format!("{label} must be an existing regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|_| format!("{label} metadata is unavailable"))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!("{label} must not be group/world accessible"));
        }
    }
    Ok(())
}

fn ephemeral_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("reserve probe port: {e}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|e| format!("read probe port: {e}"))
}

fn redact(input: &str) -> String {
    bounded_redact(input)
        .replace("ANYTYPE_API_KEY", "[REDACTED_KEY]")
        .replace("Authorization", "[REDACTED_AUTH]")
}

/// Captures are redacted before they are interpolated into a report or sent
/// to a caller. The optional secrets list is used for exact and fragment
/// matching because a credential can be split by a formatter or log prefix.
fn secret_aware_capture(input: &[u8], secrets: &[&str]) -> String {
    let text = String::from_utf8_lossy(input);
    if text.contains("API key created successfully") {
        return match parse_api_key_create_output(&text) {
            Ok(summary) => format!(
                "credential_handoff=api_key_create name_present={} key_present={}\n",
                summary.name_present, summary.key_present
            ),
            Err(_) => "[REDACTED_SECRET_CAPTURE]".to_owned(),
        };
    }
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let normalized_first = normalize_cli_line(first_line);
    if normalized_first == "No API keys found."
        || normalized_first
            .split_whitespace()
            .eq(["NAME", "ID", "KEY", "CREATED"])
    {
        return match parse_app_id_output(&text) {
            Ok(AppIdListSummary::Empty) => "credential_handoff=app_id none=true\n".to_owned(),
            Ok(AppIdListSummary::One(summary)) => format!(
                "credential_handoff=app_id fingerprint={} name_present={}\n",
                summary.app_id_fingerprint, summary.name_present
            ),
            Err(_) => "[REDACTED_SECRET_CAPTURE]".to_owned(),
        };
    }
    let redacted = redact_secrets(&text, secrets);
    if contains_secret_material(&redacted, secrets) {
        "[REDACTED_SECRET_CAPTURE]".to_owned()
    } else {
        bounded_redact(&redacted)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ApiKeyCreateSummary {
    name_present: bool,
    key_present: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct AppIdSummary {
    app_id_fingerprint: String,
    name_present: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum AppIdListSummary {
    Empty,
    One(AppIdSummary),
}

/// Parse the exact v0.3.6 API-key create handoff. The key is deliberately
/// reduced to presence; its value never enters the returned structure.
fn parse_api_key_create_output(input: &str) -> Result<ApiKeyCreateSummary, &'static str> {
    let mut success = false;
    let mut name = false;
    let mut key = false;
    for raw_line in input.lines() {
        let line = normalize_cli_line(raw_line);
        let line = line.as_str();
        if line.is_empty() {
            continue;
        }
        if line == "API key created successfully" {
            if success {
                return Err("duplicate success line");
            }
            success = true;
        } else if let Some(value) = line.strip_prefix("Name:") {
            if name || value.trim().is_empty() {
                return Err("missing or duplicate name");
            }
            name = true;
        } else if let Some(value) = line.strip_prefix("Key:") {
            if key || value.trim().is_empty() || value.contains(['\r', '\n']) {
                return Err("missing or duplicate key");
            }
            key = true;
        } else {
            return Err("unexpected handoff field");
        }
    }
    if success && name && key {
        Ok(ApiKeyCreateSummary {
            name_present: true,
            key_present: true,
        })
    } else {
        Err("incomplete handoff")
    }
}

/// Parse the v0.3.6 `auth apikey list` table. The CLI emits NAME, ID, KEY, and
/// CREATED columns; the displayed key is deliberately accepted only in its
/// shortened `8-chars...` form. The app ID is returned as a one-way digest
/// handle so the caller can identify the row for revocation without exposing
/// or retaining the identifier itself. Multiple rows are rejected because a
/// lifecycle handoff must identify exactly one key.
fn parse_app_id_output(input: &str) -> Result<AppIdListSummary, &'static str> {
    if input.chars().any(|character| {
        character == '\u{1b}'
            || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    }) {
        return Err("control or ANSI contamination");
    }
    let mut lines = input.lines().filter_map(|raw| {
        let line = normalize_cli_line(raw);
        (!line.is_empty()).then_some(line)
    });
    let first = lines.next().ok_or("list output is missing")?;
    if first == "No API keys found." {
        if lines.next().is_some() {
            return Err("unexpected output after empty-list marker");
        }
        return Ok(AppIdListSummary::Empty);
    }
    let header = first;
    if !header
        .split_whitespace()
        .eq(["NAME", "ID", "KEY", "CREATED"])
    {
        return Err("unexpected list header");
    }
    let separator = lines.next().ok_or("list separator is missing")?;
    if !separator
        .split_whitespace()
        .eq(["----", "--", "---", "----------"])
    {
        return Err("invalid list separator");
    }
    let row = lines.next().ok_or("list row is missing")?;
    if lines.next().is_some() {
        return Err("ambiguous multiple list rows");
    }
    let fields: Vec<&str> = row.split_whitespace().collect();
    if fields.len() < 5 {
        return Err("malformed list row");
    }
    let created_date = fields[fields.len() - 2];
    let created_time = fields[fields.len() - 1];
    if !is_iso_date(created_date) || !is_clock_time(created_time) {
        return Err("invalid creation timestamp");
    }
    let key = fields[fields.len() - 3];
    if key.len() != 11
        || !key.is_ascii()
        || !key.ends_with("...")
        || !key[..8].bytes().all(|b| b.is_ascii_graphic())
    {
        return Err("unredacted or malformed key column");
    }
    let app_id = fields[fields.len() - 4];
    if !is_app_id(app_id) {
        return Err("malformed app ID");
    }
    if fields[..fields.len() - 4].is_empty() {
        return Err("missing app name");
    }
    Ok(AppIdListSummary::One(AppIdSummary {
        app_id_fingerprint: sha256_hex(app_id.as_bytes()),
        name_present: true,
    }))
}

fn is_app_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_iso_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn is_clock_time(value: &str) -> bool {
    value.len() == 8
        && value.as_bytes()[2] == b':'
        && value.as_bytes()[5] == b':'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 2 | 5) || byte.is_ascii_digit())
}

fn normalize_cli_line(raw: &str) -> String {
    let mut line = String::with_capacity(raw.len());
    let mut escape = false;
    for character in raw.chars() {
        if escape {
            if character.is_ascii_alphabetic() {
                escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            escape = true;
        } else {
            line.push(character);
        }
    }
    let mut line = line.trim().to_owned();
    for prefix in ["[SUCCESS] ", "[INFO] ", "SUCCESS: ", "INFO: ", "✔ ", "✓ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            line = rest.trim().to_owned();
            break;
        }
    }
    line
}

fn redact_secrets(input: &str, secrets: &[&str]) -> String {
    let mut value = input.to_owned();
    for secret in secrets.iter().copied().filter(|secret| secret.len() >= 8) {
        value = value.replace(secret, "[REDACTED]");
        let split = secret.len() / 2;
        value = value.replace(&secret[..split], "[REDACTED_FRAGMENT]");
        value = value.replace(&secret[secret.len() - split..], "[REDACTED_FRAGMENT]");
    }
    value
}

fn contains_secret_material(input: &str, secrets: &[&str]) -> bool {
    let lower = input.to_ascii_lowercase();
    if lower.contains("authorization:") || lower.contains("bearer ") {
        return true;
    }
    secrets.iter().any(|secret| {
        if secret.len() < 8 {
            return false;
        }
        let split = secret.len() / 2;
        input.contains(secret)
            || input.contains(&secret[..split])
            || input.contains(&secret[secret.len() - split..])
    })
}

#[cfg(unix)]
fn restrict_directory(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("restrict probe state root permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_directory(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("restrict probe sentinel permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_file(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

fn bounded_redact(input: &str) -> String {
    let mut value = input.to_owned();
    for key in [
        "token",
        "access token",
        "api key",
        "account key",
        "api_key",
        "apikey",
        "password",
        "secret",
        "authorization",
        "bearer",
    ] {
        loop {
            let lower = value.to_ascii_lowercase();
            let Some(position) = lower.find(key) else {
                break;
            };
            let mut start = position + key.len();
            while value
                .as_bytes()
                .get(start)
                .is_some_and(|byte| b":=| \t\"".contains(byte))
            {
                start += 1;
            }
            let end = value[start..]
                .find(['\r', '\n', ',', ';', ' ', '\t', '"'])
                .map(|offset| start + offset)
                .unwrap_or(value.len());
            if value[start..].starts_with("[REDACTED]") {
                break;
            }
            if end <= start {
                value.replace_range(position..position + key.len(), "[REDACTED]");
            } else {
                value.replace_range(start..end, "[REDACTED]");
            }
        }
    }
    for label in [
        "Authorization",
        "authorization",
        "Bearer",
        "bearer",
        "API key",
        "api key",
        "Access token",
        "access token",
    ] {
        value = value.replace(label, "[REDACTED_CREDENTIAL]");
    }
    value.chars().take(MAX_CAPTURE).collect()
}

fn read_key_stdin() -> Result<String, String> {
    let mut key = String::new();
    std::io::stdin()
        .read_to_string(&mut key)
        .map_err(|e| format!("read API key from stdin: {e}"))?;
    let key = key.trim().to_owned();
    if key.is_empty() {
        Err("API key from stdin is empty".into())
    } else {
        Ok(key)
    }
}

fn probe_api(endpoint: &str, space: &str, key: &str) -> Result<String, String> {
    let authority = endpoint.trim_start_matches("http://").trim_end_matches('/');
    let encoded = encode_segment(space);
    let mut out = String::new();
    let mut offset = 0usize;
    for path in [
        format!("/v1/spaces/{encoded}"),
        format!("/v1/spaces/{encoded}/types?offset=0&limit=100"),
        format!("/v1/spaces/{encoded}/objects?offset={offset}&limit=100"),
        format!("/v1/spaces/{encoded}/properties?offset=0&limit=100"),
    ] {
        let body = http_get(authority, &path, key)?;
        if path.contains("offset=") {
            validate_page(
                &body,
                path.contains("/objects?"),
                path.contains("/types?"),
                path.contains("/properties?"),
            )?;
        } else if !body.contains('{') {
            return Err("probe response is not an object envelope".into());
        }
        let summary = secret_aware_capture(body.as_bytes(), &[key]);
        out.push_str(&format!(
            "GET {path} status=200 body_len={} body_sha256={} summary={}\n",
            body.len(),
            sha256_hex(body.as_bytes()),
            summary
        ));
        if path.contains("/objects?") || path.contains("/types?") || path.contains("/properties?") {
            let mut current = body;
            let mut seen = 0usize;
            while page_has_more(&current)? {
                if seen >= 100 {
                    return Err("probe pagination exceeds 100 pages".into());
                }
                let (old, limit) = page_position(&current)?;
                offset = checked_next_offset(old, limit)?;
                let resource = if path.contains("/types?") {
                    "types"
                } else if path.contains("/properties?") {
                    "properties"
                } else {
                    "objects"
                };
                let next_path =
                    format!("/v1/spaces/{encoded}/{resource}?offset={offset}&limit={limit}");
                let next_body = http_get(authority, &next_path, key)?;
                validate_page(
                    &next_body,
                    resource == "objects",
                    resource == "types",
                    resource == "properties",
                )?;
                let (new, _) = page_position(&next_body)?;
                if new <= old {
                    return Err("probe pagination offset did not advance".into());
                }
                out.push_str(&format!(
                    "GET {next_path} status=200 body_len={} body_sha256={} summary={}\n",
                    next_body.len(),
                    sha256_hex(next_body.as_bytes()),
                    secret_aware_capture(next_body.as_bytes(), &[key])
                ));
                current = next_body;
                seen += 1;
            }
        }
    }
    Ok(out)
}

fn encode_segment(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        out
    })
}

fn page_position(body: &str) -> Result<(usize, usize), String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|_| "page is not JSON")?;
    let p = root
        .get("pagination")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing pagination")?;
    Ok((
        p.get("offset")
            .and_then(serde_json::Value::as_u64)
            .ok_or("invalid offset")? as usize,
        p.get("limit")
            .and_then(serde_json::Value::as_u64)
            .ok_or("invalid limit")? as usize,
    ))
}

fn page_has_more(body: &str) -> Result<bool, String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|_| "page is not JSON")?;
    root.get("pagination")
        .and_then(|p| p.get("has_more"))
        .and_then(serde_json::Value::as_bool)
        .ok_or("invalid has_more".into())
}

fn checked_next_offset(offset: usize, limit: usize) -> Result<usize, String> {
    let next = offset
        .checked_add(limit)
        .ok_or("probe pagination overflow")?;
    if next <= offset || limit == 0 {
        return Err("probe pagination offset did not advance".into());
    }
    Ok(next)
}

#[cfg(test)]
fn validate_object_envelope(body: &str) -> Result<(), String> {
    let root: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "object envelope is not JSON")?;
    let object = root
        .get("object")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing object envelope")?;
    for key in ["id", "space_id", "name", "type", "properties", "archived"] {
        if !object.contains_key(key) {
            return Err(format!("object envelope missing {key}"));
        }
    }
    if !object["id"].is_string()
        || !object["space_id"].is_string()
        || !object["name"].is_string()
        || !valid_type_object(&object["type"])
        || !valid_properties(&object["properties"])
        || !object["archived"].is_boolean()
    {
        return Err("object envelope has invalid field types".into());
    }
    Ok(())
}

fn valid_type_object(value: &serde_json::Value) -> bool {
    let Some(value) = value.as_object() else {
        return false;
    };
    value.get("key").is_some_and(serde_json::Value::is_string)
        && value.get("name").is_some_and(serde_json::Value::is_string)
}

fn valid_properties(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            let Some(property) = item.as_object() else {
                return false;
            };
            if !property
                .get("key")
                .is_some_and(serde_json::Value::is_string)
            {
                return false;
            }
            [
                "text",
                "phone",
                "email",
                "date",
                "objects",
                "multi_select",
                "number",
                "checkbox",
                "url",
                "select",
            ]
            .iter()
            .any(|kind| property.get(*kind).is_some_and(|value| !value.is_null()))
        })
    })
}

fn validate_page(body: &str, objects: bool, types: bool, properties: bool) -> Result<(), String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|_| "page is not JSON")?;
    let data = root
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or("page data is not an array")?;
    let pagination = root
        .get("pagination")
        .and_then(serde_json::Value::as_object)
        .ok_or("page pagination is not an object")?;
    let offset = pagination
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .ok_or("page offset is not numeric")?;
    let limit = pagination
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .ok_or("page limit is not numeric")?;
    let total = pagination
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .ok_or("page total is not numeric")?;
    let has_more = pagination
        .get("has_more")
        .and_then(serde_json::Value::as_bool)
        .ok_or("page has_more is not boolean")?;
    if limit == 0
        || offset > total
        || offset.saturating_add(data.len() as u64) > total
        || (!has_more && offset.saturating_add(limit) < total)
    {
        return Err("page pagination metadata is inconsistent".into());
    }
    if objects {
        for item in data {
            let object = item.as_object().ok_or("object entry is not an object")?;
            for key in ["id", "space_id", "name", "type", "archived", "properties"] {
                if !object.contains_key(key) {
                    return Err(format!("object missing {key}"));
                }
            }
            if !object["id"].is_string()
                || !object["space_id"].is_string()
                || !object["name"].is_string()
                || !valid_type_object(&object["type"])
                || !object["archived"].is_boolean()
                || !valid_properties(&object["properties"])
            {
                return Err("object has invalid field types".into());
            }
        }
    }
    if types || properties {
        for item in data {
            let item = item.as_object().ok_or("catalog entry is not an object")?;
            for key in ["id", "key", "name"] {
                if !item.get(key).is_some_and(serde_json::Value::is_string) {
                    return Err(format!("catalog entry missing string {key}"));
                }
            }
            if properties && !item.get("format").is_some_and(serde_json::Value::is_string) {
                return Err("property entry missing format".into());
            }
        }
    }
    Ok(())
}

fn http_get(authority: &str, path: &str, key: &str) -> Result<String, String> {
    let mut stream =
        TcpStream::connect(authority).map_err(|e| format!("connect probe endpoint: {e}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\nAnytype-Version: 2025-11-08\r\nAuthorization: Bearer {key}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write probe request: {e}"))?;
    let mut bytes = Vec::new();
    stream
        .take((MAX_CAPTURE * 16 + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read probe response: {e}"))?;
    if bytes.len() > MAX_CAPTURE * 16 {
        return Err("probe response exceeds body limit".into());
    }
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed probe response")?;
    if split > 32 * 1024 {
        return Err("probe response headers exceed limit".into());
    }
    let head = String::from_utf8_lossy(&bytes[..split]);
    let status = head
        .split_whitespace()
        .nth(1)
        .ok_or("malformed probe status")?;
    if status != "200" {
        return Err(format!("probe endpoint returned HTTP {status}"));
    }
    let mut lengths = Vec::new();
    let mut content_type = None;
    let mut transfer_encodings = Vec::new();
    for line in head.lines().skip(1) {
        let (name, value) = line.split_once(':').ok_or("malformed probe header")?;
        if name.eq_ignore_ascii_case("content-length") {
            lengths.push(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid Content-Length")?,
            );
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.trim().to_ascii_lowercase());
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            transfer_encodings.push(value.trim().to_ascii_lowercase());
        }
    }
    if content_type
        .as_deref()
        .is_none_or(|v| !v.starts_with("application/json"))
    {
        return Err("probe response requires JSON Content-Type".into());
    }
    let body = &bytes[split + 4..];
    let decoded = match (lengths.as_slice(), transfer_encodings.as_slice()) {
        ([declared], []) => {
            if *declared > MAX_CAPTURE * 16 {
                return Err("probe response exceeds body limit".into());
            }
            if *declared != body.len() {
                return Err("truncated or extra probe response bytes".into());
            }
            body.to_vec()
        }
        ([], [encoding]) if encoding == "chunked" => decode_chunked(body)?,
        _ => return Err("probe response requires one Content-Length or chunked encoding".into()),
    };
    let text = String::from_utf8(decoded).map_err(|_| "probe response is not UTF-8")?;
    if !text.trim_start().starts_with('{') && !text.trim_start().starts_with('[') {
        return Err("probe response is not JSON".into());
    }
    Ok(text)
}

fn decode_chunked(input: &[u8]) -> Result<Vec<u8>, String> {
    const MAX_CHUNK_LINE: usize = 1024;
    const MAX_TRAILER_LINE: usize = 1024;
    let mut output = Vec::new();
    let mut cursor = 0;
    loop {
        let line_end = input[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("truncated chunk size")?;
        if line_end > MAX_CHUNK_LINE {
            return Err("chunk size line exceeds limit".into());
        }
        let line = &input[cursor..cursor + line_end];
        let size_text = line
            .split(|byte| *byte == b';')
            .next()
            .ok_or("invalid chunk size")?;
        let size_text = std::str::from_utf8(size_text)
            .map_err(|_| "chunk size is not ASCII")?
            .trim();
        if size_text.is_empty() || !size_text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid chunk size".into());
        }
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "invalid chunk size")?;
        cursor += line_end + 2;
        if size == 0 {
            loop {
                let trailer_end = input[cursor..]
                    .windows(2)
                    .position(|window| window == b"\r\n")
                    .ok_or("truncated chunk trailer")?;
                if trailer_end > MAX_TRAILER_LINE {
                    return Err("chunk trailer line exceeds limit".into());
                }
                cursor += trailer_end + 2;
                if trailer_end == 0 {
                    if cursor != input.len() {
                        return Err("extra bytes after chunked response".into());
                    }
                    return Ok(output);
                }
                let trailer = &input[cursor - trailer_end - 2..cursor - 2];
                if !trailer.contains(&b':') {
                    return Err("malformed chunk trailer".into());
                }
            }
        }
        if output
            .len()
            .checked_add(size)
            .is_none_or(|total| total > MAX_CAPTURE * 16)
        {
            return Err("probe response exceeds body limit".into());
        }
        let end = cursor.checked_add(size).ok_or("chunk size overflow")?;
        if end + 2 > input.len() || &input[end..end + 2] != b"\r\n" {
            return Err("truncated chunk data".into());
        }
        output.extend_from_slice(&input[cursor..end]);
        cursor = end + 2;
    }
}

#[allow(clippy::chunks_exact_to_as_chunks)]
fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = input.to_vec();
    let bit_len = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks_exact(4).take(16).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut x) = tuple8(h);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = x
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            x = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (v, n) in [a, b, c, d, e, f, g, x].into_iter().zip(h.iter_mut()) {
            *n = n.wrapping_add(v);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}
fn tuple8(h: [u32; 8]) -> (u32, u32, u32, u32, u32, u32, u32, u32) {
    (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7])
}

fn usage() -> String {
    "usage: anytype-probe [--binary PATH] [--api-url URL] [--state-root DIR] [--space-id ID] [--api-key-stdin] [--run] [--allow-write --approval-artifact PATH --app-link PATH --revoke-capability PATH] [--cleanup]".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_read_only_and_headless() {
        let options = parse_args(Vec::<String>::new()).unwrap();
        assert_eq!(options.api_url, DEFAULT_API);
        assert!(!options.allow_write && !options.run);
    }

    #[test]
    fn flatpak_and_write_guards_are_fail_closed() {
        let flatpak = Options {
            binary: Some("/var/lib/flatpak/app/anytype".into()),
            api_url: DEFAULT_API.into(),
            state_root: Some("/tmp/probe".into()),
            allow_write: false,
            confirm_write: false,
            cleanup: false,
            run: false,
            space_id: None,
            api_key_stdin: false,
            approval_artifact: None,
            app_link: None,
            revoke_capability: None,
        };
        assert!(validate_options(&flatpak).is_err());
        let write = Options {
            allow_write: true,
            ..flatpak
        };
        assert!(validate_options(&write).is_err());
    }

    #[test]
    fn credential_provisioning_requires_app_link_and_revoke_capability() {
        let error = validate_credential_prerequisites(None, None).unwrap_err();
        assert!(error.contains("account app-link"));
        let error = validate_credential_prerequisites(None, Some(std::path::Path::new("/tmp/x")))
            .unwrap_err();
        assert!(error.contains("account app-link"));
    }

    #[cfg(unix)]
    #[test]
    fn credential_prerequisites_require_private_files() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "any-cal-credential-prerequisites-{}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let app_link = root.join("account-app-link");
        let capability = root.join("revoke-capability");
        fs::write(&app_link, "synthetic-app-link").unwrap();
        fs::write(&capability, "revoke=available\n").unwrap();
        fs::set_permissions(&app_link, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&capability, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(validate_credential_prerequisites(Some(&app_link), Some(&capability)).is_ok());
        fs::set_permissions(&app_link, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_credential_prerequisites(Some(&app_link), Some(&capability)).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn redaction_is_bounded_and_key_free() {
        let value = bounded_redact("Authorization: Bearer abc token=secret email=me@example.test");
        assert!(!value.contains("secret"));
        assert!(value.len() <= MAX_CAPTURE);
    }

    #[test]
    fn redaction_handles_repeated_escaped_and_multiline_bearer_values() {
        let value = bounded_redact(
            "AUTHORIZATION:   Bearer first-secret; token=second-secret\napi_key=third-secret",
        );
        for secret in ["first-secret", "second-secret", "third-secret"] {
            assert!(!value.contains(secret));
        }
        let escaped = bounded_redact(r#"{"token":"escaped-secret","bearer":"line-secret"}"#);
        assert!(!escaped.contains("escaped-secret") && !escaped.contains("line-secret"));
    }

    #[test]
    fn secret_aware_capture_rejects_full_and_partial_synthetic_keys() {
        let sentinel = "SYNTHETIC_API_KEY_0123456789abcdef";
        let first_half = &sentinel[..sentinel.len() / 2];
        let second_half = &sentinel[sentinel.len() / 2..];
        let output = secret_aware_capture(
            format!("key={sentinel} split={first_half}|{second_half}").as_bytes(),
            &[sentinel],
        );
        assert!(!output.contains(sentinel));
        assert!(!output.contains(first_half));
        assert!(!output.contains(second_half));
    }

    #[test]
    fn secret_aware_capture_rejects_bearer_markers_before_reporting() {
        let output = secret_aware_capture(b"Authorization: Bearer SYNTHETIC_TOKEN_0123456789", &[]);
        assert_eq!(output, "[REDACTED_SECRET_CAPTURE]");
        let output = secret_aware_capture(b"bearer synthetic-fragment", &[]);
        assert_eq!(output, "[REDACTED_SECRET_CAPTURE]");
    }

    #[test]
    fn parses_redacted_v036_api_key_create_handoff_without_values() {
        let input = include_str!("../fixtures/api-key-create-success.txt");
        assert_eq!(
            parse_api_key_create_output(input).unwrap(),
            ApiKeyCreateSummary {
                name_present: true,
                key_present: true,
            }
        );
        let captured = secret_aware_capture(input.as_bytes(), &[]);
        assert_eq!(
            captured,
            "credential_handoff=api_key_create name_present=true key_present=true\n"
        );
        assert!(!captured.contains("SYNTHETIC"));
        assert!(parse_api_key_create_output(include_str!(
            "../fixtures/api-key-create-prefixed.txt"
        ))
        .is_ok());
    }

    #[test]
    fn parses_app_id_as_opaque_fingerprint_only() {
        let input = include_str!("../fixtures/api-key-list-row.txt");
        let AppIdListSummary::One(summary) = parse_app_id_output(input).unwrap() else {
            panic!("expected one API-key row");
        };
        assert!(summary.name_present);
        assert_eq!(summary.app_id_fingerprint.len(), 64);
        assert!(!summary.app_id_fingerprint.contains("SYNTHETIC"));
        assert!(!summary.app_id_fingerprint.contains("APP_ID"));
        let captured = secret_aware_capture(input.as_bytes(), &[]);
        assert!(captured.starts_with("credential_handoff=app_id fingerprint="));
        assert!(!captured.contains("SYNTHETIC"));
        let named = "NAME  ID  KEY  CREATED\n----  --  ---  ----------\nA name with spaces  aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  ABCDE123...  2026-09-20 12:34:56\n";
        assert!(matches!(
            parse_app_id_output(named),
            Ok(AppIdListSummary::One(_))
        ));
        assert_eq!(
            parse_app_id_output(include_str!("../fixtures/api-key-list-empty.txt")).unwrap(),
            AppIdListSummary::Empty
        );
        assert_eq!(
            secret_aware_capture(b"No API keys found.\n", &[]),
            "credential_handoff=app_id none=true\n"
        );
    }

    #[test]
    fn handoff_parser_rejects_ambiguous_and_unexpected_output() {
        assert!(parse_api_key_create_output(include_str!(
            "../fixtures/api-key-create-ambiguous.txt"
        ))
        .is_err());
        assert!(parse_api_key_create_output(include_str!(
            "../fixtures/api-key-create-unexpected.txt"
        ))
        .is_err());
        assert!(
            parse_app_id_output(include_str!("../fixtures/api-key-list-multiple.txt")).is_err()
        );
        assert!(
            parse_app_id_output(include_str!("../fixtures/api-key-list-unredacted.txt")).is_err()
        );
        assert!(
            parse_app_id_output(include_str!("../fixtures/api-key-list-malformed.txt")).is_err()
        );
        assert!(parse_app_id_output("App ID: SYNTHETIC\nKey: SYNTHETIC\n").is_err());
    }

    #[test]
    fn observed_app_list_shape_is_exactly_four_name_tokens_and_64_id_chars() {
        let input = include_str!("../fixtures/api-key-list-row.txt");
        let row = input.lines().nth(2).unwrap();
        let fields: Vec<&str> = row.split_whitespace().collect();
        assert_eq!(
            fields.iter().map(|field| field.len()).collect::<Vec<_>>(),
            vec![7, 10, 3, 1, 64, 11, 10, 8]
        );
        assert!(matches!(
            parse_app_id_output(input),
            Ok(AppIdListSummary::One(_))
        ));
    }

    #[test]
    fn app_list_accepts_layout_whitespace_but_rejects_ansi_and_controls() {
        let tabbed = "NAME\tID\tKEY\tCREATED\n----\t--\t---\t----------\nPERSONA\tEXAMPLE123\tVIP\tX\t aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tABCDEFGH...\t2026-09-20\t12:34:56\n";
        assert!(parse_app_id_output(tabbed).is_ok());

        let ansi = "\u{1b}[32mNAME ID KEY CREATED\u{1b}[0m\n---- -- --- ----------\n";
        assert!(parse_app_id_output(ansi).is_err());
        let control = "NAME ID KEY CREATED\n---- -- --- ----------\nPERSONA\u{0000} EXAMPLE123 VIP X aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ABCDEFGH... 2026-09-20 12:34:56\n";
        assert!(parse_app_id_output(control).is_err());
    }

    #[test]
    fn app_list_rejects_wrong_id_shape_and_extra_rows() {
        let wrong_id = "NAME ID KEY CREATED\n---- -- --- ----------\nPERSONA EXAMPLE123 VIP X not-an-id ABCDEFGH... 2026-09-20 12:34:56\n";
        assert!(parse_app_id_output(wrong_id).is_err());
        let extra = "NAME ID KEY CREATED\n---- -- --- ----------\nPERSONA EXAMPLE123 VIP X aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ABCDEFGH... 2026-09-20 12:34:56\nPERSONA EXAMPLE123 VIP X bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb IJKLMNO... 2026-09-19 11:22:33\n";
        assert!(parse_app_id_output(extra).is_err());
    }

    #[test]
    fn table_shaped_api_key_output_is_redacted_before_reporting() {
        let output = secret_aware_capture(
            b"Account | disposable\nAPI key | SYNTHETIC_API_KEY_0123456789abcdef\n",
            &[],
        );
        assert!(!output.contains("SYNTHETIC_API_KEY_0123456789abcdef"));
        assert!(!output.contains("API key"));
    }

    #[cfg(unix)]
    #[test]
    fn probe_state_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("any-cal-probe-permissions-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        restrict_directory(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir(&path).unwrap();
    }

    #[test]
    fn cleanup_requires_explicit_state_root() {
        let options = Options {
            binary: None,
            api_url: DEFAULT_API.into(),
            state_root: None,
            allow_write: false,
            confirm_write: false,
            cleanup: true,
            run: false,
            space_id: None,
            api_key_stdin: false,
            approval_artifact: None,
            app_link: None,
            revoke_capability: None,
        };
        assert!(validate_options(&options).is_err());
    }

    #[test]
    fn sha256_is_a_real_sha256_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn official_page_envelope_requires_pagination_metadata() {
        assert!(validate_page(
            r#"{"data":[],"pagination":{"offset":0,"limit":100,"total":0,"has_more":false}}"#,
            false,
            false,
            false
        )
        .is_ok());
        assert!(validate_page(r#"{"data":[],"next_offset":null}"#, false, false, false).is_err());
    }

    #[test]
    fn official_object_envelope_requires_typed_fields() {
        assert!(validate_object_envelope(r#"{"object":{"id":"o","space_id":"s","name":"N","type":{"key":"contact","name":"Contact"},"properties":[],"archived":false}}"#).is_ok());
        assert!(validate_object_envelope(r#"{"object":{"id":"o","properties":[]}}"#).is_err());
    }

    #[test]
    fn pagination_offset_guard_rejects_zero_or_overflow() {
        assert_eq!(checked_next_offset(0, 100).unwrap(), 100);
        assert!(checked_next_offset(10, 0).is_err());
        assert!(checked_next_offset(usize::MAX, 1).is_err());
    }

    #[test]
    fn chunked_decoder_accepts_extensions_and_trailers() {
        let body =
            decode_chunked(b"4;note=x\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Trace: ok\r\n\r\n").unwrap();
        assert_eq!(body, b"Wikipedia");
    }

    #[test]
    fn chunked_decoder_rejects_truncation_extra_bytes_and_invalid_sizes() {
        assert!(decode_chunked(b"4\r\nWiki\r\n0\r\n").is_err());
        assert!(decode_chunked(b"4\r\nWiki\r\n0\r\n\r\nextra").is_err());
        assert!(decode_chunked(b"nope\r\n").is_err());
        assert!(decode_chunked(b"1\r\naX\r\n0\r\n\r\n").is_err());
    }

    #[test]
    fn chunked_decoder_rejects_oversized_chunks() {
        let mut body = b"1000001\r\n".to_vec();
        body.extend(std::iter::repeat_n(b'x', 1));
        assert!(decode_chunked(&body).is_err());
    }
}
