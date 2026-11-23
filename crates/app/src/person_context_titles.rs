//! Native Person Context title ownership, independent of optional metadata.
use any_cal_core::{MaterializedReference, PERSON_CONTEXT_DAV_UID, PERSON_CONTEXT_DISPLAY_NAME};
use serde::{Deserialize, Serialize};

const MAX_SOURCE_TITLE_CHARS: usize = 1024;
const FALLBACK_TITLE: &str = "Person Context";

/// Persist only ownership and the last source-derived name we wrote. The
/// destination's native name remains authoritative for a locally owned title.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "ownership", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonContextTitleState {
    Source { last_written: String },
    Local,
}

impl PersonContextTitleState {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Source { last_written } => {
                !last_written.trim().is_empty()
                    && last_written.chars().count() <= MAX_SOURCE_TITLE_CHARS
            }
            Self::Local => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReferencePresentation {
    Unmanaged,
    PersonContext,
}

pub(crate) struct TitleUpdate {
    pub(crate) name: Option<String>,
    pub(crate) state: Option<PersonContextTitleState>,
}

impl ReferencePresentation {
    pub(crate) fn reconcile_title(
        self,
        existing: bool,
        observed_name: Option<&str>,
        previous: Option<&PersonContextTitleState>,
        reference: &MaterializedReference,
    ) -> TitleUpdate {
        if self == Self::Unmanaged || (existing && observed_name.is_none()) {
            // A missing response field is unknown, not a user clearing the
            // title. Omit the name write and retain any prior ownership marker.
            return TitleUpdate {
                name: observed_name.map(str::to_owned),
                state: previous.cloned(),
            };
        }

        if !existing
            || matches!(
                previous,
                Some(PersonContextTitleState::Source { last_written })
                    if observed_name == Some(last_written.as_str())
            )
        {
            let name = source_title(reference);
            return TitleUpdate {
                name: Some(name.clone()),
                state: Some(PersonContextTitleState::Source { last_written: name }),
            };
        }

        // Every observed legacy title belongs to the destination user unless
        // a persisted Source marker proves we wrote that exact value. Local
        // ownership remains sticky even if a later source name happens to match.
        TitleUpdate {
            name: observed_name.map(str::to_owned),
            state: Some(PersonContextTitleState::Local),
        }
    }
}

fn source_title(reference: &MaterializedReference) -> String {
    [
        reference
            .source_fields
            .get(PERSON_CONTEXT_DISPLAY_NAME)
            .and_then(serde_json::Value::as_str),
        reference
            .source_fields
            .get(PERSON_CONTEXT_DAV_UID)
            .and_then(serde_json::Value::as_str),
        reference.foreign.dav_uid.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| {
        let bounded = value
            .chars()
            .take(MAX_SOURCE_TITLE_CHARS)
            .collect::<String>();
        (!bounded.trim().is_empty()).then_some(bounded)
    })
    .unwrap_or_else(|| FALLBACK_TITLE.into())
}

pub(crate) fn confirms_name_write(intended: Option<&str>, actual: Option<&str>) -> bool {
    intended.is_none_or(|name| actual == Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition_orchestration::{
        MaterializedReferenceRecord, MATERIALIZED_RECORD_TYPE, MATERIALIZED_RECORD_VERSION,
    };
    use crate::identity::CollectionKind;
    use any_cal_core::{
        reconcile_source_snapshot, DavKind, ForeignObjectRef, SourceAvailability, SourceSnapshot,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn reference(name: Option<&str>, uid: Option<&str>) -> MaterializedReference {
        let mut fields = BTreeMap::new();
        if let Some(name) = name {
            fields.insert(PERSON_CONTEXT_DISPLAY_NAME.into(), json!(name));
        }
        if let Some(uid) = uid {
            fields.insert(PERSON_CONTEXT_DAV_UID.into(), json!(uid));
        }
        let source = SourceSnapshot::new(
            ForeignObjectRef::new(
                "a".repeat(64),
                "shared-space",
                "shared-contact",
                DavKind::Contact,
                uid.map(str::to_owned),
            )
            .unwrap(),
            fields,
            SourceAvailability::Available,
        )
        .unwrap();
        MaterializedReference::new(&source).unwrap()
    }

    fn source_state(name: &str) -> PersonContextTitleState {
        PersonContextTitleState::Source {
            last_written: name.into(),
        }
    }

    #[test]
    fn new_titles_use_bounded_unicode_display_name_then_uid_then_fallback() {
        for (reference, expected) in [
            (
                reference(Some("Zoë 東京 🦊"), Some("contact-uid")),
                "Zoë 東京 🦊",
            ),
            (reference(None, Some("contact-uid")), "contact-uid"),
            (reference(Some(" \t "), None), FALLBACK_TITLE),
        ] {
            let title =
                ReferencePresentation::PersonContext.reconcile_title(false, None, None, &reference);
            assert_eq!(title.name.as_deref(), Some(expected));
            assert_eq!(title.state, Some(source_state(expected)));
        }
        let mut reference = reference(None, None);
        reference.source_fields.insert(
            PERSON_CONTEXT_DISPLAY_NAME.into(),
            json!("🦊".repeat(MAX_SOURCE_TITLE_CHARS + 5)),
        );
        let title =
            ReferencePresentation::PersonContext.reconcile_title(false, None, None, &reference);
        assert_eq!(title.name.unwrap().chars().count(), MAX_SOURCE_TITLE_CHARS);
        assert!(title.state.unwrap().is_valid());
    }

    #[test]
    fn unchanged_source_title_follows_source_rename() {
        let title = ReferencePresentation::PersonContext.reconcile_title(
            true,
            Some("Carol Old"),
            Some(&source_state("Carol Old")),
            &reference(Some("Carol New"), None),
        );
        assert_eq!(title.name.as_deref(), Some("Carol New"));
        assert_eq!(title.state, Some(source_state("Carol New")));
    }

    #[test]
    fn native_edits_including_blank_become_sticky_local_ownership() {
        for local_name in ["Carol · Private", "", "  🦊 東京  "] {
            let title = ReferencePresentation::PersonContext.reconcile_title(
                true,
                Some(local_name),
                Some(&source_state("Carol Old")),
                &reference(Some("Carol New"), None),
            );
            assert_eq!(title.name.as_deref(), Some(local_name));
            assert_eq!(title.state, Some(PersonContextTitleState::Local));
            let later = ReferencePresentation::PersonContext.reconcile_title(
                true,
                Some(local_name),
                title.state.as_ref(),
                &reference(Some(local_name), None),
            );
            assert_eq!(later.name.as_deref(), Some(local_name));
            assert_eq!(later.state, Some(PersonContextTitleState::Local));
        }
    }

    #[test]
    fn missing_existing_native_name_keeps_every_previous_marker_and_omits_the_write() {
        for previous in [
            None,
            Some(source_state("Carol Old")),
            Some(PersonContextTitleState::Local),
        ] {
            let title = ReferencePresentation::PersonContext.reconcile_title(
                true,
                None,
                previous.as_ref(),
                &reference(Some("Carol New"), Some("contact-uid")),
            );
            assert_eq!(title.name, None);
            assert_eq!(title.state, previous);
        }
    }

    #[test]
    fn legacy_observed_titles_are_local_even_when_they_match_cached_source_names() {
        for observed in ["Carol Old", "Personal title", ""] {
            let title = ReferencePresentation::PersonContext.reconcile_title(
                true,
                Some(observed),
                None,
                &reference(Some("Carol Old"), None),
            );
            assert_eq!(title.name.as_deref(), Some(observed));
            assert_eq!(title.state, Some(PersonContextTitleState::Local));
        }
    }

    #[test]
    fn generic_refresh_preserves_title_and_marker_without_acquiring_title_ownership() {
        let previous = source_state("Carol Old");
        let title = ReferencePresentation::Unmanaged.reconcile_title(
            true,
            Some("Carol Old"),
            Some(&previous),
            &reference(Some("Carol New"), None),
        );
        assert_eq!(title.name.as_deref(), Some("Carol Old"));
        assert_eq!(title.state, Some(previous));
        let new = ReferencePresentation::Unmanaged.reconcile_title(
            false,
            None,
            None,
            &reference(Some("Carol New"), None),
        );
        assert_eq!(new.name, None);
        assert_eq!(new.state, None);
    }

    #[test]
    fn ownership_survives_record_serialization_and_old_records_stay_readable() {
        let mut record = MaterializedReferenceRecord {
            version: MATERIALIZED_RECORD_VERSION,
            record_type: MATERIALIZED_RECORD_TYPE.into(),
            profile_fingerprint: "b".repeat(64),
            source_domain_id: "shared".into(),
            source_collection: CollectionKind::Contacts,
            source_resource_id: "shared-contact.vcf".into(),
            reference: reference(Some("Carol"), None),
            person_context_title: None,
        };
        let legacy = record.canonical_json().unwrap();
        assert!(!legacy.contains("person_context_title"));
        assert_eq!(
            MaterializedReferenceRecord::from_json(&legacy).unwrap(),
            record
        );
        for state in [source_state("Carol"), PersonContextTitleState::Local] {
            record.person_context_title = Some(state);
            let reopened =
                MaterializedReferenceRecord::from_json(&record.canonical_json().unwrap()).unwrap();
            assert_eq!(reopened, record);
        }
        record.person_context_title = Some(source_state(""));
        assert!(record.canonical_json().is_err());
    }

    #[test]
    fn source_unavailability_preserves_cached_title_and_local_ownership() {
        let previous = reference(Some("Carol"), Some("contact-uid"));
        let unavailable = SourceSnapshot::new(
            previous.foreign.clone(),
            BTreeMap::new(),
            SourceAvailability::Unavailable,
        )
        .unwrap();
        let refreshed = reconcile_source_snapshot(Some(&previous), &unavailable).unwrap();
        let source = ReferencePresentation::PersonContext.reconcile_title(
            true,
            Some("Carol"),
            Some(&source_state("Carol")),
            &refreshed.reference,
        );
        assert_eq!(source.name.as_deref(), Some("Carol"));
        assert_eq!(source.state, Some(source_state("Carol")));
        let local = ReferencePresentation::PersonContext.reconcile_title(
            true,
            Some("My Carol"),
            Some(&PersonContextTitleState::Local),
            &refreshed.reference,
        );
        assert_eq!(local.name.as_deref(), Some("My Carol"));
        assert_eq!(local.state, Some(PersonContextTitleState::Local));
    }

    #[test]
    fn ambiguous_writes_require_exact_intended_names_including_blank() {
        assert!(confirms_name_write(None, Some("Unmanaged title")));
        assert!(confirms_name_write(Some("Carol"), Some("Carol")));
        assert!(!confirms_name_write(Some("Carol"), Some("Old title")));
        assert!(!confirms_name_write(Some("Carol"), None));
        assert!(confirms_name_write(Some(""), Some("")));
        assert!(!confirms_name_write(Some(""), None));
    }
}
