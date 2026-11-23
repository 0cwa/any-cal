//! Sanitized HTTP fixtures for optional materialized-reference metadata.
#[path = "support/composition_http.rs"]
#[allow(dead_code)]
mod composition_http;

use any_cal_anytype_adapter::{
    HttpAnytypeTransport, TransportError, ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_PERSON_CONTEXT_TYPE_KEY, ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY, ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY, ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
    DEFAULT_OBJECT_TYPE_KEY,
};
use any_cal_app::composition_orchestration::{
    CompositionOrchestrationError, CompositionSchemaError, MetadataProjection,
};
use any_cal_app::AppGeneric;
use any_cal_core::{project_person_context_source, RefreshKind};
use composition_http::{
    managed_schema, person_types, profile, record_from_markdown, scopes, Fixture, Reply,
    MANAGED_KEYS,
};
use serde_json::json;
use std::collections::BTreeMap;

fn materialize_typed(
    app: &mut AppGeneric<HttpAnytypeTransport>,
    type_key: &str,
) -> Result<MetadataProjection, CompositionOrchestrationError> {
    app.materialize_references_with_type(&profile(), "alice-token", &scopes(), type_key, |stored| {
        project_person_context_source(stored).unwrap()
    })
    .map(|result| result[0].metadata_projection)
}

#[test]
fn default_page_without_discovery_writes_body_only() {
    let fixture = Fixture::new();
    fixture.materialization(true);
    let mut app = fixture.app();
    assert_eq!(
        materialize_typed(&mut app, DEFAULT_OBJECT_TYPE_KEY).unwrap(),
        MetadataProjection::BodyOnly
    );
    let script = fixture.script.lock().unwrap();
    assert!(script.steps.is_empty());
    let create = &script.requests.last().unwrap().1;
    assert_eq!(create["type_key"], DEFAULT_OBJECT_TYPE_KEY);
    assert_eq!(create["properties"], json!([]));
    assert_eq!(
        record_from_markdown(&create["body"])
            .reference
            .foreign
            .dav_uid
            .as_deref(),
        Some("carol-uid")
    );
}

#[test]
fn missing_optional_key_keeps_typed_creation_body_only() {
    let fixture = Fixture::new();
    let mut properties = managed_schema();
    properties.retain(|property| property["key"] != ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
    fixture.discovery(person_types(), properties);
    fixture.materialization(true);
    let mut app = fixture.app();
    app.discover_domain_capabilities("home").unwrap();
    assert_eq!(
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY).unwrap(),
        MetadataProjection::BodyOnly
    );
    let script = fixture.script.lock().unwrap();
    assert!(script.steps.is_empty());
    let create = &script.requests.last().unwrap().1;
    assert_eq!(create["type_key"], ANYCAL_PERSON_CONTEXT_TYPE_KEY);
    assert_eq!(create["properties"], json!([]));
    assert_eq!(
        record_from_markdown(&create["body"])
            .reference
            .foreign
            .source_space_id,
        "space-shared"
    );
}

#[test]
fn typed_creation_requires_discovery_before_any_http_request() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    assert_eq!(
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY),
        Err(CompositionOrchestrationError::DestinationSchema(
            CompositionSchemaError::SnapshotRequired
        ))
    );
    assert!(fixture.script.lock().unwrap().requests.is_empty());
}

#[test]
fn invalid_destination_schema_fails_before_object_reads_or_writes() {
    let mut wrong_format = managed_schema();
    wrong_format[0]["format"] = json!("number");
    let mut duplicate_key = managed_schema();
    let mut duplicate = duplicate_key[0].clone();
    duplicate["id"] = json!("another-property");
    duplicate_key.push(duplicate);
    let mut duplicate_type = person_types();
    let mut duplicate = duplicate_type[0].clone();
    duplicate["id"] = json!("another-type");
    duplicate_type.push(duplicate);
    for (types, properties, expected) in [
        (
            Vec::new(),
            managed_schema(),
            CompositionSchemaError::RequiredTypeMissing,
        ),
        (
            duplicate_type,
            managed_schema(),
            CompositionSchemaError::RequiredTypeAmbiguous,
        ),
        (
            person_types(),
            wrong_format,
            CompositionSchemaError::PropertyFormatMismatch(MANAGED_KEYS[0]),
        ),
        (
            person_types(),
            duplicate_key,
            CompositionSchemaError::DuplicatePropertyKey(MANAGED_KEYS[0]),
        ),
    ] {
        let fixture = Fixture::new();
        fixture.discovery(types, properties);
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        assert_eq!(
            materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY),
            Err(CompositionOrchestrationError::DestinationSchema(expected))
        );
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        assert_eq!(
            script.requests.len(),
            4,
            "only discovery GETs may reach Anytype"
        );
        assert!(script.destination.is_none());
    }
}

#[test]
fn conflicting_native_metadata_from_get_is_never_overwritten() {
    for (duplicate, expected) in [
        (
            false,
            CompositionSchemaError::ExistingPropertyFormatMismatch(
                ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
            ),
        ),
        (
            true,
            CompositionSchemaError::DuplicateObjectProperty(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY),
        ),
    ] {
        let fixture = Fixture::new();
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(true);
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY).unwrap();
        let before = {
            let mut script = fixture.script.lock().unwrap();
            let destination = script.destination.as_mut().unwrap();
            let properties = destination["properties"].as_array_mut().unwrap();
            if duplicate {
                properties.push(json!({
                    "key":ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
                    "format":"text", "text":"conflicting-uid"
                }));
            } else {
                properties.retain(|property| property["key"] != ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
                properties.push(json!({
                    "key":ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
                    "format":"objects", "objects":[{"id":"private-local-object"}]
                }));
            }
            destination.clone()
        };
        fixture.discovery(person_types(), managed_schema());
        {
            let mut script = fixture.script.lock().unwrap();
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects HTTP/1.1".into(),
                Reply::Json(json!({"data":[{"id":"context-1"}]})),
            ));
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                Reply::Destination,
            ));
        }
        let mut reopened = fixture.app();
        reopened.discover_domain_capabilities("home").unwrap();
        assert_eq!(
            materialize_typed(&mut reopened, ANYCAL_PERSON_CONTEXT_TYPE_KEY),
            Err(CompositionOrchestrationError::DestinationSchema(expected))
        );
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        assert!(!script
            .requests
            .iter()
            .any(|(line, _)| line.starts_with("PATCH ")));
        assert_eq!(script.destination.as_ref().unwrap(), &before);
    }
}

#[test]
fn patch_timeout_requires_native_metadata_readback_even_when_body_is_unchanged() {
    for applied in [false, true] {
        let fixture = Fixture::new();
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(true);
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY).unwrap();
        let unchanged_markdown = {
            let mut script = fixture.script.lock().unwrap();
            let destination = script.destination.as_mut().unwrap();
            let stale = destination["properties"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|property| property["key"] == ANYCAL_SOURCE_STATUS_PROPERTY_KEY)
                .unwrap();
            stale["text"] = json!("unavailable");
            destination["markdown"].clone()
        };
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(false);
        {
            let mut script = fixture.script.lock().unwrap();
            script.steps.back_mut().unwrap().1 = if applied {
                Reply::SaveDestination { timeout: true }
            } else {
                Reply::Timeout
            };
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                Reply::Destination,
            ));
        }
        let mut reopened = fixture.app();
        reopened.discover_domain_capabilities("home").unwrap();
        let result = materialize_typed(&mut reopened, ANYCAL_PERSON_CONTEXT_TYPE_KEY);
        if applied {
            result.unwrap();
        } else {
            assert_eq!(
                result,
                Err(CompositionOrchestrationError::DestinationTransport(
                    TransportError::Timeout
                ))
            );
        }
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        let patch = &script
            .requests
            .iter()
            .find(|(line, _)| line.starts_with("PATCH "))
            .unwrap()
            .1;
        assert_eq!(patch["markdown"], unchanged_markdown);
        assert!(patch["properties"]
            .as_array()
            .unwrap()
            .iter()
            .any(|property| {
                property["key"] == ANYCAL_SOURCE_STATUS_PROPERTY_KEY
                    && property["text"] == "available"
            }));
        let observed = script.destination.as_ref().unwrap()["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|property| property["key"] == ANYCAL_SOURCE_STATUS_PROPERTY_KEY)
            .unwrap();
        assert_eq!(
            observed["text"],
            if applied { "available" } else { "unavailable" }
        );
    }
}

#[test]
fn native_metadata_round_trips_with_private_fields_across_http_refresh_and_reopen() {
    let mut fixture = Fixture::new();
    fixture.discovery(person_types(), managed_schema());
    fixture.materialization(true);
    let mut app = fixture.app();
    app.discover_domain_capabilities("home").unwrap();
    let created = app
        .materialize_person_contexts(&profile(), "alice-token", &scopes())
        .unwrap();
    assert!(created[0].created);
    assert_eq!(created[0].metadata_projection, MetadataProjection::Native);
    let original_id = created[0].destination_object_id.clone();
    let foreign_identity = created[0].reference.foreign.identity_fingerprint().unwrap();
    let private_properties = json!([
        {"key":"private_notes","format":"text","text":"Private café note"},
        {"key":"private_tags","format":"multi_select","multi_select":[{"key":"follow-up","name":"Follow up"}]},
        {"key":"private_relations","format":"objects","objects":[{"id":"home-project","name":"Private project"}]}
    ]);
    {
        let mut script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        let create = &script.requests.last().unwrap().1;
        assert_eq!(create["type_key"], ANYCAL_PERSON_CONTEXT_TYPE_KEY);
        assert_eq!(
            create["properties"].as_array().unwrap().len(),
            MANAGED_KEYS.len()
        );
        for key in MANAGED_KEYS {
            assert!(create["properties"]
                .as_array()
                .unwrap()
                .iter()
                .any(|property| property["key"] == key && property["text"].is_string()));
        }
        let destination = script.destination.as_mut().unwrap();
        let mut record = record_from_markdown(&destination["markdown"]);
        record.reference.user_fields =
            BTreeMap::from([("private_notes".into(), json!("Alice only"))]);
        destination["markdown"] = json!(format!(
            "```json\n{}\n```",
            record.canonical_json().unwrap()
        ));
        destination["properties"]
            .as_array_mut()
            .unwrap()
            .extend(private_properties.as_array().unwrap().iter().cloned());
        destination["properties"].as_array_mut().unwrap().extend([
            json!({"key":"creator","format":"objects","objects":[{"id":"alice"}]}),
            json!({"key":"created_date","format":"date","date":"2026-09-23T10:00:00Z"}),
            json!({"key":"links","format":"objects","objects":[{"id":"home-project"}]}),
        ]);
    }
    fixture
        .source
        .document
        .content
        .fields
        .get_mut("FN")
        .unwrap()[0]
        .value = "Carol Updated".into();

    // Restart before refresh: recover identity and private data from HTTP GET,
    // and require fresh binding-scoped schema discovery for the new runtime.
    for expected_kind in [RefreshKind::SourceRefreshed, RefreshKind::Unchanged] {
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(false);
        let mut reopened = fixture.app();
        reopened.discover_domain_capabilities("home").unwrap();
        let refreshed = reopened
            .materialize_person_contexts(&profile(), "alice-token", &scopes())
            .unwrap();
        assert!(!refreshed[0].created);
        assert_eq!(refreshed[0].metadata_projection, MetadataProjection::Native);
        assert_eq!(refreshed[0].destination_object_id, original_id);
        assert_eq!(refreshed[0].refresh_kind, expected_kind);
        assert_eq!(
            refreshed[0].reference.source_fields["display_name"],
            "Carol Updated"
        );
        assert_eq!(
            refreshed[0].reference.user_fields["private_notes"],
            "Alice only"
        );
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        let patch = &script.requests.last().unwrap().1;
        assert!(patch.get("type_key").is_none());
        let properties = patch["properties"]
            .as_array()
            .unwrap()
            .iter()
            .map(|property| (property["key"].as_str().unwrap(), property))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(properties.len(), MANAGED_KEYS.len() + 3);
        assert_eq!(
            properties[ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY]["text"],
            foreign_identity
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY]["text"],
            "space-shared"
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY]["text"],
            fixture.source.anytype_object_id.to_string()
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY]["text"],
            "carol-uid"
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_KIND_PROPERTY_KEY]["text"],
            "contact"
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_STATUS_PROPERTY_KEY]["text"],
            "available"
        );
        assert_eq!(properties["private_notes"]["text"], "Private café note");
        assert_eq!(
            properties["private_tags"]["multi_select"],
            json!(["follow-up"])
        );
        assert_eq!(
            properties["private_relations"]["objects"],
            json!(["home-project"])
        );
        assert!(!properties.contains_key("creator"));
        assert!(!properties.contains_key("created_date"));
        assert!(!properties.contains_key("links"));
        let persisted = record_from_markdown(&patch["markdown"]);
        assert_eq!(persisted.reference, refreshed[0].reference);
        assert_eq!(
            properties[ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY]["text"],
            persisted.profile_fingerprint
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY]["text"],
            persisted.reference.foreign.upstream_account_fingerprint
        );
    }
    let script = fixture.script.lock().unwrap();
    assert_eq!(
        script
            .requests
            .iter()
            .filter(|(line, _)| line.starts_with("POST "))
            .count(),
        1
    );
    assert_eq!(
        script
            .requests
            .iter()
            .filter(|(line, _)| line.starts_with("PATCH "))
            .count(),
        2
    );
}
