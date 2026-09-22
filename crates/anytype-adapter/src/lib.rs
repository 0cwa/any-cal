//! Anytype API boundary. No HTTP client is embedded: callers provide a
//! transport, keeping live credentials and endpoint policy outside this crate.
use any_cal_core::*;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use rustls_platform_verifier::ConfigVerifierExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod wire;

pub const API_VERSION: &str = "2025-11-08";
/// Anytype currently accepts unconditional writes and does not expose a
/// usable revision/ETag precondition.  Keep this policy explicit so callers
/// do not accidentally imply optimistic-concurrency guarantees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictPolicy {
    LaterWriteWins,
}

pub const ANYTYPE_CONFLICT_POLICY: ConflictPolicy = ConflictPolicy::LaterWriteWins;
pub const DEFAULT_RECONCILIATION_READS: u8 = 2;
const MAX_RECEIPTS: usize = 128;

/// Controls whether the adapter sends DAV-derived Anytype property links.
///
/// CanonicalBodyOnly is the safe default: the complete DAV envelope is stored
/// in the object's markdown body, so a normal Anytype Page type is sufficient
/// and no custom property schema is required. Projected is an explicit opt-in
/// for a space where the caller has provisioned the projected keys on the
/// selected type.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PropertyWriteMode {
    #[default]
    CanonicalBodyOnly,
    Projected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationPolicy {
    pub max_reads: u8,
}

impl Default for ReconciliationPolicy {
    fn default() -> Self {
        Self {
            max_reads: DEFAULT_RECONCILIATION_READS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationKind {
    Create,
    Update,
    Archive,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmbiguousMutation {
    Create,
    Update,
    Archive,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationReceipt {
    pub operation_id: String,
    pub object_id: String,
    pub kind: OperationKind,
    pub outcome: String,
    pub reconciliation_reads: u8,
}

impl OperationReceipt {
    /// Receipts intentionally contain only stable identifiers and bounded
    /// outcome labels.  Bodies, properties, endpoint URLs, and credentials
    /// never enter this structure.
    pub fn redacted_summary(&self) -> String {
        format!(
            "operation={} object={} kind={:?} outcome={} reconciliation_reads={}",
            self.operation_id, self.object_id, self.kind, self.outcome, self.reconciliation_reads
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationMetrics {
    pub mutations: u64,
    pub ambiguous_mutations: u64,
    pub reconciled_mutations: u64,
    pub failed_reconciliations: u64,
    pub archive_confirmations: u64,
}

/// Per-object serialization primitive.  The repository is synchronous, but
/// callers can share this table across adapters or dispatchers.  Multi-object
/// callers must use `with_locks`; keys are sorted before acquisition to avoid
/// lock-order inversions.
#[derive(Clone, Default)]
pub struct ObjectLocks {
    locks: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
}

impl ObjectLocks {
    pub fn canonical_order(keys: &[String]) -> Vec<String> {
        let mut sorted = keys.to_vec();
        sorted.sort();
        sorted.dedup();
        sorted
    }

    fn lock_for(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub fn with_lock<R>(&self, key: &str, operation: impl FnOnce() -> R) -> R {
        let lock = self.lock_for(key);
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        operation()
    }

    pub fn with_locks<R>(&self, keys: &[String], operation: impl FnOnce() -> R) -> R {
        let sorted = Self::canonical_order(keys);
        self.with_locks_sorted(&sorted, operation)
    }

    fn with_locks_sorted<R>(&self, keys: &[String], operation: impl FnOnce() -> R) -> R {
        if let Some((first, rest)) = keys.split_first() {
            let lock = self.lock_for(first);
            let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            self.with_locks_sorted(rest, operation)
        } else {
            operation()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObjectRecord {
    pub id: String,
    pub space_id: String,
    pub properties: Vec<(String, String)>,
    /// The Anytype property format for each key returned by the API.  Keeping
    /// this separately from the DAV-facing string value lets updates carry
    /// select/multi-select/object/file links forward without retyping them as
    /// text. Legacy callers may leave the map empty and use format inference.
    #[serde(default)]
    pub property_formats: BTreeMap<String, String>,
    pub body: String,
    pub archived: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    /// Offset continuation returned by the API. The legacy `next_cursor`
    /// spelling is accepted only when reading old fixtures.
    #[serde(
        default,
        alias = "next_cursor",
        deserialize_with = "deserialize_offset"
    )]
    pub next_offset: Option<String>,
}

fn deserialize_offset<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }))
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportError {
    NotFound,
    Conflict,
    Auth,
    Forbidden,
    RateLimited,
    Timeout,
    Malformed,
    DelayedVisibility,
    InvalidRequest(String),
    Unavailable,
    Other(String),
}

#[derive(Clone, Debug, Default)]
pub struct FakeObjectStore {
    objects: BTreeMap<(String, String), ObjectRecord>,
}

impl FakeObjectStore {
    pub fn insert(
        &mut self,
        legacy_object_id: String,
        object: ObjectRecord,
    ) -> Option<ObjectRecord> {
        assert_eq!(
            legacy_object_id, object.id,
            "fake object map key must match ObjectRecord.id"
        );
        self.insert_object(object)
    }

    pub fn insert_object(&mut self, object: ObjectRecord) -> Option<ObjectRecord> {
        let key = (object.space_id.clone(), object.id.clone());
        self.objects.insert(key, object)
    }

    pub fn get(&self, object_id: &str) -> Option<&ObjectRecord> {
        let mut matching = self
            .objects
            .values()
            .filter(|object| object.id == object_id);
        let first = matching.next()?;
        matching.next().is_none().then_some(first)
    }

    pub fn get_in_space(&self, space_id: &str, object_id: &str) -> Option<&ObjectRecord> {
        self.objects
            .get(&(space_id.to_owned(), object_id.to_owned()))
    }

    pub fn get_mut_in_space(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Option<&mut ObjectRecord> {
        self.objects
            .get_mut(&(space_id.to_owned(), object_id.to_owned()))
    }

    pub fn contains_key(&self, object_id: &str) -> bool {
        self.get(object_id).is_some()
    }

    pub fn contains_in_space(&self, space_id: &str, object_id: &str) -> bool {
        self.objects
            .contains_key(&(space_id.to_owned(), object_id.to_owned()))
    }

    pub fn values(&self) -> impl Iterator<Item = &ObjectRecord> {
        self.objects.values()
    }

    pub fn remove_in_space(&mut self, space_id: &str, object_id: &str) -> Option<ObjectRecord> {
        self.objects
            .remove(&(space_id.to_owned(), object_id.to_owned()))
    }
}

#[derive(Clone, Debug, Default)]
pub struct FakeAnytypeTransport {
    pub objects: FakeObjectStore,
    pub page_size: usize,
    pub failure: Option<TransportError>,
    pub delayed: bool,
    pub list_calls: usize,
    pub get_calls: usize,
    pub create_calls: usize,
    pub update_calls: usize,
    pub archive_calls: usize,
    pub delete_calls: usize,
    pub delay_next_read: bool,
    pub timeout_after: Option<AmbiguousMutation>,
}
impl FakeAnytypeTransport {
    pub fn new(page_size: usize) -> Self {
        Self {
            page_size: page_size.max(1),
            ..Self::default()
        }
    }
    pub fn inject(&mut self, error: TransportError) {
        self.failure = Some(error);
    }
    pub fn timeout_after(&mut self, operation: AmbiguousMutation) {
        self.timeout_after = Some(operation);
    }
    fn take(&mut self) -> Result<(), TransportError> {
        self.failure.take().map_or(Ok(()), Err)
    }
}

impl TransportError {
    /// Stable diagnostic category retained at the transport boundary. The
    /// core Repository trait predates remote auth/rate-limit variants.
    pub fn category(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Auth => "auth",
            Self::Forbidden => "forbidden",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Malformed => "malformed",
            Self::DelayedVisibility => "delayed_visibility",
            Self::InvalidRequest(_) => "invalid_request",
            Self::Unavailable => "unavailable",
            Self::Other(_) => "other",
        }
    }
}
impl AnytypeTransport for FakeAnytypeTransport {
    fn list_objects(
        &mut self,
        space_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<ObjectRecord>, TransportError> {
        self.list_calls += 1;
        self.take()?;
        if self.delay_next_read {
            self.delay_next_read = false;
            return Err(TransportError::DelayedVisibility);
        }
        let skip = cursor.and_then(|x| x.parse().ok()).unwrap_or(0);
        let matching = self
            .objects
            .values()
            .filter(|o| o.space_id == space_id && (!o.archived || self.delayed))
            .cloned()
            .collect::<Vec<_>>();
        let data: Vec<_> = matching
            .iter()
            .skip(skip)
            .take(self.page_size)
            .cloned()
            .collect();
        let next = (skip + data.len() < matching.len()).then(|| (skip + data.len()).to_string());
        Ok(Page {
            data,
            next_offset: next,
        })
    }
    fn get_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.get_calls += 1;
        self.take()?;
        if self.delay_next_read {
            self.delay_next_read = false;
            return Err(TransportError::DelayedVisibility);
        }
        self.objects
            .get_in_space(space_id, object_id)
            .cloned()
            .ok_or(TransportError::NotFound)
    }
    fn create_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        self.create_calls += 1;
        self.take()?;
        if self.objects.contains_in_space(&object.space_id, &object.id) {
            return Err(TransportError::Conflict);
        }
        self.objects.insert_object(object.clone());
        if self.timeout_after == Some(AmbiguousMutation::Create) {
            self.timeout_after = None;
            return Err(TransportError::Timeout);
        }
        if self.delayed {
            self.delay_next_read = true;
        }
        Ok(object)
    }
    fn update_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        self.update_calls += 1;
        self.take()?;
        if !self.objects.contains_in_space(&object.space_id, &object.id) {
            return Err(TransportError::NotFound);
        }
        self.objects.insert_object(object.clone());
        if self.timeout_after == Some(AmbiguousMutation::Update) {
            self.timeout_after = None;
            return Err(TransportError::Timeout);
        }
        if self.delayed {
            self.delay_next_read = true;
        }
        Ok(object)
    }
    fn archive_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.archive_calls += 1;
        self.take()?;
        let o = self
            .objects
            .get_mut_in_space(space_id, object_id)
            .ok_or(TransportError::NotFound)?;
        o.archived = true;
        let result = o.clone();
        if self.timeout_after == Some(AmbiguousMutation::Archive) {
            self.timeout_after = None;
            return Err(TransportError::Timeout);
        }
        Ok(result)
    }
    fn delete_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.delete_calls += 1;
        self.take()?;
        let object = self
            .objects
            .get_in_space(space_id, object_id)
            .cloned()
            .ok_or(TransportError::NotFound)?;
        self.objects.remove_in_space(space_id, object_id);
        if self.timeout_after == Some(AmbiguousMutation::Delete) {
            self.timeout_after = None;
            return Err(TransportError::Timeout);
        }
        Ok(object)
    }
}

pub trait AnytypeTransport {
    fn list_objects(
        &mut self,
        space_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<ObjectRecord>, TransportError>;
    fn get_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError>;
    fn create_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError>;
    fn update_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError>;
    fn archive_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError>;
    fn delete_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError>;
}

/// Minimal synchronous HTTP transport with platform-verified TLS for HTTPS.
/// Callers must provide credentials explicitly and this type never includes
/// them in diagnostics.
pub struct HttpAnytypeTransport {
    pub endpoint: String,
    pub api_version: String,
    pub token: Option<String>,
    pub timeout: Duration,
    pub max_body: usize,
    exchange: Option<Box<dyn HttpExchange>>,
    secure: bool,
    authority: String,
    host: String,
    port: u16,
    tls_config: Option<Arc<ClientConfig>>,
}
pub trait HttpExchange: Send {
    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, TransportError>;
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn endpoint_parts(endpoint: &str) -> Result<(bool, String, String, u16), TransportError> {
    let (secure, rest) = if let Some(rest) = endpoint.strip_prefix("http://") {
        (false, rest)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        (true, rest)
    } else {
        return Err(TransportError::InvalidRequest(
            "Anytype endpoint must use http:// or https://".into(),
        ));
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| TransportError::InvalidRequest("Anytype endpoint has no host".into()))?;
    if authority.contains('@')
        || authority
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(TransportError::InvalidRequest(
            "Anytype endpoint authority is invalid".into(),
        ));
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed.find(']').ok_or_else(|| {
            TransportError::InvalidRequest("bracketed endpoint host is invalid".into())
        })?;
        let host = &bracketed[..close];
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
                .ok_or_else(|| TransportError::InvalidRequest("endpoint port is invalid".into()))?
                .parse::<u16>()
                .map_err(|_| TransportError::InvalidRequest("endpoint port is invalid".into()))?
        };
        (host.to_owned(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if host.is_empty() {
            return Err(TransportError::InvalidRequest(
                "Anytype endpoint has no host".into(),
            ));
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| TransportError::InvalidRequest("endpoint port is invalid".into()))?;
        (host.to_owned(), port)
    } else {
        (authority.to_owned(), if secure { 443 } else { 80 })
    };
    if host.is_empty() {
        return Err(TransportError::InvalidRequest(
            "endpoint host is invalid".into(),
        ));
    }
    Ok((secure, authority.to_owned(), host, port))
}

fn tls_server_name(host: &str) -> Result<ServerName<'static>, TransportError> {
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(address.into()));
    }
    ServerName::try_from(host.to_owned())
        .map_err(|_| TransportError::InvalidRequest("endpoint host is invalid".into()))
}

fn connect_with_timeout(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<TcpStream, TransportError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| TransportError::Unavailable)?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(TransportError::Unavailable);
    }
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timed_out = true;
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) if matches!(error.kind(), std::io::ErrorKind::TimedOut) => timed_out = true,
            Err(_) => {}
        }
    }
    if timed_out {
        Err(TransportError::Timeout)
    } else {
        Err(TransportError::Unavailable)
    }
}

fn valid_header_value(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

impl HttpAnytypeTransport {
    pub fn from_config(
        endpoint: impl Into<String>,
        api_version: impl Into<String>,
        token: Option<String>,
    ) -> Result<Self, TransportError> {
        Self::new(endpoint, api_version, token)
    }
}
impl HttpAnytypeTransport {
    pub fn new(
        endpoint: impl Into<String>,
        api_version: impl Into<String>,
        token: Option<String>,
    ) -> Result<Self, TransportError> {
        let endpoint = endpoint.into();
        let api_version = api_version.into();
        let (secure, authority, host, port) = endpoint_parts(&endpoint)?;
        if !valid_header_value(&api_version)
            || token
                .as_deref()
                .is_some_and(|value| !valid_header_value(value))
        {
            return Err(TransportError::InvalidRequest(
                "header values must not contain control characters".into(),
            ));
        }
        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            api_version,
            token,
            timeout: Duration::from_secs(10),
            max_body: 4 * 1024 * 1024,
            exchange: None,
            secure,
            authority,
            host,
            port,
            tls_config: None,
        })
    }
    pub fn with_exchange(mut self, exchange: Box<dyn HttpExchange>) -> Self {
        self.exchange = Some(exchange);
        self
    }
    fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<HttpResponse, TransportError> {
        let body = body.unwrap_or("");
        let mut request = Vec::new();
        write!(&mut request, "{} {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nAnytype-Version: {}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n{}\r\n{}", method, path, self.authority, self.api_version, body.len(), self.token.as_ref().map(|t| format!("Authorization: Bearer {}\r\n", t)).unwrap_or_default(), body).map_err(|_| TransportError::Timeout)?;
        if self.exchange.is_none() {
            let stream = connect_with_timeout(&self.host, self.port, self.timeout)?;
            stream.set_read_timeout(Some(self.timeout)).ok();
            stream.set_write_timeout(Some(self.timeout)).ok();
            if self.secure {
                if self.tls_config.is_none() {
                    self.tls_config = Some(Arc::new(
                        ClientConfig::with_platform_verifier()
                            .map_err(|_| TransportError::Unavailable)?,
                    ));
                }
                let server_name = tls_server_name(&self.host)?;
                let mut connection = ClientConnection::new(
                    self.tls_config
                        .as_ref()
                        .expect("TLS config initialized")
                        .clone(),
                    server_name,
                )
                .map_err(|_| TransportError::Unavailable)?;
                let mut stream = stream;
                connection.complete_io(&mut stream).map_err(map_tls_error)?;
                let mut tls = StreamOwned::new(connection, stream);
                tls.write_all(&request).map_err(map_tls_error)?;
                let (status, head, payload) = read_response(&mut tls, self.max_body)?;
                return self.finish_response(status, &head, payload);
            }
            let mut stream = stream;
            stream.write_all(&request).map_err(map_io_error)?;
            let (status, head, payload) = read_response(&mut stream, self.max_body)?;
            return self.finish_response(status, &head, payload);
        }
        let bytes = self
            .exchange
            .as_mut()
            .expect("exchange checked")
            .exchange(&request)?;
        let mut reader = Cursor::new(bytes);
        let (status, head, payload) = read_response(&mut reader, self.max_body)?;
        self.finish_response(status, &head, payload)
    }
    fn finish_response(
        &self,
        status: u16,
        head: &str,
        payload_bytes: Vec<u8>,
    ) -> Result<HttpResponse, TransportError> {
        if (200..=299).contains(&status) && status != 204 {
            let content_type = head.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-type")
                    .then_some(value.trim().to_ascii_lowercase())
            });
            if !content_type
                .as_deref()
                .is_some_and(|value| value.starts_with("application/json"))
            {
                return Err(TransportError::Malformed);
            }
        }
        let payload = String::from_utf8(payload_bytes).map_err(|_| TransportError::Malformed)?;
        match status {
            200..=299 => Ok(HttpResponse {
                status,
                body: payload,
            }),
            401 => Err(TransportError::Auth),
            403 => Err(TransportError::Forbidden),
            404 => Err(TransportError::NotFound),
            409 => Err(TransportError::Conflict),
            429 => Err(TransportError::RateLimited),
            408 | 504 => Err(TransportError::Timeout),
            500..=599 => Err(TransportError::Unavailable),
            _ => Err(TransportError::Other(format!("HTTP {status}"))),
        }
    }
}

const MAX_HEADER_BYTES: usize = 64 * 1024;

fn read_response<R: Read>(
    reader: &mut R,
    max_body: usize,
) -> Result<(u16, String, Vec<u8>), TransportError> {
    let head = String::from_utf8(read_headers(reader)?).map_err(|_| TransportError::Malformed)?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(TransportError::Malformed)?;
    let mut content_length = None;
    let mut transfer_encoding = None;
    let mut connection_close = false;
    for line in lines {
        let (key, value) = line.split_once(':').ok_or(TransportError::Malformed)?;
        let value = value.trim();
        if key.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(TransportError::Malformed);
            }
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| TransportError::Malformed)?,
            );
        } else if key.eq_ignore_ascii_case("transfer-encoding") {
            if transfer_encoding.is_some() {
                return Err(TransportError::Malformed);
            }
            transfer_encoding = Some(value.to_ascii_lowercase());
        } else if key.eq_ignore_ascii_case("connection") {
            connection_close = value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"));
        }
    }
    let no_body_status = (100..200).contains(&status) || matches!(status, 204 | 304);
    let body = if no_body_status {
        if content_length.is_some_and(|length| length != 0) || transfer_encoding.is_some() {
            return Err(TransportError::Malformed);
        }
        Vec::new()
    } else if let Some(encoding) = transfer_encoding {
        if content_length.is_some()
            || !encoding
                .split(',')
                .next_back()
                .is_some_and(|value| value.trim() == "chunked")
        {
            return Err(TransportError::Malformed);
        }
        read_chunked_body(reader, max_body)?
    } else if let Some(length) = content_length {
        if length > max_body {
            return Err(TransportError::Malformed);
        }
        let mut body = vec![0; length];
        read_exact(reader, &mut body)?;
        body
    } else if connection_close {
        let mut body = Vec::new();
        reader
            .take(max_body as u64 + 1)
            .read_to_end(&mut body)
            .map_err(map_read_error)?;
        if body.len() > max_body {
            return Err(TransportError::Malformed);
        }
        body
    } else {
        return Err(TransportError::Malformed);
    };
    Ok((status, head, body))
}

fn read_headers<R: Read>(reader: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    loop {
        read_exact(reader, &mut byte)?;
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            bytes.truncate(bytes.len() - 4);
            return Ok(bytes);
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(TransportError::Malformed);
        }
    }
}

fn read_line<R: Read>(reader: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut line = Vec::new();
    let mut byte = [0; 1];
    loop {
        read_exact(reader, &mut byte)?;
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return Ok(line);
        }
        if line.len() > MAX_HEADER_BYTES {
            return Err(TransportError::Malformed);
        }
    }
}

fn read_chunked_body<R: Read>(reader: &mut R, max_body: usize) -> Result<Vec<u8>, TransportError> {
    let mut body = Vec::new();
    loop {
        let line = String::from_utf8(read_line(reader)?).map_err(|_| TransportError::Malformed)?;
        let size_text = line
            .split(';')
            .next()
            .ok_or(TransportError::Malformed)?
            .trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| TransportError::Malformed)?;
        if size == 0 {
            loop {
                if read_line(reader)?.is_empty() {
                    return Ok(body);
                }
            }
        }
        if body
            .len()
            .checked_add(size)
            .is_none_or(|length| length > max_body)
        {
            return Err(TransportError::Malformed);
        }
        let start = body.len();
        body.resize(start + size, 0);
        read_exact(reader, &mut body[start..])?;
        let mut crlf = [0; 2];
        read_exact(reader, &mut crlf)?;
        if crlf != *b"\r\n" {
            return Err(TransportError::Malformed);
        }
    }
}

fn read_exact<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), TransportError> {
    reader.read_exact(buffer).map_err(map_read_error)
}

fn map_read_error(error: std::io::Error) -> TransportError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        TransportError::Timeout
    } else {
        TransportError::Malformed
    }
}

fn map_io_error(error: std::io::Error) -> TransportError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        TransportError::Timeout
    } else {
        TransportError::Unavailable
    }
}

fn map_tls_error(error: std::io::Error) -> TransportError {
    map_io_error(error)
}

impl AnytypeTransport for HttpAnytypeTransport {
    fn list_objects(
        &mut self,
        space: &str,
        offset: Option<&str>,
    ) -> Result<Page<ObjectRecord>, TransportError> {
        let mut path = format!("/v1/spaces/{}/objects", encode_path_segment(space));
        if let Some(offset) = offset {
            path.push_str("?offset=");
            path.push_str(offset);
            path.push_str("&limit=100");
        }
        let response = self.request("GET", &path, None)?;
        let (data, next_offset) = wire::decode_list(&response.body, Some(space))?;
        Ok(Page { data, next_offset })
    }
    fn get_object(&mut self, space: &str, id: &str) -> Result<ObjectRecord, TransportError> {
        let response = self.request(
            "GET",
            &format!(
                "/v1/spaces/{}/objects/{}",
                encode_path_segment(space),
                encode_path_segment(id)
            ),
            None,
        )?;
        wire::decode_object(&response.body, Some(space))
    }
    fn create_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        let body = wire::encode_create(&object)?;
        let response = self.request(
            "POST",
            &format!(
                "/v1/spaces/{}/objects",
                encode_path_segment(&object.space_id)
            ),
            Some(&body),
        )?;
        wire::decode_object(&response.body, Some(&object.space_id))
    }
    fn update_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        let body = wire::encode_update(&object)?;
        let response = self.request(
            "PATCH",
            &format!(
                "/v1/spaces/{}/objects/{}",
                encode_path_segment(&object.space_id),
                encode_path_segment(&object.id)
            ),
            Some(&body),
        )?;
        wire::decode_object(&response.body, Some(&object.space_id))
    }
    fn archive_object(&mut self, space: &str, id: &str) -> Result<ObjectRecord, TransportError> {
        // Anytype's archive operation is represented by the documented
        // object DELETE. The returned tombstone is intentionally decoded as
        // an object so callers can update their rebuildable cache.
        let response = self.request(
            "DELETE",
            &format!(
                "/v1/spaces/{}/objects/{}",
                encode_path_segment(space),
                encode_path_segment(id)
            ),
            None,
        )?;
        if response.status == 204 {
            return Ok(ObjectRecord {
                id: id.into(),
                space_id: space.into(),
                properties: Vec::new(),
                property_formats: BTreeMap::new(),
                body: String::new(),
                archived: true,
                revision: 0,
            });
        }
        wire::decode_object(&response.body, Some(space))
    }
    fn delete_object(&mut self, space: &str, id: &str) -> Result<ObjectRecord, TransportError> {
        let response = self.request(
            "DELETE",
            &format!(
                "/v1/spaces/{}/objects/{}",
                encode_path_segment(space),
                encode_path_segment(id)
            ),
            None,
        )?;
        if response.status == 204 {
            return Ok(ObjectRecord {
                id: id.into(),
                space_id: space.into(),
                properties: Vec::new(),
                property_formats: BTreeMap::new(),
                body: String::new(),
                archived: true,
                revision: 0,
            });
        }
        wire::decode_object(&response.body, Some(space))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryBinding {
    pub domain_id: String,
    pub binding_fingerprint: String,
    pub upstream_account_fingerprint: String,
    pub space_id: String,
}

impl RepositoryBinding {
    pub fn from_domain(
        binding: &DomainBinding,
        upstream_account_fingerprint: &str,
    ) -> Result<Self, DomainBindingsError> {
        Ok(Self {
            domain_id: binding.domain_id.clone(),
            binding_fingerprint: binding.fingerprint(upstream_account_fingerprint)?,
            upstream_account_fingerprint: upstream_account_fingerprint.into(),
            space_id: binding.space_id.clone(),
        })
    }

    fn legacy(space_id: &str) -> Self {
        let bindings = DomainBindings::legacy_single_space(space_id, "contacts", "tasks")
            .expect("legacy repository space must form a valid binding");
        Self::from_domain(&bindings.bindings[0], "legacy-unscoped")
            .expect("legacy repository binding must be valid")
    }

    fn operation_id(&self, kind: &str, identity: &str) -> String {
        format!("{}:{kind}:{identity}", self.binding_fingerprint)
    }

    fn lock_key(&self, object_id: &str) -> String {
        format!("{}:{object_id}", self.binding_fingerprint)
    }

    fn accepts(&self, object: &ObjectRecord, expected_id: Option<&str>) -> bool {
        object.space_id == self.binding.space_id
            && expected_id.is_none_or(|expected| object.id == expected)
    }
}

/// Repository adapter. The in-memory repository is intentionally the durable
/// contract cache for now; syncing to Anytype is explicit and transport-driven.
pub struct AnytypeRepository<T> {
    pub transport: T,
    pub cache: MemoryRepository,
    pub binding: RepositoryBinding,
    pub property_write_mode: PropertyWriteMode,
    pub locks: ObjectLocks,
    pub reconciliation: ReconciliationPolicy,
    pub metrics: OperationMetrics,
    pub receipts: Vec<OperationReceipt>,
}
impl<T> AnytypeRepository<T> {
    pub fn new(transport: T, space_id: impl Into<String>) -> Self {
        let space_id = space_id.into();
        Self::with_binding(transport, RepositoryBinding::legacy(&space_id))
    }

    pub fn with_binding(transport: T, binding: RepositoryBinding) -> Self {
        Self {
            transport,
            cache: MemoryRepository::new(),
            binding,
            property_write_mode: PropertyWriteMode::CanonicalBodyOnly,
            locks: ObjectLocks::default(),
            reconciliation: ReconciliationPolicy::default(),
            metrics: OperationMetrics::default(),
            receipts: Vec::new(),
        }
    }

    pub fn with_reconciliation_policy(mut self, policy: ReconciliationPolicy) -> Self {
        self.reconciliation = ReconciliationPolicy {
            max_reads: policy.max_reads.max(1),
        };
        self
    }

    /// Enable the optional DAV-to-Anytype property projection.  The caller is
    /// responsible for creating/linking those property keys on the Anytype
    /// type first; canonical-body-only operation never needs that schema.
    pub fn with_property_write_mode(mut self, mode: PropertyWriteMode) -> Self {
        self.property_write_mode = mode;
        self
    }

    pub fn conflict_policy(&self) -> ConflictPolicy {
        ANYTYPE_CONFLICT_POLICY
    }

    fn receipt(
        &mut self,
        operation_id: impl Into<String>,
        object_id: impl Into<String>,
        kind: OperationKind,
        outcome: impl Into<String>,
        reconciliation_reads: u8,
    ) {
        if self.receipts.len() == MAX_RECEIPTS {
            self.receipts.remove(0);
        }
        self.receipts.push(OperationReceipt {
            operation_id: operation_id.into(),
            object_id: object_id.into(),
            kind,
            outcome: outcome.into(),
            reconciliation_reads,
        });
    }
}
impl<T: AnytypeTransport> Repository for AnytypeRepository<T> {
    fn list_collections(&mut self) -> Result<Vec<Collection>, RepositoryError> {
        self.hydrate()?;
        self.cache.list_collections()
    }
    fn get_collection(&mut self, id: &CollectionId) -> Result<Option<Collection>, RepositoryError> {
        self.hydrate()?;
        self.cache.get_collection(id)
    }
    fn create_collection(&mut self, collection: Collection) -> Result<(), RepositoryError> {
        self.cache.create_collection(collection)
    }
    fn delete_collection(&mut self, id: &CollectionId) -> Result<(), RepositoryError> {
        self.cache.delete_collection(id)
    }
    fn list_resources(
        &mut self,
        id: &CollectionId,
        archived: bool,
    ) -> Result<Vec<StoredResource>, RepositoryError> {
        self.hydrate()?;
        let mut rows = self.cache.list_resources(id, archived)?;
        if id.as_str() == "contacts" {
            rows.retain(|r| matches!(r.envelope.kind, DavKind::Contact));
        }
        if id.as_str() == "tasks" {
            rows.retain(|r| matches!(r.envelope.kind, DavKind::Task));
        }
        Ok(rows)
    }
    fn get_resource(&mut self, id: &ResourceId) -> Result<Option<StoredResource>, RepositoryError> {
        self.hydrate()?;
        let row = self.cache.get_resource(id)?;
        Ok(row.filter(|r| matches!(r.envelope.kind, DavKind::Contact | DavKind::Task)))
    }
    fn create_resource(
        &mut self,
        e: ResourceEnvelope,
        c: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        // Validate every local envelope, identity, and conditional-write
        // rule before the remote side effect.  A rejected DAV precondition
        // must never become a remote mutation followed by a local 412.
        let mut preflight = self.cache.clone();
        preflight
            .create_collection(Collection {
                id: e.collection_id.clone(),
                name: if e.kind == DavKind::Contact {
                    "Contacts".into()
                } else {
                    "Tasks".into()
                },
            })
            .or_else(|error| match error {
                RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                other => Err(other),
            })?;
        preflight.create_resource(e.clone(), c.clone())?;
        let result = self.push_create(e.clone())?;
        self.cache
            .create_collection(Collection {
                id: result.envelope.collection_id.clone(),
                name: if result.envelope.kind == DavKind::Contact {
                    "Contacts".into()
                } else {
                    "Tasks".into()
                },
            })
            .or_else(|error| match error {
                RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                other => Err(other),
            })?;
        self.cache.create_resource(result.envelope.clone(), c)?;
        Ok(result)
    }
    fn update_resource(
        &mut self,
        e: ResourceEnvelope,
        c: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        // Anytype's observed API ignores If-Match/revision preconditions;
        // `c` remains a local cache contract while the remote policy is
        // explicitly later-write-wins. Drift is recorded by the sync layer.
        self.hydrate()?;
        let mut preflight = self.cache.clone();
        preflight.update_resource(e.clone(), c.clone())?;
        let result = self.push_update(e.clone())?;
        self.cache
            .create_collection(Collection {
                id: result.envelope.collection_id.clone(),
                name: if result.envelope.kind == DavKind::Contact {
                    "Contacts".into()
                } else {
                    "Tasks".into()
                },
            })
            .or_else(|error| match error {
                RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                other => Err(other),
            })?;
        self.cache.update_resource(result.envelope.clone(), c)?;
        Ok(result)
    }
    fn archive_resource(
        &mut self,
        id: &ResourceId,
        c: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        self.hydrate()?;
        let mut preflight = self.cache.clone();
        preflight.archive_resource(id, c.clone())?;
        let current = self
            .cache
            .get_resource(id)?
            .ok_or(RepositoryError::ResourceNotFound(id.clone()))?;
        let object_id = current.envelope.anytype_object_id.to_string();
        let operation_id = self.binding.operation_id("archive", &object_id);
        let lock_key = self.binding.lock_key(&object_id);
        let locks = self.locks.clone();
        let ((), reads) = locks
            .with_lock(&lock_key, || {
                self.metrics.mutations = self.metrics.mutations.saturating_add(1);
                let expected = ObjectRecord {
                    id: object_id.clone(),
                    space_id: self.binding.space_id.clone(),
                    properties: Vec::new(),
                    property_formats: BTreeMap::new(),
                    body: String::new(),
                    archived: true,
                    revision: 0,
                };
                match self.transport.archive_object(&self.binding.space_id, &object_id) {
                    Ok(actual) if self.binding.accepts(&actual, Some(&object_id)) => self
                        .confirm_archive(&object_id, false)
                        .map(|(_, reads)| ((), reads)),
                    Ok(_) => Err(TransportError::Malformed),
                    Err(TransportError::Timeout) => {
                        self.metrics.ambiguous_mutations =
                            self.metrics.ambiguous_mutations.saturating_add(1);
                        self.reconcile_write(&expected, OperationKind::Archive)
                            .map(|(_, reads)| ((), reads))
                    }
                    Err(error) => Err(error),
                }
            })
            .map_err(Self::map_transport_error)?;
        self.receipt(
            operation_id,
            object_id,
            OperationKind::Archive,
            if reads > 0 { "reconciled" } else { "applied" },
            reads,
        );
        let result = self.cache.archive_resource(id, c)?;
        Ok(result)
    }
    fn delete_resource(
        &mut self,
        id: &ResourceId,
        c: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        self.hydrate()?;
        let mut preflight = self.cache.clone();
        preflight.delete_resource(id, c.clone())?;
        let current = self
            .cache
            .get_resource(id)?
            .ok_or(RepositoryError::ResourceNotFound(id.clone()))?;
        let object_id = current.envelope.anytype_object_id.to_string();
        let operation_id = self.binding.operation_id("delete", &object_id);
        let lock_key = self.binding.lock_key(&object_id);
        let locks = self.locks.clone();
        let ((), reads) = locks
            .with_lock(&lock_key, || {
                self.metrics.mutations = self.metrics.mutations.saturating_add(1);
                let expected = ObjectRecord {
                    id: object_id.clone(),
                    space_id: self.binding.space_id.clone(),
                    properties: Vec::new(),
                    property_formats: BTreeMap::new(),
                    body: String::new(),
                    archived: true,
                    revision: 0,
                };
                match self.transport.delete_object(&self.binding.space_id, &object_id) {
                    Ok(actual) if self.binding.accepts(&actual, Some(&object_id)) => self
                        .confirm_archive(&object_id, true)
                        .map(|(_, reads)| ((), reads)),
                    Ok(_) => Err(TransportError::Malformed),
                    Err(TransportError::Timeout) => {
                        self.metrics.ambiguous_mutations =
                            self.metrics.ambiguous_mutations.saturating_add(1);
                        self.reconcile_write(&expected, OperationKind::Delete)
                            .or_else(|error| match error {
                                TransportError::Timeout | TransportError::NotFound => {
                                    Ok((expected.clone(), 1))
                                }
                                other => Err(other),
                            })
                            .map(|(_, reads)| ((), reads))
                    }
                    Err(error) => Err(error),
                }
            })
            .map_err(Self::map_transport_error)?;
        self.receipt(
            operation_id,
            object_id,
            OperationKind::Delete,
            if reads > 0 { "reconciled" } else { "applied" },
            reads,
        );
        self.cache.delete_resource(id, c)
    }
}
impl<T: AnytypeTransport> AnytypeRepository<T> {
    fn hydrate(&mut self) -> Result<(), RepositoryError> {
        // Collection definitions are local configuration, not Anytype
        // objects. Preserve them while replacing the resource snapshot so an
        // empty remote Space still exposes the configured DAV home/collection
        // routes instead of turning collection discovery into a 404.
        let configured_collections = self.cache.list_collections()?;
        let mut cursor = None;
        let mut staged = Vec::new();
        loop {
            let page = self
                .transport
                .list_objects(&self.binding.space_id, cursor.as_deref())
                .map_err(Self::map_transport_error)?;
            for listed in page.data {
                if !self.binding.accepts(&listed, None) {
                    return Err(RepositoryError::MalformedState);
                }
                // API 2025-11-08 list responses intentionally contain the
                // summary Object shape, not the full markdown body. Fetch
                // the full object before attempting to hydrate our opaque
                // canonical envelope.
                let object = if listed.body.is_empty() && !listed.id.is_empty() {
                    self.transport
                        .get_object(&self.binding.space_id, &listed.id)
                        .map_err(Self::map_transport_error)?
                } else {
                    listed
                };
                if !self.binding.accepts(&object, None) {
                    return Err(RepositoryError::MalformedState);
                }
                // A Space can contain ordinary Anytype notes/pages alongside
                // DAV resources.  They are outside this adapter's namespace
                // and must not make DAV discovery fail.  Once a body carries
                // one of the envelope identity keys, malformed JSON is
                // treated as a real adapter-state error below.
                if !looks_like_envelope(&object.body) {
                    continue;
                }
                let mut envelope = ResourceEnvelope::from_json(&object.body)
                    .map_err(|error| RepositoryError::InvalidEnvelope(error.to_string()))?;
                // The current adapter profile exposes only CardDAV contacts
                // and CalDAV VTODOs. Calendar events and contact groups are
                // retained in Anytype but are not part of this DAV view.
                if !matches!(envelope.kind, DavKind::Contact | DavKind::Task) {
                    continue;
                }
                // Anytype's documented Object/ObjectWithBody schemas do not
                // expose a revision field. A non-zero value is accepted only
                // for our legacy/fake transport fixtures; zero means
                // "unknown", not a conflict with the DAV envelope revision.
                if envelope.collection_id.as_str().is_empty()
                    || (object.revision != 0 && envelope.revision != object.revision)
                {
                    return Err(RepositoryError::MalformedState);
                }
                let remote_id = AnytypeObjectId::try_from(object.id.clone())
                    .map_err(|_| RepositoryError::MalformedState)?;
                if envelope.anytype_object_id != remote_id {
                    // A create request cannot know the server-generated
                    // object ID, so its first stored markdown may contain
                    // the provisional DAV-derived ID. The projected DAV
                    // markers are the durable mapping contract; once they
                    // match, the remote object ID is authoritative.
                    if !remote_identity_matches(&object, &envelope) {
                        return Err(RepositoryError::MalformedState);
                    }
                    envelope.anytype_object_id = remote_id;
                }
                validate_properties(&object, &envelope)?;
                let collection = envelope.collection_id.clone();
                let name = if envelope.kind == DavKind::Contact {
                    "Contacts"
                } else if envelope.kind == DavKind::Task {
                    "Tasks"
                } else {
                    "DAV"
                };
                staged.push((collection, name.to_string(), envelope));
            }
            cursor = page.next_offset;
            if cursor.is_none() {
                break;
            }
        }
        let mut refreshed = MemoryRepository::new();
        for (collection, name, envelope) in staged {
            refreshed
                .create_collection(Collection {
                    id: collection,
                    name,
                })
                .or_else(|error| match error {
                    RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                    other => Err(other),
                })?;
            refreshed.create_resource(envelope, WriteCondition::Unconditional)?;
        }
        // Swap only after every page, envelope, identity, and property has
        // validated successfully: failed refreshes cannot damage a populated
        // cache, and successful refreshes remove stale/archived rows.
        self.cache = refreshed;
        for collection in configured_collections {
            self.cache
                .create_collection(collection)
                .or_else(|error| match error {
                    RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                    other => Err(other),
                })?;
        }
        Ok(())
    }
    /// Convert remote failures while retaining the typed remote category.
    pub fn map_transport_error(error: TransportError) -> RepositoryError {
        match error {
            TransportError::NotFound => {
                RepositoryError::ResourceNotFound(ResourceId::try_from("remote").unwrap())
            }
            TransportError::Conflict => RepositoryError::IdentityAlreadyExists("remote".into()),
            TransportError::Timeout => RepositoryError::Timeout,
            TransportError::Unavailable => RepositoryError::Unavailable,
            TransportError::RateLimited => RepositoryError::RateLimited,
            TransportError::DelayedVisibility => RepositoryError::ReadAfterWriteDelay,
            TransportError::Malformed => RepositoryError::MalformedState,
            TransportError::Auth => RepositoryError::Auth,
            TransportError::Forbidden => RepositoryError::Forbidden,
            TransportError::InvalidRequest(reason) | TransportError::Other(reason) => {
                RepositoryError::InvalidEnvelope(reason)
            }
        }
    }
    fn push_create(
        &mut self,
        mut envelope: ResourceEnvelope,
    ) -> Result<StoredResource, RepositoryError> {
        let object = self.object(&envelope, false)?;
        let provisional_id = object.id.clone();
        let operation_id = self
            .binding
            .operation_id("create", envelope.dav_uid.as_str());
        let lock_key = self.binding.lock_key(&provisional_id);
        let locks = self.locks.clone();
        let (remote, reads) = locks
            .with_lock(&lock_key, || {
                self.metrics.mutations = self.metrics.mutations.saturating_add(1);
                match self.transport.create_object(object.clone()) {
                    Ok(remote) => Ok((remote, 0)),
                    Err(TransportError::Timeout) => {
                        self.metrics.ambiguous_mutations =
                            self.metrics.ambiguous_mutations.saturating_add(1);
                        self.reconcile_create(&object)
                    }
                    Err(error) => Err(error),
                }
            })
            .map_err(Self::map_transport_error)?;
        if remote.space_id != self.binding.space_id || remote.id.trim().is_empty() {
            return Err(RepositoryError::MalformedState);
        }
        envelope.anytype_object_id = AnytypeObjectId::try_from(remote.id.clone())
            .map_err(|_| RepositoryError::MalformedState)?;
        self.receipt(
            operation_id,
            remote.id,
            OperationKind::Create,
            if reads > 0 { "reconciled" } else { "applied" },
            reads,
        );
        let etag = typed_etag_for_bytes(self.object_body(&envelope)?.as_bytes());
        Ok(StoredResource {
            envelope,
            etag,
            archived: false,
            modified_at: ModifiedAt::now(),
        })
    }
    fn push_update(
        &mut self,
        envelope: ResourceEnvelope,
    ) -> Result<StoredResource, RepositoryError> {
        let mut object = self.object(&envelope, false)?;
        let object_id = object.id.clone();
        let operation_id = self.binding.operation_id("update", &object_id);
        let lock_key = self.binding.lock_key(&object_id);
        let locks = self.locks.clone();
        let ((), reads) = locks.with_lock(&lock_key, || {
            // Unknown Anytype properties are outside the DAV projection but must
            // survive a DAV edit. Fetch and carry them forward atomically with the
            // remote update rather than rebuilding the object from the envelope.
            match self.transport.get_object(&self.binding.space_id, &object.id) {
                Ok(existing) => {
                    if !self.binding.accepts(&existing, Some(&object.id)) {
                        return Err(RepositoryError::MalformedState);
                    }
                    let projected = self.property_write_mode == PropertyWriteMode::Projected;
                    let formats = existing.property_formats;
                    for (key, value) in existing.properties {
                        if is_reserved_system_property(&key)
                            || (projected
                                && (key == "dav_uid"
                                    || key == "dav_kind"
                                    || key.starts_with("dav_property_")
                                    || key.starts_with("dav.property.")))
                        {
                            continue;
                        }
                        object.properties.push((key.clone(), value));
                        if let Some(format) = formats.get(&key) {
                            object.property_formats.insert(key, format.clone());
                        }
                    }
                }
                Err(TransportError::NotFound) => {}
                Err(error) => return Err(Self::map_transport_error(error)),
            }
            self.metrics.mutations = self.metrics.mutations.saturating_add(1);
            match self.transport.update_object(object.clone()) {
                Ok(actual) if self.binding.accepts(&actual, Some(&object.id)) => Ok(((), 0)),
                Ok(_) => Err(RepositoryError::MalformedState),
                Err(TransportError::Timeout) => {
                    self.metrics.ambiguous_mutations =
                        self.metrics.ambiguous_mutations.saturating_add(1);
                    self.reconcile_write(&object, OperationKind::Update)
                        .map(|(_, reads)| ((), reads))
                        .map_err(Self::map_transport_error)
                }
                Err(error) => Err(Self::map_transport_error(error)),
            }
        })?;
        self.receipt(
            operation_id,
            object_id,
            OperationKind::Update,
            if reads > 0 { "reconciled" } else { "applied" },
            reads,
        );
        let etag = typed_etag_for_bytes(self.object_body(&envelope)?.as_bytes());
        Ok(StoredResource {
            envelope,
            etag,
            archived: false,
            modified_at: ModifiedAt::now(),
        })
    }

    /// Resolve an ambiguous mutation by reading the same stable object ID a
    /// bounded number of times. A timeout never causes a blind mutation
    /// retry: a matching object proves the write committed.
    fn reconcile_write(
        &mut self,
        expected: &ObjectRecord,
        kind: OperationKind,
    ) -> Result<(ObjectRecord, u8), TransportError> {
        let mut reads: u8 = 0;
        for _ in 0..self.reconciliation.max_reads {
            reads = reads.saturating_add(1);
            match self.transport.get_object(&expected.space_id, &expected.id) {
                Ok(actual) if mutation_matches(expected, &actual, kind) => {
                    self.metrics.reconciled_mutations =
                        self.metrics.reconciled_mutations.saturating_add(1);
                    return Ok((actual, reads));
                }
                Ok(_)
                | Err(TransportError::NotFound)
                | Err(TransportError::DelayedVisibility)
                | Err(TransportError::Timeout) => {}
                Err(error) => return Err(error),
            }
        }
        self.metrics.failed_reconciliations = self.metrics.failed_reconciliations.saturating_add(1);
        Err(TransportError::Timeout)
    }

    /// Resolve an ambiguous create without assuming the provisional DAV UID
    /// is an Anytype object ID. The API generates the real ID server-side, so
    /// scan the bounded list pages for the stable DAV identity projection.
    fn reconcile_create(
        &mut self,
        expected: &ObjectRecord,
    ) -> Result<(ObjectRecord, u8), TransportError> {
        let mut cursor = None;
        let mut reads: u8 = 0;
        for _ in 0..self.reconciliation.max_reads {
            reads = reads.saturating_add(1);
            let page = self
                .transport
                .list_objects(&expected.space_id, cursor.as_deref())?;
            let mut candidate = None;
            for listed in page.data {
                let actual = if listed.body.is_empty() && !listed.id.is_empty() {
                    self.transport.get_object(&expected.space_id, &listed.id)?
                } else {
                    listed
                };
                if create_identity_matches(expected, &actual) {
                    candidate = Some(actual);
                    break;
                }
            }
            if let Some(actual) = candidate {
                self.metrics.reconciled_mutations =
                    self.metrics.reconciled_mutations.saturating_add(1);
                return Ok((actual, reads));
            }
            cursor = page.next_offset;
            if cursor.is_none() {
                break;
            }
        }
        self.metrics.failed_reconciliations = self.metrics.failed_reconciliations.saturating_add(1);
        Err(TransportError::Timeout)
    }

    fn confirm_archive(
        &mut self,
        object_id: &str,
        allow_missing: bool,
    ) -> Result<(ObjectRecord, u8), TransportError> {
        let mut reads: u8 = 0;
        for _ in 0..self.reconciliation.max_reads {
            reads = reads.saturating_add(1);
            match self.transport.get_object(&self.binding.space_id, object_id) {
                Ok(actual)
                    if self.binding.accepts(&actual, Some(object_id)) && actual.archived =>
                {
                    self.metrics.archive_confirmations =
                        self.metrics.archive_confirmations.saturating_add(1);
                    return Ok((actual, reads));
                }
                Ok(actual) if !self.binding.accepts(&actual, Some(object_id)) => {
                    return Err(TransportError::Malformed);
                }
                Ok(_) | Err(TransportError::DelayedVisibility) | Err(TransportError::Timeout) => {}
                Err(TransportError::NotFound) if allow_missing => {
                    self.metrics.archive_confirmations =
                        self.metrics.archive_confirmations.saturating_add(1);
                    return Ok((
                        ObjectRecord {
                            id: object_id.into(),
                            space_id: self.binding.space_id.clone(),
                            properties: Vec::new(),
                            property_formats: BTreeMap::new(),
                            body: String::new(),
                            archived: true,
                            revision: 0,
                        },
                        reads,
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        self.metrics.failed_reconciliations = self.metrics.failed_reconciliations.saturating_add(1);
        Err(TransportError::Timeout)
    }
    fn object_body(&self, envelope: &ResourceEnvelope) -> Result<String, RepositoryError> {
        envelope
            .canonical_json()
            .map_err(|e| RepositoryError::InvalidEnvelope(e.to_string()))
    }
    fn object(
        &self,
        envelope: &ResourceEnvelope,
        archived: bool,
    ) -> Result<ObjectRecord, RepositoryError> {
        Ok(ObjectRecord {
            id: envelope.anytype_object_id.to_string(),
            space_id: self.binding.space_id.clone(),
            properties: match self.property_write_mode {
                PropertyWriteMode::CanonicalBodyOnly => Vec::new(),
                PropertyWriteMode::Projected => projected_properties(envelope),
            },
            property_formats: BTreeMap::new(),
            body: self.object_body(envelope)?,
            archived,
            revision: envelope.revision,
        })
    }
}

/// The object response includes read-only system properties. Anytype rejects
/// sending those keys in an object PATCH, while custom properties are valid
/// and must survive a DAV edit. This list is source-backed by the current
/// API's live error response and intentionally remains separate from the
/// caller's mutable/custom property namespace.
fn is_reserved_system_property(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "backlinks"
            | "creator"
            | "created_date"
            | "last_modified_date"
            | "last_modified_by"
            | "links"
    )
}

/// The envelope is authoritative and remains opaque, while these stable
/// properties make the useful DAV fields searchable/editable in Anytype.
/// Unknown DAV fields stay in the envelope and are also represented here by
/// deterministic snake_case dav_property_<name>_<occurrence> keys.
fn projected_properties(envelope: &ResourceEnvelope) -> Vec<(String, String)> {
    let mut properties = vec![
        ("dav_uid".into(), envelope.dav_uid.to_string()),
        ("dav_kind".into(), kind_name(&envelope.kind).into()),
    ];
    for (name, values) in &envelope.document.content.fields {
        for (index, occurrence) in values.iter().enumerate() {
            properties.push((
                format!("dav_property_{}_{}", property_key_segment(name), index),
                occurrence.value.clone(),
            ));
        }
    }
    properties
}

fn property_key_segment(name: &str) -> String {
    let mut segment = String::new();
    let mut previous_separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            segment.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            segment.push('_');
            previous_separator = true;
        }
    }
    let segment = segment.trim_matches('_');
    if segment.is_empty() {
        "field".into()
    } else {
        segment.into()
    }
}

/// Return the exact stable property projection used for Anytype writes.
pub fn projected_anytype_properties(envelope: &ResourceEnvelope) -> Vec<(String, String)> {
    projected_properties(envelope)
}

fn mutation_matches(expected: &ObjectRecord, actual: &ObjectRecord, kind: OperationKind) -> bool {
    if expected.id != actual.id || expected.space_id != actual.space_id {
        return false;
    }
    match kind {
        OperationKind::Create | OperationKind::Update => {
            !actual.archived
                && actual.body == expected.body
                && actual.properties == expected.properties
        }
        OperationKind::Archive | OperationKind::Delete => actual.archived,
    }
}

fn create_identity_matches(expected: &ObjectRecord, actual: &ObjectRecord) -> bool {
    if actual.space_id != expected.space_id || actual.archived {
        return false;
    }
    let expected_markers = expected
        .properties
        .iter()
        .filter(|(key, _)| key == "dav_uid" || key == "dav_kind")
        .collect::<Vec<_>>();
    if !expected_markers.is_empty() {
        return expected_markers.iter().all(|marker| {
            actual
                .properties
                .iter()
                .any(|candidate| candidate == *marker)
        });
    }
    if actual.body == expected.body && actual.properties == expected.properties {
        return true;
    }
    canonical_identity_matches(&expected.body, &actual.body)
}

fn remote_identity_matches(object: &ObjectRecord, envelope: &ResourceEnvelope) -> bool {
    let markers_match = [
        ("dav_uid", envelope.dav_uid.to_string()),
        ("dav_kind", kind_name(&envelope.kind).to_string()),
    ]
    .iter()
    .all(|marker| {
        object
            .properties
            .iter()
            .any(|candidate| candidate.0 == marker.0 && candidate.1 == marker.1)
    });
    markers_match
        || canonical_identity_matches(&envelope.canonical_json().unwrap_or_default(), &object.body)
}

fn canonical_identity_matches(expected_body: &str, actual_body: &str) -> bool {
    let Ok(expected) = ResourceEnvelope::from_json(expected_body) else {
        return false;
    };
    let Ok(actual) = ResourceEnvelope::from_json(actual_body) else {
        return false;
    };
    expected.collection_id == actual.collection_id
        && expected.resource_id == actual.resource_id
        && expected.kind == actual.kind
        && expected.dav_uid == actual.dav_uid
}

fn looks_like_envelope(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| {
            [
                "collection_id",
                "resource_id",
                "kind",
                "dav_uid",
                "document",
            ]
            .iter()
            .any(|key| object.contains_key(*key))
        })
}

fn kind_name(kind: &DavKind) -> &'static str {
    match kind {
        DavKind::Contact => "Contact",
        DavKind::ContactGroup => "ContactGroup",
        DavKind::Task => "Task",
        DavKind::Event => "Event",
    }
}

fn validate_properties(
    object: &ObjectRecord,
    envelope: &ResourceEnvelope,
) -> Result<(), RepositoryError> {
    let properties: BTreeMap<_, _> = object.properties.iter().cloned().collect();
    for (key, expected) in [
        ("dav_uid", envelope.dav_uid.to_string()),
        ("dav_kind", kind_name(&envelope.kind).to_string()),
    ] {
        if let Some(actual) = properties.get(key) {
            if actual != &expected {
                return Err(RepositoryError::MalformedState);
            }
        }
    }
    for (key, expected) in projected_properties(envelope) {
        if let Some(actual) = properties.get(&key) {
            if actual != &expected {
                return Err(RepositoryError::MalformedState);
            }
        }
    }
    Ok(())
}
