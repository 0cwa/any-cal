use crate::composition::{CompositionAuthorizationError, CompositionSourceScope};
use crate::identity::CollectionKind;
use crate::AppGeneric;
use any_cal_anytype_adapter::{
    AnytypeTransport, AnytypeTypedTransport, ObjectRecord, TransportError,
    ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY, ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY, ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY, ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
    DEFAULT_OBJECT_TYPE_KEY,
};
use any_cal_core::{
    reconcile_source_snapshot, CompositionError, CompositionProfile, ForeignObjectRef,
    MaterializedReference, RefreshKind, Repository, RepositoryError, ResourceId,
    SourceAvailability, SourceSnapshot, StoredResource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MATERIALIZED_RECORD_VERSION: u32 = 1;
pub const MATERIALIZED_RECORD_TYPE: &str = "any_cal_materialized_reference";
const MAX_DESTINATION_SCAN_PAGES: usize = 100;
const MAX_DESTINATION_SCAN_OBJECTS: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedReferenceRecord {
    pub version: u32,
    pub record_type: String,
    pub profile_fingerprint: String,
    pub source_domain_id: String,
    pub source_collection: CollectionKind,
    pub source_resource_id: String,
    pub reference: MaterializedReference,
}

impl MaterializedReferenceRecord {
    fn new(
        profile_fingerprint: String,
        source: &CompositionSourceScope,
        reference: MaterializedReference,
    ) -> Result<Self, CompositionOrchestrationError> {
        let resource_id = source.resource_id.clone().ok_or(
            CompositionOrchestrationError::SourceResourceRequired(source.domain_id.clone()),
        )?;
        let record = Self {
            version: MATERIALIZED_RECORD_VERSION,
            record_type: MATERIALIZED_RECORD_TYPE.into(),
            profile_fingerprint,
            source_domain_id: source.domain_id.clone(),
            source_collection: source.collection,
            source_resource_id: resource_id,
            reference,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn from_json(input: &str) -> Result<Self, CompositionOrchestrationError> {
        let record: Self = serde_json::from_str(input)
            .map_err(|_| CompositionOrchestrationError::MalformedDestinationRecord)?;
        record.validate()?;
        Ok(record)
    }

    pub fn canonical_json(&self) -> Result<String, CompositionOrchestrationError> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|_| CompositionOrchestrationError::MalformedDestinationRecord)
    }

    fn validate(&self) -> Result<(), CompositionOrchestrationError> {
        if self.version != MATERIALIZED_RECORD_VERSION
            || self.record_type != MATERIALIZED_RECORD_TYPE
            || self.profile_fingerprint.len() != 64
            || !self
                .profile_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.source_domain_id.trim().is_empty()
            || self.source_resource_id.trim().is_empty()
            || self.source_resource_id.chars().any(char::is_control)
            || matches!(self.source_collection, CollectionKind::Composition)
        {
            return Err(CompositionOrchestrationError::MalformedDestinationRecord);
        }
        self.reference
            .validate()
            .map_err(CompositionOrchestrationError::Composition)
    }

    fn correlation_key(&self) -> CorrelationKey {
        CorrelationKey {
            domain_id: self.source_domain_id.clone(),
            collection: self.source_collection,
            resource_id: self.source_resource_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationResult {
    pub source_domain_id: String,
    pub source_resource_id: String,
    pub destination_domain_id: String,
    pub destination_object_id: String,
    pub created: bool,
    pub refresh_kind: RefreshKind,
    pub reference: MaterializedReference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionOrchestrationError {
    Authorization(CompositionAuthorizationError),
    Composition(CompositionError),
    SourceResourceRequired(String),
    InvalidSourceResourceId(String),
    SourceNotFound {
        domain_id: String,
        resource_id: String,
    },
    SourceUnavailableWithoutReference {
        domain_id: String,
        resource_id: String,
    },
    SourceRepository {
        domain_id: String,
        error: RepositoryError,
    },
    SourceKindMismatch(String),
    DestinationTransport(TransportError),
    MalformedDestinationRecord,
    DuplicateDestinationReference,
    DestinationBindingMismatch,
    ScanLimitExceeded,
    RepeatedPaginationOffset,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CorrelationKey {
    domain_id: String,
    collection: CollectionKind,
    resource_id: String,
}

struct LoadedDestination {
    object: ObjectRecord,
    record: MaterializedReferenceRecord,
}

impl<T: AnytypeTransport + AnytypeTypedTransport> AppGeneric<T> {
    /// Read one or more authorized canonical source resources, reconcile them
    /// against private materialized references, and write only the configured
    /// destination domain. The projector is pure and decides which bounded
    /// canonical fields become source-owned cached fields.
    pub fn materialize_references<F>(
        &mut self,
        profile: &CompositionProfile,
        token: &str,
        source_scopes: &[CompositionSourceScope],
        projector: F,
    ) -> Result<Vec<MaterializationResult>, CompositionOrchestrationError>
    where
        F: Fn(&StoredResource) -> BTreeMap<String, Value>,
    {
        self.materialize_references_with_type(
            profile,
            token,
            source_scopes,
            DEFAULT_OBJECT_TYPE_KEY,
            projector,
        )
    }

    pub fn materialize_references_with_type<F>(
        &mut self,
        profile: &CompositionProfile,
        token: &str,
        source_scopes: &[CompositionSourceScope],
        destination_type_key: &str,
        projector: F,
    ) -> Result<Vec<MaterializationResult>, CompositionOrchestrationError>
    where
        F: Fn(&StoredResource) -> BTreeMap<String, Value>,
    {
        let plan = self
            .authorize_composition(profile, token, source_scopes)
            .map_err(CompositionOrchestrationError::Authorization)?;

        let mut existing = self.scan_destination_materialized(&plan.destination_domain_id)?;
        existing.retain(|loaded| loaded.record.profile_fingerprint == plan.profile_fingerprint);
        let mut by_correlation = BTreeMap::<CorrelationKey, usize>::new();
        let mut by_identity = BTreeMap::<String, usize>::new();
        for (index, loaded) in existing.iter().enumerate() {
            if by_correlation
                .insert(loaded.record.correlation_key(), index)
                .is_some()
            {
                return Err(CompositionOrchestrationError::DuplicateDestinationReference);
            }
            let identity = loaded
                .record
                .reference
                .foreign
                .identity_fingerprint()
                .map_err(CompositionOrchestrationError::Composition)?;
            if by_identity.insert(identity, index).is_some() {
                return Err(CompositionOrchestrationError::DuplicateDestinationReference);
            }
        }

        let profile_fingerprint = plan.profile_fingerprint.clone();
        let destination_domain_id = plan.destination_domain_id.clone();
        let mut results = Vec::with_capacity(plan.sources.len());
        let mut touched_destination_ids = BTreeSet::new();

        for authorized in &plan.sources {
            let source = source_scopes
                .iter()
                .find(|candidate| {
                    candidate.domain_id == authorized.domain_id
                        && candidate.collection == authorized.collection
                        && candidate.resource_id == authorized.resource_id
                })
                .expect("authorization plan is derived from source scopes");
            let resource_id_text = source.resource_id.as_deref().ok_or_else(|| {
                CompositionOrchestrationError::SourceResourceRequired(source.domain_id.clone())
            })?;
            let resource_id = ResourceId::try_from(resource_id_text).map_err(|_| {
                CompositionOrchestrationError::InvalidSourceResourceId(resource_id_text.to_owned())
            })?;
            let correlation = CorrelationKey {
                domain_id: source.domain_id.clone(),
                collection: source.collection,
                resource_id: resource_id_text.to_owned(),
            };
            let existing_index = by_correlation.get(&correlation).copied();

            let source_read = {
                let context = self
                    .registry
                    .context_mut(&source.domain_id)
                    .ok_or_else(|| {
                        CompositionOrchestrationError::Authorization(
                            CompositionAuthorizationError::UnknownDomain(source.domain_id.clone()),
                        )
                    })?;
                context.server.repository.get_resource(&resource_id)
            };

            let snapshot = match source_read {
                Ok(Some(stored)) => {
                    self.registry
                        .observe_upstream_response(&source.domain_id, false, 200);
                    validate_source_kind(source.collection, &stored)?;
                    SourceSnapshot::new(
                        ForeignObjectRef::new(
                            self.registry
                                .context(&source.domain_id)
                                .expect("source context remains configured")
                                .server
                                .repository
                                .binding
                                .upstream_account_fingerprint
                                .clone(),
                            self.registry
                                .context(&source.domain_id)
                                .expect("source context remains configured")
                                .binding
                                .space_id
                                .clone(),
                            stored.envelope.anytype_object_id.to_string(),
                            stored.envelope.kind.clone(),
                            Some(stored.envelope.dav_uid.to_string()),
                        )
                        .map_err(CompositionOrchestrationError::Composition)?,
                        projector(&stored),
                        if stored.archived {
                            SourceAvailability::Archived
                        } else {
                            SourceAvailability::Available
                        },
                    )
                    .map_err(CompositionOrchestrationError::Composition)?
                }
                Ok(None) => {
                    let Some(index) = existing_index else {
                        return Err(CompositionOrchestrationError::SourceNotFound {
                            domain_id: source.domain_id.clone(),
                            resource_id: resource_id_text.to_owned(),
                        });
                    };
                    SourceSnapshot::new(
                        existing[index].record.reference.foreign.clone(),
                        BTreeMap::new(),
                        SourceAvailability::Deleted,
                    )
                    .map_err(CompositionOrchestrationError::Composition)?
                }
                Err(RepositoryError::Auth) => {
                    self.registry
                        .observe_upstream_response(&source.domain_id, false, 401);
                    unavailable_snapshot(&existing, existing_index, source, resource_id_text)?
                }
                Err(RepositoryError::Forbidden) => {
                    self.registry
                        .observe_upstream_response(&source.domain_id, false, 403);
                    unavailable_snapshot(&existing, existing_index, source, resource_id_text)?
                }
                Err(error) => {
                    return Err(CompositionOrchestrationError::SourceRepository {
                        domain_id: source.domain_id.clone(),
                        error,
                    })
                }
            };

            let identity = snapshot
                .foreign
                .identity_fingerprint()
                .map_err(CompositionOrchestrationError::Composition)?;
            let identity_index = by_identity.get(&identity).copied();
            let selected_index = match (existing_index, identity_index) {
                (Some(left), Some(right)) if left != right => {
                    return Err(CompositionOrchestrationError::DuplicateDestinationReference)
                }
                (Some(index), _) | (_, Some(index)) => Some(index),
                (None, None) => None,
            };

            let merge = reconcile_source_snapshot(
                selected_index.map(|index| &existing[index].record.reference),
                &snapshot,
            )
            .map_err(CompositionOrchestrationError::Composition)?;
            let persisted = MaterializedReferenceRecord::new(
                profile_fingerprint.clone(),
                source,
                merge.reference.clone(),
            )?;
            let body = persisted.canonical_json()?;

            let (object, created) = match selected_index {
                Some(index) => {
                    let mut object = existing[index].object.clone();
                    object.body = body;
                    object.archived = false;
                    apply_managed_reference_properties(&mut object, &persisted)?;
                    let result = self.write_destination_object(
                        &destination_domain_id,
                        object,
                        false,
                        destination_type_key,
                        &persisted,
                    )?;
                    existing[index] = LoadedDestination {
                        object: result.clone(),
                        record: persisted.clone(),
                    };
                    (result, false)
                }
                None => {
                    let placeholder_id = format!(
                        "anycal-ref-{}-{}",
                        profile_fingerprint.chars().take(12).collect::<String>(),
                        identity.chars().take(20).collect::<String>()
                    );
                    let destination_space = self
                        .registry
                        .context(&destination_domain_id)
                        .expect("authorized destination remains configured")
                        .binding
                        .space_id
                        .clone();
                    let mut object = ObjectRecord {
                        id: placeholder_id,
                        space_id: destination_space,
                        properties: Vec::new(),
                        property_formats: BTreeMap::new(),
                        body,
                        archived: false,
                        revision: 0,
                    };
                    apply_managed_reference_properties(&mut object, &persisted)?;
                    let result = self.write_destination_object(
                        &destination_domain_id,
                        object,
                        true,
                        destination_type_key,
                        &persisted,
                    )?;
                    let index = existing.len();
                    existing.push(LoadedDestination {
                        object: result.clone(),
                        record: persisted.clone(),
                    });
                    by_correlation.insert(correlation.clone(), index);
                    by_identity.insert(identity.clone(), index);
                    (result, true)
                }
            };

            if !touched_destination_ids.insert(object.id.clone()) {
                return Err(CompositionOrchestrationError::DuplicateDestinationReference);
            }
            results.push(MaterializationResult {
                source_domain_id: source.domain_id.clone(),
                source_resource_id: resource_id_text.to_owned(),
                destination_domain_id: destination_domain_id.clone(),
                destination_object_id: object.id,
                created,
                refresh_kind: merge.kind,
                reference: merge.reference,
            });
        }

        Ok(results)
    }

    fn scan_destination_materialized(
        &mut self,
        domain_id: &str,
    ) -> Result<Vec<LoadedDestination>, CompositionOrchestrationError> {
        let context = self.registry.context(domain_id).ok_or_else(|| {
            CompositionOrchestrationError::Authorization(
                CompositionAuthorizationError::UnknownDomain(domain_id.to_owned()),
            )
        })?;
        let space_id = context.binding.space_id.clone();
        let mut cursor = None::<String>;
        let mut rows = Vec::new();

        for _ in 0..MAX_DESTINATION_SCAN_PAGES {
            let page = {
                let context = self
                    .registry
                    .context_mut(domain_id)
                    .expect("destination context remains configured");
                context
                    .server
                    .repository
                    .transport
                    .list_objects(&space_id, cursor.as_deref())
            }
            .map_err(CompositionOrchestrationError::DestinationTransport)?;

            for listed in page.data {
                if rows.len() >= MAX_DESTINATION_SCAN_OBJECTS {
                    return Err(CompositionOrchestrationError::ScanLimitExceeded);
                }
                if listed.space_id != space_id {
                    return Err(CompositionOrchestrationError::DestinationBindingMismatch);
                }
                if listed.archived {
                    continue;
                }
                let object = if listed.body.is_empty() {
                    let context = self
                        .registry
                        .context_mut(domain_id)
                        .expect("destination context remains configured");
                    context
                        .server
                        .repository
                        .transport
                        .get_object(&space_id, &listed.id)
                        .map_err(CompositionOrchestrationError::DestinationTransport)?
                } else {
                    listed
                };
                if object.space_id != space_id {
                    return Err(CompositionOrchestrationError::DestinationBindingMismatch);
                }
                match MaterializedReferenceRecord::from_json(&object.body) {
                    Ok(record) => rows.push(LoadedDestination { object, record }),
                    Err(_) if object.body.contains(MATERIALIZED_RECORD_TYPE) => {
                        return Err(CompositionOrchestrationError::MalformedDestinationRecord)
                    }
                    Err(_) => {}
                }
            }

            match page.next_offset {
                Some(next) if cursor.as_deref() == Some(next.as_str()) => {
                    return Err(CompositionOrchestrationError::RepeatedPaginationOffset)
                }
                Some(next) => cursor = Some(next),
                None => return Ok(rows),
            }
        }

        Err(CompositionOrchestrationError::ScanLimitExceeded)
    }

    fn write_destination_object(
        &mut self,
        domain_id: &str,
        object: ObjectRecord,
        create: bool,
        destination_type_key: &str,
        expected_record: &MaterializedReferenceRecord,
    ) -> Result<ObjectRecord, CompositionOrchestrationError> {
        let space_id = self
            .registry
            .context(domain_id)
            .expect("authorized destination remains configured")
            .binding
            .space_id
            .clone();
        if object.space_id != space_id {
            return Err(CompositionOrchestrationError::DestinationBindingMismatch);
        }

        let result = {
            let context = self
                .registry
                .context_mut(domain_id)
                .expect("destination context remains configured");
            if create {
                context
                    .server
                    .repository
                    .transport
                    .create_object_with_type(object.clone(), destination_type_key)
            } else {
                context
                    .server
                    .repository
                    .transport
                    .update_object(object.clone())
            }
        };

        match result {
            Ok(actual) => {
                self.registry
                    .observe_upstream_response(domain_id, true, 200);
                if actual.space_id != space_id || (!create && actual.id != object.id) {
                    return Err(CompositionOrchestrationError::DestinationBindingMismatch);
                }
                Ok(actual)
            }
            Err(TransportError::Auth) => {
                self.registry
                    .observe_upstream_response(domain_id, true, 401);
                Err(CompositionOrchestrationError::DestinationTransport(
                    TransportError::Auth,
                ))
            }
            Err(TransportError::Forbidden) => {
                self.registry
                    .observe_upstream_response(domain_id, true, 403);
                Err(CompositionOrchestrationError::DestinationTransport(
                    TransportError::Forbidden,
                ))
            }
            Err(TransportError::Timeout) => {
                if create {
                    self.reconcile_destination_create(domain_id, expected_record)
                } else {
                    let actual = {
                        let context = self
                            .registry
                            .context_mut(domain_id)
                            .expect("destination context remains configured");
                        context
                            .server
                            .repository
                            .transport
                            .get_object(&space_id, &object.id)
                    }
                    .map_err(CompositionOrchestrationError::DestinationTransport)?;
                    let observed = MaterializedReferenceRecord::from_json(&actual.body)?;
                    if observed == *expected_record {
                        Ok(actual)
                    } else {
                        Err(CompositionOrchestrationError::DestinationTransport(
                            TransportError::Timeout,
                        ))
                    }
                }
            }
            Err(error) => Err(CompositionOrchestrationError::DestinationTransport(error)),
        }
    }

    fn reconcile_destination_create(
        &mut self,
        domain_id: &str,
        expected: &MaterializedReferenceRecord,
    ) -> Result<ObjectRecord, CompositionOrchestrationError> {
        let candidates = self.scan_destination_materialized(domain_id)?;
        let expected_identity = expected
            .reference
            .foreign
            .identity_fingerprint()
            .map_err(CompositionOrchestrationError::Composition)?;
        let mut matching = candidates.into_iter().filter(|candidate| {
            candidate
                .record
                .reference
                .foreign
                .identity_fingerprint()
                .ok()
                .as_deref()
                == Some(expected_identity.as_str())
                && candidate.record.profile_fingerprint == expected.profile_fingerprint
                && candidate.record.source_domain_id == expected.source_domain_id
                && candidate.record.source_resource_id == expected.source_resource_id
        });
        let first = matching
            .next()
            .ok_or(CompositionOrchestrationError::DestinationTransport(
                TransportError::Timeout,
            ))?;
        if matching.next().is_some() {
            return Err(CompositionOrchestrationError::DuplicateDestinationReference);
        }
        Ok(first.object)
    }
}

fn apply_managed_reference_properties(
    object: &mut ObjectRecord,
    record: &MaterializedReferenceRecord,
) -> Result<(), CompositionOrchestrationError> {
    let foreign_identity = record
        .reference
        .foreign
        .identity_fingerprint()
        .map_err(CompositionOrchestrationError::Composition)?;
    let source_status = match record.reference.source_availability {
        SourceAvailability::Available => "available",
        SourceAvailability::Archived => "archived",
        SourceAvailability::Deleted => "deleted",
        SourceAvailability::Unavailable => "unavailable",
    };
    let source_kind = match record.reference.foreign.kind {
        any_cal_core::DavKind::Contact => "contact",
        any_cal_core::DavKind::ContactGroup => "contact_group",
        any_cal_core::DavKind::Task => "task",
        any_cal_core::DavKind::Event => "event",
    };

    let mut managed = vec![
        (
            ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
            record.profile_fingerprint.clone(),
        ),
        (ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY, foreign_identity),
        (
            ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY,
            record
                .reference
                .foreign
                .upstream_account_fingerprint
                .clone(),
        ),
        (
            ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY,
            record.reference.foreign.source_space_id.clone(),
        ),
        (
            ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
            record.reference.foreign.source_object_id.clone(),
        ),
        (ANYCAL_SOURCE_KIND_PROPERTY_KEY, source_kind.into()),
        (ANYCAL_SOURCE_STATUS_PROPERTY_KEY, source_status.into()),
    ];
    if let Some(dav_uid) = record.reference.foreign.dav_uid.clone() {
        managed.push((ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY, dav_uid));
    }

    let managed_keys = managed
        .iter()
        .map(|(key, _)| *key)
        .collect::<BTreeSet<_>>();
    object
        .properties
        .retain(|(key, _)| !managed_keys.contains(key.as_str()));
    object
        .property_formats
        .retain(|key, _| !managed_keys.contains(key.as_str()));
    for (key, value) in managed {
        object.properties.push((key.into(), value));
        object.property_formats.insert(key.into(), "text".into());
    }
    object.properties.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(())
}

fn unavailable_snapshot(
    existing: &[LoadedDestination],
    existing_index: Option<usize>,
    source: &CompositionSourceScope,
    resource_id: &str,
) -> Result<SourceSnapshot, CompositionOrchestrationError> {
    let Some(index) = existing_index else {
        return Err(
            CompositionOrchestrationError::SourceUnavailableWithoutReference {
                domain_id: source.domain_id.clone(),
                resource_id: resource_id.to_owned(),
            },
        );
    };
    SourceSnapshot::new(
        existing[index].record.reference.foreign.clone(),
        BTreeMap::new(),
        SourceAvailability::Unavailable,
    )
    .map_err(CompositionOrchestrationError::Composition)
}

fn validate_source_kind(
    collection: CollectionKind,
    stored: &StoredResource,
) -> Result<(), CompositionOrchestrationError> {
    let valid = match collection {
        CollectionKind::Contacts => matches!(stored.envelope.kind, any_cal_core::DavKind::Contact),
        CollectionKind::Tasks => matches!(stored.envelope.kind, any_cal_core::DavKind::Task),
        CollectionKind::Composition => false,
    };
    valid.then_some(()).ok_or_else(|| {
        CompositionOrchestrationError::SourceKindMismatch(stored.envelope.resource_id.to_string())
    })
}
