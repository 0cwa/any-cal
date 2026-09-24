use any_cal_app::composition::CompositionSourceScope;
use any_cal_app::composition_orchestration::MaterializedReferenceRecord;
use any_cal_app::identity::{
    AccessPolicy, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::person_context::private_person_context_fields;
use any_cal_app::{App, AppConfig};
use any_cal_core::{CompositionProfile, ProjectionPolicy, ResourceEnvelope};
use any_cal_dav_server::Request;
use std::collections::BTreeSet;

fn config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-must-not-compose".into();
    config.credential_profile_id = "primary".into();
    config.domain_bindings_json = Some(
        r#"{
          "version":1,
          "bindings":[
            {
              "domain_id":"alice-home",
              "label":"Alice Home",
              "space_id":"space-alice",
              "credential_profile_id":"primary",
              "routes":[{"collection":"contacts","component":"vcard","path":"/carddav/alice-home"}],
              "schema_profile":"default",
              "checkpoint_namespace":"alice-home",
              "visibility":"private",
              "lifecycle":"configured"
            },
            {
              "domain_id":"bob-home",
              "label":"Bob Home",
              "space_id":"space-bob",
              "credential_profile_id":"primary",
              "routes":[{"collection":"contacts","component":"vcard","path":"/carddav/bob-home"}],
              "schema_profile":"default",
              "checkpoint_namespace":"bob-home",
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
            }
          ]
        }"#
        .into(),
    );
    config
}

fn request(method: &str, path: &str, body: &[u8], token: Option<&str>) -> Request {
    let mut headers = Vec::new();
    if method == "PUT" {
        headers.push(("Content-Type".into(), "text/vcard".into()));
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
    let bob = PrincipalId::new("bob").unwrap();
    let mut policy = AccessPolicy::new();

    for (principal, home) in [(alice.clone(), "alice-home"), (bob.clone(), "bob-home")] {
        assert!(policy.grant_domain_collection(
            principal.clone(),
            "shared",
            CollectionKind::Contacts,
            Operation::Read,
        ));
        assert!(policy.grant_domain_collection(
            principal.clone(),
            home,
            CollectionKind::Composition,
            Operation::Write,
        ));
        assert!(policy.grant_domain_collection(
            principal,
            home,
            CollectionKind::Contacts,
            Operation::Read,
        ));
    }

    let mut identity = IdentityStore::new(policy);
    for (id, principal, token) in [
        ("alice-key", alice, "alice-token"),
        ("bob-key", bob, "bob-token"),
    ] {
        identity
            .add_credential(
                CredentialSpec {
                    id: id.into(),
                    principal,
                    not_before: 100,
                    expires_at: 200,
                    capabilities: BTreeSet::new(),
                },
                token,
            )
            .unwrap();
    }
    identity
}

fn profile(principal: &str, destination: &str) -> CompositionProfile {
    CompositionProfile::new(
        principal,
        destination,
        vec!["shared".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap()
}

fn scope() -> Vec<CompositionSourceScope> {
    vec![CompositionSourceScope::resource(
        "shared",
        CollectionKind::Contacts,
        "carol",
    )]
}

#[test]
fn same_shared_contact_materializes_as_two_independent_private_person_contexts() {
    let mut app = App::fake(config()).unwrap();
    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/shared/carol.vcf",
            b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:carol-uid\r\nFN:Carol Smith\r\nEMAIL:carol@example.test\r\nTEL:+1-555-0100\r\nEND:VCARD\r\n",
            None,
        ))
        .status,
        201
    );
    let mut app = app.with_identity(identity(), 150);

    let alice = app
        .materialize_person_contexts(
            &profile("alice", "alice-home"),
            "alice-token",
            &scope(),
        )
        .unwrap();
    let bob = app
        .materialize_person_contexts(&profile("bob", "bob-home"), "bob-token", &scope())
        .unwrap();
    assert_eq!(alice.len(), 1);
    assert_eq!(bob.len(), 1);

    // The deterministic local object label may match across private Spaces;
    // Space identity keeps them distinct.
    let alice_id = alice[0].destination_object_id.clone();
    let bob_id = bob[0].destination_object_id.clone();

    app.with_transport_mut(|transport| {
        let alice_object = transport
            .objects
            .get_mut_in_space("space-alice", &alice_id)
            .unwrap();
        let mut alice_record =
            MaterializedReferenceRecord::from_json(&alice_object.body).unwrap();
        alice_record.reference.user_fields = private_person_context_fields(
            "Alice private note",
            ["friend".to_owned()],
        );
        alice_object.body = alice_record.canonical_json().unwrap();

        let bob_object = transport
            .objects
            .get_mut_in_space("space-bob", &bob_id)
            .unwrap();
        let mut bob_record = MaterializedReferenceRecord::from_json(&bob_object.body).unwrap();
        bob_record.reference.user_fields =
            private_person_context_fields("Bob private note", ["vendor".to_owned()]);
        bob_object.body = bob_record.canonical_json().unwrap();

        let source_id = alice[0].reference.foreign.source_object_id.clone();
        let source = transport
            .objects
            .get_mut_in_space("space-shared", &source_id)
            .unwrap();
        let mut envelope = ResourceEnvelope::from_json(&source.body).unwrap();
        envelope.document.content.fields.get_mut("FN").unwrap()[0].value =
            "Carol Renamed".into();
        source.body = envelope.canonical_json().unwrap();
    });

    let alice = app
        .materialize_person_contexts(
            &profile("alice", "alice-home"),
            "alice-token",
            &scope(),
        )
        .unwrap();
    let bob = app
        .materialize_person_contexts(&profile("bob", "bob-home"), "bob-token", &scope())
        .unwrap();

    assert_eq!(alice[0].reference.source_fields["display_name"], "Carol Renamed");
    assert_eq!(bob[0].reference.source_fields["display_name"], "Carol Renamed");
    assert_eq!(
        alice[0].reference.user_fields["private_notes"],
        "Alice private note"
    );
    assert_eq!(
        bob[0].reference.user_fields["private_notes"],
        "Bob private note"
    );
    assert_eq!(
        alice[0].reference.user_fields["private_tags"],
        serde_json::json!(["friend"])
    );
    assert_eq!(
        bob[0].reference.user_fields["private_tags"],
        serde_json::json!(["vendor"])
    );

    let transport = app.transport_snapshot();
    let shared_source = transport
        .objects
        .values()
        .find(|object| object.space_id == "space-shared")
        .unwrap();
    assert!(!shared_source.body.contains("Alice private note"));
    assert!(!shared_source.body.contains("Bob private note"));
    assert_eq!(
        transport
            .objects
            .values()
            .filter(|object| {
                object.space_id == "space-alice"
                    && MaterializedReferenceRecord::from_json(&object.body).is_ok()
            })
            .count(),
        1
    );
    assert_eq!(
        transport
            .objects
            .values()
            .filter(|object| {
                object.space_id == "space-bob"
                    && MaterializedReferenceRecord::from_json(&object.body).is_ok()
            })
            .count(),
        1
    );

    for (path, token, id) in [
        ("/carddav/alice-home", "alice-token", alice_id.as_str()),
        ("/carddav/bob-home", "bob-token", bob_id.as_str()),
    ] {
        let report = app.handle(request(
            "REPORT",
            path,
            br#"<c:addressbook-query xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:d="DAV:"><d:prop><d:getetag/><c:address-data/></d:prop></c:addressbook-query>"#,
            Some(token),
        ));
        assert_eq!(report.status, 207);
        let body = String::from_utf8(report.body).unwrap();
        assert!(!body.contains(id));
        assert!(!body.contains("private note"));
    }
}
