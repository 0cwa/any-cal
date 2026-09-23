use crate::AppGeneric;
use any_cal_anytype_adapter::{
    AnytypeDiscoveryTransport, AnytypeTransport, DiscoveredMember, DiscoveredProperty,
    DiscoveredSpace, DiscoveredTag, DiscoveredType, DiscoveredView, Page, TransportError,
};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_DISCOVERY_PAGES: usize = 100;
const MAX_DISCOVERY_RECORDS: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    UnknownDomain,
    SpaceNotAccessible,
    Transport(TransportError),
    LimitExceeded,
    InvalidPagination,
}

impl From<TransportError> for DiscoveryError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Present,
    Missing,
    WrongFormat,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SchemaRequirementDiagnostic {
    pub workflow: String,
    pub key: String,
    pub expected_format: Option<String>,
    pub observed_id: Option<String>,
    pub observed_format: Option<String>,
    pub state: RequirementState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SpaceDiscoverySnapshot {
    pub domain_id: String,
    pub space_id: String,
    pub binding_fingerprint: String,
    pub schema_profile: String,
    pub space: DiscoveredSpace,
    pub stable_api_version: String,
    pub body_only_available: bool,
    pub schema_ready: bool,
    pub types: Vec<DiscoveredType>,
    pub properties: Vec<DiscoveredProperty>,
    pub members: Vec<DiscoveredMember>,
    pub tags_by_property_id: BTreeMap<String, Vec<DiscoveredTag>>,
    pub diagnostics: Vec<SchemaRequirementDiagnostic>,
}

impl<T> AppGeneric<T>
where
    T: AnytypeTransport + AnytypeDiscoveryTransport,
{
    /// Discover the configured Space behind one canonical domain using only
    /// stable v1 read operations. The caller selects a configured domain, never
    /// an arbitrary Space ID.
    pub fn discover_domain_capabilities(
        &mut self,
        domain_id: &str,
    ) -> Result<SpaceDiscoverySnapshot, DiscoveryError> {
        let context = self
            .registry
            .context(domain_id)
            .ok_or(DiscoveryError::UnknownDomain)?;
        let space_id = context.binding.space_id.clone();
        let schema_profile = context.binding.schema_profile.clone();
        let binding_fingerprint = context
            .server
            .repository
            .binding
            .binding_fingerprint
            .clone();

        let spaces = collect_pages(|offset| {
            self.registry
                .with_transport_mut(|transport| transport.list_spaces(offset))
        })?;
        let space = spaces
            .into_iter()
            .find(|candidate| candidate.id == space_id)
            .ok_or(DiscoveryError::SpaceNotAccessible)?;

        let mut types = collect_pages(|offset| {
            self.registry
                .with_transport_mut(|transport| transport.list_types(&space_id, offset))
        })?;
        let mut properties = collect_pages(|offset| {
            self.registry
                .with_transport_mut(|transport| transport.list_properties(&space_id, offset))
        })?;
        let mut members = collect_pages(|offset| {
            self.registry
                .with_transport_mut(|transport| transport.list_members(&space_id, offset))
        })?;

        types.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| left.id.cmp(&right.id))
        });
        properties.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| left.id.cmp(&right.id))
        });
        members.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));

        let mut tags_by_property_id = BTreeMap::new();
        for property in &properties {
            if !matches!(property.format.as_str(), "select" | "multi_select") {
                continue;
            }
            let mut tags = collect_pages(|offset| {
                self.registry.with_transport_mut(|transport| {
                    transport.list_tags(&space_id, &property.id, offset)
                })
            })?;
            tags.sort_by(|left, right| left.id.cmp(&right.id));
            tags_by_property_id.insert(property.id.clone(), tags);
        }

        let diagnostics = default_schema_diagnostics(&types, &properties);
        let schema_ready = diagnostics
            .iter()
            .all(|diagnostic| diagnostic.state == RequirementState::Present);

        let snapshot = SpaceDiscoverySnapshot {
            domain_id: domain_id.to_owned(),
            space_id,
            binding_fingerprint,
            schema_profile,
            space,
            stable_api_version: any_cal_anytype_adapter::API_VERSION.to_owned(),
            body_only_available: true,
            schema_ready,
            types,
            properties,
            members,
            tags_by_property_id,
            diagnostics,
        };
        self.discovery_snapshots
            .insert(domain_id.to_owned(), snapshot.clone());
        Ok(snapshot)
    }

    pub fn discovery_snapshot(&self, domain_id: &str) -> Option<&SpaceDiscoverySnapshot> {
        self.discovery_snapshots.get(domain_id)
    }

    /// Discover views for a list that is already known inside the configured
    /// domain. Space selection remains binding-controlled.
    pub fn discover_list_views(
        &mut self,
        domain_id: &str,
        list_id: &str,
    ) -> Result<Vec<DiscoveredView>, DiscoveryError> {
        if list_id.trim().is_empty() || list_id.chars().any(char::is_control) {
            return Err(DiscoveryError::InvalidPagination);
        }
        let space_id = self
            .registry
            .context(domain_id)
            .ok_or(DiscoveryError::UnknownDomain)?
            .binding
            .space_id
            .clone();
        let mut views = collect_pages(|offset| {
            self.registry
                .with_transport_mut(|transport| transport.list_views(&space_id, list_id, offset))
        })?;
        views.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(views)
    }
}

fn collect_pages<T>(
    mut read: impl FnMut(Option<&str>) -> Result<Page<T>, TransportError>,
) -> Result<Vec<T>, DiscoveryError> {
    let mut rows = Vec::new();
    let mut offset = None::<String>;
    for _ in 0..MAX_DISCOVERY_PAGES {
        let page = read(offset.as_deref())?;
        rows.extend(page.data);
        if rows.len() > MAX_DISCOVERY_RECORDS {
            return Err(DiscoveryError::LimitExceeded);
        }
        match page.next_offset {
            Some(next) if offset.as_deref() == Some(next.as_str()) => {
                return Err(DiscoveryError::InvalidPagination)
            }
            Some(next) => offset = Some(next),
            None => return Ok(rows),
        }
    }
    Err(DiscoveryError::LimitExceeded)
}

fn default_schema_diagnostics(
    types: &[DiscoveredType],
    properties: &[DiscoveredProperty],
) -> Vec<SchemaRequirementDiagnostic> {
    let mut diagnostics = [
        ("Project", "project"),
        ("Task", "task"),
        ("Person", "person"),
        ("Event", "event"),
        ("Person Context", "person_context"),
        ("Event Reference", "event_reference"),
    ]
    .into_iter()
    .map(|(workflow, key)| {
        let observed = types.iter().find(|candidate| candidate.key == key);
        SchemaRequirementDiagnostic {
            workflow: workflow.into(),
            key: key.into(),
            expected_format: None,
            observed_id: observed.map(|candidate| candidate.id.clone()),
            observed_format: None,
            state: if observed.is_some() {
                RequirementState::Present
            } else {
                RequirementState::Missing
            },
        }
    })
    .collect::<Vec<_>>();

    for (workflow, key, expected_format) in [
        ("Task projection", "description", "text"),
        ("Task projection", "done", "checkbox"),
    ] {
        let observed = properties
            .iter()
            .find(|candidate| candidate.key.as_deref() == Some(key));
        diagnostics.push(SchemaRequirementDiagnostic {
            workflow: workflow.into(),
            key: key.into(),
            expected_format: Some(expected_format.into()),
            observed_id: observed.map(|candidate| candidate.id.clone()),
            observed_format: observed.map(|candidate| candidate.format.clone()),
            state: match observed {
                Some(candidate) if candidate.format == expected_format => RequirementState::Present,
                Some(_) => RequirementState::WrongFormat,
                None => RequirementState::Missing,
            },
        });
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_diagnostics_use_stable_keys_and_formats_not_display_names() {
        let types = vec![DiscoveredType {
            id: "type-task".into(),
            key: "task".into(),
            name: "Not relied upon".into(),
            layout: None,
            extra: BTreeMap::new(),
        }];
        let properties = vec![
            DiscoveredProperty {
                id: "p-description".into(),
                key: Some("description".into()),
                name: "Anything".into(),
                format: "text".into(),
                extra: BTreeMap::new(),
            },
            DiscoveredProperty {
                id: "p-done".into(),
                key: Some("done".into()),
                name: "Anything".into(),
                format: "text".into(),
                extra: BTreeMap::new(),
            },
        ];
        let diagnostics = default_schema_diagnostics(&types, &properties);
        assert_eq!(
            diagnostics
                .iter()
                .find(|item| item.key == "task")
                .unwrap()
                .state,
            RequirementState::Present
        );
        assert_eq!(
            diagnostics
                .iter()
                .find(|item| item.key == "done")
                .unwrap()
                .state,
            RequirementState::WrongFormat
        );
        assert_eq!(
            diagnostics
                .iter()
                .find(|item| item.key == "person")
                .unwrap()
                .state,
            RequirementState::Missing
        );
    }

    #[test]
    fn repeated_pagination_offset_fails_closed() {
        let error = collect_pages::<u8>(|_| {
            Ok(Page {
                data: vec![1],
                next_offset: Some("0".into()),
            })
        })
        .unwrap_err();
        assert_eq!(error, DiscoveryError::InvalidPagination);
    }
}
