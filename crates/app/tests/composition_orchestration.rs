#[path = "support/composition_fixture.rs"]
mod composition_fixture;

use any_cal_anytype_adapter::{
    AmbiguousMutation, AnytypeTransport, HttpAnytypeTransport, HttpExchange, ObjectRecord,
    TransportError, ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY, ANYCAL_PERSON_CONTEXT_TYPE_KEY,
    ANYCAL_PRIVATE_RELATIONS_PROPERTY_KEY, ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY, ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY, ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
    DEFAULT_OBJECT_TYPE_KEY,
};
use any_cal_app::composition::CompositionSourceScope;
use any_cal_app::composition_orchestration::{
    MaterializedReferenceRecord, MATERIALIZED_RECORD_TYPE,
};
use any_cal_app::identity::{
    AccessPolicy, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::{App, AppConfig, AppGeneric};
use any_cal_core::{CompositionProfile, ProjectionPolicy, RefreshKind, ResourceEnvelope};
use any_cal_dav_server::Request;
use composition_fixture::{CompositionFixtureTransport, CompositionSchemaFixture};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct HttpScript {
    steps: VecDeque<(String, serde_json::Value)>,
    requests: Vec<String>,
}

struct ScriptedHttp(Arc<Mutex<HttpScript>>);

impl HttpExchange for ScriptedHttp {
    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        let text = std::str::from_utf8(request).unwrap();
        let (headers, body) = text.split_once("\r\n\r\n").unwrap();
        let mut script = self.0.lock().unwrap();
        let (expected, response) = script.steps.pop_front().expect("unexpected HTTP request");
        assert_eq!(headers.lines().next().unwrap(), expected);
        assert!(headers.contains("Anytype-Version: 2025-11-08"));
        assert!(headers.contains("Authorization: Bearer fixture-upstream-token"));
        let declared = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length:"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        assert_eq!(declared, body.len());
        script.requests.push(text.to_owned());
        let response = response.to_string();
        Ok(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{response}",
            response.len()
        )
        .into_bytes())
    }
}

fn config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-must-not-compose".into();
    config.credential_profile_id = "primary".into();
    config.domain_bindings_json = Some(
        r#"{
          "version":1,
          "bindings":[
            {
              "domain_id":"home",
              "label":"Home",
              "space_id":"space-home",
              "credential_profile_id":"primary",
              "routes":[{"collection":"contacts","component":"vcard","path":"/carddav/home"}],
              "schema_profile":"default",
              "checkpoint_namespace":"home",
              "visibility":"private",
              "lifecycle":"configured"
            },
            {
              "domain_id":"shared",
              "label":"Shared",
              "space_id":"space-shared",
              "credential_profile_id":"primary",
              "routes":[{"collection":"contacts","component":"vcard","path":"/carddav/shared"}],
              "schema_profile":"default",
              "checkpoint_namespace":"shared",
              "visibility":"shared",
              "lifecycle":"configured"
            },
            {
              "domain_id":"tasks",
              "label":"Tasks",
              "space_id":"space-tasks",
              "credential_profile_id":"primary",
              "routes":[{"collection":"tasks","component":"vtodo","path":"/caldav/tasks"}],
              "schema_profile":"default",
              "checkpoint_namespace":"tasks",
              "visibility":"shared",
              "lifecycle":"configured"
            }
          ]
        }"#
        .into(),
    );
    config
}

fn req(method: &str, path: &str, body: &[u8], token: Option<&str>) -> Request {
    let mut headers = Vec::new();
    if method == "PUT" {
        headers.push((
            "Content-Type".into(),
            if path.ends_with(".vcf") {
                "text/vcard".into()
            } else {
                "text/calendar".into()
            },
        ));
    }
    if let Some(token) = token {
        headers.push(("Authorization".into(), format!("Bearer {token}")));
    }
    Request {
        method: method.into(),
        path: path.into(),
        headers,
        body: body.to_vec(),
    }
}

fn identity() -> IdentityStore {
    let alice = PrincipalId::new("alice").unwrap();
    let mut policy = AccessPolicy::new();
    for (domain, collection, operation) in [
        ("shared", CollectionKind::Contacts, Operation::Read),
        ("tasks", CollectionKind::Tasks, Operation::Read),
        ("home", CollectionKind::Composition, Operation::Write),
        ("home", CollectionKind::Contacts, Operation::Read),
    ] {
        assert!(policy.grant_domain_collection(alice.clone(), domain, collection, operation));
    }
    let mut identity = IdentityStore::new(policy);
    identity
        .add_credential(
            CredentialSpec {
                id: "alice-key".into(),
                principal: alice,
                not_before: 100,
                expires_at: 200,
                capabilities: BTreeSet::new(),
            },
            "alice-token",
        )
        .unwrap();
    identity
}

fn seed() -> App {
    seed_app(App::fake(config()).unwrap())
}

fn seed_with_schema(properties: &[(&str, &str)]) -> AppGeneric<CompositionFixtureTransport> {
    let transport = CompositionFixtureTransport::new(
        100,
        BTreeMap::from([(
            "space-home".into(),
            CompositionSchemaFixture::new(
                &[DEFAULT_OBJECT_TYPE_KEY, ANYCAL_PERSON_CONTEXT_TYPE_KEY],
                properties,
            ),
        )]),
    );
    let mut app = seed_app(AppGeneric::with_transport(config(), transport).unwrap());
    app.discover_domain_capabilities("home").unwrap();
    app
}

fn seed_app<T: AnytypeTransport>(mut app: AppGeneric<T>) -> AppGeneric<T> {
    assert_eq!(
        app.handle(req(
            "PUT",
            "/carddav/shared/carol.vcf",
            b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:carol-uid\r\nFN:Carol Smith\r\nEMAIL:carol@example.test\r\nEND:VCARD\r\n",
            None,
        ))
        .status,
        201
    );
    assert_eq!(
        app.handle(req(
            "PUT",
            "/caldav/tasks/task-1.ics",
            b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:task-uid\r\nSUMMARY:Call Carol\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
            None,
        ))
        .status,
        201
    );
    app.with_identity(identity(), 150)
}

fn profile() -> CompositionProfile {
    CompositionProfile::new(
        "alice",
        "home",
        vec!["shared".into(), "tasks".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap()
}

fn scopes() -> Vec<CompositionSourceScope> {
    vec![
        CompositionSourceScope::resource("shared", CollectionKind::Contacts, "carol"),
        CompositionSourceScope::resource("tasks", CollectionKind::Tasks, "task-1"),
    ]
}

fn projector(stored: &any_cal_core::StoredResource) -> BTreeMap<String, serde_json::Value> {
    let field = match stored.envelope.kind {
        any_cal_core::DavKind::Contact => "FN",
        any_cal_core::DavKind::Task => "SUMMARY",
        _ => return BTreeMap::new(),
    };
    let title = stored
        .envelope
        .document
        .content
        .fields
        .get(field)
        .and_then(|values| values.first())
        .map(|occurrence| occurrence.value.clone())
        .unwrap_or_default();
    BTreeMap::from([("title".into(), json!(title))])
}

#[test]
fn read_many_write_one_is_idempotent_private_and_dav_invisible() {
    let mut app = seed();

    let first = app
        .materialize_references(&profile(), "alice-token", &scopes(), projector)
        .unwrap();
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|result| result.created));

    let contact = first
        .iter()
        .find(|result| result.source_domain_id == "shared")
        .unwrap();
    let destination_id = contact.destination_object_id.clone();
    let source_object_id = contact.reference.foreign.source_object_id.clone();

    let transport = app.transport_snapshot();
    assert_eq!(
        transport
            .objects
            .values()
            .filter(|object| {
                object.space_id == "space-home"
                    && MaterializedReferenceRecord::from_json(&object.body).is_ok()
            })
            .count(),
        2
    );
    assert!(!transport.objects.values().any(|object| {
        object.space_id != "space-home" && object.body.contains(MATERIALIZED_RECORD_TYPE)
    }));

    app.with_transport_mut(|transport| {
        let destination = transport
            .objects
            .get_mut_in_space("space-home", &destination_id)
            .unwrap();
        let mut record = MaterializedReferenceRecord::from_json(&destination.body).unwrap();
        record
            .reference
            .user_fields
            .insert("private_notes".into(), json!("Alice only"));
        destination.body = record.canonical_json().unwrap();

        let source = transport
            .objects
            .get_mut_in_space("space-shared", &source_object_id)
            .unwrap();
        let mut envelope = ResourceEnvelope::from_json(&source.body).unwrap();
        envelope.document.content.fields.get_mut("FN").unwrap()[0].value = "Carol Updated".into();
        source.body = envelope.canonical_json().unwrap();
    });

    let second = app
        .materialize_references(&profile(), "alice-token", &scopes(), projector)
        .unwrap();
    assert!(second.iter().all(|result| !result.created));
    let contact = second
        .iter()
        .find(|result| result.source_domain_id == "shared")
        .unwrap();
    assert_eq!(contact.destination_object_id, destination_id);
    assert_eq!(contact.refresh_kind, RefreshKind::SourceRefreshed);
    assert_eq!(contact.reference.source_fields["title"], "Carol Updated");
    assert_eq!(contact.reference.user_fields["private_notes"], "Alice only");

    let report = app.handle(req(
        "REPORT",
        "/carddav/home",
        br#"<c:addressbook-query xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:d="DAV:"><d:prop><d:getetag/><c:address-data/></d:prop></c:addressbook-query>"#,
        Some("alice-token"),
    ));
    assert_eq!(report.status, 207);
    let body = String::from_utf8(report.body).unwrap();
    assert!(!body.contains(&destination_id));
    assert!(!body.contains("Carol Updated"));

    app.with_transport_mut(|transport| {
        transport
            .objects
            .remove_in_space("space-shared", &source_object_id)
            .unwrap();
    });
    let deleted = app
        .materialize_references(
            &profile(),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                "carol",
            )],
            projector,
        )
        .unwrap();
    assert_eq!(deleted[0].refresh_kind, RefreshKind::SourceDeleted);
    assert_eq!(
        deleted[0].reference.user_fields["private_notes"],
        "Alice only"
    );
    assert_eq!(deleted[0].reference.source_fields["title"], "Carol Updated");
}

#[test]
fn http_refresh_filters_system_properties_and_preserves_private_native_fields() {
    let source_scopes = [CompositionSourceScope::resource(
        "shared",
        CollectionKind::Contacts,
        "carol",
    )];
    let mut seeded = seed();
    let initial = seeded
        .materialize_references(&profile(), "alice-token", &source_scopes, projector)
        .unwrap();
    let destination_id = &initial[0].destination_object_id;
    let source_id = &initial[0].reference.foreign.source_object_id;
    let snapshot = seeded.transport_snapshot();
    let destination = snapshot
        .objects
        .get_in_space("space-home", destination_id)
        .unwrap();
    let source = snapshot
        .objects
        .get_in_space("space-shared", source_id)
        .unwrap();

    // A fresh HTTP app must recover the reference from Anytype, with no cached
    // fake-transport state or custom metadata schema to identify the object.
    let script = Arc::new(Mutex::new(HttpScript::default()));
    let mut http_config = config();
    http_config.token = Some("fixture-upstream-token".into());
    let mut app = AppGeneric::<HttpAnytypeTransport>::http_with_exchange(
        http_config,
        Box::new(ScriptedHttp(Arc::clone(&script))),
    )
    .unwrap()
    .with_identity(identity(), 150);
    let mut record = MaterializedReferenceRecord::from_json(&destination.body).unwrap();
    record.reference.foreign.upstream_account_fingerprint =
        app.upstream_binding_identity("shared").unwrap().1;
    record.reference.user_fields = BTreeMap::from([
        ("private_notes".into(), json!("Alice only")),
        ("private_tags".into(), json!(["follow-up"])),
        ("private_relations".into(), json!(["home-project"])),
    ]);
    let mut source_envelope = ResourceEnvelope::from_json(&source.body).unwrap();
    source_envelope
        .document
        .content
        .fields
        .get_mut("FN")
        .unwrap()[0]
        .value = "Carol Updated".into();
    let mut refreshed = record.clone();
    refreshed
        .reference
        .source_fields
        .insert("title".into(), json!("Carol Updated"));

    let properties = json!([
        {"key":"backlinks","format":"objects","objects":[{"id":"home-note"}]},
        {"key":"creator","format":"objects","objects":[{"id":"alice"}]},
        {"key":"created_date","format":"date","date":"2026-09-23T10:00:00Z"},
        {"key":"last_modified_date","format":"date","date":"2026-09-24T10:00:00Z"},
        {"key":"last_modified_by","format":"objects","objects":[{"id":"alice"}]},
        {"key":"links","format":"objects","objects":[{"id":"home-project"}]},
        {"key":"private_notes","format":"text","text":"Private café note"},
        {"key":"private_tags","format":"multi_select","multi_select":[
            {"key":"follow-up","name":"Follow up"},{"key":"friends","name":"Friends"}
        ]},
        {"key":"private_relations","format":"objects","objects":[
            {"id":"home-project","name":"Private project"},{"id":"home-note","name":"Private note"}
        ]}
    ]);
    let destination_response = |record: &MaterializedReferenceRecord| {
        json!({"object":{
            "id":destination_id,
            "type":{"key":"page"},
            "markdown":format!("```json\n{}\n```", record.canonical_json().unwrap()),
            "properties":properties
        }})
    };
    script.lock().unwrap().steps = [
        (
            "GET /v1/spaces/space-home/objects HTTP/1.1".into(),
            json!({"data":[{"id":destination_id,"type":{"key":"page"}}]}),
        ),
        (
            format!("GET /v1/spaces/space-home/objects/{destination_id} HTTP/1.1"),
            destination_response(&record),
        ),
        (
            "GET /v1/spaces/space-shared/objects HTTP/1.1".into(),
            json!({"data":[{"id":source_id,"type":{"key":"page"}}]}),
        ),
        (
            format!("GET /v1/spaces/space-shared/objects/{source_id} HTTP/1.1"),
            json!({"object":{
                "id":source_id,
                "markdown":format!("```json\n{}\n```", source_envelope.canonical_json().unwrap()),
                "properties":[]
            }}),
        ),
        (
            format!("PATCH /v1/spaces/space-home/objects/{destination_id} HTTP/1.1"),
            destination_response(&refreshed),
        ),
    ]
    .into_iter()
    .collect();

    let results = app
        .materialize_references(&profile(), "alice-token", &source_scopes, projector)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(!results[0].created);
    assert_eq!(&results[0].destination_object_id, destination_id);
    assert_eq!(results[0].refresh_kind, RefreshKind::SourceRefreshed);
    assert_eq!(results[0].reference, refreshed.reference);

    let script = script.lock().unwrap();
    assert!(script.steps.is_empty());
    assert_eq!(script.requests.len(), 5);
    let patch: serde_json::Value =
        serde_json::from_str(script.requests[4].split_once("\r\n\r\n").unwrap().1).unwrap();
    let patch_properties = patch["properties"].as_array().unwrap();
    assert_eq!(patch_properties.len(), 3);
    for expected in [
        json!({"key":"private_notes","text":"Private café note"}),
        json!({"key":"private_tags","multi_select":["follow-up","friends"]}),
        json!({"key":"private_relations","objects":["home-project","home-note"]}),
    ] {
        assert!(patch_properties.contains(&expected));
    }
    assert!(patch.get("type_key").is_none());
    assert!(patch.get("body").is_none());
    let persisted = MaterializedReferenceRecord::from_json(
        patch["markdown"]
            .as_str()
            .unwrap()
            .strip_prefix("```json\n")
            .unwrap()
            .strip_suffix("\n```")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(persisted, refreshed);
    assert_eq!(
        persisted.reference.user_fields,
        record.reference.user_fields
    );
}

#[test]
fn ambiguous_create_is_reconciled_without_duplicate_materialization() {
    let mut app = seed();
    app.with_transport_mut(|transport| transport.timeout_after(AmbiguousMutation::Create));

    let result = app
        .materialize_references(
            &profile(),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                "carol",
            )],
            projector,
        )
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].created);

    let transport = app.transport_snapshot();
    assert_eq!(
        transport
            .objects
            .values()
            .filter(|object| {
                object.space_id == "space-home"
                    && MaterializedReferenceRecord::from_json(&object.body).is_ok()
            })
            .count(),
        1
    );
}

#[test]
fn typed_materialization_uses_explicit_type_key_while_default_stays_page() {
    let mut typed = seed_with_schema(&[]);
    typed
        .materialize_references_with_type(
            &profile(),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                "carol",
            )],
            any_cal_anytype_adapter::ANYCAL_PERSON_CONTEXT_TYPE_KEY,
            projector,
        )
        .unwrap();
    assert_eq!(
        typed.transport_snapshot().inner.create_type_keys,
        vec![any_cal_anytype_adapter::ANYCAL_PERSON_CONTEXT_TYPE_KEY.to_owned()]
    );
    assert!(typed
        .transport_snapshot()
        .inner
        .objects
        .values()
        .filter(|object| object.space_id == "space-home")
        .all(|object| object.properties.is_empty()));

    let mut default = seed();
    default
        .materialize_references(
            &profile(),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                "carol",
            )],
            projector,
        )
        .unwrap();
    assert_eq!(
        default.transport_snapshot().create_type_keys,
        vec![any_cal_anytype_adapter::DEFAULT_OBJECT_TYPE_KEY.to_owned()]
    );
}

#[test]
fn managed_source_metadata_refreshes_while_private_same_space_relation_survives_reopen() {
    let mut app = seed_with_schema(&[
        (ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY, "text"),
        (ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY, "text"),
        (ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY, "text"),
        (ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, "text"),
        (ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY, "text"),
        (ANYCAL_SOURCE_KIND_PROPERTY_KEY, "text"),
        (ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY, "text"),
        (ANYCAL_SOURCE_STATUS_PROPERTY_KEY, "text"),
    ]);
    let result = app
        .materialize_references(
            &profile(),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                "carol",
            )],
            projector,
        )
        .unwrap();
    let context_id = result[0].destination_object_id.clone();
    let source_id = result[0].reference.foreign.source_object_id.clone();
    let foreign_identity = result[0].reference.foreign.identity_fingerprint().unwrap();

    app.with_transport_mut(|transport| {
        transport.inner.objects.insert_object(ObjectRecord {
            id: "local-project".into(),
            space_id: "space-home".into(),
            properties: Vec::new(),
            property_formats: BTreeMap::new(),
            body: "ordinary local object".into(),
            archived: false,
            revision: 0,
        });

        let context = transport
            .inner
            .objects
            .get_mut_in_space("space-home", &context_id)
            .unwrap();
        context.properties.push((
            ANYCAL_PRIVATE_RELATIONS_PROPERTY_KEY.into(),
            serde_json::json!(["local-project"]).to_string(),
        ));
        context.property_formats.insert(
            ANYCAL_PRIVATE_RELATIONS_PROPERTY_KEY.into(),
            "objects".into(),
        );

        let source = transport
            .inner
            .objects
            .get_mut_in_space("space-shared", &source_id)
            .unwrap();
        let mut envelope = ResourceEnvelope::from_json(&source.body).unwrap();
        envelope.document.content.fields.get_mut("FN").unwrap()[0].value =
            "Carol Relation Refresh".into();
        source.body = envelope.canonical_json().unwrap();
    });

    let remote = app.transport_snapshot();
    let mut reopened = AppGeneric::with_transport(config(), remote)
        .unwrap()
        .with_identity(identity(), 150);
    reopened.discover_domain_capabilities("home").unwrap();
    reopened
        .materialize_references(
            &profile(),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                "carol",
            )],
            projector,
        )
        .unwrap();

    let remote = reopened.transport_snapshot();
    let context = remote
        .inner
        .objects
        .get_in_space("space-home", &context_id)
        .unwrap();
    let properties = context
        .properties
        .iter()
        .cloned()
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        properties[ANYCAL_PRIVATE_RELATIONS_PROPERTY_KEY],
        serde_json::json!(["local-project"]).to_string()
    );
    assert_eq!(
        context.property_formats[ANYCAL_PRIVATE_RELATIONS_PROPERTY_KEY],
        "objects"
    );
    assert_eq!(
        properties[ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY],
        "space-shared"
    );
    assert_eq!(properties[ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY], source_id);
    assert_eq!(properties[ANYCAL_SOURCE_STATUS_PROPERTY_KEY], "available");
    assert_eq!(
        properties[ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY],
        foreign_identity
    );

    let record = MaterializedReferenceRecord::from_json(&context.body).unwrap();
    assert_eq!(
        record.reference.source_fields["title"],
        "Carol Relation Refresh"
    );
}
