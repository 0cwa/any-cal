use crate::{DavKind, MaterializedReference, SourceSnapshot, StoredResource};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const PERSON_CONTEXT_DISPLAY_NAME: &str = "display_name";
pub const PERSON_CONTEXT_EMAILS: &str = "emails";
pub const PERSON_CONTEXT_PHONES: &str = "phones";
pub const PERSON_CONTEXT_ORGANIZATIONS: &str = "organizations";
pub const PERSON_CONTEXT_DAV_UID: &str = "dav_uid";
pub const PERSON_CONTEXT_TITLE_OVERRIDE: &str = "title_override";

const MAX_VALUES_PER_FIELD: usize = 32;
const MAX_VALUE_CHARS: usize = 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersonContextProjectionError {
    NotContact,
}

/// Project only the small, useful canonical Contact fields that a private
/// Person Context needs for local views and same-Space relations.
///
/// This is deliberately a cache, not another canonical contact
/// representation. Unknown vCard fields, parameters, notes, photos, addresses,
/// and vendor extensions remain only in the canonical source object.
pub fn project_person_context_source(
    stored: &StoredResource,
) -> Result<BTreeMap<String, Value>, PersonContextProjectionError> {
    if stored.envelope.kind != DavKind::Contact {
        return Err(PersonContextProjectionError::NotContact);
    }

    let fields = &stored.envelope.document.content.fields;
    let mut projected = BTreeMap::new();

    if let Some(name) = first_value(fields, "FN") {
        projected.insert(PERSON_CONTEXT_DISPLAY_NAME.into(), json!(name));
    }

    let emails = bounded_values(fields, "EMAIL");
    if !emails.is_empty() {
        projected.insert(PERSON_CONTEXT_EMAILS.into(), json!(emails));
    }

    let phones = bounded_values(fields, "TEL");
    if !phones.is_empty() {
        projected.insert(PERSON_CONTEXT_PHONES.into(), json!(phones));
    }

    let organizations = bounded_values(fields, "ORG");
    if !organizations.is_empty() {
        projected.insert(PERSON_CONTEXT_ORGANIZATIONS.into(), json!(organizations));
    }

    projected.insert(
        PERSON_CONTEXT_DAV_UID.into(),
        json!(bounded_text(stored.envelope.dav_uid.as_str())),
    );

    Ok(projected)
}

/// Local title customization is destination-owned. A source rename changes the
/// effective title only while no explicit destination override is present.
pub fn effective_person_context_title(reference: &MaterializedReference) -> Option<String> {
    reference
        .user_fields
        .get(PERSON_CONTEXT_TITLE_OVERRIDE)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            reference
                .source_fields
                .get(PERSON_CONTEXT_DISPLAY_NAME)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            reference
                .source_fields
                .get(PERSON_CONTEXT_DAV_UID)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        })
}

fn first_value(
    fields: &BTreeMap<String, Vec<crate::Occurrence>>,
    key: &str,
) -> Option<String> {
    fields
        .get(key)
        .and_then(|values| values.iter().find(|value| !value.value.trim().is_empty()))
        .map(|value| bounded_text(&value.value))
}

fn bounded_values(
    fields: &BTreeMap<String, Vec<crate::Occurrence>>,
    key: &str,
) -> Vec<String> {
    fields
        .get(key)
        .into_iter()
        .flat_map(|values| values.iter())
        .filter(|value| !value.value.trim().is_empty())
        .take(MAX_VALUES_PER_FIELD)
        .map(|value| bounded_text(&value.value))
        .collect()
}

fn bounded_text(value: &str) -> String {
    value.chars().take(MAX_VALUE_CHARS).collect()
}

pub fn person_context_source_snapshot(
    foreign: crate::ForeignObjectRef,
    stored: &StoredResource,
    availability: crate::SourceAvailability,
) -> Result<SourceSnapshot, crate::CompositionError> {
    let source_fields = project_person_context_source(stored)
        .map_err(|_| crate::CompositionError::InvalidField("person_context_source"))?;
    SourceSnapshot::new(foreign, source_fields, availability)
}
