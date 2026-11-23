//! Native Anytype title ownership exercised through sanitized HTTP requests.
#[path = "support/composition_http.rs"]
#[allow(dead_code)]
mod composition_http;

use any_cal_anytype_adapter::TransportError;
use any_cal_app::composition_orchestration::{
    CompositionOrchestrationError, MaterializationResult, MaterializedReferenceRecord,
    MetadataProjection, PersonContextTitleState,
};
use any_cal_app::person_context::PersonContextMaterializationError;
use composition_http::{
    managed_schema, person_types, profile, record_from_markdown, scopes, Fixture, Reply,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn materialize(
    fixture: &Fixture,
    create: bool,
    native_metadata: bool,
) -> Vec<MaterializationResult> {
    fixture.discovery(
        person_types(),
        if native_metadata {
            managed_schema()
        } else {
            Vec::new()
        },
    );
    fixture.materialization(create);
    // Every call opens a new app, so title ownership must come from the GET
    // response and durable record rather than an in-memory title cache.
    let mut app = fixture.app();
    app.discover_domain_capabilities("home").unwrap();
    let results = app
        .materialize_person_contexts(&profile(), "alice-token", &scopes())
        .unwrap();
    assert!(fixture.script.lock().unwrap().steps.is_empty());
    results
}

fn source_title(fixture: &mut Fixture, name: &str) {
    fixture
        .source
        .document
        .content
        .fields
        .get_mut("FN")
        .unwrap()[0]
        .value = name.into();
}

fn native_title(fixture: &Fixture, name: Option<&str>) {
    let mut script = fixture.script.lock().unwrap();
    let destination = script
        .destination
        .as_mut()
        .unwrap()
        .as_object_mut()
        .unwrap();
    if let Some(name) = name {
        destination.insert("name".into(), json!(name));
    } else {
        destination.remove("name");
    }
}

fn record(fixture: &Fixture) -> MaterializedReferenceRecord {
    let script = fixture.script.lock().unwrap();
    record_from_markdown(&script.destination.as_ref().unwrap()["markdown"])
}

fn last_write(fixture: &Fixture) -> Value {
    fixture
        .script
        .lock()
        .unwrap()
        .requests
        .iter()
        .rev()
        .find(|(line, _)| line.starts_with("POST ") || line.starts_with("PATCH "))
        .unwrap()
        .1
        .clone()
}

fn assert_destination_writes(fixture: &Fixture) {
    let script = fixture.script.lock().unwrap();
    assert!(script.steps.is_empty());
    for (line, body) in &script.requests {
        if line.starts_with("POST ") || line.starts_with("PATCH ") {
            assert!(
                line.starts_with("POST /v1/spaces/space-home/objects ")
                    || line.starts_with("PATCH /v1/spaces/space-home/objects/context-1 "),
                "composition must write only the private destination"
            );
            assert!(body["properties"]
                .as_array()
                .unwrap()
                .iter()
                .all(|property| property["key"] != "name"));
        }
    }
}

#[test]
fn new_person_context_title_follows_source_renames_across_reopen() {
    let mut fixture = Fixture::new();
    let created = materialize(&fixture, true, false);
    let object_id = created[0].destination_object_id.clone();
    assert!(created[0].created);
    assert_eq!(created[0].metadata_projection, MetadataProjection::BodyOnly);
    assert_eq!(last_write(&fixture)["name"], "Carol Smith");
    assert_eq!(
        record(&fixture).person_context_title,
        Some(PersonContextTitleState::Source {
            last_written: "Carol Smith".into()
        })
    );

    for name in ["Carol Updated", "Caról 東京 · Updated again"] {
        source_title(&mut fixture, name);
        let refreshed = materialize(&fixture, false, false);
        assert!(!refreshed[0].created);
        assert_eq!(refreshed[0].destination_object_id, object_id);
        assert_eq!(refreshed[0].reference.source_fields["display_name"], name);
        assert_eq!(last_write(&fixture)["name"], name);
        assert_eq!(
            record(&fixture).person_context_title,
            Some(PersonContextTitleState::Source {
                last_written: name.into()
            })
        );
    }
    assert_destination_writes(&fixture);
}

#[test]
fn new_title_uses_bounded_unicode_source_name_or_the_existing_uid_fallback() {
    for (name, expected) in [
        ("界".repeat(1_025), "界".repeat(1_024)),
        (" ".into(), "carol-uid".into()),
    ] {
        let mut fixture = Fixture::new();
        source_title(&mut fixture, &name);
        materialize(&fixture, true, false);
        assert_eq!(last_write(&fixture)["name"], expected);
        assert_eq!(
            record(&fixture).person_context_title,
            Some(PersonContextTitleState::Source {
                last_written: expected
            })
        );
        assert_destination_writes(&fixture);
    }
}

#[test]
fn ordinary_local_title_and_private_property_edits_survive_source_refresh_and_reopen() {
    for local_title in ["Caról · 私の連絡先 👋", "", " \t "] {
        let mut fixture = Fixture::new();
        materialize(&fixture, true, true);
        native_title(&fixture, Some(local_title));
        {
            let mut script = fixture.script.lock().unwrap();
            script.destination.as_mut().unwrap()["properties"]
                .as_array_mut()
                .unwrap()
                .extend([
                    json!({"key":"private_notes","format":"text","text":"Private café note"}),
                    json!({"key":"private_tags","format":"multi_select","multi_select":[{"key":"follow-up","name":"Follow up"}]}),
                    json!({"key":"private_relations","format":"objects","objects":[{"id":"home-project","name":"Private project"}]}),
                ]);
        }

        for source_name in ["Carol Renamed", "Carol Renamed Again"] {
            source_title(&mut fixture, source_name);
            let refreshed = materialize(&fixture, false, true);
            assert_eq!(refreshed[0].metadata_projection, MetadataProjection::Native);
            assert_eq!(
                refreshed[0].reference.source_fields["display_name"],
                source_name
            );
            assert_eq!(
                record(&fixture).person_context_title,
                Some(PersonContextTitleState::Local)
            );
            let patch = last_write(&fixture);
            assert_eq!(patch["name"], local_title);
            let properties = patch["properties"]
                .as_array()
                .unwrap()
                .iter()
                .map(|property| (property["key"].as_str().unwrap(), property))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(properties["private_notes"]["text"], "Private café note");
            assert_eq!(
                properties["private_tags"]["multi_select"],
                json!(["follow-up"])
            );
            assert_eq!(
                properties["private_relations"]["objects"],
                json!(["home-project"])
            );
            assert_eq!(
                fixture.script.lock().unwrap().destination.as_ref().unwrap()["name"],
                local_title
            );
        }
        assert_destination_writes(&fixture);
    }
}

#[test]
fn an_observed_local_override_stays_local_when_it_later_equals_the_old_source_title() {
    let mut fixture = Fixture::new();
    materialize(&fixture, true, false);
    native_title(&fixture, Some("My private contact"));
    materialize(&fixture, false, false);
    assert_eq!(
        record(&fixture).person_context_title,
        Some(PersonContextTitleState::Local)
    );

    native_title(&fixture, Some("Carol Smith"));
    for source_name in ["Carol Renamed", "Carol Renamed Again"] {
        source_title(&mut fixture, source_name);
        materialize(&fixture, false, false);
        assert_eq!(last_write(&fixture)["name"], "Carol Smith");
        assert_eq!(
            record(&fixture).person_context_title,
            Some(PersonContextTitleState::Local)
        );
    }
    assert_destination_writes(&fixture);
}

#[test]
fn legacy_records_without_title_history_never_claim_an_observed_or_missing_native_name() {
    for observed in [Some("Existing private name"), Some(""), None] {
        let mut fixture = Fixture::new();
        materialize(&fixture, true, false);
        {
            let mut script = fixture.script.lock().unwrap();
            let destination = script.destination.as_mut().unwrap();
            let mut legacy = record_from_markdown(&destination["markdown"]);
            legacy.person_context_title = None;
            destination["markdown"] = json!(format!(
                "```json\n{}\n```",
                legacy.canonical_json().unwrap()
            ));
        }
        native_title(&fixture, observed);

        for source_name in ["Carol Renamed", "Carol Renamed Again"] {
            source_title(&mut fixture, source_name);
            materialize(&fixture, false, false);
            let patch = last_write(&fixture);
            assert_eq!(patch.get("name").and_then(Value::as_str), observed);
            assert_eq!(
                record(&fixture).person_context_title,
                observed.map(|_| PersonContextTitleState::Local)
            );
        }
        assert_destination_writes(&fixture);
    }
}

#[test]
fn missing_native_name_preserves_existing_source_or_local_ownership_without_a_name_write() {
    for local in [false, true] {
        let mut fixture = Fixture::new();
        materialize(&fixture, true, false);
        if local {
            native_title(&fixture, Some("Private title"));
            materialize(&fixture, false, false);
        }
        let previous_state = record(&fixture).person_context_title;
        native_title(&fixture, None);
        source_title(&mut fixture, "Carol Renamed While Name Was Missing");
        materialize(&fixture, false, false);
        assert!(last_write(&fixture).get("name").is_none());
        assert_eq!(record(&fixture).person_context_title, previous_state);
        assert!(fixture
            .script
            .lock()
            .unwrap()
            .destination
            .as_ref()
            .unwrap()
            .get("name")
            .is_none());

        // Once the response supplies a name again, the preserved ownership
        // state still decides whether another source rename may change it.
        native_title(
            &fixture,
            Some(if local {
                "Private title"
            } else {
                "Carol Smith"
            }),
        );
        source_title(&mut fixture, "Carol Renamed After Recovery");
        materialize(&fixture, false, false);
        assert_eq!(
            last_write(&fixture)["name"],
            if local {
                "Private title"
            } else {
                "Carol Renamed After Recovery"
            }
        );
        assert_destination_writes(&fixture);
    }
}

#[test]
fn timed_out_title_updates_require_native_name_readback_even_with_matching_body_and_metadata() {
    for native_metadata in [false, true] {
        for name_applied in [false, true] {
            let mut fixture = Fixture::new();
            materialize(&fixture, true, native_metadata);
            source_title(&mut fixture, "Carol New Title");
            fixture.discovery(
                person_types(),
                if native_metadata {
                    managed_schema()
                } else {
                    Vec::new()
                },
            );
            fixture.materialization(false);
            {
                let mut script = fixture.script.lock().unwrap();
                script.steps.back_mut().unwrap().1 = if name_applied {
                    Reply::SaveDestination { timeout: true }
                } else {
                    Reply::SaveDestinationIgnoringName
                };
                script.steps.push_back((
                    "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                    Reply::Destination,
                ));
            }
            let mut reopened = fixture.app();
            reopened.discover_domain_capabilities("home").unwrap();
            let result = reopened.materialize_person_contexts(&profile(), "alice-token", &scopes());
            if name_applied {
                assert!(!result.unwrap()[0].created);
            } else {
                assert_eq!(
                    result,
                    Err(PersonContextMaterializationError::Orchestration(
                        CompositionOrchestrationError::DestinationTransport(
                            TransportError::Timeout
                        )
                    ))
                );
            }
            let patch = last_write(&fixture);
            assert_eq!(patch["name"], "Carol New Title");
            let script = fixture.script.lock().unwrap();
            let destination = script.destination.as_ref().unwrap();
            assert_eq!(destination["markdown"], patch["markdown"]);
            assert_eq!(
                destination["name"],
                if name_applied {
                    "Carol New Title"
                } else {
                    "Carol Smith"
                }
            );
            assert_eq!(
                script
                    .requests
                    .iter()
                    .filter(|(line, _)| line.starts_with("PATCH "))
                    .count(),
                1
            );
            drop(script);
            assert_destination_writes(&fixture);
        }
    }
}

#[test]
fn timed_out_title_creates_require_native_name_readback_without_retrying_the_create() {
    for native_metadata in [false, true] {
        for name_applied in [false, true] {
            let fixture = Fixture::new();
            fixture.discovery(
                person_types(),
                if native_metadata {
                    managed_schema()
                } else {
                    Vec::new()
                },
            );
            fixture.materialization(true);
            {
                let mut script = fixture.script.lock().unwrap();
                script.steps.back_mut().unwrap().1 = if name_applied {
                    Reply::SaveDestination { timeout: true }
                } else {
                    Reply::SaveDestinationIgnoringName
                };
                script.steps.push_back((
                    "GET /v1/spaces/space-home/objects HTTP/1.1".into(),
                    Reply::Json(json!({"data":[{"id":"context-1"}]})),
                ));
                script.steps.push_back((
                    "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                    Reply::Destination,
                ));
            }
            let mut app = fixture.app();
            app.discover_domain_capabilities("home").unwrap();
            let result = app.materialize_person_contexts(&profile(), "alice-token", &scopes());
            if name_applied {
                assert!(result.unwrap()[0].created);
            } else {
                assert_eq!(
                    result,
                    Err(PersonContextMaterializationError::Orchestration(
                        CompositionOrchestrationError::DestinationTransport(
                            TransportError::Timeout
                        )
                    ))
                );
            }
            let create = last_write(&fixture);
            assert_eq!(create["name"], "Carol Smith");
            let script = fixture.script.lock().unwrap();
            let destination = script.destination.as_ref().unwrap();
            assert_eq!(destination["markdown"], create["body"]);
            assert_eq!(
                destination.get("name").and_then(Value::as_str),
                name_applied.then_some("Carol Smith")
            );
            assert_eq!(
                script
                    .requests
                    .iter()
                    .filter(|(line, _)| line.starts_with("POST "))
                    .count(),
                1
            );
            drop(script);
            assert_destination_writes(&fixture);
        }
    }
}

#[test]
fn timed_out_create_requires_the_full_durable_record_even_when_native_fields_match() {
    for native_metadata in [false, true] {
        let fixture = Fixture::new();
        fixture.discovery(
            person_types(),
            if native_metadata {
                managed_schema()
            } else {
                Vec::new()
            },
        );
        fixture.materialization(true);
        {
            let mut script = fixture.script.lock().unwrap();
            script.steps.back_mut().unwrap().1 =
                Reply::SaveDestinationWithRecordMutation(|record| {
                    record
                        .reference
                        .user_fields
                        .insert("private_notes".into(), json!("Concurrent private edit"));
                });
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects HTTP/1.1".into(),
                Reply::Json(json!({"data":[{"id":"context-1"}]})),
            ));
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                Reply::Destination,
            ));
        }
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        assert_eq!(
            app.materialize_person_contexts(&profile(), "alice-token", &scopes()),
            Err(PersonContextMaterializationError::Orchestration(
                CompositionOrchestrationError::DestinationTransport(TransportError::Timeout)
            ))
        );

        let create = last_write(&fixture);
        let script = fixture.script.lock().unwrap();
        let destination = script.destination.as_ref().unwrap();
        assert_eq!(destination["name"], create["name"]);
        let intended = record_from_markdown(&create["body"]);
        let observed = record_from_markdown(&destination["markdown"]);
        assert_eq!(observed.profile_fingerprint, intended.profile_fingerprint);
        assert_eq!(observed.reference.foreign, intended.reference.foreign);
        assert_eq!(observed.person_context_title, intended.person_context_title);
        assert_ne!(observed, intended);
        assert_eq!(
            observed.reference.user_fields["private_notes"],
            "Concurrent private edit"
        );
        let native_values = |properties: &Value| {
            properties
                .as_array()
                .unwrap()
                .iter()
                .map(|property| (property["key"].clone(), property["text"].clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            native_values(&destination["properties"]),
            native_values(&create["properties"])
        );
        assert_eq!(
            script
                .requests
                .iter()
                .filter(|(line, _)| line.starts_with("POST "))
                .count(),
            1
        );
        drop(script);
        assert_destination_writes(&fixture);
    }
}
