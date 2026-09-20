//! Bounded, secret-safe operational events and recovery artifacts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_MESSAGE: usize = 256;
pub const MAX_FIELD: usize = 128;
pub const MAX_REPORT_EVENTS: usize = 128;
pub const MAX_AUDIT_READBACK: usize = 64;
pub const MAX_RETAINED_EVENTS: usize = 512;
const AUDIT_ACTIVE_FILE: &str = "active.jsonl";
const AUDIT_SEGMENT_PREFIX: &str = "segment-";
const AUDIT_SEGMENT_SUFFIX: &str = ".jsonl";
const AUDIT_WRITER_LOCK_FILE: &str = ".writer.lock";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCategory {
    Config,
    Transport,
    Auth,
    Protocol,
    Anytype,
    Timeout,
    Conflict,
    Recovery,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    Io,
    InvalidPath,
    Recovery,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub timestamp_ms: u64,
    pub request_id: Option<String>,
    pub sync_id: Option<String>,
    pub kind: String,
    pub category: Option<ErrorCategory>,
    pub message: String,
    pub duration_ms: Option<u64>,
    pub error_code: Option<ErrorCode>,
    pub correlation_id: Option<String>,
}

impl Event {
    pub fn new(
        kind: impl Into<String>,
        category: Option<ErrorCategory>,
        message: impl AsRef<str>,
    ) -> Self {
        Self {
            timestamp_ms: now_ms(),
            request_id: None,
            sync_id: None,
            kind: bounded_field(kind.into()),
            category,
            message: redact(message.as_ref()),
            duration_ms: None,
            error_code: None,
            correlation_id: None,
        }
    }
    pub fn request(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(bounded_field(id.into()));
        self
    }
    pub fn sync(mut self, id: impl Into<String>) -> Self {
        self.sync_id = Some(bounded_field(id.into()));
        self
    }
    pub fn duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }
    pub fn error_code(mut self, code: ErrorCode) -> Self {
        self.error_code = Some(code);
        self
    }
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(bounded_field(id.into()));
        self
    }
}

/// Correlation carried across HTTP, repository, and sync lifecycle hooks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Correlation {
    pub request_id: Option<String>,
    pub sync_id: Option<String>,
}

impl Correlation {
    pub fn event(
        &self,
        kind: impl Into<String>,
        category: Option<ErrorCategory>,
        message: &str,
    ) -> Event {
        let mut event = Event::new(kind, category, message);
        if let Some(id) = &self.request_id {
            event = event.request(id.clone());
        }
        if let Some(id) = &self.sync_id {
            event = event.sync(id.clone());
        }
        event
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureMode {
    Timeout,
    Auth,
    Malformed,
    Archive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Retry,
    Reauthenticate,
    Reconcile,
    Abort,
}

pub fn recovery_action(mode: FailureMode) -> RecoveryAction {
    match mode {
        FailureMode::Timeout => RecoveryAction::Retry,
        FailureMode::Auth => RecoveryAction::Reauthenticate,
        FailureMode::Malformed | FailureMode::Archive => RecoveryAction::Reconcile,
    }
}

#[derive(Clone, Debug, Default)]
pub struct EventBuffer {
    events: Vec<Event>,
}

/// A durable, local-only journal for already-redacted operational events.
///
/// The journal intentionally has no automatic retention policy. Each append
/// rewrites the active segment through a temporary file and rename, so a
/// process crash leaves either the previous complete segment or the new one.
/// An incomplete final line from an older implementation/crash is treated as
/// a recoverable tail; malformed interior records are rejected.
#[derive(Clone, Debug)]
pub struct AuditEventStore {
    directory: PathBuf,
}

/// A bounded, operator-facing projection of one durable event.  This is
/// intentionally not the stored event: messages, resource identifiers, and
/// credentials are never returned by the readback contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditSummary {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub kind: String,
    pub category: Option<ErrorCategory>,
    pub operation: Option<String>,
    pub status: Option<String>,
    pub correlation_digest: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditReadbackQuery {
    pub after_sequence: Option<u64>,
    pub limit: usize,
}

impl AuditReadbackQuery {
    pub fn validate(&self) -> io::Result<()> {
        if self.limit == 0 || self.limit > MAX_AUDIT_READBACK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit readback limit is out of bounds",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredEvent {
    sequence: u64,
    event: Event,
}

impl AuditEventStore {
    /// Open or create a private audit directory. This is explicit and opt-in.
    pub fn open(directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        if path_has_symlink_component(&directory)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "symlink audit path is refused",
            ));
        }
        fs::create_dir_all(&directory)?;
        set_private_directory_mode(&directory)?;
        let store = Self { directory };
        store.recover_active()?;
        Ok(store)
    }

    fn active_path(&self) -> PathBuf {
        self.directory.join(AUDIT_ACTIVE_FILE)
    }

    fn records_in(&self, path: &Path) -> io::Result<Vec<StoredEvent>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(path)?;
        let mut records = Vec::new();
        let mut offset = 0;
        for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            let is_final = offset + line.len() == bytes.len();
            offset += line.len() + 1;
            if line.is_empty() && is_final {
                continue;
            }
            match serde_json::from_slice::<StoredEvent>(line) {
                Ok(record) => records.push(record),
                Err(error) if is_final && error.is_eof() => {
                    // Only an actually incomplete JSON value can be a torn
                    // final append. A complete but malformed final line is
                    // corruption and must fail closed rather than silently
                    // dropping an audit record.
                    break;
                }
                Err(error) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("audit record {index}: {error}"),
                    ));
                }
            }
        }
        Ok(records)
    }

    fn all_records(&self) -> io::Result<Vec<StoredEvent>> {
        let mut paths = fs::read_dir(&self.directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| {
                        name == AUDIT_ACTIVE_FILE
                            || (name.starts_with(AUDIT_SEGMENT_PREFIX)
                                && name.ends_with(AUDIT_SEGMENT_SUFFIX))
                    })
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut records = Vec::new();
        for path in paths {
            records.extend(self.records_in(&path)?);
        }
        records.sort_by_key(|record| record.sequence);
        Self::validate_sequence(&records)?;
        Ok(records)
    }

    fn validate_sequence(records: &[StoredEvent]) -> io::Result<()> {
        for pair in records.windows(2) {
            if pair[1].sequence != pair[0].sequence.saturating_add(1) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "audit sequence is not contiguous",
                ));
            }
        }
        Ok(())
    }

    fn write_records_atomic(&self, path: &Path, records: &[StoredEvent]) -> io::Result<()> {
        let mut bytes = Vec::new();
        for record in records {
            serde_json::to_writer(&mut bytes, record)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
        }
        let tmp = temp_path(path);
        fs::write(&tmp, bytes)?;
        set_private_mode(&tmp)?;
        sync_file(&tmp)?;
        fs::rename(&tmp, path)?;
        set_private_mode(path)?;
        sync_parent(path)
    }

    fn recover_active(&self) -> io::Result<()> {
        let path = self.active_path();
        let records = self.records_in(&path)?;
        Self::validate_sequence(&records)?;
        if path.exists() {
            self.write_records_atomic(&path, &records)?;
        }
        Ok(())
    }

    /// Append a pre-redacted event and return its durable sequence number.
    pub fn append(&self, mut event: Event) -> io::Result<u64> {
        // Re-apply the same bounds/redaction at the persistence boundary so
        // callers cannot bypass Event::new with a raw struct.
        event.kind = bounded_field(event.kind);
        event.message = redact(&event.message);
        event.request_id = event.request_id.map(bounded_field);
        event.sync_id = event.sync_id.map(bounded_field);
        event.correlation_id = event.correlation_id.map(bounded_field);
        let mut records = self.all_records()?;
        let sequence = records
            .iter()
            .map(|record| record.sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        records.push(StoredEvent { sequence, event });
        let active = self.active_path();
        let active_records = self.records_in(&active)?;
        let mut updated = active_records;
        updated.push(records.last().cloned().expect("record appended"));
        self.write_records_atomic(&active, &updated)?;
        Ok(sequence)
    }

    /// Return all records in sequence order as deterministic JSONL.
    pub fn export_jsonl(&self) -> io::Result<String> {
        let mut output = String::new();
        for record in self.all_records()? {
            let line = serde_json::to_string(&record)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            output.push_str(&line);
            output.push('\n');
        }
        Ok(output)
    }

    /// Explicitly close the active segment. No records are deleted.
    pub fn rollover(&self) -> io::Result<Option<PathBuf>> {
        let active = self.active_path();
        let records = self.records_in(&active)?;
        if records.is_empty() {
            return Ok(None);
        }
        let next = fs::read_dir(&self.directory)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_prefix(AUDIT_SEGMENT_PREFIX)
                    .and_then(|value| value.strip_suffix(AUDIT_SEGMENT_SUFFIX))
                    .and_then(|value| value.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let segment = self.directory.join(format!(
            "{AUDIT_SEGMENT_PREFIX}{next:020}{AUDIT_SEGMENT_SUFFIX}"
        ));
        fs::rename(&active, &segment)?;
        set_private_mode(&segment)?;
        sync_parent(&segment)?;
        Ok(Some(segment))
    }

    pub fn event_count(&self) -> io::Result<usize> {
        Ok(self.all_records()?.len())
    }

    /// Read only a bounded, redacted projection after an optional sequence.
    pub fn readback(&self, query: &AuditReadbackQuery) -> io::Result<Vec<AuditSummary>> {
        query.validate()?;
        self.all_records()?
            .into_iter()
            .filter(|record| {
                query
                    .after_sequence
                    .is_none_or(|after| record.sequence > after)
            })
            .take(query.limit)
            .map(|record| Ok(summary(record)))
            .collect()
    }
}

fn summary(record: StoredEvent) -> AuditSummary {
    let operation = bounded_token(&record.event.message, "operation");
    let status = bounded_token(&record.event.message, "status");
    let correlation_digest = record
        .event
        .request_id
        .as_deref()
        .or(record.event.sync_id.as_deref())
        .or(record.event.correlation_id.as_deref())
        .map(digest_correlation);
    AuditSummary {
        sequence: record.sequence,
        timestamp_ms: record.event.timestamp_ms,
        kind: record.event.kind,
        category: record.event.category,
        operation,
        status,
        correlation_digest,
    }
}

fn bounded_token(message: &str, key: &str) -> Option<String> {
    message
        .split_whitespace()
        .find_map(|part| part.strip_prefix(&format!("{key}=")))
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_FIELD
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
                })
        })
        .map(str::to_owned)
}

fn digest_correlation(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::from("sha256:");
    for byte in digest.iter().take(12) {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

/// A bounded, process-exclusive writer for the local audit journal.
///
/// Producers submit already-redactable `Event` values through a bounded
/// channel. Only the worker touches the persistence primitive, which gives
/// each accepted event one linearization point and prevents concurrent
/// read/rename races in `AuditEventStore`.
pub struct AuditEventWriter {
    sender: Option<SyncSender<WriterCommand>>,
    join: Option<JoinHandle<io::Result<()>>>,
    lock_path: PathBuf,
    health: Arc<Mutex<WriterHealth>>,
}

/// Operational state of the optional local audit journal.  This state is
/// deliberately coarse: callers can decide whether a failed audit guarantee
/// should fail a request, without exposing filesystem paths or raw I/O text.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditHealthState {
    #[default]
    Healthy,
    Degraded,
    Unavailable,
}

/// Stable, secret-free classes for audit persistence failures.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditFailureClass {
    Permission,
    Corrupt,
    Locked,
    Missing,
    Io,
    Queue,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditOperation {
    Open,
    Append,
    Checkpoint,
    Export,
    Lock,
}

/// Classify an audit-store error without retaining its path or message.
pub fn classify_audit_failure(operation: AuditOperation, error: &io::Error) -> AuditFailureClass {
    match error.kind() {
        io::ErrorKind::PermissionDenied => AuditFailureClass::Permission,
        io::ErrorKind::InvalidData => AuditFailureClass::Corrupt,
        io::ErrorKind::AlreadyExists if matches!(operation, AuditOperation::Lock) => {
            AuditFailureClass::Locked
        }
        io::ErrorKind::WouldBlock
            if matches!(operation, AuditOperation::Open | AuditOperation::Lock) =>
        {
            AuditFailureClass::Locked
        }
        io::ErrorKind::NotFound => AuditFailureClass::Missing,
        _ => AuditFailureClass::Io,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterHealth {
    pub accepted: u64,
    pub persisted: u64,
    pub failed: u64,
    pub queued: usize,
    pub queue_capacity: usize,
    pub last_sequence: Option<u64>,
    pub running: bool,
    pub state: AuditHealthState,
    pub last_failure: Option<AuditFailureClass>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditCheckpoint {
    pub event_count: usize,
    pub last_sequence: Option<u64>,
}

enum WriterCommand {
    Append(Event, mpsc::Sender<io::Result<u64>>),
    Export(mpsc::Sender<io::Result<String>>),
    Checkpoint(mpsc::Sender<io::Result<AuditCheckpoint>>),
    Readback(
        AuditReadbackQuery,
        mpsc::Sender<io::Result<Vec<AuditSummary>>>,
    ),
    Shutdown(mpsc::Sender<io::Result<()>>),
}

impl AuditEventWriter {
    /// Open a process-exclusive writer with a bounded queue.
    pub fn open(directory: impl Into<PathBuf>, queue_capacity: usize) -> io::Result<Self> {
        if queue_capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit writer queue capacity must be positive",
            ));
        }
        let directory = directory.into();
        let store = AuditEventStore::open(&directory)?;
        let lock_path = directory.join(AUDIT_WRITER_LOCK_FILE);
        acquire_writer_lock(&lock_path)?;
        let health = Arc::new(Mutex::new(WriterHealth {
            queue_capacity,
            running: true,
            ..WriterHealth::default()
        }));
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let worker_health = Arc::clone(&health);
        let join = thread::spawn(move || writer_loop(store, receiver, worker_health));
        Ok(Self {
            sender: Some(sender),
            join: Some(join),
            lock_path,
            health,
        })
    }

    /// Submit an event, applying bounded backpressure until the worker accepts it.
    pub fn append(&self, event: Event) -> io::Result<u64> {
        let sender = self.sender.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "audit writer is shut down")
        })?;
        let (reply, result) = mpsc::channel();
        {
            let mut health = self.health.lock().expect("writer health mutex poisoned");
            health.accepted = health.accepted.saturating_add(1);
            health.queued = health.queued.saturating_add(1);
        }
        if let Err(error) = sender.send(WriterCommand::Append(event, reply)) {
            let mut health = self.health.lock().expect("writer health mutex poisoned");
            health.accepted = health.accepted.saturating_sub(1);
            health.queued = health.queued.saturating_sub(1);
            health.failed = health.failed.saturating_add(1);
            health.state = AuditHealthState::Degraded;
            health.last_failure = Some(AuditFailureClass::Queue);
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()));
        }
        result
            .recv()
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?
    }

    pub fn export_jsonl(&self) -> io::Result<String> {
        let sender = self.sender.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "audit writer is shut down")
        })?;
        let (reply, result) = mpsc::channel();
        sender
            .send(WriterCommand::Export(reply))
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?;
        result
            .recv()
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?
    }

    pub fn checkpoint(&self) -> io::Result<AuditCheckpoint> {
        let sender = self.sender.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "audit writer is shut down")
        })?;
        let (reply, result) = mpsc::channel();
        sender
            .send(WriterCommand::Checkpoint(reply))
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?;
        result
            .recv()
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?
    }

    pub fn readback(&self, query: AuditReadbackQuery) -> io::Result<Vec<AuditSummary>> {
        query.validate()?;
        let sender = self.sender.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "audit writer is shut down")
        })?;
        let (reply, result) = mpsc::channel();
        sender
            .send(WriterCommand::Readback(query, reply))
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?;
        result
            .recv()
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?
    }

    pub fn health(&self) -> WriterHealth {
        self.health
            .lock()
            .expect("writer health mutex poisoned")
            .clone()
    }

    /// Stop after all queued commands have been processed and release the lock.
    pub fn shutdown(mut self) -> io::Result<()> {
        self.shutdown_inner()
    }

    fn shutdown_inner(&mut self) -> io::Result<()> {
        let mut result = Ok(());
        if let Some(sender) = self.sender.take() {
            let (reply, response) = mpsc::channel();
            if let Err(error) = sender.send(WriterCommand::Shutdown(reply)) {
                result = Err(io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()));
            } else if let Ok(worker_result) = response.recv() {
                result = worker_result;
            } else {
                result = Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "audit writer worker stopped before shutdown acknowledgement",
                ));
            }
        }
        if let Some(join) = self.join.take() {
            match join.join() {
                Ok(worker_result) => {
                    if result.is_ok() {
                        result = worker_result;
                    }
                }
                Err(_) => {
                    result = Err(io::Error::other("audit writer worker panicked"));
                }
            }
        }
        if let Err(error) = fs::remove_file(&self.lock_path) {
            if error.kind() != io::ErrorKind::NotFound && result.is_ok() {
                result = Err(error);
            }
        }
        result
    }

    /// Remove an exact-path lock only when its recorded process is absent.
    pub fn recover_stale_lock(directory: impl AsRef<Path>) -> io::Result<()> {
        let directory = directory.as_ref();
        if path_has_symlink_component(directory)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "symlink audit path is refused",
            ));
        }
        let lock_path = directory.join(AUDIT_WRITER_LOCK_FILE);
        let content = fs::read_to_string(&lock_path)?;
        let pid = content
            .strip_prefix("pid=")
            .and_then(|value| value.lines().next())
            .and_then(|value| value.trim().parse::<u32>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid writer lock"))?;
        if process_is_alive(pid) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "audit writer lock is still owned",
            ));
        }
        fs::remove_file(lock_path)
    }
}

impl Drop for AuditEventWriter {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

fn acquire_writer_lock(lock_path: &Path) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "audit writer is already open",
            ));
        }
        Err(error) => return Err(error),
    };
    set_private_mode(lock_path)?;
    writeln!(file, "pid={}", std::process::id())?;
    file.sync_all()
}

fn process_is_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn writer_loop(
    store: AuditEventStore,
    receiver: Receiver<WriterCommand>,
    health: Arc<Mutex<WriterHealth>>,
) -> io::Result<()> {
    for command in receiver {
        match command {
            WriterCommand::Append(event, reply) => {
                let result = store.append(event);
                {
                    let mut state = health.lock().expect("writer health mutex poisoned");
                    state.queued = state.queued.saturating_sub(1);
                    match &result {
                        Ok(sequence) => {
                            state.persisted = state.persisted.saturating_add(1);
                            state.last_sequence = Some(*sequence);
                            state.state = AuditHealthState::Healthy;
                            state.last_failure = None;
                        }
                        Err(error) => {
                            state.failed = state.failed.saturating_add(1);
                            state.state = AuditHealthState::Unavailable;
                            state.last_failure =
                                Some(classify_audit_failure(AuditOperation::Append, error));
                        }
                    }
                }
                let _ = reply.send(result);
            }
            WriterCommand::Export(reply) => {
                let result = store.export_jsonl();
                let mut state = health.lock().expect("writer health mutex poisoned");
                match &result {
                    Ok(_) => {
                        state.state = AuditHealthState::Healthy;
                        state.last_failure = None;
                    }
                    Err(error) => {
                        state.state = AuditHealthState::Degraded;
                        state.last_failure =
                            Some(classify_audit_failure(AuditOperation::Export, error));
                    }
                }
                drop(state);
                let _ = reply.send(result);
            }
            WriterCommand::Checkpoint(reply) => {
                let result = store.all_records().map(|records| AuditCheckpoint {
                    event_count: records.len(),
                    last_sequence: records.last().map(|record| record.sequence),
                });
                let mut state = health.lock().expect("writer health mutex poisoned");
                match &result {
                    Ok(_) => {
                        state.state = AuditHealthState::Healthy;
                        state.last_failure = None;
                    }
                    Err(error) => {
                        state.state = AuditHealthState::Degraded;
                        state.last_failure =
                            Some(classify_audit_failure(AuditOperation::Checkpoint, error));
                    }
                }
                drop(state);
                let _ = reply.send(result);
            }
            WriterCommand::Readback(query, reply) => {
                let result = store.readback(&query);
                let mut state = health.lock().expect("writer health mutex poisoned");
                if let Err(error) = &result {
                    state.state = AuditHealthState::Degraded;
                    state.last_failure =
                        Some(classify_audit_failure(AuditOperation::Checkpoint, error));
                }
                drop(state);
                let _ = reply.send(result);
            }
            WriterCommand::Shutdown(reply) => {
                health.lock().expect("writer health mutex poisoned").running = false;
                let _ = reply.send(Ok(()));
                return Ok(());
            }
        }
    }
    health.lock().expect("writer health mutex poisoned").running = false;
    Ok(())
}

impl EventBuffer {
    pub fn push(&mut self, event: Event) {
        if self.events.len() == MAX_RETAINED_EVENTS {
            self.events.remove(0);
        }
        self.events.push(event);
    }
    pub fn as_slice(&self) -> &[Event] {
        &self.events
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counters {
    pub events: u64,
    pub successes: u64,
    pub failures: u64,
    pub total_latency_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Health {
    pub last_success_ms: Option<u64>,
    pub last_failure_ms: Option<u64>,
    pub last_error: Option<ErrorCategory>,
    pub counters: Counters,
}

impl Health {
    /// Update the bounded health seam after a request or reconciliation step.
    pub fn record(&mut self, success: bool, duration_ms: u64, category: Option<ErrorCategory>) {
        self.counters.events += 1;
        self.counters.total_latency_ms = self.counters.total_latency_ms.saturating_add(duration_ms);
        if success {
            self.counters.successes += 1;
            self.last_success_ms = Some(now_ms());
        } else {
            self.counters.failures += 1;
            self.last_failure_ms = Some(now_ms());
            self.last_error = category;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub sync_id: String,
    pub scanned: u64,
    pub changed: u64,
    pub omitted: u64,
    pub conflicts: u64,
    pub bounded: bool,
    pub events: Vec<Event>,
}

impl ReconciliationReport {
    pub fn new(sync_id: impl Into<String>) -> Self {
        Self {
            sync_id: bounded_field(sync_id.into()),
            scanned: 0,
            changed: 0,
            omitted: 0,
            conflicts: 0,
            bounded: true,
            events: Vec::new(),
        }
    }
    pub fn push(&mut self, event: Event) {
        if self.events.len() < MAX_REPORT_EVENTS {
            self.events.push(event);
        }
    }
}

pub fn redact(input: &str) -> String {
    let mut out = input.to_owned();
    for key in [
        "token",
        "authorization",
        "api_key",
        "apikey",
        "password",
        "email",
        "phone",
        "address",
        "full_name",
    ] {
        let mut cursor = 0;
        while let Some(relative_pos) = out.to_ascii_lowercase()[cursor..].find(key) {
            let pos = cursor + relative_pos;
            if let Some(delimiter) = out[pos..].find([':', '=']) {
                let value_start = pos + delimiter + 1;
                let start =
                    value_start + out[value_start..].len() - out[value_start..].trim_start().len();
                if out[start..].starts_with("[REDACTED]") {
                    cursor = start + "[REDACTED]".len();
                    continue;
                }
                let (start, end) =
                    if out[start..].starts_with('"') || out[start..].starts_with('\'') {
                        let quote = out.as_bytes()[start] as char;
                        (
                            start,
                            out[start + 1..]
                                .find(quote)
                                .map(|x| start + x + 2)
                                .unwrap_or(out.len()),
                        )
                    } else {
                        (
                            start,
                            out[start..]
                                .find([';', ',', '\n', '}'])
                                .map(|x| start + x)
                                .unwrap_or(out.len()),
                        )
                    };
                out.replace_range(start..end, "[REDACTED]");
                cursor = start + "[REDACTED]".len();
            } else {
                break;
            }
        }
    }
    let mut chars = out.chars();
    let bounded: String = chars.by_ref().take(MAX_MESSAGE).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn bounded_field(input: String) -> String {
    let redacted = redact(&input);
    let mut chars = redacted.chars();
    let bounded: String = chars.by_ref().take(MAX_FIELD).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

pub fn classify_error(message: &str) -> ErrorCategory {
    let m = message.to_ascii_lowercase();
    if m.contains("timeout") {
        ErrorCategory::Timeout
    } else if m.contains("401") || m.contains("403") || m.contains("auth") {
        ErrorCategory::Auth
    } else if m.contains("conflict") || m.contains("412") {
        ErrorCategory::Conflict
    } else if m.contains("malformed") || m.contains("protocol") {
        ErrorCategory::Protocol
    } else if m.contains("anytype") {
        ErrorCategory::Anytype
    } else if m.contains("transport") || m.contains("connect") {
        ErrorCategory::Transport
    } else {
        ErrorCategory::Unknown
    }
}

pub fn atomic_backup(source: &Path, backup: &Path) -> io::Result<()> {
    validate_artifact_paths(source, backup)?;
    let data = fs::read(source)?;
    let tmp = temp_path(backup);
    fs::write(&tmp, data)?;
    set_private_mode(&tmp)?;
    sync_file(&tmp)?;
    fs::rename(&tmp, backup)?;
    set_private_mode(backup)?;
    sync_parent(backup)
}
pub fn atomic_restore(backup: &Path, destination: &Path) -> io::Result<()> {
    validate_artifact_paths(backup, destination)?;
    let data = fs::read(backup)?;
    let tmp = temp_path(destination);
    fs::write(&tmp, data)?;
    set_private_mode(&tmp)?;
    sync_file(&tmp)?;
    fs::rename(&tmp, destination)?;
    set_private_mode(destination)?;
    sync_parent(destination)
}

/// Roll back an artifact to a previously created backup.
pub fn atomic_rollback(backup: &Path, destination: &Path) -> io::Result<()> {
    atomic_restore(backup, destination)
}
fn temp_path(path: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_extension(format!("tmp-{}-{nonce}", std::process::id()))
}
fn validate_artifact_paths(source: &Path, destination: &Path) -> io::Result<()> {
    if source == destination {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and destination must differ",
        ));
    }
    for path in [source, destination] {
        if path_has_symlink_component(path)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "symlink artifact paths are refused",
            ));
        }
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}
fn path_has_symlink_component(path: &Path) -> io::Result<bool> {
    for component in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(component) {
            if metadata.file_type().is_symlink() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn sync_file(path: &Path) -> io::Result<()> {
    use std::fs::File;
    File::open(path)?.sync_all()
}
fn sync_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        use std::fs::File;
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
fn set_private_mode(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
fn set_private_directory_mode(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn redacts_and_bounds() {
        let e = Event::new(
            "request",
            None,
            format!("token: secret; {}", "x".repeat(400)),
        );
        assert!(!e.message.contains("secret"));
        assert!(e.message.len() <= MAX_MESSAGE + 3);
    }
    #[test]
    fn redacts_headers_json_and_pii() {
        let value =
            redact(r#"Authorization: Bearer abc123 token="json-secret", email: me@example.test"#);
        assert!(!value.contains("abc123"));
        assert!(!value.contains("json-secret"));
        assert!(!value.contains("me@example.test"));
        assert!(value.contains("[REDACTED]"));
    }
    #[test]
    fn redaction_handles_escaped_multiple_and_multiline_secrets() {
        let value = redact("{\"token\": \"a\\\"b\", \"authorization\": Bearer    second,\nemail=person@example.test}");
        assert!(!value.contains("a\\\"b"));
        assert!(!value.contains("second"));
        assert!(!value.contains("person@example.test"));
    }
    #[test]
    fn redaction_removes_repeated_secret_fields() {
        let value = redact("token=first; token=second; api_key=third");
        assert!(!value.contains("first"));
        assert!(!value.contains("second"));
        assert!(!value.contains("third"));
        assert_eq!(value.matches("[REDACTED]").count(), 3);
    }

    #[test]
    fn diagnostic_identity_fields_have_structural_bounds() {
        let event = Event::new("kind", None, "safe")
            .request(format!("token=secret;{}", "r".repeat(MAX_FIELD + 40)))
            .sync("s".repeat(MAX_FIELD + 40))
            .correlation_id("c".repeat(MAX_FIELD + 40));
        let request_id = event.request_id.as_deref().unwrap();
        assert!(!request_id.contains("secret"));
        assert!(request_id.chars().count() <= MAX_FIELD + 1);
        assert!(event.sync_id.as_ref().unwrap().chars().count() <= MAX_FIELD + 1);
        assert!(event.correlation_id.as_ref().unwrap().chars().count() <= MAX_FIELD + 1);
    }

    #[test]
    fn concurrent_events_keep_request_and_sync_attribution() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for worker in 0..8 {
            let events = Arc::clone(&events);
            handles.push(thread::spawn(move || {
                let correlation = Correlation {
                    request_id: Some(format!("req-{worker}")),
                    sync_id: Some(format!("sync-{worker}")),
                };
                for sequence in 0..32 {
                    events.lock().unwrap().push(
                        correlation
                            .event("synthetic.fault", Some(ErrorCategory::Timeout), "timeout")
                            .correlation_id(format!("corr-{worker}-{sequence}")),
                    );
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 8 * 32);
        for event in events.iter() {
            let request = event.request_id.as_deref().unwrap();
            let sync = event.sync_id.as_deref().unwrap();
            assert_eq!(
                request.strip_prefix("req-").unwrap(),
                sync.strip_prefix("sync-").unwrap()
            );
            assert!(event.correlation_id.is_some());
        }
    }

    #[test]
    fn concurrent_backups_use_distinct_temporary_paths() {
        use std::thread;
        let root =
            std::env::temp_dir().join(format!("any-cal-observe-concurrent-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut handles = Vec::new();
        for index in 0..8 {
            let dir = root.clone();
            handles.push(thread::spawn(move || {
                let source = dir.join(format!("source-{index}"));
                let backup = dir.join(format!("backup-{index}"));
                fs::write(&source, format!("value-{index}")).unwrap();
                atomic_backup(&source, &backup).unwrap();
                assert_eq!(
                    fs::read_to_string(backup).unwrap(),
                    format!("value-{index}")
                );
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn single_writer_serializes_concurrent_producers_with_bounded_queue() {
        use std::sync::Arc;
        use std::thread;
        let root = std::env::temp_dir().join(format!(
            "any-cal-observe-writer-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let writer = Arc::new(AuditEventWriter::open(&root, 2).unwrap());
        let mut handles = Vec::new();
        for worker in 0..8 {
            let writer = Arc::clone(&writer);
            handles.push(thread::spawn(move || {
                (0..16)
                    .map(|index| {
                        writer.append(Event::new(
                            "synthetic.auth",
                            Some(ErrorCategory::Auth),
                            format!("worker={worker}; index={index}; token=secret"),
                        ))
                    })
                    .collect::<Vec<_>>()
            }));
        }
        let mut sequences = Vec::new();
        for handle in handles {
            sequences.extend(handle.join().unwrap().into_iter().map(Result::unwrap));
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (1..=128).collect::<Vec<_>>());
        let export = writer.export_jsonl().unwrap();
        assert_eq!(export.matches("\"sequence\"").count(), 128);
        assert!(!export.contains("secret"));
        let health = writer.health();
        assert_eq!(health.queue_capacity, 2);
        assert_eq!(health.persisted, 128);
        assert_eq!(health.failed, 0);
        assert_eq!(health.queued, 0);
        assert!(health.running);
        let checkpoint = writer.checkpoint().unwrap();
        assert_eq!(checkpoint.event_count, 128);
        assert_eq!(checkpoint.last_sequence, Some(128));
        drop(writer);
        assert!(!root.join(AUDIT_WRITER_LOCK_FILE).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn single_writer_restarts_and_requires_explicit_stale_lock_recovery() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-observe-restart-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let writer = AuditEventWriter::open(&root, 1).unwrap();
        writer.append(Event::new("first", None, "safe")).unwrap();
        writer.shutdown().unwrap();
        let writer = AuditEventWriter::open(&root, 1).unwrap();
        writer.append(Event::new("second", None, "safe")).unwrap();
        writer.shutdown().unwrap();
        fs::write(root.join(AUDIT_WRITER_LOCK_FILE), "pid=4294967295\n").unwrap();
        assert!(AuditEventWriter::open(&root, 1).is_err());
        AuditEventWriter::recover_stale_lock(&root).unwrap();
        let writer = AuditEventWriter::open(&root, 1).unwrap();
        let checkpoint = writer.checkpoint().unwrap();
        assert_eq!(checkpoint.event_count, 2);
        assert_eq!(checkpoint.last_sequence, Some(2));
        writer.shutdown().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn correlation_flows_through_lifecycle_event() {
        let c = Correlation {
            request_id: Some("req-1".into()),
            sync_id: Some("sync-1".into()),
        };
        let event = c.event("repository.write", Some(ErrorCategory::Anytype), "safe");
        assert_eq!(event.request_id.as_deref(), Some("req-1"));
        assert_eq!(event.sync_id.as_deref(), Some("sync-1"));
    }
    #[test]
    fn deterministic_json_shape() {
        let e = Event {
            timestamp_ms: 1,
            request_id: Some("r".into()),
            sync_id: None,
            kind: "ok".into(),
            category: None,
            message: "m".into(),
            duration_ms: Some(2),
            error_code: None,
            correlation_id: None,
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"timestamp_ms":1,"request_id":"r","sync_id":null,"kind":"ok","category":null,"message":"m","duration_ms":2,"error_code":null,"correlation_id":null}"#
        );
    }
    #[test]
    fn classification_is_stable() {
        assert_eq!(
            classify_error("timeout talking to transport"),
            ErrorCategory::Timeout
        );
        assert_eq!(classify_error("412 conflict"), ErrorCategory::Conflict);
        assert_eq!(classify_error("malformed JSON"), ErrorCategory::Protocol);
    }
    #[test]
    fn local_failure_matrix_has_bounded_recovery() {
        for (mode, action) in [
            (FailureMode::Timeout, RecoveryAction::Retry),
            (FailureMode::Auth, RecoveryAction::Reauthenticate),
            (FailureMode::Malformed, RecoveryAction::Reconcile),
            (FailureMode::Archive, RecoveryAction::Reconcile),
        ] {
            assert_eq!(recovery_action(mode), action);
        }
    }
    #[test]
    fn report_is_bounded() {
        let mut r = ReconciliationReport::new("s");
        for _ in 0..(MAX_REPORT_EVENTS + 5) {
            r.push(Event::new("x", None, "m"));
        }
        assert_eq!(r.events.len(), MAX_REPORT_EVENTS);
        assert!(r.bounded);
    }
    #[test]
    fn health_records_bounded_outcomes() {
        let mut health = Health::default();
        health.record(true, 4, None);
        health.record(false, 7, Some(ErrorCategory::Timeout));
        assert_eq!(health.counters.events, 2);
        assert_eq!(health.counters.total_latency_ms, 11);
        assert_eq!(health.last_error, Some(ErrorCategory::Timeout));
    }
    #[test]
    fn backup_restore_are_atomic() {
        let d = std::env::temp_dir().join(format!("any-cal-observability-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let source = d.join("source");
        let backup = d.join("backup");
        let dest = d.join("dest");
        fs::write(&source, b"one").unwrap();
        atomic_backup(&source, &backup).unwrap();
        fs::write(&dest, b"two").unwrap();
        atomic_restore(&backup, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn audit_store_is_atomic_recoverable_and_deterministic() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-store-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let store = AuditEventStore::open(&root).unwrap();
        let first = store
            .append(Event::new(
                "dav.auth",
                Some(ErrorCategory::Auth),
                "authenticated",
            ))
            .unwrap();
        let second = store
            .append(Event::new(
                "dav.auth",
                Some(ErrorCategory::Auth),
                "invalid token=secret",
            ))
            .unwrap();
        let third = store
            .append(Event {
                timestamp_ms: 3,
                request_id: Some("authorization=raw-request".into()),
                sync_id: None,
                kind: "raw".into(),
                category: None,
                message: "api_key=raw-key".into(),
                duration_ms: None,
                error_code: None,
                correlation_id: None,
            })
            .unwrap();
        assert_eq!((first, second, third), (1, 2, 3));
        let before = store.export_jsonl().unwrap();
        assert!(!before.contains("secret"));
        assert!(!before.contains("raw-key"));
        assert!(!before.contains("raw-request"));
        assert_eq!(store.event_count().unwrap(), 3);

        let active = root.join(AUDIT_ACTIVE_FILE);
        let mut torn = fs::read(&active).unwrap();
        torn.extend_from_slice(b"{\"sequence\":3");
        fs::write(&active, torn).unwrap();
        let reopened = AuditEventStore::open(&root).unwrap();
        assert_eq!(reopened.event_count().unwrap(), 3);
        assert_eq!(reopened.export_jsonl().unwrap(), before);

        let segment = reopened.rollover().unwrap().unwrap();
        assert!(segment.exists());
        assert_eq!(reopened.event_count().unwrap(), 3);
        let after_rollover = reopened.export_jsonl().unwrap();
        assert_eq!(after_rollover, before);
        let fourth = reopened
            .append(Event::new("dav.auth", Some(ErrorCategory::Auth), "expired"))
            .unwrap();
        assert_eq!(fourth, 4);
        assert_eq!(reopened.event_count().unwrap(), 4);
        assert_eq!(
            reopened
                .export_jsonl()
                .unwrap()
                .matches("\"sequence\"")
                .count(),
            4
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&segment).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_store_rejects_interior_corruption_without_deleting_data() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-corrupt-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let store = AuditEventStore::open(&root).unwrap();
        store.append(Event::new("one", None, "safe")).unwrap();
        store.append(Event::new("two", None, "safe")).unwrap();
        let active = root.join(AUDIT_ACTIVE_FILE);
        let contents = fs::read_to_string(&active).unwrap();
        let mut lines = contents.lines();
        let first = lines.next().unwrap();
        let second = lines.next().unwrap();
        fs::write(&active, format!("{first}\nnot-json\n{second}\n")).unwrap();
        let result = AuditEventStore::open(&root);
        assert!(result.is_err());
        assert!(fs::read_to_string(active).unwrap().contains("not-json"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_store_rejects_complete_corrupt_tail_but_recovers_torn_tail() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-tail-integrity-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let store = AuditEventStore::open(&root).unwrap();
        store.append(Event::new("one", None, "safe")).unwrap();
        let active = root.join(AUDIT_ACTIVE_FILE);
        let valid = fs::read_to_string(&active).unwrap();

        fs::write(&active, format!("{valid}not-json\n")).unwrap();
        let corrupt = AuditEventStore::open(&root);
        assert!(matches!(
            corrupt,
            Err(error) if error.kind() == io::ErrorKind::InvalidData
        ));
        assert!(fs::read_to_string(&active).unwrap().contains("not-json"));

        fs::write(&active, format!("{valid}{{\"sequence\":2")).unwrap();
        let recovered = AuditEventStore::open(&root).unwrap();
        assert_eq!(recovered.event_count().unwrap(), 1);
        assert_eq!(recovered.export_jsonl().unwrap(), valid);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_store_rejects_sequence_gaps_and_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-sequence-integrity-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let store = AuditEventStore::open(&root).unwrap();
        store.append(Event::new("one", None, "safe")).unwrap();
        store.append(Event::new("two", None, "safe")).unwrap();
        let active = root.join(AUDIT_ACTIVE_FILE);
        let lines = fs::read_to_string(&active).unwrap();
        let first = lines.lines().next().unwrap();
        let second = lines.lines().nth(1).unwrap();
        let duplicate = second.replace("\"sequence\":2", "\"sequence\":1");
        fs::write(&active, format!("{first}\n{duplicate}\n")).unwrap();
        let duplicate_result = AuditEventStore::open(&root);
        assert!(matches!(
            duplicate_result,
            Err(error) if error.kind() == io::ErrorKind::InvalidData
        ));

        fs::write(
            &active,
            format!(
                "{first}\n{}\n",
                second.replace("\"sequence\":2", "\"sequence\":4")
            ),
        )
        .unwrap();
        let gap_result = AuditEventStore::open(&root);
        assert!(matches!(
            gap_result,
            Err(error) if error.kind() == io::ErrorKind::InvalidData
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_failures_have_secret_free_stable_classes() {
        let missing = io::Error::new(io::ErrorKind::NotFound, "secret/path/token=hidden");
        assert_eq!(
            classify_audit_failure(AuditOperation::Open, &missing),
            AuditFailureClass::Missing
        );
        let locked = io::Error::new(io::ErrorKind::AlreadyExists, "private/path");
        assert_eq!(
            classify_audit_failure(AuditOperation::Lock, &locked),
            AuditFailureClass::Locked
        );
        let corrupt = io::Error::new(io::ErrorKind::InvalidData, "api_key=hidden");
        assert_eq!(
            classify_audit_failure(AuditOperation::Export, &corrupt),
            AuditFailureClass::Corrupt
        );
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "private/path");
        assert_eq!(
            classify_audit_failure(AuditOperation::Append, &denied),
            AuditFailureClass::Permission
        );
        let rendered = format!("{:?}", AuditFailureClass::Corrupt);
        assert!(!rendered.contains("hidden"));
    }

    #[test]
    fn audit_writer_reports_corrupt_export_and_recovers_truthfully() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-health-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let writer = AuditEventWriter::open(&root, 1).unwrap();
        writer
            .append(Event::new("auth", Some(ErrorCategory::Auth), "safe"))
            .unwrap();
        let active = root.join(AUDIT_ACTIVE_FILE);
        let valid = fs::read_to_string(&active).unwrap();
        fs::write(&active, format!("not-json\n{valid}")).unwrap();

        let export = writer.export_jsonl();
        assert!(export.is_err());
        let degraded = writer.health();
        assert_eq!(degraded.state, AuditHealthState::Degraded);
        assert_eq!(degraded.last_failure, Some(AuditFailureClass::Corrupt));
        assert_eq!(degraded.persisted, 1);

        fs::write(&active, valid).unwrap();
        let export = writer.export_jsonl().unwrap();
        assert_eq!(export.matches("\"sequence\"").count(), 1);
        let recovered = writer.health();
        assert_eq!(recovered.state, AuditHealthState::Healthy);
        assert_eq!(recovered.last_failure, None);
        writer.shutdown().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_writer_open_fails_closed_when_store_is_unavailable() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-readonly-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        // A regular file in the directory position is a deterministic,
        // privilege-independent stand-in for an unavailable/read-only store.
        let unavailable = root.join("store");
        fs::write(&unavailable, b"not-a-directory").unwrap();
        let open = AuditEventWriter::open(&unavailable, 1);
        assert!(open.is_err());
        if let Err(error) = open {
            let class = classify_audit_failure(AuditOperation::Open, &error);
            assert_eq!(class, AuditFailureClass::Io);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_writer_lock_failure_is_classified_without_path_details() {
        let root = std::env::temp_dir().join(format!(
            "any-cal-audit-lock-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let writer = AuditEventWriter::open(&root, 1).unwrap();
        let second = AuditEventWriter::open(&root, 1);
        assert!(second.is_err());
        if let Err(error) = second {
            assert_eq!(
                classify_audit_failure(AuditOperation::Lock, &error),
                AuditFailureClass::Locked
            );
        }
        writer.shutdown().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_artifacts_are_refused() {
        use std::os::unix::fs::symlink;
        let d =
            std::env::temp_dir().join(format!("any-cal-observability-link-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let source = d.join("source");
        let link = d.join("link");
        fs::write(&source, b"one").unwrap();
        symlink(&source, &link).unwrap();
        assert!(atomic_backup(&link, &d.join("backup")).is_err());
        let _ = fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_parent_artifact_paths_are_refused() {
        use std::os::unix::fs::symlink;
        let d = std::env::temp_dir().join(format!(
            "any-cal-observability-parent-link-{}",
            std::process::id()
        ));
        let real = d.join("real");
        let link = d.join("link");
        fs::create_dir_all(&real).unwrap();
        symlink(&real, &link).unwrap();
        let source = d.join("source");
        fs::write(&source, b"one").unwrap();
        assert!(atomic_backup(&source, &link.join("backup")).is_err());
        let _ = fs::remove_dir_all(d);
    }
}
