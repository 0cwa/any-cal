//! Deterministic identity and ACL policy for local, synthetic DAV fixtures.
//!
//! This module is deliberately not wired into the current single-credential
//! HTTP entry point.  It provides the narrow policy seam needed to validate
//! expiry, revocation, and per-principal authorization before introducing a
//! persistent multi-account configuration format.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PrincipalId(String);

impl PrincipalId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(IdentityError::InvalidPrincipal);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum CollectionKind {
    Contacts,
    Tasks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum Operation {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthOutcome {
    Authenticated,
    Invalid,
    NotYetValid,
    Expired,
    Revoked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessDecision {
    Allowed,
    Forbidden,
    NotFound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityError {
    InvalidPrincipal,
    InvalidCredentialId,
    InvalidDomainId,
    InvalidWindow,
    DuplicateCredential,
    Io(String),
    Corrupt(String),
    UnsafePath,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialSpec {
    pub id: String,
    pub principal: PrincipalId,
    pub not_before: i64,
    pub expires_at: i64,
    pub capabilities: BTreeSet<Capability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum Capability {
    ReadContacts,
    WriteContacts,
    ReadTasks,
    WriteTasks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Credential {
    spec: CredentialSpec,
    token_digest: [u8; 32],
    revoked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessPolicy {
    collections: BTreeMap<(PrincipalId, String, CollectionKind), BTreeSet<Operation>>,
    resources: BTreeMap<(PrincipalId, String, CollectionKind, String), BTreeSet<Operation>>,
}

impl AccessPolicy {
    pub fn new() -> Self {
        Self {
            collections: BTreeMap::new(),
            resources: BTreeMap::new(),
        }
    }

    /// Legacy single-domain convenience. New multi-domain callers should use
    /// [`Self::grant_domain_collection`] explicitly.
    pub fn grant_collection(
        &mut self,
        principal: PrincipalId,
        collection: CollectionKind,
        operation: Operation,
    ) {
        let inserted =
            self.grant_domain_collection(principal, legacy_domain_id(), collection, operation);
        debug_assert!(inserted, "legacy domain id is always valid");
    }

    pub fn grant_domain_collection(
        &mut self,
        principal: PrincipalId,
        domain_id: impl Into<String>,
        collection: CollectionKind,
        operation: Operation,
    ) -> bool {
        let domain_id = domain_id.into();
        if !valid_domain_id(&domain_id) {
            return false;
        }
        self.collections
            .entry((principal, domain_id, collection))
            .or_default()
            .insert(operation);
        true
    }

    /// Legacy single-domain convenience. New multi-domain callers should use
    /// [`Self::grant_domain_resource`] explicitly.
    pub fn grant_resource(
        &mut self,
        principal: PrincipalId,
        collection: CollectionKind,
        resource_id: impl Into<String>,
        operation: Operation,
    ) {
        let inserted = self.grant_domain_resource(
            principal,
            legacy_domain_id(),
            collection,
            resource_id,
            operation,
        );
        debug_assert!(inserted, "legacy domain id is always valid");
    }

    pub fn grant_domain_resource(
        &mut self,
        principal: PrincipalId,
        domain_id: impl Into<String>,
        collection: CollectionKind,
        resource_id: impl Into<String>,
        operation: Operation,
    ) -> bool {
        let domain_id = domain_id.into();
        if !valid_domain_id(&domain_id) {
            return false;
        }
        self.resources
            .entry((principal, domain_id, collection, resource_id.into()))
            .or_default()
            .insert(operation);
        true
    }

    /// Legacy single-domain convenience for callers that still use the
    /// synthesized `legacy-default` binding.
    pub fn authorize(
        &self,
        principal: &PrincipalId,
        collection: CollectionKind,
        resource_id: Option<&str>,
        operation: Operation,
    ) -> AccessDecision {
        self.authorize_domain(
            principal,
            legacy_domain_id(),
            collection,
            resource_id,
            operation,
        )
    }

    pub fn authorize_domain(
        &self,
        principal: &PrincipalId,
        domain_id: &str,
        collection: CollectionKind,
        resource_id: Option<&str>,
        operation: Operation,
    ) -> AccessDecision {
        if !valid_domain_id(domain_id) {
            return if resource_id.is_some() {
                AccessDecision::NotFound
            } else {
                AccessDecision::Forbidden
            };
        }
        let operations = resource_id
            .and_then(|resource_id| {
                self.resources.get(&(
                    principal.clone(),
                    domain_id.to_owned(),
                    collection,
                    resource_id.to_owned(),
                ))
            })
            .or_else(|| {
                self.collections
                    .get(&(principal.clone(), domain_id.to_owned(), collection))
            });
        if operations.is_some_and(|operations| operations.contains(&operation)) {
            return AccessDecision::Allowed;
        }
        if resource_id.is_some() {
            AccessDecision::NotFound
        } else {
            AccessDecision::Forbidden
        }
    }

    pub fn capabilities(&self, principal: &PrincipalId) -> BTreeSet<Capability> {
        self.capabilities_for_domain(principal, legacy_domain_id())
    }

    pub fn capabilities_for_domain(
        &self,
        principal: &PrincipalId,
        domain_id: &str,
    ) -> BTreeSet<Capability> {
        [
            (CollectionKind::Contacts, Capability::ReadContacts),
            (CollectionKind::Contacts, Capability::WriteContacts),
            (CollectionKind::Tasks, Capability::ReadTasks),
            (CollectionKind::Tasks, Capability::WriteTasks),
        ]
        .into_iter()
        .filter_map(|(collection, capability)| {
            let operation = match capability {
                Capability::ReadContacts | Capability::ReadTasks => Operation::Read,
                Capability::WriteContacts | Capability::WriteTasks => Operation::Write,
            };
            (self.authorize_domain(principal, domain_id, collection, None, operation)
                == AccessDecision::Allowed)
                .then_some(capability)
        })
        .collect()
    }
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityStore {
    credentials: BTreeMap<String, Credential>,
    pub policy: AccessPolicy,
}

impl IdentityStore {
    pub fn new(policy: AccessPolicy) -> Self {
        Self {
            credentials: BTreeMap::new(),
            policy,
        }
    }

    pub fn add_credential(
        &mut self,
        spec: CredentialSpec,
        token: &str,
    ) -> Result<(), IdentityError> {
        if spec.id.is_empty()
            || spec.id.len() > 128
            || spec.id.chars().any(|character| character.is_control())
        {
            return Err(IdentityError::InvalidCredentialId);
        }
        if spec.expires_at <= spec.not_before
            || token.is_empty()
            || token.chars().any(char::is_control)
        {
            return Err(IdentityError::InvalidWindow);
        }
        if self.credentials.contains_key(&spec.id) {
            return Err(IdentityError::DuplicateCredential);
        }
        self.credentials.insert(
            spec.id.clone(),
            Credential {
                spec,
                token_digest: digest(token),
                revoked: false,
            },
        );
        Ok(())
    }

    pub fn revoke(&mut self, credential_id: &str) -> bool {
        self.credentials
            .get_mut(credential_id)
            .map(|credential| {
                credential.revoked = true;
                true
            })
            .unwrap_or(false)
    }

    pub fn authenticate(&self, token: &str, now: i64) -> Option<(PrincipalId, AuthOutcome)> {
        if token.is_empty() || token.chars().any(char::is_control) {
            return None;
        }
        let supplied = digest(token);
        self.credentials.values().find_map(|credential| {
            if !constant_time_equal(&supplied, &credential.token_digest) {
                return None;
            }
            let outcome = if credential.revoked {
                AuthOutcome::Revoked
            } else if now < credential.spec.not_before {
                AuthOutcome::NotYetValid
            } else if now >= credential.spec.expires_at {
                AuthOutcome::Expired
            } else {
                AuthOutcome::Authenticated
            };
            Some((credential.spec.principal.clone(), outcome))
        })
    }

    pub fn authenticate_outcome(&self, token: &str, now: i64) -> AuthOutcome {
        self.authenticate(token, now)
            .map_or(AuthOutcome::Invalid, |(_, outcome)| outcome)
    }

    /// Return only non-sensitive policy metadata for the legacy synthesized
    /// domain. Credential IDs and tokens are intentionally excluded.
    pub fn capabilities(&self, token: &str, now: i64) -> BTreeSet<Capability> {
        self.capabilities_for_domain(token, now, legacy_domain_id())
    }

    pub fn capabilities_for_domain(
        &self,
        token: &str,
        now: i64,
        domain_id: &str,
    ) -> BTreeSet<Capability> {
        match self.authenticate(token, now) {
            Some((principal, AuthOutcome::Authenticated)) => {
                let policy_capabilities =
                    self.policy.capabilities_for_domain(&principal, domain_id);
                let supplied = digest(token);
                let credential = self
                    .credentials
                    .values()
                    .find(|credential| {
                        credential.spec.principal == principal
                            && constant_time_equal(&supplied, &credential.token_digest)
                    })
                    .expect("authenticated credential must be present");
                if credential.spec.capabilities.is_empty() {
                    policy_capabilities
                } else {
                    policy_capabilities
                        .intersection(&credential.spec.capabilities)
                        .copied()
                        .collect()
                }
            }
            _ => BTreeSet::new(),
        }
    }

    /// Check only the credential's declared capability restriction. An empty
    /// capability set means the credential is governed by the ACL policy. The
    /// caller still must apply `AccessPolicy::authorize` for the principal and
    /// requested collection/resource.
    pub fn credential_allows(&self, token: &str, now: i64, capability: Capability) -> bool {
        let Some((_, AuthOutcome::Authenticated)) = self.authenticate(token, now) else {
            return false;
        };
        let supplied = digest(token);
        self.credentials.values().any(|credential| {
            constant_time_equal(&supplied, &credential.token_digest)
                && (credential.spec.capabilities.is_empty()
                    || credential.spec.capabilities.contains(&capability))
        })
    }

    /// Persist only the policy and SHA-256 credential digests. Raw tokens are
    /// intentionally not representable in this snapshot format.
    pub fn save_to(&self, path: &Path) -> Result<(), IdentityError> {
        validate_path(path)?;
        let snapshot = Snapshot::from_store(self);
        let bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| IdentityError::Corrupt(error.to_string()))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| IdentityError::Io(error.to_string()))?
            .as_nanos();
        let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), nonce));
        {
            let mut file = fs::File::create(&tmp).map_err(io_string)?;
            set_private_mode(&tmp)?;
            file.write_all(&bytes).map_err(io_string)?;
            file.sync_all().map_err(io_string)?;
        }
        fs::rename(&tmp, path).map_err(io_string)?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .map_err(io_string)?
                .sync_all()
                .map_err(io_string)?;
        }
        Ok(())
    }

    /// Load a complete snapshot or reject it without returning a partially
    /// activated policy. The caller must explicitly attach the result to the
    /// service with `AppGeneric::with_identity`.
    pub fn load_from(path: &Path) -> Result<Self, IdentityError> {
        validate_path(path)?;
        require_private_mode(path)?;
        let bytes = fs::read(path).map_err(io_string)?;
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|error| IdentityError::Corrupt(error.to_string()))?;
        snapshot.into_store()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    credentials: Vec<CredentialSnapshot>,
    collections: Vec<CollectionGrant>,
    resources: Vec<ResourceGrant>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CredentialSnapshot {
    id: String,
    principal: String,
    not_before: i64,
    expires_at: i64,
    capabilities: BTreeSet<Capability>,
    token_digest_hex: String,
    revoked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CollectionGrant {
    principal: String,
    #[serde(default = "legacy_domain_owned")]
    domain_id: String,
    collection: CollectionKind,
    operations: BTreeSet<Operation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResourceGrant {
    principal: String,
    #[serde(default = "legacy_domain_owned")]
    domain_id: String,
    collection: CollectionKind,
    resource_id: String,
    operations: BTreeSet<Operation>,
}

impl Snapshot {
    fn from_store(store: &IdentityStore) -> Self {
        Self {
            version: 2,
            credentials: store
                .credentials
                .values()
                .map(|credential| CredentialSnapshot {
                    id: credential.spec.id.clone(),
                    principal: credential.spec.principal.as_str().to_owned(),
                    not_before: credential.spec.not_before,
                    expires_at: credential.spec.expires_at,
                    capabilities: credential.spec.capabilities.clone(),
                    token_digest_hex: hex_encode(&credential.token_digest),
                    revoked: credential.revoked,
                })
                .collect(),
            collections: store
                .policy
                .collections
                .iter()
                .map(
                    |((principal, domain_id, collection), operations)| CollectionGrant {
                        principal: principal.as_str().to_owned(),
                        domain_id: domain_id.clone(),
                        collection: *collection,
                        operations: operations.clone(),
                    },
                )
                .collect(),
            resources: store
                .policy
                .resources
                .iter()
                .map(
                    |((principal, domain_id, collection, resource_id), operations)| ResourceGrant {
                        principal: principal.as_str().to_owned(),
                        domain_id: domain_id.clone(),
                        collection: *collection,
                        resource_id: resource_id.clone(),
                        operations: operations.clone(),
                    },
                )
                .collect(),
        }
    }

    fn into_store(self) -> Result<IdentityStore, IdentityError> {
        if !matches!(self.version, 1 | 2) {
            return Err(IdentityError::Corrupt(
                "unsupported identity snapshot version".into(),
            ));
        }
        let mut policy = AccessPolicy::new();
        for grant in self.collections {
            let principal = PrincipalId::new(grant.principal)?;
            if !valid_domain_id(&grant.domain_id) {
                return Err(IdentityError::Corrupt(
                    "invalid authorization domain id".into(),
                ));
            }
            for operation in grant.operations {
                if !policy.grant_domain_collection(
                    principal.clone(),
                    grant.domain_id.clone(),
                    grant.collection,
                    operation,
                ) {
                    return Err(IdentityError::Corrupt(
                        "invalid authorization domain id".into(),
                    ));
                }
            }
        }
        for grant in self.resources {
            let principal = PrincipalId::new(grant.principal)?;
            if grant.resource_id.is_empty() {
                return Err(IdentityError::Corrupt("empty resource id".into()));
            }
            if !valid_domain_id(&grant.domain_id) {
                return Err(IdentityError::Corrupt(
                    "invalid authorization domain id".into(),
                ));
            }
            for operation in grant.operations {
                if !policy.grant_domain_resource(
                    principal.clone(),
                    grant.domain_id.clone(),
                    grant.collection,
                    grant.resource_id.clone(),
                    operation,
                ) {
                    return Err(IdentityError::Corrupt(
                        "invalid authorization domain id".into(),
                    ));
                }
            }
        }
        let mut store = IdentityStore::new(policy);
        for credential in self.credentials {
            let principal = PrincipalId::new(credential.principal)?;
            if credential.token_digest_hex.len() != 64
                || !credential
                    .token_digest_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(IdentityError::Corrupt("invalid credential digest".into()));
            }
            if credential.expires_at <= credential.not_before {
                return Err(IdentityError::InvalidWindow);
            }
            if store.credentials.contains_key(&credential.id) {
                return Err(IdentityError::DuplicateCredential);
            }
            let mut digest = [0u8; 32];
            for (index, pair) in credential.token_digest_hex.as_bytes().chunks(2).enumerate() {
                digest[index] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
            }
            store.credentials.insert(
                credential.id.clone(),
                Credential {
                    spec: CredentialSpec {
                        id: credential.id,
                        principal,
                        not_before: credential.not_before,
                        expires_at: credential.expires_at,
                        capabilities: credential.capabilities,
                    },
                    token_digest: digest,
                    revoked: credential.revoked,
                },
            );
        }
        Ok(store)
    }
}

const LEGACY_DOMAIN_ID: &str = "legacy-default";

fn legacy_domain_id() -> &'static str {
    LEGACY_DOMAIN_ID
}

fn legacy_domain_owned() -> String {
    LEGACY_DOMAIN_ID.to_owned()
}

fn valid_domain_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn io_string(error: io::Error) -> IdentityError {
    IdentityError::Io(error.to_string())
}

fn validate_path(path: &Path) -> Result<(), IdentityError> {
    if path.as_os_str().is_empty() || path.file_name().is_none() || path.is_symlink() {
        return Err(IdentityError::UnsafePath);
    }
    Ok(())
}

fn set_private_mode(path: &Path) -> Result<(), IdentityError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).map_err(io_string)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions).map_err(io_string)?;
    }
    Ok(())
}

fn require_private_mode(path: &Path) -> Result<(), IdentityError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path).map_err(io_string)?.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(IdentityError::UnsafePath);
        }
    }
    Ok(())
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 0xf) as usize] as char,
            ]
        })
        .collect()
}

fn hex_value(byte: u8) -> Result<u8, IdentityError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(IdentityError::Corrupt("invalid credential digest".into())),
    }
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}
