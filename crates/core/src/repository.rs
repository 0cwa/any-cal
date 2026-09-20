use crate::{Collection, CollectionId, ETag, ResourceEnvelope, ResourceId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// HTTP-date-compatible, UTC second precision modification timestamp.
///
/// DAV dates deliberately do not retain sub-second precision.  The
/// repository clamps each new value to at least one second after the prior
/// value, so rapid writes and a backwards-moving wall clock cannot make a
/// resource appear older than its previous representation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ModifiedAt(u64);

impl ModifiedAt {
    pub const UNIX_EPOCH: Self = Self(0);

    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    pub fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        Self(seconds)
    }

    pub const fn after(self, wall_clock: Self) -> Self {
        if wall_clock.0 > self.0 {
            wall_clock
        } else {
            Self(self.0.saturating_add(1))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredResource {
    pub envelope: ResourceEnvelope,
    pub etag: ETag,
    pub archived: bool,
    /// Persisted separately from the canonical envelope so ETag identity and
    /// canonical document bytes remain unchanged by clock metadata.
    #[serde(default)]
    pub modified_at: ModifiedAt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriteCondition {
    Unconditional,
    IfMatch(ETag),
    IfNoneMatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureMode {
    Timeout,
    MalformedState,
    ArchiveFailure,
    /// A synthetic transient result used by the fake to model eventual
    /// consistency. This is deliberately distinct from a genuine absence so
    /// an adapter can retry rather than return HTTP 404.
    ReadAfterWriteDelay,
    DuplicateWrite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositoryError {
    InvalidEnvelope(String),
    CollectionNotFound(CollectionId),
    ResourceNotFound(ResourceId),
    CollectionAlreadyExists(CollectionId),
    ResourceAlreadyExists(ResourceId),
    IdentityAlreadyExists(String),
    PreconditionFailed {
        expected: Option<ETag>,
        actual: Option<ETag>,
    },
    Timeout,
    Auth,
    Forbidden,
    RateLimited,
    Unavailable,
    MalformedState,
    ArchiveFailure,
    /// A synthetic transient result used by the fake to model eventual
    /// consistency. This is deliberately distinct from a genuine absence so
    /// an adapter can retry rather than return HTTP 404.
    ReadAfterWriteDelay,
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvelope(reason) => write!(f, "invalid envelope: {reason}"),
            Self::CollectionNotFound(id) => write!(f, "collection not found: {id}"),
            Self::ResourceNotFound(id) => write!(f, "resource not found: {id}"),
            Self::CollectionAlreadyExists(id) => write!(f, "collection already exists: {id}"),
            Self::ResourceAlreadyExists(id) => write!(f, "resource already exists: {id}"),
            Self::IdentityAlreadyExists(id) => write!(f, "identity already exists: {id}"),
            Self::PreconditionFailed { expected, actual } => {
                write!(
                    f,
                    "precondition failed (expected {expected:?}, actual {actual:?})"
                )
            }
            Self::Timeout => f.write_str("repository operation timed out"),
            Self::Auth => f.write_str("repository authentication failed"),
            Self::Forbidden => f.write_str("repository access was forbidden"),
            Self::RateLimited => f.write_str("repository request was rate limited"),
            Self::Unavailable => f.write_str("repository service is unavailable"),
            Self::MalformedState => f.write_str("repository state is malformed"),
            Self::ArchiveFailure => f.write_str("resource archive failed"),
            Self::ReadAfterWriteDelay => f.write_str("resource visibility is delayed"),
        }
    }
}

impl std::error::Error for RepositoryError {}

pub trait Repository {
    fn list_collections(&mut self) -> Result<Vec<Collection>, RepositoryError>;
    fn get_collection(&mut self, id: &CollectionId) -> Result<Option<Collection>, RepositoryError>;
    fn create_collection(&mut self, collection: Collection) -> Result<(), RepositoryError>;
    fn delete_collection(&mut self, id: &CollectionId) -> Result<(), RepositoryError>;

    fn list_resources(
        &mut self,
        collection_id: &CollectionId,
        include_archived: bool,
    ) -> Result<Vec<StoredResource>, RepositoryError>;
    fn get_resource(&mut self, id: &ResourceId) -> Result<Option<StoredResource>, RepositoryError>;
    fn create_resource(
        &mut self,
        envelope: ResourceEnvelope,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError>;
    fn update_resource(
        &mut self,
        envelope: ResourceEnvelope,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError>;
    fn archive_resource(
        &mut self,
        id: &ResourceId,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError>;
    fn delete_resource(
        &mut self,
        id: &ResourceId,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError>;
}
