use any_cal_anytype_adapter::{
    AmbiguousMutation, ObjectRecord, ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_PRIVATE_RELATIONS_PROPERTY_KEY, ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
};
use any_cal_app::composition::CompositionSourceScope;
use any_cal_app::composition_orchestration::{
    MaterializedReferenceRecord, MATERIALIZED_RECORD_TYPE,
};
use any_cal_app::identity::{
    AccessPolicy, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::{App, AppConfig};
use any_cal_core::{CompositionProfile, ProjectionPolicy, RefreshKind, ResourceEnvelope};
use any_cal_dav_server::Request;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

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
    let mut app = App::fake(config()).unwrap();
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
    let mut typed = seed();
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
        typed.transport_snapshot().create_type_keys,
        vec![any_cal_anytype_adapter::ANYCAL_PERSON_CONTEXT_TYPE_KEY.to_owned()]
    );

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
    let mut app = seed();
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
        transport.objects.insert_object(ObjectRecord {
            id: "local-project".into(),
            space_id: "space-home".into(),
            properties: Vec::new(),
            property_formats: BTreeMap::new(),
            body: "ordinary local object".into(),
            archived: false,
            revision: 0,
        });

        let context = transport
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
            .objects
            .get_mut_in_space("space-shared", &source_id)
            .unwrap();
        let mut envelope = ResourceEnvelope::from_json(&source.body).unwrap();
        envelope.document.content.fields.get_mut("FN").unwrap()[0].value =
            "Carol Relation Refresh".into();
        source.body = envelope.canonical_json().unwrap();
    });

    let remote = app.transport_snapshot();
    let mut reopened = App::with_transport(config(), remote)
        .unwrap()
        .with_identity(identity(), 150);
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
    assert_eq!(properties[ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY], "space-shared");
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
