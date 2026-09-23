use crate::{DavKind, VisibilityIntent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const COMPOSITION_PROFILE_SCHEMA_VERSION: u32 = 1;
pub const MATERIALIZED_REFERENCE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForeignObjectRef {
    pub upstream_account_fingerprint: String,
    pub source_space_id: String,
    pub source_object_id: String,
    pub kind: DavKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav_uid: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionPolicy {
    PrivateDestinationOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionProfile {
    pub version: u32,
    pub principal_id: String,
    pub destination_domain_id: String,
    pub source_domain_ids: Vec<String>,
    pub projection_policy: ProjectionPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAvailability {
    Available,
    Archived,
    Deleted,
    Unavailable,
}

impl SourceAvailability {
    pub fn is_stale(self) -> bool {
        self != Self::Available
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldOwnership {
    Source,
    DestinationUser,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    pub foreign: ForeignObjectRef,
    #[serde(default)]
    pub source_fields: BTreeMap<String, Value>,
    pub availability: SourceAvailability,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedReference {
    pub version: u32,
    pub foreign: ForeignObjectRef,
    #[serde(default)]
    pub source_fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub user_fields: BTreeMap<String, Value>,
    pub source_availability: SourceAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshKind {
    Created,
    SourceRefreshed,
    SourceUnavailable,
    SourceArchived,
    SourceDeleted,
    Reauthorized,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeResult {
    pub reference: MaterializedReference,
    pub kind: RefreshKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DestinationMutation {
    ReplaceUserFields(BTreeMap<String, Value>),
    DeleteReference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestinationDeletion {
    pub foreign_identity_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DestinationMutationResult {
    Updated(MaterializedReference),
    Deleted(DestinationDeletion),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionError {
    InvalidJson,
    UnsupportedVersion,
    InvalidField(&'static str),
    InvalidFingerprint,
    DuplicateSourceDomain,
    DestinationInSources,
    IdentityMismatch,
    FieldOwnershipConflict(String),
    SourceDomainNotAllowed,
    DestinationNotPrivate,
}

impl ForeignObjectRef {
    pub fn new(
        upstream_account_fingerprint: impl Into<String>,
        source_space_id: impl Into<String>,
        source_object_id: impl Into<String>,
        kind: DavKind,
        dav_uid: Option<String>,
    ) -> Result<Self, CompositionError> {
        let mut value = Self {
            upstream_account_fingerprint: upstream_account_fingerprint.into(),
            source_space_id: source_space_id.into(),
            source_object_id: source_object_id.into(),
            kind,
            dav_uid,
        };
        value.normalize();
        value.validate()?;
        Ok(value)
    }

    pub fn from_json(input: &str) -> Result<Self, CompositionError> {
        let mut value: Self =
            serde_json::from_str(input).map_err(|_| CompositionError::InvalidJson)?;
        value.normalize();
        value.validate()?;
        Ok(value)
    }

    pub fn canonical_json(&self) -> Result<String, CompositionError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn validate(&self) -> Result<(), CompositionError> {
        if self.upstream_account_fingerprint.len() != 64
            || !self
                .upstream_account_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CompositionError::InvalidFingerprint);
        }
        validate_opaque(&self.source_space_id, "source_space_id")?;
        validate_opaque(&self.source_object_id, "source_object_id")?;
        if let Some(dav_uid) = self.dav_uid.as_deref() {
            validate_opaque(dav_uid, "dav_uid")?;
        }
        Ok(())
    }

    pub fn identity_fingerprint(&self) -> Result<String, CompositionError> {
        self.validate()?;
        let mut hasher = Sha256::new();
        hash_field(&mut hasher, "any-cal-foreign-object");
        hash_field(&mut hasher, &self.upstream_account_fingerprint.to_ascii_lowercase());
        hash_field(&mut hasher, &self.source_space_id);
        hash_field(&mut hasher, &self.source_object_id);
        Ok(hex_digest(hasher.finalize().as_slice()))
    }

    fn normalize(&mut self) {
        self.upstream_account_fingerprint =
            self.upstream_account_fingerprint.to_ascii_lowercase();
    }
}

impl CompositionProfile {
    pub fn new(
        principal_id: impl Into<String>,
        destination_domain_id: impl Into<String>,
        source_domain_ids: Vec<String>,
        projection_policy: ProjectionPolicy,
    ) -> Result<Self, CompositionError> {
        let mut value = Self {
            version: COMPOSITION_PROFILE_SCHEMA_VERSION,
            principal_id: principal_id.into(),
            destination_domain_id: destination_domain_id.into(),
            source_domain_ids,
            projection_policy,
        };
        value.validate()?;
        value.normalize();
        Ok(value)
    }

    pub fn from_json(input: &str) -> Result<Self, CompositionError> {
        let mut value: Self =
            serde_json::from_str(input).map_err(|_| CompositionError::InvalidJson)?;
        value.validate()?;
        value.normalize();
        Ok(value)
    }

    pub fn to_json(&self) -> Result<String, CompositionError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.normalize();
        canonical_json(&normalized)
    }

    pub fn fingerprint(&self) -> Result<String, CompositionError> {
        let json = self.to_json()?;
        let mut hasher = Sha256::new();
        hash_field(&mut hasher, "any-cal-composition-profile");
        hash_field(&mut hasher, &json);
        Ok(hex_digest(hasher.finalize().as_slice()))
    }

    pub fn validate(&self) -> Result<(), CompositionError> {
        if self.version != COMPOSITION_PROFILE_SCHEMA_VERSION {
            return Err(CompositionError::UnsupportedVersion);
        }
        validate_identifier(&self.principal_id, "principal_id")?;
        validate_identifier(&self.destination_domain_id, "destination_domain_id")?;
        if self.source_domain_ids.is_empty() {
            return Err(CompositionError::InvalidField("source_domain_ids"));
        }

        let mut source_domains = BTreeSet::new();
        for source_domain_id in &self.source_domain_ids {
            validate_identifier(source_domain_id, "source_domain_ids")?;
            if source_domain_id == &self.destination_domain_id {
                return Err(CompositionError::DestinationInSources);
            }
            if !source_domains.insert(source_domain_id.as_str()) {
                return Err(CompositionError::DuplicateSourceDomain);
            }
        }
        Ok(())
    }

    pub fn authorize_automatic_projection(
        &self,
        source_domain_id: &str,
        destination_visibility: VisibilityIntent,
    ) -> Result<(), CompositionError> {
        self.validate()?;
        if !self
            .source_domain_ids
            .iter()
            .any(|configured| configured == source_domain_id)
        {
            return Err(CompositionError::SourceDomainNotAllowed);
        }
        if destination_visibility != VisibilityIntent::Private {
            return Err(CompositionError::DestinationNotPrivate);
        }
        match self.projection_policy {
            ProjectionPolicy::PrivateDestinationOnly => Ok(()),
        }
    }

    fn normalize(&mut self) {
        self.source_domain_ids.sort();
    }
}

impl SourceSnapshot {
    pub fn new(
        foreign: ForeignObjectRef,
        source_fields: BTreeMap<String, Value>,
        availability: SourceAvailability,
    ) -> Result<Self, CompositionError> {
        let value = Self {
            foreign,
            source_fields,
            availability,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), CompositionError> {
        self.foreign.validate()?;
        validate_fields(&self.source_fields)?;
        Ok(())
    }
}

impl MaterializedReference {
    pub fn new(source: &SourceSnapshot) -> Result<Self, CompositionError> {
        source.validate()?;
        let value = Self {
            version: MATERIALIZED_REFERENCE_SCHEMA_VERSION,
            foreign: source.foreign.clone(),
            source_fields: source.source_fields.clone(),
            user_fields: BTreeMap::new(),
            source_availability: source.availability,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn from_json(input: &str) -> Result<Self, CompositionError> {
        let value: Self =
            serde_json::from_str(input).map_err(|_| CompositionError::InvalidJson)?;
        value.validate()?;
        Ok(value)
    }

    pub fn canonical_json(&self) -> Result<String, CompositionError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn validate(&self) -> Result<(), CompositionError> {
        if self.version != MATERIALIZED_REFERENCE_SCHEMA_VERSION {
            return Err(CompositionError::UnsupportedVersion);
        }
        self.foreign.validate()?;
        validate_fields(&self.source_fields)?;
        validate_fields(&self.user_fields)?;
        for key in self.source_fields.keys() {
            if self.user_fields.contains_key(key) {
                return Err(CompositionError::FieldOwnershipConflict(key.clone()));
            }
        }
        Ok(())
    }

    pub fn field_ownership(&self, key: &str) -> Option<FieldOwnership> {
        if self.source_fields.contains_key(key) {
            Some(FieldOwnership::Source)
        } else if self.user_fields.contains_key(key) {
            Some(FieldOwnership::DestinationUser)
        } else {
            None
        }
    }

    pub fn source_is_stale(&self) -> bool {
        self.source_availability.is_stale()
    }
}

pub fn reconcile_source_snapshot(
    existing: Option<&MaterializedReference>,
    source: &SourceSnapshot,
) -> Result<MergeResult, CompositionError> {
    source.validate()?;
    let Some(existing) = existing else {
        return Ok(MergeResult {
            reference: MaterializedReference::new(source)?,
            kind: RefreshKind::Created,
        });
    };

    existing.validate()?;
    if existing.foreign.identity_fingerprint()? != source.foreign.identity_fingerprint()? {
        return Err(CompositionError::IdentityMismatch);
    }

    let previous_availability = existing.source_availability;
    let mut merged = existing.clone();
    merged.foreign = source.foreign.clone();
    merged.source_availability = source.availability;

    match source.availability {
        SourceAvailability::Available => {
            merged.source_fields = source.source_fields.clone();
        }
        SourceAvailability::Unavailable => {
            if merged.source_fields.is_empty() && !source.source_fields.is_empty() {
                merged.source_fields = source.source_fields.clone();
            }
        }
        SourceAvailability::Archived | SourceAvailability::Deleted => {
            if !source.source_fields.is_empty() {
                merged.source_fields = source.source_fields.clone();
            }
        }
    }
    merged.validate()?;

    let kind = if merged == *existing {
        RefreshKind::Unchanged
    } else {
        match source.availability {
            SourceAvailability::Available
                if previous_availability != SourceAvailability::Available =>
            {
                RefreshKind::Reauthorized
            }
            SourceAvailability::Available => RefreshKind::SourceRefreshed,
            SourceAvailability::Unavailable => RefreshKind::SourceUnavailable,
            SourceAvailability::Archived => RefreshKind::SourceArchived,
            SourceAvailability::Deleted => RefreshKind::SourceDeleted,
        }
    };

    Ok(MergeResult {
        reference: merged,
        kind,
    })
}

pub fn apply_destination_mutation(
    existing: &MaterializedReference,
    mutation: DestinationMutation,
) -> Result<DestinationMutationResult, CompositionError> {
    existing.validate()?;
    match mutation {
        DestinationMutation::ReplaceUserFields(user_fields) => {
            validate_fields(&user_fields)?;
            if let Some(key) = user_fields
                .keys()
                .find(|key| existing.source_fields.contains_key(*key))
            {
                return Err(CompositionError::FieldOwnershipConflict(key.clone()));
            }
            let mut updated = existing.clone();
            updated.user_fields = user_fields;
            updated.validate()?;
            Ok(DestinationMutationResult::Updated(updated))
        }
        DestinationMutation::DeleteReference => Ok(DestinationMutationResult::Deleted(
            DestinationDeletion {
                foreign_identity_fingerprint: existing.foreign.identity_fingerprint()?,
            },
        )),
    }
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), CompositionError> {
    if value.is_empty()
        || value.len() > 128
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return Err(CompositionError::InvalidField(field));
    }
    Ok(())
}

fn validate_opaque(value: &str, field: &'static str) -> Result<(), CompositionError> {
    if value.trim().is_empty()
        || value.len() > 512
        || value.chars().any(char::is_control)
    {
        return Err(CompositionError::InvalidField(field));
    }
    Ok(())
}

fn validate_fields(fields: &BTreeMap<String, Value>) -> Result<(), CompositionError> {
    for key in fields.keys() {
        if key.trim().is_empty()
            || key.len() > 128
            || key.chars().any(char::is_control)
        {
            return Err(CompositionError::InvalidField("fields"));
        }
    }
    Ok(())
}

fn canonical_json<T: Serialize>(value: &T) -> Result<String, CompositionError> {
    let value = serde_json::to_value(value).map_err(|_| CompositionError::InvalidJson)?;
    serde_json::to_string(&canonicalize_json(value)).map_err(|_| CompositionError::InvalidJson)
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.into_iter().map(canonicalize_json).collect())
        }
        Value::Object(values) => {
            let ordered = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            let mut result = serde_json::Map::new();
            for (key, value) in ordered {
                result.insert(key, value);
            }
            Value::Object(result)
        }
        other => other,
    }
}

fn hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}
