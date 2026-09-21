use any_cal_core::{
    BridgeError, BridgeErrorCode, BridgeRequest, BridgeResponse, BRIDGE_SCHEMA_VERSION,
};
use jni::{
    jni_str,
    objects::{JClass, JObject, JString},
    strings::JNIString,
    sys::{jint, jstring},
    Env, EnvUnowned,
};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use rustls_platform_verifier::ConfigVerifierExt;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

fn error_response(code: BridgeErrorCode, message: impl Into<String>) -> BridgeResponse {
    BridgeResponse {
        schema_version: BRIDGE_SCHEMA_VERSION,
        checkpoint: None,
        decisions: Vec::new(),
        error: Some(BridgeError {
            code,
            message: message.into(),
        }),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_anycal_android_NativeRustBridge_nativeSchemaVersion(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
) -> jint {
    BRIDGE_SCHEMA_VERSION as jint
}

#[no_mangle]
pub extern "system" fn Java_org_anycal_android_NativeRustBridge_nativeHealth(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
) -> jint {
    1
}

/// Initializes rustls-platform-verifier with the Android process JVM and
/// application context before any HTTPS exchange is attempted.
#[no_mangle]
pub extern "system" fn Java_org_anycal_android_NativeRustBridge_nativeInitializeVerifier(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    context: JObject<'_>,
) -> jint {
    #[cfg(target_os = "android")]
    {
        return env
            .with_env(|env| {
                rustls_platform_verifier::android::init_with_env(env, context)?;
                Ok::<jint, jni::errors::Error>(1)
            })
            .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (&mut env, context);
        1
    }
}

#[no_mangle]
pub extern "system" fn Java_org_anycal_android_NativeRustBridge_nativeBridgeJson(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    request: JString<'_>,
    endpoint: JString<'_>,
    credential: JString<'_>,
) -> jstring {
    env.with_env(|env| -> Result<jstring, jni::errors::Error> {
        let input = match request.try_to_string(env) {
            Ok(value) => value,
            Err(error) => {
                return Ok(response_string(
                    env,
                    error_response(
                        BridgeErrorCode::InvalidRequest,
                        format!("request string unavailable: ${error}"),
                    ),
                ))
            }
        };
        let endpoint = match endpoint.try_to_string(env) {
            Ok(value) => value,
            Err(_) => {
                return Ok(response_string(
                    env,
                    error_response(
                        BridgeErrorCode::InvalidRequest,
                        "bridge endpoint is unavailable",
                    ),
                ))
            }
        };
        let credential = match credential.try_to_string(env) {
            Ok(value) => value,
            Err(_) => {
                return Ok(response_string(
                    env,
                    error_response(
                        BridgeErrorCode::InvalidRequest,
                        "bridge credential is unavailable",
                    ),
                ))
            }
        };
        let response = match BridgeRequest::from_json(&input) {
            Err(error) => response_string(
                env,
                error_response(BridgeErrorCode::InvalidRequest, error.to_string()),
            ),
            Ok(request) => {
                if endpoint.trim().is_empty() {
                    response_string(
                        env,
                        error_response(
                            BridgeErrorCode::NotLinked,
                            "bridge endpoint is not configured",
                        ),
                    )
                } else if credential.trim().is_empty() {
                    response_string(
                        env,
                        error_response(
                            BridgeErrorCode::TransportUnavailable,
                            "bridge credential is unavailable",
                        ),
                    )
                } else {
                    response_string_json(
                        env,
                        exchange_response_json(&request, &endpoint, &credential),
                    )
                }
            }
        };
        Ok(response)
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

fn exchange_response_json(request: &BridgeRequest, endpoint: &str, credential: &str) -> String {
    let body = match serde_json::to_vec(request) {
        Ok(body) => body,
        Err(_) => {
            return error_json(error_response(
                BridgeErrorCode::InvalidRequest,
                "bridge request could not be encoded",
            ))
        }
    };
    match post_json(endpoint, credential, &body) {
        Ok(body) => match serde_json::from_slice::<Value>(&body) {
            Ok(value) => {
                if response_has_secret_like_error(&value) {
                    return error_json(error_response(
                        BridgeErrorCode::TransportUnavailable,
                        "bridge response was rejected",
                    ));
                }
                match serde_json::from_value::<BridgeResponse>(value.clone()) {
                    Ok(response) if response.validate().is_ok() => serde_json::to_string(&value)
                        .unwrap_or_else(|_| {
                            error_json(error_response(
                                BridgeErrorCode::InvalidRequest,
                                "bridge response was invalid",
                            ))
                        }),
                    _ => error_json(error_response(
                        BridgeErrorCode::InvalidRequest,
                        "bridge response was invalid",
                    )),
                }
            }
            Err(_) => error_json(error_response(
                BridgeErrorCode::InvalidRequest,
                "bridge response was not JSON",
            )),
        },
        Err(ExchangeError::Auth) => error_json(error_response(
            BridgeErrorCode::PermissionDenied,
            "bridge authorization failed",
        )),
        Err(ExchangeError::Conflict) => error_json(error_response(
            BridgeErrorCode::Conflict,
            "bridge reported a conflict",
        )),
        Err(_) => error_json(error_response(
            BridgeErrorCode::TransportUnavailable,
            "bridge exchange failed",
        )),
    }
}

#[derive(Debug)]
enum ExchangeError {
    Auth,
    Conflict,
    Io,
    Invalid,
    Tls,
}

fn response_has_secret_like_error(value: &Value) -> bool {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .is_some_and(|message| {
            let lower = message.to_ascii_lowercase();
            ["token", "api_key", "authorization", "password", "secret"]
                .iter()
                .any(|needle| lower.contains(needle))
        })
}

fn post_json(endpoint: &str, credential: &str, body: &[u8]) -> Result<Vec<u8>, ExchangeError> {
    if credential
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(ExchangeError::Invalid);
    }
    let parts = Endpoint::parse(endpoint)?;
    let path = format!("{}/android/sync", parts.path.trim_end_matches('/'));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\nContent-Type: application/json\r\nAuthorization: Bearer {credential}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len(),
        path = path,
        authority = parts.authority,
        credential = credential,
    );
    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(body);
    let mut stream = connect(&parts.host, parts.port)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| ExchangeError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| ExchangeError::Io)?;
    let response = if parts.secure {
        let config = platform_client_config()?;
        let server_name = server_name(&parts.host)?;
        let connection =
            ClientConnection::new(config, server_name).map_err(|_| ExchangeError::Tls)?;
        let mut tls = StreamOwned::new(connection, stream);
        tls.write_all(&bytes).map_err(|_| ExchangeError::Io)?;
        read_http_response(&mut tls)?
    } else {
        stream.write_all(&bytes).map_err(|_| ExchangeError::Io)?;
        read_http_response(&mut stream)?
    };
    match response.status {
        200..=299 => Ok(response.body),
        401 | 403 => Err(ExchangeError::Auth),
        409 => Err(ExchangeError::Conflict),
        _ => Err(ExchangeError::Io),
    }
}

struct Endpoint {
    secure: bool,
    authority: String,
    host: String,
    port: u16,
    path: String,
}

impl Endpoint {
    fn parse(input: &str) -> Result<Self, ExchangeError> {
        let (secure, rest) = if let Some(rest) = input.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = input.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(ExchangeError::Invalid);
        };
        let slash = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..slash];
        let path = if slash == rest.len() {
            String::new()
        } else {
            rest[slash..].to_owned()
        };
        if authority.is_empty()
            || authority.contains('@')
            || authority
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(ExchangeError::Invalid);
        }
        let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
            let close = bracketed.find(']').ok_or(ExchangeError::Invalid)?;
            let host = bracketed[..close].to_owned();
            let suffix = &bracketed[close + 1..];
            let port = if suffix.is_empty() {
                if secure {
                    443
                } else {
                    80
                }
            } else {
                suffix
                    .strip_prefix(':')
                    .ok_or(ExchangeError::Invalid)?
                    .parse()
                    .map_err(|_| ExchangeError::Invalid)?
            };
            (host, port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            if host.is_empty() {
                return Err(ExchangeError::Invalid);
            }
            (
                host.to_owned(),
                port.parse().map_err(|_| ExchangeError::Invalid)?,
            )
        } else {
            (authority.to_owned(), if secure { 443 } else { 80 })
        };
        if host.is_empty()
            || path.chars().any(|c| c.is_control() || c.is_whitespace())
            || (!secure
                && !matches!(
                    host.to_ascii_lowercase().as_str(),
                    "localhost" | "127.0.0.1" | "::1"
                ))
        {
            return Err(ExchangeError::Invalid);
        }
        Ok(Self {
            secure,
            authority: authority.to_owned(),
            host,
            port,
            path,
        })
    }
}

fn connect(host: &str, port: u16) -> Result<TcpStream, ExchangeError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| ExchangeError::Io)?;
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) {
            return Ok(stream);
        }
    }
    Err(ExchangeError::Io)
}

fn server_name(host: &str) -> Result<ServerName<'static>, ExchangeError> {
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(address.into()));
    }
    ServerName::try_from(host.to_owned()).map_err(|_| ExchangeError::Invalid)
}

fn platform_client_config() -> Result<Arc<ClientConfig>, ExchangeError> {
    ClientConfig::with_platform_verifier()
        .map(Arc::new)
        .map_err(|_| ExchangeError::Tls)
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn read_http_response<R: Read>(reader: &mut R) -> Result<HttpResponse, ExchangeError> {
    const MAX_BODY: usize = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).map_err(|_| ExchangeError::Io)?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > MAX_BODY + 64 * 1024 {
            return Err(ExchangeError::Invalid);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") && bytes.len() > MAX_BODY {
            return Err(ExchangeError::Invalid);
        }
    }
    let marker = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(ExchangeError::Invalid)?;
    let head = std::str::from_utf8(&bytes[..marker]).map_err(|_| ExchangeError::Invalid)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .ok_or(ExchangeError::Invalid)?;
    let body = bytes[marker + 4..].to_vec();
    if body.len() > MAX_BODY {
        return Err(ExchangeError::Invalid);
    }
    // The request asks the peer to close the connection, but still honor a
    // declared length.  This prevents accepting a truncated response and
    // makes the framing explicit for servers that keep connections alive.
    if let Some(length) = head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            Some(value.trim().parse::<usize>().ok())
        } else {
            None
        }
    }) {
        let length = length.ok_or(ExchangeError::Invalid)?;
        if length > MAX_BODY || body.len() != length {
            return Err(ExchangeError::Invalid);
        }
    }
    Ok(HttpResponse { status, body })
}

fn response_string(env: &mut Env<'_>, response: BridgeResponse) -> jstring {
    response_string_json(env, error_json(response))
}

fn error_json(response: BridgeResponse) -> String {
    response.canonical_json().unwrap_or_else(|_| "{\"schema_version\":1,\"checkpoint\":null,\"decisions\":[],\"error\":{\"code\":\"transport_unavailable\",\"message\":\"bridge exchange failed\"}}".to_owned())
}

fn response_string_json(env: &mut Env<'_>, json: String) -> jstring {
    match env.new_string(json) {
        Ok(value) => value.into_raw(),
        Err(error) => {
            let _ = env.throw_new(
                jni_str!("java/lang/IllegalStateException"),
                JNIString::from(error.to_string()),
            );
            std::ptr::null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_stable() {
        assert_eq!(any_cal_core::BRIDGE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn platform_verifier_config_builds_for_secure_exchange() {
        assert!(platform_client_config().is_ok());
    }

    #[test]
    fn endpoint_policy_rejects_userinfo_and_accepts_loopback_path() {
        assert!(Endpoint::parse("https://user:pass@example.invalid").is_err());
        assert!(Endpoint::parse("http://example.invalid").is_err());
        assert!(Endpoint::parse("http://127.0.0.1/a b").is_err());
        let endpoint = Endpoint::parse("http://127.0.0.1:8080/gateway").unwrap();
        assert!(!endpoint.secure);
        assert_eq!(endpoint.path, "/gateway");
        assert_eq!(endpoint.port, 8080);
    }

    #[test]
    fn response_secret_text_is_rejected_without_echoing_it() {
        let value = serde_json::json!({
            "error": { "message": "authorization token must not appear" }
        });
        assert!(response_has_secret_like_error(&value));
        let safe = error_json(error_response(
            BridgeErrorCode::TransportUnavailable,
            "bridge response was rejected",
        ));
        assert!(!safe.contains("authorization token"));
    }

    #[test]
    fn http_response_requires_complete_headers() {
        let mut reader = &b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"[..];
        let response = read_http_response(&mut reader).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{}".to_vec());
        let mut malformed = &b"HTTP/1.1 200 OK\r\n{}"[..];
        assert!(read_http_response(&mut malformed).is_err());
        let mut truncated = &b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n{}"[..];
        assert!(read_http_response(&mut truncated).is_err());
    }
}
