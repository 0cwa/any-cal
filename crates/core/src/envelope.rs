use crate::model::{
    AnytypeObjectId, CollectionId, DavKind, DavUid, ResourceId, StructuredDocument,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CanonicalDocument {
    pub version: u8,
    pub content: StructuredDocument,
    /// The parsed calendar tree is retained when a calendar resource arrived
    /// over DAV.  The property projection remains the editable Anytype-facing
    /// view, while this optional tree preserves component order and opaque
    /// nested components (VEVENT/ VTODO/ VALARM/ VTIMEZONE and extensions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opaque_calendar: Option<crate::ical::Calendar>,
}

impl CanonicalDocument {
    pub fn new(content: StructuredDocument) -> Self {
        Self {
            version: 1,
            content,
            opaque_calendar: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceEnvelope {
    pub collection_id: CollectionId,
    pub resource_id: ResourceId,
    pub kind: DavKind,
    pub anytype_object_id: AnytypeObjectId,
    pub dav_uid: DavUid,
    pub document: CanonicalDocument,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvelopeError {
    InvalidJson(String),
    InvalidField(&'static str),
    UnsupportedVersion(u8),
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(e) => write!(f, "invalid envelope JSON: {e}"),
            Self::InvalidField(field) => write!(f, "invalid envelope field: {field}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported document version: {v}"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl ResourceEnvelope {
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if self.collection_id.as_str().trim().is_empty() {
            return Err(EnvelopeError::InvalidField("collection_id"));
        }
        for (name, value) in [
            ("resource_id", self.resource_id.as_str()),
            ("anytype_object_id", self.anytype_object_id.as_str()),
            ("dav_uid", self.dav_uid.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(EnvelopeError::InvalidField(name));
            }
        }
        if self.document.version != 1 {
            return Err(EnvelopeError::UnsupportedVersion(self.document.version));
        }
        Ok(())
    }

    pub fn from_json(input: &str) -> Result<Self, EnvelopeError> {
        let value: Self =
            serde_json::from_str(input).map_err(|e| EnvelopeError::InvalidJson(e.to_string()))?;
        value.validate()?;
        Ok(value)
    }

    /// Compact, stable JSON: BTreeMap ordering makes property and parameter
    /// keys deterministic; newline normalization avoids platform drift.
    pub fn canonical_json(&self) -> Result<String, EnvelopeError> {
        self.validate()?;
        let value =
            serde_json::to_value(self).map_err(|e| EnvelopeError::InvalidJson(e.to_string()))?;
        let value = normalize_json(value);
        serde_json::to_string(&value).map_err(|e| EnvelopeError::InvalidJson(e.to_string()))
    }

    pub fn semantic_eq(&self, other: &Self) -> bool {
        self.canonical_json().ok() == other.canonical_json().ok()
    }
}

fn normalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            serde_json::Value::String(s.replace("\r\n", "\n").replace('\r', "\n"))
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(normalize_json).collect())
        }
        serde_json::Value::Object(o) => {
            serde_json::Value::Object(o.into_iter().map(|(k, v)| (k, normalize_json(v))).collect())
        }
        other => other,
    }
}
