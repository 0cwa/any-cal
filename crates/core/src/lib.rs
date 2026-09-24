//! Language-neutral data contracts used by the DAV adapters.
pub mod bridge;
pub mod composition;
pub mod domain;
pub mod envelope;
pub mod etag;
pub mod freebusy;
pub mod ical;
pub mod memory_repository;
pub mod model;
pub mod person_context;
pub mod repository;
pub mod vcard;
pub mod vfreebusy;
pub mod vtodo;

pub use bridge::{
    BridgeCheckpoint, BridgeContractError, BridgeDecision, BridgeError, BridgeErrorCode,
    BridgeRequest, BridgeResponse, BridgeTombstone, SyncDecision, BRIDGE_SCHEMA_VERSION,
};
pub use composition::{
    apply_destination_mutation, reconcile_source_snapshot, CompositionError, CompositionProfile,
    DestinationDeletion, DestinationMutation, DestinationMutationResult, FieldOwnership,
    ForeignObjectRef, MaterializedReference, MergeResult, ProjectionPolicy, RefreshKind,
    SourceAvailability, SourceSnapshot, COMPOSITION_PROFILE_SCHEMA_VERSION,
    MATERIALIZED_REFERENCE_SCHEMA_VERSION,
};
pub use domain::{
    BindingLifecycle, DavComponent, DavRoute, DomainBinding, DomainBindings, DomainBindingsError,
    DomainCollection, VisibilityIntent, DOMAIN_BINDINGS_SCHEMA_VERSION,
};
pub use envelope::{CanonicalDocument, EnvelopeError, ResourceEnvelope};
pub use etag::{etag, etag_for_bytes, typed_etag_for_bytes, ETag};
pub use memory_repository::MemoryRepository;
pub use person_context::{
    effective_person_context_title, person_context_source_snapshot, project_person_context_source,
    PersonContextProjectionError, PERSON_CONTEXT_DAV_UID, PERSON_CONTEXT_DISPLAY_NAME,
    PERSON_CONTEXT_EMAILS, PERSON_CONTEXT_ORGANIZATIONS, PERSON_CONTEXT_PHONES,
    PERSON_CONTEXT_TITLE_OVERRIDE,
};
pub use model::{
    AnytypeObjectId, Collection, CollectionId, DavKind, DavUid, Occurrence, Resource, ResourceId,
    StructuredDocument,
};
pub use repository::{
    FailureMode, ModifiedAt, Repository, RepositoryError, StoredResource, WriteCondition,
};
