use crate::{ResourceEnvelope, ResourceId};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const BRIDGE_SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BridgeCheckpoint {
    pub cursor: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BridgeTombstone {
    pub resource_id: ResourceId,
    pub canonical_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BridgeRequest {
    pub schema_version: u8,
    pub account_name: String,
    pub account_type: String,
    pub authority: String,
    pub checkpoint: Option<BridgeCheckpoint>,
    pub resources: Vec<ResourceEnvelope>,
    pub tombstones: Vec<BridgeTombstone>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncDecision {
    Noop,
    Upsert,
    Archive,
    Conflict,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BridgeDecision {
    pub resource_id: ResourceId,
    pub decision: SyncDecision,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeErrorCode {
    UnsupportedVersion,
    InvalidRequest,
    NotLinked,
    PermissionDenied,
    TransportUnavailable,
    Conflict,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BridgeError {
    pub code: BridgeErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BridgeResponse {
    pub schema_version: u8,
    pub checkpoint: Option<BridgeCheckpoint>,
    pub decisions: Vec<BridgeDecision>,
    pub error: Option<BridgeError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeContractError {
    InvalidField(&'static str),
    UnsupportedVersion(u8),
    InvalidJson(String),
    SecretLikeMessage,
}

impl fmt::Display for BridgeContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid bridge field: {field}"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported bridge version: {version}"),
            Self::InvalidJson(error) => write!(f, "invalid bridge JSON: {error}"),
            Self::SecretLikeMessage => f.write_str("bridge message must not contain credentials"),
        }
    }
}

impl std::error::Error for BridgeContractError {}

impl BridgeRequest {
    pub fn validate(&self) -> Result<(), BridgeContractError> {
        if self.schema_version != BRIDGE_SCHEMA_VERSION {
            return Err(BridgeContractError::UnsupportedVersion(self.schema_version));
        }
        for (name, value) in [
            ("account_name", &self.account_name),
            ("account_type", &self.account_type),
            ("authority", &self.authority),
        ] {
            if value.trim().is_empty() {
                return Err(BridgeContractError::InvalidField(name));
            }
        }
        for resource in &self.resources {
            resource
                .validate()
                .map_err(|_| BridgeContractError::InvalidField("resource"))?;
        }
        Ok(())
    }

    pub fn from_json(input: &str) -> Result<Self, BridgeContractError> {
        let request: Self = serde_json::from_str(input)
            .map_err(|error| BridgeContractError::InvalidJson(error.to_string()))?;
        request.validate()?;
        Ok(request)
    }

    pub fn canonical_json(&self) -> Result<String, BridgeContractError> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|error| BridgeContractError::InvalidJson(error.to_string()))
    }
}

impl BridgeResponse {
    pub fn validate(&self) -> Result<(), BridgeContractError> {
        if self.schema_version != BRIDGE_SCHEMA_VERSION {
            return Err(BridgeContractError::UnsupportedVersion(self.schema_version));
        }
        if let Some(error) = &self.error {
            if contains_secret_like(&error.message) {
                return Err(BridgeContractError::SecretLikeMessage);
            }
        }
        if self
            .decisions
            .iter()
            .filter_map(|decision| decision.reason.as_deref())
            .any(contains_secret_like)
        {
            return Err(BridgeContractError::SecretLikeMessage);
        }
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, BridgeContractError> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|error| BridgeContractError::InvalidJson(error.to_string()))
    }
}

fn contains_secret_like(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["token", "api_key", "authorization", "password", "secret"]
        .iter()
        .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnytypeObjectId, CanonicalDocument, CollectionId, DavKind, DavUid, StructuredDocument,
    };

    fn request() -> BridgeRequest {
        BridgeRequest {
            schema_version: BRIDGE_SCHEMA_VERSION,
            account_name: "space".into(),
            account_type: "invalid.example.anycal".into(),
            authority: "com.android.contacts".into(),
            checkpoint: Some(BridgeCheckpoint {
                cursor: "c1".into(),
                revision: 4,
            }),
            resources: vec![ResourceEnvelope {
                collection_id: CollectionId::try_from("contacts").unwrap(),
                resource_id: ResourceId::try_from("r1").unwrap(),
                kind: DavKind::Contact,
                anytype_object_id: AnytypeObjectId::try_from("o1").unwrap(),
                dav_uid: DavUid::try_from("u1").unwrap(),
                document: CanonicalDocument::new(StructuredDocument::default()),
                revision: 4,
            }],
            tombstones: vec![BridgeTombstone {
                resource_id: ResourceId::try_from("r0").unwrap(),
                canonical_id: "o0".into(),
                revision: 3,
            }],
        }
    }

    #[test]
    fn request_round_trip_and_stable_json() {
        let request = request();
        let json = request.canonical_json().unwrap();
        assert_eq!(json, request.clone().canonical_json().unwrap());
        assert_eq!(BridgeRequest::from_json(&json).unwrap(), request);
    }

    #[test]
    fn malformed_and_wrong_versions_fail_closed() {
        assert!(BridgeRequest::from_json("not json").is_err());
        let mut request = request();
        request.schema_version = 2;
        assert!(matches!(
            request.validate(),
            Err(BridgeContractError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn response_rejects_secret_like_messages() {
        let response = BridgeResponse {
            schema_version: BRIDGE_SCHEMA_VERSION,
            checkpoint: None,
            decisions: vec![],
            error: Some(BridgeError {
                code: BridgeErrorCode::TransportUnavailable,
                message: "token=leak".into(),
            }),
        };
        assert_eq!(
            response.validate(),
            Err(BridgeContractError::SecretLikeMessage)
        );
        let response = BridgeResponse {
            schema_version: BRIDGE_SCHEMA_VERSION,
            checkpoint: None,
            decisions: vec![BridgeDecision {
                resource_id: ResourceId::try_from("r1").unwrap(),
                decision: SyncDecision::Unsupported,
                reason: Some("authorization header leaked".into()),
            }],
            error: None,
        };
        assert_eq!(
            response.validate(),
            Err(BridgeContractError::SecretLikeMessage)
        );
    }

    #[test]
    fn checkpoint_and_tombstone_survive_round_trip() {
        let request = request();
        let decoded = BridgeRequest::from_json(&request.canonical_json().unwrap()).unwrap();
        assert_eq!(decoded.checkpoint.unwrap().revision, 4);
        assert_eq!(decoded.tombstones[0].canonical_id, "o0");
    }
}
