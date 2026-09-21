use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const DOMAIN_BINDINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainBindings {
    pub version: u32,
    pub bindings: Vec<DomainBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainBinding {
    pub domain_id: String,
    pub label: String,
    pub space_id: String,
    pub credential_profile_id: String,
    pub routes: Vec<DavRoute>,
    pub schema_profile: String,
    pub checkpoint_namespace: String,
    pub visibility: VisibilityIntent,
    pub lifecycle: BindingLifecycle,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DavRoute {
    pub collection: DomainCollection,
    pub component: DavComponent,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainCollection {
    Contacts,
    Tasks,
    Events,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DavComponent {
    Vcard,
    Vtodo,
    Vevent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityIntent {
    Private,
    Shared,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingLifecycle {
    Configured,
    Verified,
    AccessDenied,
    Stale,
    MigrationRequired,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainBindingsError {
    InvalidJson,
    UnsupportedVersion,
    EmptyBindings,
    InvalidField(&'static str),
    InvalidRoute,
    ComponentMismatch,
    DuplicateDomainId,
    DuplicateRoute,
    RouteNotFound,
}

impl DavRoute {
    pub fn contacts(path: impl Into<String>) -> Self {
        Self {
            collection: DomainCollection::Contacts,
            component: DavComponent::Vcard,
            path: path.into(),
        }
    }

    pub fn tasks(path: impl Into<String>) -> Self {
        Self {
            collection: DomainCollection::Tasks,
            component: DavComponent::Vtodo,
            path: path.into(),
        }
    }

    pub fn events(path: impl Into<String>) -> Self {
        Self {
            collection: DomainCollection::Events,
            component: DavComponent::Vevent,
            path: path.into(),
        }
    }
}

impl DomainBindings {
    pub fn new(bindings: Vec<DomainBinding>) -> Result<Self, DomainBindingsError> {
        let mut contract = Self {
            version: DOMAIN_BINDINGS_SCHEMA_VERSION,
            bindings,
        };
        contract.validate()?;
        contract.normalize();
        Ok(contract)
    }

    pub fn from_json(input: &str) -> Result<Self, DomainBindingsError> {
        let mut contract: Self =
            serde_json::from_str(input).map_err(|_| DomainBindingsError::InvalidJson)?;
        contract.validate()?;
        contract.normalize();
        Ok(contract)
    }

    pub fn to_json(&self) -> Result<String, DomainBindingsError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.normalize();
        serde_json::to_string(&normalized).map_err(|_| DomainBindingsError::InvalidJson)
    }

    pub fn validate(&self) -> Result<(), DomainBindingsError> {
        if self.version != DOMAIN_BINDINGS_SCHEMA_VERSION {
            return Err(DomainBindingsError::UnsupportedVersion);
        }
        if self.bindings.is_empty() {
            return Err(DomainBindingsError::EmptyBindings);
        }

        let mut domain_ids = BTreeSet::new();
        let mut routes = BTreeSet::new();
        for binding in &self.bindings {
            validate_binding(binding)?;
            if !domain_ids.insert(binding.domain_id.as_str()) {
                return Err(DomainBindingsError::DuplicateDomainId);
            }
            for route in &binding.routes {
                if !routes.insert(route.path.as_str()) {
                    return Err(DomainBindingsError::DuplicateRoute);
                }
            }
        }
        Ok(())
    }

    pub fn resolve_route(
        &self,
        path: &str,
    ) -> Result<(&DomainBinding, &DavRoute), DomainBindingsError> {
        self.validate()?;
        self.bindings
            .iter()
            .find_map(|binding| {
                binding
                    .routes
                    .iter()
                    .find(|route| route.path == path)
                    .map(|route| (binding, route))
            })
            .ok_or(DomainBindingsError::RouteNotFound)
    }

    pub fn legacy_single_space(
        space_id: impl Into<String>,
        contacts_collection: impl AsRef<str>,
        tasks_collection: impl AsRef<str>,
    ) -> Result<Self, DomainBindingsError> {
        Self::new(vec![DomainBinding {
            domain_id: "legacy-default".into(),
            label: "Legacy default".into(),
            space_id: space_id.into(),
            credential_profile_id: "legacy-default".into(),
            routes: vec![
                DavRoute::contacts(format!("/carddav/{}", contacts_collection.as_ref())),
                DavRoute::tasks(format!("/caldav/{}", tasks_collection.as_ref())),
            ],
            schema_profile: "legacy-default".into(),
            checkpoint_namespace: "legacy-default".into(),
            visibility: VisibilityIntent::Unknown,
            lifecycle: BindingLifecycle::Configured,
        }])
    }

    fn normalize(&mut self) {
        for binding in &mut self.bindings {
            binding.routes.sort();
        }
        self.bindings
            .sort_by(|left, right| left.domain_id.cmp(&right.domain_id));
    }
}

impl DomainBinding {
    pub fn fingerprint(
        &self,
        upstream_account_fingerprint: &str,
    ) -> Result<String, DomainBindingsError> {
        validate_binding(self)?;
        validate_opaque(upstream_account_fingerprint, "upstream_account_fingerprint")?;

        let mut hasher = Sha256::new();
        hash_field(&mut hasher, "any-cal-domain-binding");
        hash_field(&mut hasher, &DOMAIN_BINDINGS_SCHEMA_VERSION.to_string());
        hash_field(&mut hasher, &self.domain_id);
        hash_field(&mut hasher, upstream_account_fingerprint);
        hash_field(&mut hasher, &self.credential_profile_id);
        hash_field(&mut hasher, &self.space_id);
        hash_field(&mut hasher, &self.schema_profile);
        hash_field(&mut hasher, &self.checkpoint_namespace);

        let mut routes = self.routes.clone();
        routes.sort();
        for route in routes {
            hash_field(&mut hasher, route.collection.as_str());
            hash_field(&mut hasher, route.component.as_str());
            hash_field(&mut hasher, &route.path);
        }

        let digest = hasher.finalize();
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut fingerprint = String::with_capacity(digest.len() * 2);
        for byte in digest {
            fingerprint.push(HEX[(byte >> 4) as usize] as char);
            fingerprint.push(HEX[(byte & 0x0f) as usize] as char);
        }
        Ok(fingerprint)
    }
}

impl DomainCollection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contacts => "contacts",
            Self::Tasks => "tasks",
            Self::Events => "events",
        }
    }
}

impl DavComponent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vcard => "vcard",
            Self::Vtodo => "vtodo",
            Self::Vevent => "vevent",
        }
    }
}

fn validate_binding(binding: &DomainBinding) -> Result<(), DomainBindingsError> {
    validate_identifier(&binding.domain_id, "domain_id")?;
    validate_label(&binding.label)?;
    validate_opaque(&binding.space_id, "space_id")?;
    validate_identifier(&binding.credential_profile_id, "credential_profile_id")?;
    validate_identifier(&binding.schema_profile, "schema_profile")?;
    validate_identifier(&binding.checkpoint_namespace, "checkpoint_namespace")?;
    if binding.routes.is_empty() {
        return Err(DomainBindingsError::InvalidField("routes"));
    }

    let mut paths = BTreeSet::new();
    for route in &binding.routes {
        validate_route(route)?;
        if !paths.insert(route.path.as_str()) {
            return Err(DomainBindingsError::DuplicateRoute);
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), DomainBindingsError> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return Err(DomainBindingsError::InvalidField(field));
    }
    Ok(())
}

fn validate_label(value: &str) -> Result<(), DomainBindingsError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(DomainBindingsError::InvalidField("label"));
    }
    Ok(())
}

fn validate_opaque(value: &str, field: &'static str) -> Result<(), DomainBindingsError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(DomainBindingsError::InvalidField(field));
    }
    Ok(())
}

fn validate_route(route: &DavRoute) -> Result<(), DomainBindingsError> {
    if !route.path.starts_with('/')
        || route.path.len() == 1
        || route
            .path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || route.path.contains('?')
        || route.path.contains('#')
    {
        return Err(DomainBindingsError::InvalidRoute);
    }

    let expected = match route.collection {
        DomainCollection::Contacts => DavComponent::Vcard,
        DomainCollection::Tasks => DavComponent::Vtodo,
        DomainCollection::Events => DavComponent::Vevent,
    };
    if route.component != expected {
        return Err(DomainBindingsError::ComponentMismatch);
    }
    Ok(())
}

fn hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}
