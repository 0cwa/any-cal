//! Language-neutral data contracts used by the DAV adapters.
pub mod bridge;
pub mod domain;
pub mod envelope;
pub mod etag;
pub mod freebusy;
pub mod ical;
pub mod memory_repository;
pub mod model;
pub mod repository;
pub mod vcard;
pub mod vfreebusy;
pub mod vtodo;

pub use bridge::{
    BridgeCheckpoint, BridgeContractError, BridgeDecision, BridgeError, BridgeErrorCode,
    BridgeRequest, BridgeResponse, BridgeTombstone, SyncDecision, BRIDGE_SCHEMA_VERSION,
};
pub use domain::{
    BindingLifecycle, DavComponent, DavRoute, DomainBinding, DomainBindings, DomainBindingsError,
    DomainCollection, VisibilityIntent, DOMAIN_BINDINGS_SCHEMA_VERSION,
};
pub use envelope::{CanonicalDocument, EnvelopeError, ResourceEnvelope};
pub use etag::{etag, etag_for_bytes, typed_etag_for_bytes, ETag};
pub use memory_repository::MemoryRepository;
pub use model::{
    AnytypeObjectId, Collection, CollectionId, DavKind, DavUid, Occurrence, Resource, ResourceId,
    StructuredDocument,
};
pub use repository::{
    FailureMode, ModifiedAt, Repository, RepositoryError, StoredResource, WriteCondition,
};
