//! Durable sync bookkeeping. It records observed identities and pending
//! operations without assuming a live Anytype change-stream API.
use any_cal_core::{DomainBinding, ResourceEnvelope, ResourceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const VERSION: u8 = 1;
const EXPORT_FORMAT: &str = "any-cal.sync-export";
const EXPORT_VERSION: u8 = 1;
pub const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Anytype currently ignores If-Match/ETag-style preconditions.  The sync
/// layer therefore records drift and uses the server's observed later-write
/// wins behavior; it must not claim conflict prevention.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConflictPolicy {
    LaterWriteWins,
}

pub const ANYTYPE_CONFLICT_POLICY: ConflictPolicy = ConflictPolicy::LaterWriteWins;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationOutcome {
    NotSent,
    Applied,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryDecision {
    Retry { attempt: u32 },
    Reconcile,
    Stop,
}

pub fn retry_decision(
    operation: &PendingOperation,
    transient: bool,
    outcome: MutationOutcome,
) -> RetryDecision {
    if matches!(outcome, MutationOutcome::Unknown) {
        return RetryDecision::Reconcile;
    }
    if !transient || operation.attempts >= MAX_RETRY_ATTEMPTS {
        return RetryDecision::Stop;
    }
    RetryDecision::Retry {
        attempt: operation.attempts.saturating_add(1),
    }
}

pub fn classify_with_policy(
    local: Option<&ObservedResource>,
    remote: Option<&ObservedResource>,
    pending: bool,
    policy: ConflictPolicy,
) -> Reconciliation {
    match (local, remote) {
        (None, None) => Reconciliation::Unchanged,
        (Some(_), None) => Reconciliation::RemoteArchived,
        (None, Some(_)) => Reconciliation::RemoteChanged,
        (Some(old), Some(new)) if old.etag == new.etag && old.revision == new.revision => {
            Reconciliation::Unchanged
        }
        (Some(_), Some(new)) if new.archived => Reconciliation::RemoteArchived,
        (Some(_), Some(_)) if pending && matches!(policy, ConflictPolicy::LaterWriteWins) => {
            Reconciliation::RemoteChanged
        }
        (Some(_), Some(_)) if pending => Reconciliation::Conflict,
        (Some(_), Some(_)) => Reconciliation::RemoteChanged,
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedResource {
    pub resource_id: ResourceId,
    pub anytype_object_id: String,
    pub dav_uid: String,
    pub revision: u64,
    pub etag: String,
    /// DAV Last-Modified metadata, retained independently of representation ETag.
    #[serde(default)]
    pub modified_at: u64,
    pub archived: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OperationKind {
    Create,
    Update,
    Archive,
    Delete,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingOperation {
    pub operation_id: String,
    pub kind: OperationKind,
    pub resource_id: ResourceId,
    pub expected_etag: Option<String>,
    pub expected_revision: Option<u64>,
    pub envelope: Option<ResourceEnvelope>,
    pub attempts: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Tombstone {
    pub resource_id: ResourceId,
    pub anytype_object_id: String,
    pub dav_uid: String,
    pub revision: u64,
    pub etag: String,
    #[serde(default)]
    pub modified_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reconciliation {
    Unchanged,
    RemoteChanged,
    RemoteArchived,
    Conflict,
}

/// Compare a previously observed version with a newly polled version. A
/// revision or ETag drift while a local operation is pending is always a
/// conflict: callers must not overwrite the remote object implicitly.
pub fn classify(
    local: Option<&ObservedResource>,
    remote: Option<&ObservedResource>,
    pending: bool,
) -> Reconciliation {
    match (local, remote) {
        (None, None) => Reconciliation::Unchanged,
        (Some(_), None) => Reconciliation::RemoteArchived,
        (None, Some(_)) => Reconciliation::RemoteChanged,
        (Some(old), Some(new)) if old.etag == new.etag && old.revision == new.revision => {
            Reconciliation::Unchanged
        }
        (Some(_), Some(new)) if new.archived => Reconciliation::RemoteArchived,
        (Some(_), Some(_)) if pending => Reconciliation::Conflict,
        (Some(_), Some(_)) => Reconciliation::RemoteChanged,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncScope {
    pub domain_id: String,
    pub binding_fingerprint: String,
    pub endpoint_fingerprint: String,
    pub account_fingerprint: String,
    pub space_id: String,
}

impl SyncScope {
    pub fn for_binding(
        binding: &DomainBinding,
        endpoint: &str,
        account_fingerprint: &str,
    ) -> Result<Self, StoreError> {
        if endpoint.trim().is_empty() || endpoint.chars().any(char::is_control) {
            return Err(StoreError::Invalid("invalid endpoint identity".into()));
        }
        let binding_fingerprint = binding
            .fingerprint(account_fingerprint)
            .map_err(|_| StoreError::Invalid("invalid binding identity".into()))?;
        let scope = Self {
            domain_id: binding.domain_id.clone(),
            binding_fingerprint,
            endpoint_fingerprint: digest(endpoint.as_bytes()),
            account_fingerprint: account_fingerprint.into(),
            space_id: binding.space_id.clone(),
        };
        validate_scope(&scope)?;
        Ok(scope)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncState {
    pub version: u8,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<SyncScope>,
    pub observed: BTreeMap<ResourceId, ObservedResource>,
    pub pending: BTreeMap<String, PendingOperation>,
    pub tombstones: BTreeMap<ResourceId, Tombstone>,
}

/// A portable, operator-triggered snapshot of local sync bookkeeping.
///
/// The payload is intentionally limited to sync state: credentials, transport
/// configuration, and audit logs are not part of this artifact.  The digest is
/// over the canonical JSON payload and is checked before any destination is
/// changed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncExport {
    pub format: String,
    pub export_version: u8,
    pub state_version: u8,
    pub payload: SyncState,
    pub payload_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportReceipt {
    pub format: &'static str,
    pub export_version: u8,
    pub state_generation: u64,
    pub observed: usize,
    pub pending: usize,
    pub tombstones: usize,
}
impl Default for SyncState {
    fn default() -> Self {
        Self {
            version: VERSION,
            generation: 0,
            scope: None,
            observed: BTreeMap::new(),
            pending: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Corrupt(String),
    Invalid(String),
    Resurrection(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitFault {
    None,
    BeforeWrite,
    PartialWrite,
    BeforeRename,
    BeforeDirectorySync,
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "sync store I/O: {e}"),
            Self::Corrupt(e) => write!(f, "corrupt sync state: {e}"),
            Self::Invalid(e) => write!(f, "invalid sync state: {e}"),
            Self::Resurrection(e) => write!(f, "tombstoned resource reappeared: {e}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A checkpoint with backup recovery. Temporary and backup files are confined
/// to the caller-provided path; indexes are derived and never authoritative.
pub struct SyncStore {
    path: PathBuf,
    state: SyncState,
    lock: fs::File,
    fault: CommitFault,
    /// The primary was unreadable when this store opened.  Preserve the
    /// validated backup until a new primary has been published; otherwise a
    /// failed follow-up commit could overwrite the only good recovery copy
    /// with the still-corrupt primary.
    recovered_from_backup: bool,
}
impl SyncStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        Self::open_with_scope(path.into(), None)
    }

    pub fn open_scoped(
        path: impl Into<PathBuf>,
        scope: SyncScope,
    ) -> Result<Self, StoreError> {
        validate_scope(&scope)?;
        Self::open_with_scope(path.into(), Some(scope))
    }

    fn open_with_scope(
        path: PathBuf,
        expected_scope: Option<SyncScope>,
    ) -> Result<Self, StoreError> {
        validate_path(&path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock_path = path.with_extension("lock");
        let lock = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    StoreError::Invalid("sync store is already open by another writer".into())
                } else {
                    e.into()
                }
            })?;
        set_private_mode(&lock_path)?;
        let (mut state, recovered_from_backup) = match read_state(&path) {
            Ok(s) => (s, false),
            Err(primary) => match read_state(&backup(&path)) {
                Ok(s) => (s, true),
                Err(second) => {
                    if primary.kind() == io::ErrorKind::NotFound
                        && second.kind() == io::ErrorKind::NotFound
                    {
                        (SyncState::default(), false)
                    } else {
                        let _ = fs::remove_file(&lock_path);
                        return Err(StoreError::Corrupt(format!(
                            "primary: {primary}; backup: {second}"
                        )));
                    }
                }
            },
        };
        if let Err(error) = validate(&state) {
            let _ = fs::remove_file(&lock_path);
            return Err(error);
        }
        if let Err(error) = bind_scope(&mut state, expected_scope) {
            let _ = fs::remove_file(&lock_path);
            return Err(error);
        }
        Ok(Self {
            path,
            state,
            lock,
            fault: CommitFault::None,
            recovered_from_backup,
        })
    }
    pub fn state(&self) -> &SyncState {
        &self.state
    }

    pub fn scope(&self) -> Option<&SyncScope> {
        self.state.scope.as_ref()
    }

    /// Export the current state to a private, atomically published artifact.
    pub fn export_to(&self, destination: impl AsRef<Path>) -> Result<ExportReceipt, StoreError> {
        let destination = destination.as_ref();
        validate_artifact_path(destination)?;
        let export = make_export(self.state.clone())?;
        let bytes = serde_json::to_vec_pretty(&export)
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        atomic_write(destination, &bytes)?;
        Ok(receipt(&export))
    }

    /// Restore a previously exported state, validating it before publication.
    /// Existing destination state is left untouched on every validation or
    /// staging failure.
    pub fn restore_from(
        export_path: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<ExportReceipt, StoreError> {
        let export_path = export_path.as_ref();
        let destination = destination.as_ref();
        validate_artifact_path(export_path)?;
        validate_artifact_path(destination)?;
        if export_path == destination {
            return Err(StoreError::Invalid(
                "export and destination must differ".into(),
            ));
        }
        let bytes = fs::read(export_path)?;
        let export: SyncExport = serde_json::from_slice(&bytes)
            .map_err(|error| StoreError::Corrupt(format!("invalid export: {error}")))?;
        validate_export(&export)?;
        let encoded = serde_json::to_vec_pretty(&export.payload)
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        atomic_write(destination, &encoded)?;
        Ok(receipt(&export))
    }

    /// Restore a validated export into this already-open store.  Publication
    /// is atomic and the in-memory state is advanced only after the
    /// destination file has been published successfully.  This is the
    /// service-safe counterpart to [`Self::restore_from`]: callers do not
    /// need to close the active checkpoint (and therefore cannot accidentally
    /// leave a stale in-memory checkpoint running after a restore).
    pub fn restore_export(
        &mut self,
        export_path: impl AsRef<Path>,
    ) -> Result<ExportReceipt, StoreError> {
        let export_path = export_path.as_ref();
        validate_artifact_path(export_path)?;
        if export_path == self.path {
            return Err(StoreError::Invalid(
                "export and destination must differ".into(),
            ));
        }
        let bytes = fs::read(export_path)?;
        let export: SyncExport = serde_json::from_slice(&bytes)
            .map_err(|error| StoreError::Corrupt(format!("invalid export: {error}")))?;
        validate_export(&export)?;
        if self.state.scope != export.payload.scope {
            return Err(StoreError::Invalid(
                "sync export binding scope mismatch".into(),
            ));
        }
        let encoded = serde_json::to_vec_pretty(&export.payload)
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        atomic_write(&self.path, &encoded)?;
        self.state = export.payload.clone();
        self.recovered_from_backup = false;
        Ok(receipt(&export))
    }
    pub fn inject_commit_fault(&mut self, fault: CommitFault) {
        self.fault = fault;
    }
    pub fn pending(&self) -> impl Iterator<Item = &PendingOperation> {
        self.state.pending.values()
    }
    pub fn observe(&mut self, item: ObservedResource) -> Result<(), StoreError> {
        self.transaction(|state| {
            state.observed.insert(item.resource_id.clone(), item);
        })
    }
    /// Atomically replace the observed remote snapshot and derive tombstones
    /// for resources omitted from that complete snapshot. The caller must
    /// have performed an omission-safe remote listing before invoking this.
    pub fn replace_observed(
        &mut self,
        items: impl IntoIterator<Item = ObservedResource>,
    ) -> Result<(), StoreError> {
        self.replace_observed_inner(items, false)
    }
    /// Explicitly accept a tombstoned resource as a new create/resurrection.
    /// This is separate from normal refresh so omission-safe polling cannot
    /// accidentally resurrect deleted data.
    pub fn replace_observed_explicit_resurrection(
        &mut self,
        items: impl IntoIterator<Item = ObservedResource>,
    ) -> Result<(), StoreError> {
        self.replace_observed_inner(items, true)
    }
    fn replace_observed_inner(
        &mut self,
        items: impl IntoIterator<Item = ObservedResource>,
        allow_resurrection: bool,
    ) -> Result<(), StoreError> {
        let replacement = items
            .into_iter()
            .map(|item| (item.resource_id.clone(), item))
            .collect::<BTreeMap<_, _>>();
        if !allow_resurrection {
            if let Some(id) = replacement
                .keys()
                .find(|id| self.state.tombstones.contains_key(*id))
            {
                return Err(StoreError::Resurrection(id.to_string()));
            }
        }
        self.transaction(|state| {
            if allow_resurrection {
                for id in replacement.keys() {
                    state.tombstones.remove(id);
                }
            }
            for (id, old) in state.observed.iter() {
                if !replacement.contains_key(id) {
                    state.tombstones.insert(
                        id.clone(),
                        Tombstone {
                            resource_id: id.clone(),
                            anytype_object_id: old.anytype_object_id.clone(),
                            dav_uid: old.dav_uid.clone(),
                            revision: old.revision,
                            etag: old.etag.clone(),
                            modified_at: old.modified_at,
                        },
                    );
                }
            }
            state.observed = replacement;
        })
    }
    pub fn enqueue(&mut self, operation: PendingOperation) -> Result<(), StoreError> {
        if operation.operation_id.trim().is_empty() {
            return Err(StoreError::Invalid("operation ID is empty".into()));
        }
        if let Some(old) = self.state.pending.get(&operation.operation_id) {
            if old != &operation {
                return Err(StoreError::Invalid(
                    "operation ID reused with different operation".into(),
                ));
            }
            return Ok(());
        }
        self.transaction(|state| {
            state
                .pending
                .insert(operation.operation_id.clone(), operation);
        })
    }
    pub fn mark_applied(
        &mut self,
        operation_id: &str,
        observed: Option<ObservedResource>,
    ) -> Result<(), StoreError> {
        let op = self
            .state
            .pending
            .get(operation_id)
            .cloned()
            .ok_or_else(|| StoreError::Invalid("unknown operation ID".into()))?;
        self.transaction(|state| {
            state.pending.remove(operation_id);
            if let Some(item) = observed {
                state.observed.insert(item.resource_id.clone(), item);
            }
            if matches!(op.kind, OperationKind::Archive | OperationKind::Delete) {
                if let Some(old) = state.observed.remove(&op.resource_id) {
                    state.tombstones.insert(
                        op.resource_id.clone(),
                        Tombstone {
                            resource_id: old.resource_id,
                            anytype_object_id: old.anytype_object_id,
                            dav_uid: old.dav_uid,
                            revision: old.revision,
                            etag: old.etag,
                            modified_at: old.modified_at,
                        },
                    );
                }
            }
        })
    }
    pub fn record_tombstone(&mut self, tombstone: Tombstone) -> Result<(), StoreError> {
        self.transaction(|state| {
            state.observed.remove(&tombstone.resource_id);
            state
                .tombstones
                .insert(tombstone.resource_id.clone(), tombstone);
        })
    }
    pub fn retryable(operation: &PendingOperation, transient: bool) -> Result<u32, StoreError> {
        if !transient || operation.attempts >= MAX_RETRY_ATTEMPTS {
            return Err(StoreError::Invalid(
                "failure requires reconciliation or retry bound was reached".into(),
            ));
        }
        Ok(operation.attempts.saturating_add(1))
    }
    fn transaction<F: FnOnce(&mut SyncState)>(&mut self, apply: F) -> Result<(), StoreError> {
        let mut candidate = self.state.clone();
        apply(&mut candidate);
        candidate.generation = candidate.generation.saturating_add(1);
        validate(&candidate)?;
        self.commit(&candidate)?;
        self.state = candidate;
        Ok(())
    }
    fn commit(&mut self, candidate: &SyncState) -> Result<(), StoreError> {
        let bytes =
            serde_json::to_vec_pretty(candidate).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if self.fault == CommitFault::BeforeWrite {
            return Err(StoreError::Io(io::Error::other(
                "injected pre-write failure",
            )));
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| StoreError::Invalid(e.to_string()))?
            .as_nanos();
        let tmp = self
            .path
            .with_extension(format!("tmp-{}-{}", std::process::id(), nonce));
        {
            let mut f = fs::File::create(&tmp)?;
            set_private_mode(&tmp)?;
            if self.fault == CommitFault::PartialWrite {
                f.write_all(&bytes[..bytes.len().min(3)])?;
                let _ = fs::remove_file(&tmp);
                return Err(StoreError::Io(io::Error::other("injected partial write")));
            }
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        if self.path.exists() && !self.recovered_from_backup {
            let bak = backup(&self.path);
            fs::copy(&self.path, &bak)?;
            set_private_mode(&bak)?;
            fs::File::open(&bak)?.sync_all()?;
        }
        if self.fault == CommitFault::BeforeRename {
            let _ = fs::remove_file(&tmp);
            return Err(StoreError::Io(io::Error::other(
                "injected pre-rename failure",
            )));
        }
        fs::rename(&tmp, &self.path)?;
        if self.fault == CommitFault::BeforeDirectorySync {
            return Err(StoreError::Io(io::Error::other(
                "injected directory-sync failure",
            )));
        }
        if let Some(parent) = self.path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        self.recovered_from_backup = false;
        Ok(())
    }
}

fn make_export(state: SyncState) -> Result<SyncExport, StoreError> {
    validate(&state)?;
    let payload =
        serde_json::to_vec(&state).map_err(|error| StoreError::Corrupt(error.to_string()))?;
    Ok(SyncExport {
        format: EXPORT_FORMAT.into(),
        export_version: EXPORT_VERSION,
        state_version: state.version,
        payload: state,
        payload_sha256: digest(&payload),
    })
}

fn validate_export(export: &SyncExport) -> Result<(), StoreError> {
    if export.format != EXPORT_FORMAT {
        return Err(StoreError::Corrupt("unsupported export format".into()));
    }
    if export.export_version != EXPORT_VERSION {
        return Err(StoreError::Corrupt("unsupported export version".into()));
    }
    if export.state_version != export.payload.version {
        return Err(StoreError::Corrupt("export state version mismatch".into()));
    }
    validate(&export.payload)?;
    let payload = serde_json::to_vec(&export.payload)
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    if export.payload_sha256 != digest(&payload) {
        return Err(StoreError::Corrupt(
            "export payload integrity failure".into(),
        ));
    }
    Ok(())
}

fn digest(payload: &[u8]) -> String {
    Sha256::digest(payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn receipt(export: &SyncExport) -> ExportReceipt {
    ExportReceipt {
        format: EXPORT_FORMAT,
        export_version: export.export_version,
        state_generation: export.payload.generation,
        observed: export.payload.observed.len(),
        pending: export.payload.pending.len(),
        tombstones: export.payload.tombstones.len(),
    }
}

fn validate_artifact_path(path: &Path) -> Result<(), StoreError> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err(StoreError::Invalid("unsafe artifact path".into()));
    }
    if path_has_symlink_component(path)? {
        return Err(StoreError::Invalid(
            "symlink artifact path is not allowed".into(),
        ));
    }
    if let Some(parent) = path.parent() {
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

fn atomic_write(destination: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| StoreError::Invalid(error.to_string()))?
        .as_nanos();
    let temporary =
        destination.with_extension(format!("tmp-export-{}-{nonce}", std::process::id()));
    {
        let mut file = fs::File::create(&temporary)?;
        set_private_mode(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    set_private_mode(destination)?;
    if let Some(parent) = destination.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
impl Drop for SyncStore {
    fn drop(&mut self) {
        let _ = self.lock.sync_all();
        let _ = fs::remove_file(self.path.with_extension("lock"));
    }
}
fn validate_path(path: &Path) -> Result<(), StoreError> {
    if path.as_os_str().is_empty()
        || path.file_name().is_none()
        || path.file_name().is_some_and(|n| n == "." || n == "..")
    {
        return Err(StoreError::Invalid("unsafe checkpoint path".into()));
    }
    if path.is_symlink() || backup(path).is_symlink() {
        return Err(StoreError::Invalid(
            "checkpoint symlink is not allowed".into(),
        ));
    }
    Ok(())
}
fn backup(path: &Path) -> PathBuf {
    path.with_extension("bak")
}
fn set_private_mode(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
fn read_state(path: &Path) -> io::Result<SyncState> {
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
fn validate_scope(scope: &SyncScope) -> Result<(), StoreError> {
    for (name, value) in [
        ("domain_id", scope.domain_id.as_str()),
        ("binding_fingerprint", scope.binding_fingerprint.as_str()),
        ("endpoint_fingerprint", scope.endpoint_fingerprint.as_str()),
        ("account_fingerprint", scope.account_fingerprint.as_str()),
        ("space_id", scope.space_id.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(StoreError::Invalid(format!("invalid sync scope {name}")));
        }
    }
    Ok(())
}

fn bind_scope(
    state: &mut SyncState,
    expected_scope: Option<SyncScope>,
) -> Result<(), StoreError> {
    match (state.scope.as_ref(), expected_scope) {
        (Some(_), None) => Err(StoreError::Invalid(
            "scoped checkpoint requires an expected binding scope".into(),
        )),
        (Some(actual), Some(expected)) if actual != &expected => Err(StoreError::Invalid(
            "sync checkpoint binding scope mismatch".into(),
        )),
        (Some(_), Some(_)) | (None, None) => Ok(()),
        (None, Some(expected)) => {
            let contains_prior_state = state.generation != 0
                || !state.observed.is_empty()
                || !state.pending.is_empty()
                || !state.tombstones.is_empty();
            if contains_prior_state {
                return Err(StoreError::Invalid(
                    "unscoped checkpoint with existing state cannot adopt a binding".into(),
                ));
            }
            state.scope = Some(expected);
            Ok(())
        }
    }
}

fn validate(state: &SyncState) -> Result<(), StoreError> {
    if state.version != VERSION {
        return Err(StoreError::Corrupt(format!(
            "unsupported version {}",
            state.version
        )));
    }
    if let Some(scope) = &state.scope {
        validate_scope(scope)?;
    }
    for (id, op) in &state.pending {
        if id != &op.operation_id {
            return Err(StoreError::Corrupt("pending operation key mismatch".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use any_cal_core::{
        AnytypeObjectId, BindingLifecycle, CanonicalDocument, CollectionId, DavKind, DavRoute,
        DavUid, DomainBinding, StructuredDocument, VisibilityIntent,
    };
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "any-cal-sync-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn clean(p: &Path) {
        let _ = fs::remove_file(p);
        let _ = fs::remove_file(backup(p));
        let _ = fs::remove_file(p.with_extension("tmp"));
        let _ = fs::remove_file(p.with_extension("export"));
    }
    fn env() -> ResourceEnvelope {
        ResourceEnvelope {
            collection_id: CollectionId::try_from("contacts").unwrap(),
            resource_id: ResourceId::try_from("r1").unwrap(),
            kind: DavKind::Contact,
            anytype_object_id: AnytypeObjectId::try_from("o1").unwrap(),
            dav_uid: DavUid::try_from("u1").unwrap(),
            document: CanonicalDocument::new(StructuredDocument::default()),
            revision: 1,
        }
    }
    fn op(id: &str, kind: OperationKind) -> PendingOperation {
        PendingOperation {
            operation_id: id.into(),
            kind,
            resource_id: ResourceId::try_from("r1").unwrap(),
            expected_etag: None,
            expected_revision: Some(1),
            envelope: Some(env()),
            attempts: 0,
        }
    }

    fn scope(space_id: &str, account: &str, endpoint: &str) -> SyncScope {
        let binding = DomainBinding {
            domain_id: "personal-contacts".into(),
            label: "Personal contacts".into(),
            space_id: space_id.into(),
            credential_profile_id: "personal-account".into(),
            routes: vec![DavRoute::contacts("/dav/contacts/personal")],
            schema_profile: "default".into(),
            checkpoint_namespace: "personal-contacts".into(),
            visibility: VisibilityIntent::Private,
            lifecycle: BindingLifecycle::Configured,
        };
        SyncScope::for_binding(&binding, endpoint, account).unwrap()
    }

    #[test]
    fn sync_scope_qualifies_binding_endpoint_account_and_space() {
        let base = scope("space-a", "account-a", "https://anytype.example.test");
        assert_eq!(base.domain_id, "personal-contacts");
        assert_eq!(base.space_id, "space-a");
        assert_eq!(base.endpoint_fingerprint.len(), 64);
        assert_eq!(base.binding_fingerprint.len(), 64);

        assert_ne!(
            base,
            scope("space-b", "account-a", "https://anytype.example.test")
        );
        assert_ne!(
            base,
            scope("space-a", "account-b", "https://anytype.example.test")
        );
        assert_ne!(
            base,
            scope("space-a", "account-a", "https://other.example.test")
        );
    }

    #[test]
    fn scoped_checkpoint_rejects_mismatched_reopen_and_unscoped_bypass() {
        let p = path();
        let expected = scope("space-a", "account-a", "https://anytype.example.test");
        {
            let mut store = SyncStore::open_scoped(&p, expected.clone()).unwrap();
            assert_eq!(store.scope(), Some(&expected));
            store.enqueue(op("bound-op", OperationKind::Update)).unwrap();
        }

        {
            let reopened = SyncStore::open_scoped(&p, expected.clone()).unwrap();
            assert_eq!(reopened.pending().count(), 1);
        }

        assert!(matches!(
            SyncStore::open_scoped(
                &p,
                scope("space-b", "account-a", "https://anytype.example.test")
            ),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            SyncStore::open_scoped(
                &p,
                scope("space-a", "account-b", "https://anytype.example.test")
            ),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            SyncStore::open_scoped(
                &p,
                scope("space-a", "account-a", "https://other.example.test")
            ),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            SyncStore::open(&p),
            Err(StoreError::Invalid(_))
        ));
        clean(&p);
    }

    #[test]
    fn nonempty_legacy_checkpoint_cannot_silently_adopt_scope() {
        let p = path();
        {
            let mut legacy = SyncStore::open(&p).unwrap();
            legacy.enqueue(op("legacy-op", OperationKind::Update)).unwrap();
        }

        assert!(matches!(
            SyncStore::open_scoped(
                &p,
                scope("space-a", "account-a", "https://anytype.example.test")
            ),
            Err(StoreError::Invalid(_))
        ));
        clean(&p);
    }

    #[test]
    fn scoped_restore_rejects_export_from_another_binding() {
        let destination = path();
        let source = path().with_extension("source.json");
        let export = source.with_extension("export");

        let destination_scope =
            scope("space-a", "account-a", "https://anytype.example.test");
        let source_scope =
            scope("space-b", "account-a", "https://anytype.example.test");

        let mut destination_store =
            SyncStore::open_scoped(&destination, destination_scope.clone()).unwrap();
        destination_store
            .enqueue(op("destination-op", OperationKind::Update))
            .unwrap();
        let before = destination_store.state().clone();

        {
            let mut source_store =
                SyncStore::open_scoped(&source, source_scope).unwrap();
            source_store
                .enqueue(op("source-op", OperationKind::Create))
                .unwrap();
            source_store.export_to(&export).unwrap();
        }

        assert!(matches!(
            destination_store.restore_export(&export),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(destination_store.state(), &before);

        drop(destination_store);
        clean(&destination);
        clean(&source);
        let _ = fs::remove_file(export);
    }
    #[test]
    fn restart_preserves_pending() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.enqueue(op("a", OperationKind::Update)).unwrap();
        let g = s.state().generation;
        drop(s);
        let s = SyncStore::open(&p).unwrap();
        assert_eq!(s.state().generation, g);
        assert_eq!(s.pending().count(), 1);
        clean(&p)
    }

    #[test]
    fn export_restore_preserves_sync_state_and_is_private() {
        let source = path();
        let export = source.with_extension("export");
        let destination = source.with_extension("restored");
        let mut store = SyncStore::open(&source).unwrap();
        store
            .observe(ObservedResource {
                resource_id: ResourceId::try_from("r1").unwrap(),
                anytype_object_id: "object-1".into(),
                dav_uid: "uid-1".into(),
                revision: 9,
                etag: "etag-9".into(),
                modified_at: 9,
                archived: false,
            })
            .unwrap();
        store
            .enqueue(op("pending-1", OperationKind::Update))
            .unwrap();
        store
            .record_tombstone(Tombstone {
                resource_id: ResourceId::try_from("r2").unwrap(),
                anytype_object_id: "object-2".into(),
                dav_uid: "uid-2".into(),
                revision: 4,
                etag: "etag-4".into(),
                modified_at: 4,
            })
            .unwrap();
        let before = store.state().clone();
        let receipt = store.export_to(&export).unwrap();
        assert_eq!(receipt.observed, 1);
        assert_eq!(receipt.pending, 1);
        assert_eq!(receipt.tombstones, 1);
        drop(store);
        let restored = SyncStore::restore_from(&export, &destination).unwrap();
        assert_eq!(restored.state_generation, before.generation);
        let reopened = SyncStore::open(&destination).unwrap();
        assert_eq!(reopened.state(), &before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&export).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(reopened);
        clean(&source);
        clean(&export);
        clean(&destination);
    }

    #[test]
    fn restore_rejects_tampering_and_leaves_destination_unchanged() {
        let source = path();
        let export = source.with_extension("export");
        let destination = source.with_extension("restored");
        let mut store = SyncStore::open(&source).unwrap();
        store
            .observe(ObservedResource {
                resource_id: ResourceId::try_from("r1").unwrap(),
                anytype_object_id: "object-1".into(),
                dav_uid: "uid-1".into(),
                revision: 1,
                etag: "etag-1".into(),
                modified_at: 1,
                archived: false,
            })
            .unwrap();
        store.export_to(&export).unwrap();
        drop(store);
        fs::write(&destination, b"active-state").unwrap();
        let original = fs::read(&destination).unwrap();
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&export).unwrap()).unwrap();
        document["payload"]["generation"] = serde_json::json!(999);
        fs::write(&export, serde_json::to_vec(&document).unwrap()).unwrap();
        let error = SyncStore::restore_from(&export, &destination).unwrap_err();
        assert!(error.to_string().contains("integrity"));
        assert_eq!(fs::read(&destination).unwrap(), original);
        clean(&source);
        clean(&export);
        clean(&destination);
    }

    #[test]
    fn restore_rejects_truncated_and_incompatible_exports() {
        let source = path();
        let export = source.with_extension("export");
        let destination = source.with_extension("restored");
        let store = SyncStore::open(&source).unwrap();
        store.export_to(&export).unwrap();
        drop(store);
        fs::write(&destination, b"unchanged").unwrap();
        let original = fs::read(&destination).unwrap();
        fs::write(&export, b"{\"format\":\"any-cal.sync-export\"").unwrap();
        assert!(SyncStore::restore_from(&export, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), original);
        let store = SyncStore::open(&source).unwrap();
        store.export_to(&export).unwrap();
        drop(store);
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&export).unwrap()).unwrap();
        document["export_version"] = serde_json::json!(2);
        fs::write(&export, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(SyncStore::restore_from(&export, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), original);
        clean(&source);
        clean(&export);
        clean(&destination);
    }

    #[cfg(unix)]
    #[test]
    fn export_and_restore_reject_symlink_paths() {
        use std::os::unix::fs::symlink;
        let source = path();
        let target = source.with_extension("target");
        let link = source.with_extension("link");
        fs::write(&target, b"target").unwrap();
        symlink(&target, &link).unwrap();
        let store = SyncStore::open(&source).unwrap();
        assert!(store.export_to(&link).is_err());
        drop(store);
        let _ = fs::remove_file(&link);
        clean(&source);
        clean(&target);
    }
    #[cfg(unix)]
    #[test]
    fn checkpoint_backup_and_lock_are_private_files() {
        use std::os::unix::fs::PermissionsExt;
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        assert_eq!(
            fs::metadata(p.with_extension("lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        s.enqueue(op("a", OperationKind::Update)).unwrap();
        s.enqueue(op("b", OperationKind::Update)).unwrap();
        drop(s);
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(backup(&p)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        clean(&p)
    }
    #[test]
    fn duplicate_is_idempotent_but_drift_rejected() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.enqueue(op("a", OperationKind::Update)).unwrap();
        s.enqueue(op("a", OperationKind::Update)).unwrap();
        assert!(s.enqueue(op("a", OperationKind::Delete)).is_err());
        clean(&p)
    }
    #[test]
    fn corrupt_primary_recovers_backup() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.enqueue(op("a", OperationKind::Update)).unwrap();
        s.enqueue(op("b", OperationKind::Update)).unwrap();
        drop(s);
        fs::write(&p, b"broken").unwrap();
        let s = SyncStore::open(&p).unwrap();
        assert_eq!(s.pending().count(), 1);
        clean(&p)
    }

    #[test]
    fn recovery_does_not_overwrite_valid_backup_before_republish() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.enqueue(op("a", OperationKind::Update)).unwrap();
        s.enqueue(op("b", OperationKind::Update)).unwrap();
        drop(s);

        fs::write(&p, b"truncated checkpoint").unwrap();
        let mut recovered = SyncStore::open(&p).unwrap();
        assert_eq!(recovered.pending().count(), 1);
        recovered.inject_commit_fault(CommitFault::BeforeRename);
        assert!(recovered.enqueue(op("c", OperationKind::Update)).is_err());
        drop(recovered);

        let reopened = SyncStore::open(&p).unwrap();
        assert_eq!(reopened.pending().count(), 1);
        assert!(reopened
            .pending()
            .any(|operation| operation.operation_id == "a"));
        drop(reopened);
        clean(&p)
    }

    #[test]
    fn unrecoverable_corruption_does_not_leave_a_stale_lock() {
        let p = path();
        fs::write(&p, b"truncated primary").unwrap();
        fs::write(backup(&p), b"truncated backup").unwrap();
        assert!(matches!(SyncStore::open(&p), Err(StoreError::Corrupt(_))));
        assert!(!p.with_extension("lock").exists());
        assert!(matches!(SyncStore::open(&p), Err(StoreError::Corrupt(_))));
        assert!(!p.with_extension("lock").exists());
        clean(&p)
    }
    #[test]
    fn tombstone_removes_observed() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        let id = ResourceId::try_from("r1").unwrap();
        s.observe(ObservedResource {
            resource_id: id.clone(),
            anytype_object_id: "o1".into(),
            dav_uid: "u1".into(),
            revision: 2,
            etag: any_cal_core::typed_etag_for_bytes(b"x").as_str().to_owned(),
            modified_at: 2,
            archived: false,
        })
        .unwrap();
        s.record_tombstone(Tombstone {
            resource_id: id.clone(),
            anytype_object_id: "o1".into(),
            dav_uid: "u1".into(),
            revision: 3,
            etag: any_cal_core::typed_etag_for_bytes(b"y").as_str().to_owned(),
            modified_at: 3,
        })
        .unwrap();
        assert!(!s.state().observed.contains_key(&id));
        assert!(s.state().tombstones.contains_key(&id));
        clean(&p)
    }
    #[test]
    fn non_transient_is_not_retryable() {
        assert!(SyncStore::retryable(&op("a", OperationKind::Update), false).is_err());
        assert_eq!(
            SyncStore::retryable(&op("a", OperationKind::Update), true).unwrap(),
            1
        )
    }

    #[test]
    fn drift_with_pending_write_is_never_an_implicit_overwrite() {
        let old = ObservedResource {
            resource_id: ResourceId::try_from("r1").unwrap(),
            anytype_object_id: "o1".into(),
            dav_uid: "u1".into(),
            revision: 1,
            etag: "a".into(),
            modified_at: 1,
            archived: false,
        };
        let new = ObservedResource {
            etag: "b".into(),
            revision: 2,
            ..old.clone()
        };
        assert_eq!(
            classify(Some(&old), Some(&new), true),
            Reconciliation::Conflict
        );
        assert_eq!(
            classify(Some(&old), Some(&new), false),
            Reconciliation::RemoteChanged
        );
    }

    #[test]
    fn failed_commit_does_not_advance_state_and_lock_is_exclusive() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.inject_commit_fault(CommitFault::BeforeRename);
        assert!(s.enqueue(op("a", OperationKind::Update)).is_err());
        assert!(s.state().pending.is_empty());
        assert!(SyncStore::open(&p).is_err());
        drop(s);
        let mut reopened = SyncStore::open(&p).unwrap();
        assert!(reopened.state().pending.is_empty());
        reopened.inject_commit_fault(CommitFault::PartialWrite);
        assert!(reopened.enqueue(op("b", OperationKind::Update)).is_err());
        assert!(reopened.state().pending.is_empty());
        clean(&p);
    }

    #[test]
    fn replacing_snapshot_is_one_atomic_publication() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.observe(ObservedResource {
            resource_id: ResourceId::try_from("old").unwrap(),
            anytype_object_id: "o-old".into(),
            dav_uid: "u-old".into(),
            revision: 1,
            etag: "e-old".into(),
            modified_at: 1,
            archived: false,
        })
        .unwrap();
        s.inject_commit_fault(CommitFault::BeforeWrite);
        let result = s.replace_observed([ObservedResource {
            resource_id: ResourceId::try_from("new").unwrap(),
            anytype_object_id: "o-new".into(),
            dav_uid: "u-new".into(),
            revision: 2,
            etag: "e-new".into(),
            modified_at: 2,
            archived: false,
        }]);
        assert!(result.is_err());
        assert!(s
            .state()
            .observed
            .contains_key(&ResourceId::try_from("old").unwrap()));
        assert!(!s
            .state()
            .observed
            .contains_key(&ResourceId::try_from("new").unwrap()));
        clean(&p);
    }

    #[test]
    fn tombstoned_reappearance_requires_explicit_resurrection() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        let id = ResourceId::try_from("r1").unwrap();
        let item = ObservedResource {
            resource_id: id.clone(),
            anytype_object_id: "o1".into(),
            dav_uid: "u1".into(),
            revision: 1,
            etag: "e1".into(),
            modified_at: 1,
            archived: false,
        };
        s.observe(item.clone()).unwrap();
        s.record_tombstone(Tombstone {
            resource_id: id.clone(),
            anytype_object_id: "o1".into(),
            dav_uid: "u1".into(),
            revision: 2,
            etag: "e2".into(),
            modified_at: 2,
        })
        .unwrap();
        assert!(matches!(
            s.replace_observed([item.clone()]),
            Err(StoreError::Resurrection(_))
        ));
        assert!(!s.state().observed.contains_key(&id));
        s.replace_observed_explicit_resurrection([item]).unwrap();
        assert!(s.state().observed.contains_key(&id));
        assert!(!s.state().tombstones.contains_key(&id));
        clean(&p);
    }

    #[test]
    fn symlink_checkpoint_is_rejected() {
        let p = path();
        let target = p.with_extension("target");
        fs::write(&target, b"{}").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &p).unwrap();
        #[cfg(unix)]
        assert!(SyncStore::open(&p).is_err());
        let _ = fs::remove_file(&p);
        let _ = fs::remove_file(&target);
    }

    #[test]
    fn transient_retry_increments_attempt_without_mutating_operation() {
        let operation = op("retry", OperationKind::Update);
        assert_eq!(SyncStore::retryable(&operation, true).unwrap(), 1);
        assert_eq!(operation.attempts, 0);
        assert!(SyncStore::retryable(&operation, false).is_err());
    }

    #[test]
    fn ambiguous_mutation_requires_reconciliation_and_retry_is_bounded() {
        let operation = op("ambiguous", OperationKind::Update);
        assert_eq!(
            retry_decision(&operation, true, MutationOutcome::Unknown),
            RetryDecision::Reconcile
        );
        assert_eq!(
            retry_decision(&operation, true, MutationOutcome::NotSent),
            RetryDecision::Retry { attempt: 1 }
        );
        let mut exhausted = operation;
        exhausted.attempts = MAX_RETRY_ATTEMPTS;
        assert_eq!(
            retry_decision(&exhausted, true, MutationOutcome::NotSent),
            RetryDecision::Stop
        );
    }

    #[test]
    fn later_write_wins_policy_records_stale_concurrent_write_without_claiming_prevention() {
        let old = ObservedResource {
            resource_id: ResourceId::try_from("r1").unwrap(),
            anytype_object_id: "o1".into(),
            dav_uid: "u1".into(),
            revision: 1,
            etag: "old".into(),
            modified_at: 1,
            archived: false,
        };
        let newer = ObservedResource {
            revision: 2,
            etag: "new".into(),
            ..old.clone()
        };
        assert_eq!(
            classify_with_policy(Some(&old), Some(&newer), true, ANYTYPE_CONFLICT_POLICY),
            Reconciliation::RemoteChanged
        );
    }

    #[test]
    fn directory_sync_failure_does_not_publish_candidate_state() {
        let p = path();
        let mut s = SyncStore::open(&p).unwrap();
        s.observe(ObservedResource {
            resource_id: ResourceId::try_from("stable").unwrap(),
            anytype_object_id: "o-stable".into(),
            dav_uid: "u-stable".into(),
            revision: 1,
            etag: "e-stable".into(),
            modified_at: 1,
            archived: false,
        })
        .unwrap();
        s.inject_commit_fault(CommitFault::BeforeDirectorySync);
        assert!(s
            .observe(ObservedResource {
                resource_id: ResourceId::try_from("candidate").unwrap(),
                anytype_object_id: "o-candidate".into(),
                dav_uid: "u-candidate".into(),
                revision: 2,
                etag: "e-candidate".into(),
                modified_at: 2,
                archived: false,
            })
            .is_err());
        assert!(s
            .state()
            .observed
            .contains_key(&ResourceId::try_from("stable").unwrap()));
        assert!(!s
            .state()
            .observed
            .contains_key(&ResourceId::try_from("candidate").unwrap()));
        clean(&p);
    }
}
