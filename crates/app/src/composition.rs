use crate::identity::{
    AccessDecision, AuthOutcome, Capability, CollectionKind, Operation,
};
use crate::AppGeneric;
use any_cal_anytype_adapter::AnytypeTransport;
use any_cal_core::{CompositionError, CompositionProfile, DomainCollection};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CompositionSourceScope {
    pub domain_id: String,
    pub collection: CollectionKind,
    pub resource_id: Option<String>,
}

impl CompositionSourceScope {
    pub fn collection(domain_id: impl Into<String>, collection: CollectionKind) -> Self {
        Self {
            domain_id: domain_id.into(),
            collection,
            resource_id: None,
        }
    }

    pub fn resource(
        domain_id: impl Into<String>,
        collection: CollectionKind,
        resource_id: impl Into<String>,
    ) -> Self {
        Self {
            domain_id: domain_id.into(),
            collection,
            resource_id: Some(resource_id.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedCompositionSource {
    pub domain_id: String,
    pub collection: CollectionKind,
    pub resource_id: Option<String>,
    pub binding_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionAuthorizationPlan {
    pub profile_fingerprint: String,
    pub principal_id: String,
    pub destination_domain_id: String,
    pub destination_binding_fingerprint: String,
    pub sources: Vec<AuthorizedCompositionSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionAuthorizationError {
    InvalidProfile(CompositionError),
    IdentityUnavailable,
    AuthenticationFailed(AuthOutcome),
    PrincipalMismatch,
    EmptySources,
    DuplicateSourceScope,
    InvalidResourceId,
    UnknownDomain(String),
    SourceCollectionUnsupported,
    SourceCollectionUnavailable(String),
    SourceReadDenied(String),
    SourceCredentialDenied(String),
    SourceUpstreamDenied { domain_id: String, status: u16 },
    DestinationWriteDenied,
    DestinationCredentialDenied,
    DestinationUpstreamDenied(u16),
}

impl<T: AnytypeTransport> AppGeneric<T> {
    /// Authorize a composition operation without performing source reads or a
    /// destination write. Callers supply canonical domain IDs only; Spaces are
    /// resolved internally from the configured registry.
    ///
    /// Source data must already be readable through its canonical Contacts or
    /// Tasks scope. The destination uses the distinct Composition scope so
    /// materializing a private facet/reference never implies permission to
    /// mutate or advertise a canonical DAV resource.
    pub fn authorize_composition(
        &self,
        profile: &CompositionProfile,
        token: &str,
        source_scopes: &[CompositionSourceScope],
    ) -> Result<CompositionAuthorizationPlan, CompositionAuthorizationError> {
        profile
            .validate()
            .map_err(CompositionAuthorizationError::InvalidProfile)?;
        let profile_fingerprint = profile
            .fingerprint()
            .map_err(CompositionAuthorizationError::InvalidProfile)?;

        let identity = self
            .identity
            .as_ref()
            .ok_or(CompositionAuthorizationError::IdentityUnavailable)?;
        let Some((principal, outcome)) = identity.authenticate(token, self.identity_now) else {
            return Err(CompositionAuthorizationError::AuthenticationFailed(
                AuthOutcome::Invalid,
            ));
        };
        if outcome != AuthOutcome::Authenticated {
            return Err(CompositionAuthorizationError::AuthenticationFailed(outcome));
        }
        if principal.as_str() != profile.principal_id {
            return Err(CompositionAuthorizationError::PrincipalMismatch);
        }

        if source_scopes.is_empty() {
            return Err(CompositionAuthorizationError::EmptySources);
        }

        let destination = self
            .registry
            .context(&profile.destination_domain_id)
            .ok_or_else(|| {
                CompositionAuthorizationError::UnknownDomain(profile.destination_domain_id.clone())
            })?;

        // This also enforces the initial private-destination disclosure policy.
        for source in source_scopes {
            profile
                .authorize_automatic_projection(&source.domain_id, destination.binding.visibility)
                .map_err(CompositionAuthorizationError::InvalidProfile)?;
        }

        if identity.policy.authorize_domain(
            &principal,
            &profile.destination_domain_id,
            CollectionKind::Composition,
            None,
            Operation::Write,
        ) != AccessDecision::Allowed
        {
            return Err(CompositionAuthorizationError::DestinationWriteDenied);
        }
        if !identity.credential_allows(token, self.identity_now, Capability::WriteComposition) {
            return Err(CompositionAuthorizationError::DestinationCredentialDenied);
        }
        if let Some(status) = self
            .registry
            .upstream_preflight(&profile.destination_domain_id, true)
        {
            return Err(CompositionAuthorizationError::DestinationUpstreamDenied(
                status,
            ));
        }

        let mut seen = BTreeSet::new();
        let mut authorized_sources = Vec::with_capacity(source_scopes.len());
        for source in source_scopes {
            if matches!(source.collection, CollectionKind::Composition) {
                return Err(CompositionAuthorizationError::SourceCollectionUnsupported);
            }
            if source.resource_id.as_deref().is_some_and(|resource_id| {
                resource_id.trim().is_empty()
                    || resource_id.len() > 512
                    || resource_id.chars().any(char::is_control)
            }) {
                return Err(CompositionAuthorizationError::InvalidResourceId);
            }
            if !seen.insert((
                source.domain_id.clone(),
                source.collection,
                source.resource_id.clone(),
            )) {
                return Err(CompositionAuthorizationError::DuplicateSourceScope);
            }

            let context = self.registry.context(&source.domain_id).ok_or_else(|| {
                CompositionAuthorizationError::UnknownDomain(source.domain_id.clone())
            })?;
            let domain_collection = match source.collection {
                CollectionKind::Contacts => DomainCollection::Contacts,
                CollectionKind::Tasks => DomainCollection::Tasks,
                CollectionKind::Composition => unreachable!("rejected above"),
            };
            if !context
                .binding
                .routes
                .iter()
                .any(|route| route.collection == domain_collection)
            {
                return Err(CompositionAuthorizationError::SourceCollectionUnavailable(
                    source.domain_id.clone(),
                ));
            }

            if identity.policy.authorize_domain(
                &principal,
                &source.domain_id,
                source.collection,
                source.resource_id.as_deref(),
                Operation::Read,
            ) != AccessDecision::Allowed
            {
                return Err(CompositionAuthorizationError::SourceReadDenied(
                    source.domain_id.clone(),
                ));
            }
            let capability = match source.collection {
                CollectionKind::Contacts => Capability::ReadContacts,
                CollectionKind::Tasks => Capability::ReadTasks,
                CollectionKind::Composition => unreachable!("rejected above"),
            };
            if !identity.credential_allows(token, self.identity_now, capability) {
                return Err(CompositionAuthorizationError::SourceCredentialDenied(
                    source.domain_id.clone(),
                ));
            }
            if let Some(status) = self.registry.upstream_preflight(&source.domain_id, false) {
                return Err(CompositionAuthorizationError::SourceUpstreamDenied {
                    domain_id: source.domain_id.clone(),
                    status,
                });
            }

            authorized_sources.push(AuthorizedCompositionSource {
                domain_id: source.domain_id.clone(),
                collection: source.collection,
                resource_id: source.resource_id.clone(),
                binding_fingerprint: context
                    .server
                    .repository
                    .binding
                    .binding_fingerprint
                    .clone(),
            });
        }

        authorized_sources.sort_by(|left, right| {
            left.domain_id
                .cmp(&right.domain_id)
                .then_with(|| left.collection.cmp(&right.collection))
                .then_with(|| left.resource_id.cmp(&right.resource_id))
        });

        Ok(CompositionAuthorizationPlan {
            profile_fingerprint,
            principal_id: principal.as_str().to_owned(),
            destination_domain_id: profile.destination_domain_id.clone(),
            destination_binding_fingerprint: destination
                .server
                .repository
                .binding
                .binding_fingerprint
                .clone(),
            sources: authorized_sources,
        })
    }
}
