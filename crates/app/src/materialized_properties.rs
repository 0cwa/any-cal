//! Schema-gated native indexes for durable materialized-reference records.
//!
//! The record body remains authoritative. These optional text properties are
//! emitted only as a complete, validated group; private properties never enter
//! that ownership set.
use crate::composition_orchestration::MaterializedReferenceRecord;
use crate::discovery::SpaceDiscoverySnapshot;
use crate::AppGeneric;
use any_cal_anytype_adapter::{
    AnytypeTransport, ObjectRecord, ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY, ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY, ANYCAL_SOURCE_KIND_PROPERTY_KEY,
    ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY, ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_STATUS_PROPERTY_KEY, DEFAULT_OBJECT_TYPE_KEY,
};
use any_cal_core::{CompositionError, DavKind, SourceAvailability};

pub(crate) const MANAGED_REFERENCE_KEYS: [&str; 8] = [
    ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY,
    ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
    ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
];

/// Bounded diagnostics contain only fixed schema keys, never upstream values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionSchemaError {
    SnapshotRequired,
    StaleSnapshot,
    RequiredTypeMissing,
    RequiredTypeAmbiguous,
    RequiredTypeUnavailable,
    RequiredTypeMalformed,
    PropertyFormatMismatch(&'static str),
    DuplicatePropertyKey(&'static str),
    PropertyDescriptorInvalid(&'static str),
    ExistingPropertyFormatMismatch(&'static str),
    DuplicateObjectProperty(&'static str),
}

/// Whether the destination schema supports optional native reference indexes.
/// Body-only mode retains the complete durable materialized-reference record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataProjection {
    BodyOnly,
    Native,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReferencePropertyPolicy {
    BodyOnly,
    Native,
}

impl<T: AnytypeTransport> AppGeneric<T> {
    pub(crate) fn materialized_property_policy(
        &self,
        domain_id: &str,
        type_key: &str,
    ) -> Result<ReferencePropertyPolicy, CompositionSchemaError> {
        let current = self.discovery_snapshot(domain_id);
        let stale = current.is_none()
            && self
                .discovery_snapshots
                .values()
                .any(|snapshot| snapshot.domain_id == domain_id);
        ReferencePropertyPolicy::from_snapshot(current, stale, type_key)
    }
}

impl ReferencePropertyPolicy {
    fn from_snapshot(
        snapshot: Option<&SpaceDiscoverySnapshot>,
        stale: bool,
        type_key: &str,
    ) -> Result<Self, CompositionSchemaError> {
        if stale {
            return Err(CompositionSchemaError::StaleSnapshot);
        }
        let Some(snapshot) = snapshot else {
            return if type_key == DEFAULT_OBJECT_TYPE_KEY {
                Ok(Self::BodyOnly)
            } else {
                Err(CompositionSchemaError::SnapshotRequired)
            };
        };

        if type_key != DEFAULT_OBJECT_TYPE_KEY {
            let mut types = snapshot.types.iter().filter(|kind| kind.key == type_key);
            let Some(kind) = types.next() else {
                return Err(CompositionSchemaError::RequiredTypeMissing);
            };
            if types.next().is_some() {
                return Err(CompositionSchemaError::RequiredTypeAmbiguous);
            }
            if !valid_descriptor_id(&kind.id) {
                return Err(CompositionSchemaError::RequiredTypeMalformed);
            }
            if !active_descriptor(&kind.extra) {
                return Err(CompositionSchemaError::RequiredTypeUnavailable);
            }
        }

        let mut complete = true;
        for key in MANAGED_REFERENCE_KEYS {
            let mut properties = snapshot
                .properties
                .iter()
                .filter(|property| property.key.as_deref() == Some(key));
            let Some(property) = properties.next() else {
                complete = false;
                continue;
            };
            if properties.next().is_some() {
                return Err(CompositionSchemaError::DuplicatePropertyKey(key));
            }
            if !valid_descriptor_id(&property.id) || !active_descriptor(&property.extra) {
                return Err(CompositionSchemaError::PropertyDescriptorInvalid(key));
            }
            if property.format != "text" {
                return Err(CompositionSchemaError::PropertyFormatMismatch(key));
            }
        }
        Ok(if complete {
            Self::Native
        } else {
            Self::BodyOnly
        })
    }

    pub(crate) fn projection(self) -> MetadataProjection {
        match self {
            Self::BodyOnly => MetadataProjection::BodyOnly,
            Self::Native => MetadataProjection::Native,
        }
    }

    /// Validate all loaded references before the first destination mutation.
    /// A conflicting native value must not be silently claimed by our schema.
    pub(crate) fn validate_existing(
        self,
        object: &ObjectRecord,
    ) -> Result<(), CompositionSchemaError> {
        if self == Self::BodyOnly {
            return Ok(());
        }
        for key in MANAGED_REFERENCE_KEYS {
            let mut values = object.properties.iter().filter(|(name, _)| name == key);
            if values.next().is_none() {
                continue;
            }
            if values.next().is_some() {
                return Err(CompositionSchemaError::DuplicateObjectProperty(key));
            }
            if object.property_formats.get(key).map(String::as_str) != Some("text") {
                return Err(CompositionSchemaError::ExistingPropertyFormatMismatch(key));
            }
        }
        Ok(())
    }

    /// A timed-out write is confirmed only when its native values can also be
    /// read back. Equal durable bodies alone do not prove an index-only update.
    pub(crate) fn confirms_write(self, intended: &ObjectRecord, actual: &ObjectRecord) -> bool {
        if self == Self::BodyOnly {
            return true;
        }
        if self.validate_existing(actual).is_err() {
            return false;
        }
        MANAGED_REFERENCE_KEYS.into_iter().all(|key| {
            let intended = intended
                .properties
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str());
            let actual = actual
                .properties
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str());
            match (intended, actual) {
                (Some(expected), Some(observed)) => expected == observed,
                (Some(""), None) if key == ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY => true,
                _ => false,
            }
        })
    }

    pub(crate) fn apply(
        self,
        object: &mut ObjectRecord,
        record: &MaterializedReferenceRecord,
    ) -> Result<(), CompositionError> {
        let managed = if self == Self::Native {
            Some([
                record.profile_fingerprint.clone(),
                record.reference.foreign.identity_fingerprint()?,
                record
                    .reference
                    .foreign
                    .upstream_account_fingerprint
                    .clone(),
                record.reference.foreign.source_space_id.clone(),
                record.reference.foreign.source_object_id.clone(),
                match record.reference.foreign.kind {
                    DavKind::Contact => "contact",
                    DavKind::ContactGroup => "contact_group",
                    DavKind::Task => "task",
                    DavKind::Event => "event",
                }
                .into(),
                // PATCH omission would retain an old server value. A managed
                // optional text field is explicitly cleared when absent.
                record.reference.foreign.dav_uid.clone().unwrap_or_default(),
                match record.reference.source_availability {
                    SourceAvailability::Available => "available",
                    SourceAvailability::Archived => "archived",
                    SourceAvailability::Deleted => "deleted",
                    SourceAvailability::Unavailable => "unavailable",
                }
                .into(),
            ])
        } else {
            None
        };

        // In body-only mode omit unvalidated native keys from the write rather
        // than replaying arbitrary values or clearing destination data. PATCH
        // preserves omitted properties; the durable body remains usable.
        object
            .properties
            .retain(|(key, _)| !MANAGED_REFERENCE_KEYS.contains(&key.as_str()));
        object
            .property_formats
            .retain(|key, _| !MANAGED_REFERENCE_KEYS.contains(&key.as_str()));
        if let Some(managed) = managed {
            for (key, value) in MANAGED_REFERENCE_KEYS.into_iter().zip(managed) {
                object.properties.push((key.into(), value));
                object.property_formats.insert(key.into(), "text".into());
            }
        }
        object
            .properties
            .sort_by(|left, right| left.0.cmp(&right.0));
        Ok(())
    }
}

fn valid_descriptor_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn active_descriptor(extra: &std::collections::BTreeMap<String, serde_json::Value>) -> bool {
    matches!(
        extra.get("archived"),
        None | Some(serde_json::Value::Bool(false))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use any_cal_anytype_adapter::{DiscoveredProperty, DiscoveredSpace, DiscoveredType};
    use any_cal_core::{ForeignObjectRef, MaterializedReference, SourceSnapshot};
    use std::collections::BTreeMap;

    fn schema() -> SpaceDiscoverySnapshot {
        SpaceDiscoverySnapshot {
            domain_id: "home".into(),
            space_id: "private-space".into(),
            binding_fingerprint: "a".repeat(64),
            schema_profile: "default".into(),
            space: DiscoveredSpace {
                id: "private-space".into(),
                name: "Home".into(),
                extra: BTreeMap::new(),
            },
            stable_api_version: any_cal_anytype_adapter::API_VERSION.into(),
            body_only_available: true,
            schema_ready: false,
            types: vec![DiscoveredType {
                id: "type-context".into(),
                key: "anycal_person_context".into(),
                name: "Person Context".into(),
                layout: None,
                extra: BTreeMap::new(),
            }],
            properties: MANAGED_REFERENCE_KEYS
                .into_iter()
                .map(|key| DiscoveredProperty {
                    id: format!("id-{key}"),
                    key: Some(key.into()),
                    name: "Display name does not select schema".into(),
                    format: "text".into(),
                    extra: BTreeMap::new(),
                })
                .collect(),
            members: Vec::new(),
            tags_by_property_id: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn object() -> ObjectRecord {
        ObjectRecord {
            id: "context".into(),
            space_id: "private-space".into(),
            name: None,
            properties: vec![
                (ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY.into(), "old-uid".into()),
                ("private_notes".into(), "Only in my Space".into()),
                ("private_relations".into(), "[\"local-task\"]".into()),
            ],
            property_formats: BTreeMap::from([
                (ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY.into(), "text".into()),
                ("private_notes".into(), "text".into()),
                ("private_relations".into(), "objects".into()),
            ]),
            body: String::new(),
            archived: false,
            revision: 0,
        }
    }

    fn record() -> MaterializedReferenceRecord {
        let source = SourceSnapshot::new(
            ForeignObjectRef::new(
                "a".repeat(64),
                "shared-space",
                "source-contact",
                DavKind::Contact,
                None,
            )
            .unwrap(),
            BTreeMap::new(),
            SourceAvailability::Available,
        )
        .unwrap();
        MaterializedReferenceRecord {
            version: 1,
            record_type: "any_cal_materialized_reference".into(),
            profile_fingerprint: "b".repeat(64),
            source_domain_id: "shared".into(),
            source_collection: crate::identity::CollectionKind::Contacts,
            source_resource_id: "source-contact.vcf".into(),
            person_context_title: None,
            reference: MaterializedReference::new(&source).unwrap(),
        }
    }

    #[test]
    fn absent_optional_schema_keeps_body_only_but_typed_workflow_needs_discovery() {
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(None, false, DEFAULT_OBJECT_TYPE_KEY),
            Ok(ReferencePropertyPolicy::BodyOnly)
        );
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(None, false, "anycal_person_context"),
            Err(CompositionSchemaError::SnapshotRequired)
        );
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(None, true, DEFAULT_OBJECT_TYPE_KEY),
            Err(CompositionSchemaError::StaleSnapshot)
        );
        let mut snapshot = schema();
        snapshot.properties.pop();
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, "anycal_person_context"),
            Ok(ReferencePropertyPolicy::BodyOnly)
        );
    }

    #[test]
    fn missing_and_duplicate_required_types_fail_closed() {
        let mut snapshot = schema();
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, "unknown_type"),
            Err(CompositionSchemaError::RequiredTypeMissing)
        );
        snapshot.types.push(snapshot.types[0].clone());
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, "anycal_person_context"),
            Err(CompositionSchemaError::RequiredTypeAmbiguous)
        );
    }

    #[test]
    fn archived_or_malformed_type_and_property_descriptors_fail_closed() {
        let mut snapshot = schema();
        snapshot.types[0]
            .extra
            .insert("archived".into(), serde_json::json!(true));
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, "anycal_person_context"),
            Err(CompositionSchemaError::RequiredTypeUnavailable)
        );
        snapshot.types[0].extra.clear();
        for id in ["", " ", "bad\nidentifier"] {
            snapshot.types[0].id = id.into();
            assert_eq!(
                ReferencePropertyPolicy::from_snapshot(
                    Some(&snapshot),
                    false,
                    "anycal_person_context"
                ),
                Err(CompositionSchemaError::RequiredTypeMalformed)
            );
        }
        snapshot.types[0].id = "valid-type-id".into();
        snapshot.properties[0].id.clear();
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, DEFAULT_OBJECT_TYPE_KEY),
            Err(CompositionSchemaError::PropertyDescriptorInvalid(
                ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY
            ))
        );
        snapshot.properties[0].id = "valid-property-id".into();
        snapshot.properties[0]
            .extra
            .insert("archived".into(), serde_json::json!(true));
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, DEFAULT_OBJECT_TYPE_KEY),
            Err(CompositionSchemaError::PropertyDescriptorInvalid(
                ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY
            ))
        );
    }

    #[test]
    fn changing_runtime_space_schema_or_account_invalidates_the_discovered_snapshot() {
        for changed in ["space", "schema", "account"] {
            let mut config = crate::AppConfig::defaults();
            config.space_id = "private-space".into();
            let mut app = crate::App::fake(config).unwrap();
            let context = app.registry.primary();
            let domain_id = context.binding.domain_id.clone();
            let mut snapshot = schema();
            snapshot.domain_id = domain_id.clone();
            snapshot.space_id = context.binding.space_id.clone();
            snapshot.schema_profile = context.binding.schema_profile.clone();
            snapshot.binding_fingerprint = context
                .server
                .repository
                .binding
                .binding_fingerprint
                .clone();
            app.discovery_snapshots
                .insert(snapshot.discovery_key(), snapshot);
            assert_eq!(
                app.materialized_property_policy(&domain_id, "anycal_person_context"),
                Ok(ReferencePropertyPolicy::Native)
            );

            let context = app.registry.primary_mut();
            let mut account = context
                .server
                .repository
                .binding
                .upstream_account_fingerprint
                .clone();
            match changed {
                "space" => context.binding.space_id = "different-space".into(),
                "schema" => context.binding.schema_profile = "different-schema".into(),
                "account" => account = "c".repeat(64),
                _ => unreachable!(),
            }
            context.server.repository.binding =
                any_cal_anytype_adapter::RepositoryBinding::from_domain(&context.binding, &account)
                    .unwrap();
            for type_key in [DEFAULT_OBJECT_TYPE_KEY, "anycal_person_context"] {
                assert_eq!(
                    app.materialized_property_policy(&domain_id, type_key),
                    Err(CompositionSchemaError::StaleSnapshot),
                    "a change of {changed} must not reuse or silently downgrade the old schema"
                );
            }
        }
    }

    #[test]
    fn incompatible_schema_is_not_hidden_by_another_missing_optional_key() {
        let mut snapshot = schema();
        snapshot.properties.remove(0);
        snapshot.properties[0].format = "objects".into();
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, DEFAULT_OBJECT_TYPE_KEY),
            Err(CompositionSchemaError::PropertyFormatMismatch(
                ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY
            ))
        );
        snapshot.properties[0].format = "text".into();
        snapshot.properties.push(snapshot.properties[0].clone());
        assert_eq!(
            ReferencePropertyPolicy::from_snapshot(Some(&snapshot), false, DEFAULT_OBJECT_TYPE_KEY),
            Err(CompositionSchemaError::DuplicatePropertyKey(
                ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY
            ))
        );
    }

    #[test]
    fn native_refresh_clears_optional_uid_and_preserves_private_formats() {
        let policy =
            ReferencePropertyPolicy::from_snapshot(Some(&schema()), false, "anycal_person_context")
                .unwrap();
        assert_eq!(policy, ReferencePropertyPolicy::Native);
        let mut object = object();
        policy.validate_existing(&object).unwrap();
        policy.apply(&mut object, &record()).unwrap();
        assert_eq!(
            object
                .properties
                .iter()
                .find(|(key, _)| key == ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY)
                .unwrap()
                .1,
            ""
        );
        assert!(object
            .properties
            .contains(&("private_notes".into(), "Only in my Space".into())));
        assert!(object
            .properties
            .contains(&("private_relations".into(), "[\"local-task\"]".into())));
        assert_eq!(object.property_formats["private_relations"], "objects");
        assert_eq!(object.properties.len(), MANAGED_REFERENCE_KEYS.len() + 2);
    }

    #[test]
    fn body_only_omits_unvalidated_managed_properties_without_touching_private_values() {
        let mut object = object();
        object
            .property_formats
            .insert(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY.into(), "objects".into());
        ReferencePropertyPolicy::BodyOnly
            .apply(&mut object, &record())
            .unwrap();
        assert_eq!(object.properties.len(), 2);
        assert_eq!(object.property_formats.len(), 2);
        assert_eq!(object.property_formats["private_relations"], "objects");
        assert!(object
            .properties
            .contains(&("private_notes".into(), "Only in my Space".into())));
    }

    #[test]
    fn native_refresh_rejects_untyped_conflicting_or_duplicate_existing_managed_values() {
        let mut object = object();
        object
            .property_formats
            .remove(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
        assert_eq!(
            ReferencePropertyPolicy::Native.validate_existing(&object),
            Err(CompositionSchemaError::ExistingPropertyFormatMismatch(
                ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY
            ))
        );
        object
            .property_formats
            .insert(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY.into(), "objects".into());
        assert_eq!(
            ReferencePropertyPolicy::Native.validate_existing(&object),
            Err(CompositionSchemaError::ExistingPropertyFormatMismatch(
                ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY
            ))
        );
        object
            .property_formats
            .insert(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY.into(), "text".into());
        object
            .properties
            .push((ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY.into(), "another".into()));
        assert_eq!(
            ReferencePropertyPolicy::Native.validate_existing(&object),
            Err(CompositionSchemaError::DuplicateObjectProperty(
                ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY
            ))
        );
    }

    #[test]
    fn timed_out_native_writes_require_matching_values_and_formats_on_readback() {
        let policy = ReferencePropertyPolicy::Native;
        let mut intended = object();
        policy.apply(&mut intended, &record()).unwrap();
        let mut actual = intended.clone();
        assert!(policy.confirms_write(&intended, &actual));
        actual
            .properties
            .iter_mut()
            .find(|(key, _)| key == ANYCAL_SOURCE_STATUS_PROPERTY_KEY)
            .unwrap()
            .1 = "stale".into();
        assert!(!policy.confirms_write(&intended, &actual));
        actual = intended.clone();
        actual
            .property_formats
            .insert(ANYCAL_SOURCE_STATUS_PROPERTY_KEY.into(), "objects".into());
        assert!(!policy.confirms_write(&intended, &actual));
        actual = intended.clone();
        actual
            .properties
            .retain(|(key, _)| key != ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
        actual
            .property_formats
            .remove(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
        assert!(policy.confirms_write(&intended, &actual));
        actual
            .properties
            .retain(|(key, _)| key != ANYCAL_SOURCE_STATUS_PROPERTY_KEY);
        assert!(!policy.confirms_write(&intended, &actual));
    }
}
