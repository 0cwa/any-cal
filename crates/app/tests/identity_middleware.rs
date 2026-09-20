use any_cal_app::identity::{
    AccessPolicy, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::{App, AppConfig};
use any_cal_dav_server::Request;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

fn principal(value: &str) -> PrincipalId {
    PrincipalId::new(value).unwrap()
}

fn request(method: &str, path: &str, token: Option<&str>, body: &[u8]) -> Request {
    let mut headers = token
        .map(|token| vec![("Authorization".into(), format!("Bearer {token}"))])
        .unwrap_or_default();
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
    Request {
        method: method.into(),
        path: path.into(),
        headers,
        body: body.to_vec(),
    }
}

fn identity() -> IdentityStore {
    let alice = principal("alice");
    let bob = principal("bob");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(alice.clone(), CollectionKind::Contacts, Operation::Read);
    policy.grant_collection(alice.clone(), CollectionKind::Contacts, Operation::Write);
    policy.grant_collection(bob.clone(), CollectionKind::Tasks, Operation::Read);
    policy.grant_resource(
        bob.clone(),
        CollectionKind::Contacts,
        "shared.vcf",
        Operation::Read,
    );
    let mut store = IdentityStore::new(policy);
    for (id, owner, token, not_before, expires_at) in [
        ("alice-key", alice, "alice-token", 100, 200),
        ("bob-key", bob, "bob-token", 100, 200),
    ] {
        store
            .add_credential(
                CredentialSpec {
                    id: id.into(),
                    principal: owner,
                    not_before,
                    expires_at,
                    capabilities: BTreeSet::new(),
                },
                token,
            )
            .unwrap();
    }
    store
}

fn app() -> App {
    let mut config = AppConfig::defaults();
    config.space_id = "identity-middleware-space".into();
    App::fake(config).unwrap().with_identity(identity(), 150)
}

#[test]
fn identity_middleware_authenticates_before_dav_lookup_and_maps_denials() {
    let mut app = app();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:shared\r\nFN:Shared\r\nEND:VCARD\r\n";

    let missing = app.handle(request("GET", "/carddav/contacts/missing.vcf", None, &[]));
    assert_eq!(missing.status, 401);
    assert_eq!(missing.body, b"unauthorized");

    // Alice creates a resource through the same request path after auth and
    // collection ACL checks have succeeded.
    let created = app.handle(request(
        "PUT",
        "/carddav/contacts/shared.vcf",
        Some("alice-token"),
        body,
    ));
    assert_eq!(created.status, 201);

    // Bob has a resource-level read grant, so the existing resource is
    // visible without granting him the whole collection.
    let shared = app.handle(request(
        "GET",
        "/carddav/contacts/shared.vcf",
        Some("bob-token"),
        &[],
    ));
    assert_eq!(shared.status, 200);

    // A denied resource and an absent resource are intentionally equivalent.
    let denied = app.handle(request(
        "GET",
        "/carddav/contacts/private.vcf",
        Some("bob-token"),
        &[],
    ));
    let absent = app.handle(request(
        "GET",
        "/carddav/contacts/does-not-exist.vcf",
        Some("bob-token"),
        &[],
    ));
    assert_eq!(denied.status, 404);
    assert_eq!(denied.status, absent.status);
    assert_eq!(denied.body, absent.body);
    assert!(!String::from_utf8_lossy(&denied.body).contains("private"));

    // Collection discovery is denied before the DAV server can enumerate it.
    let collection = app.handle(request(
        "PROPFIND",
        "/carddav/contacts",
        Some("bob-token"),
        b"<allprop/>",
    ));
    assert_eq!(collection.status, 403);
}

#[test]
fn identity_middleware_enforces_expiry_revocation_and_scope_before_mutation() {
    let mut app = app();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:guarded\r\nFN:Guarded\r\nEND:VCARD\r\n";

    app.set_identity_now(200);
    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/contacts/guarded.vcf",
            Some("alice-token"),
            body,
        ))
        .status,
        401
    );
    app.set_identity_now(150);

    // Bob can read tasks but cannot mutate them or Alice's contact
    // collection. The repository remains untouched by both denied writes.
    let bob_task_write = app.handle(request(
        "PUT",
        "/caldav/tasks/task.ics",
        Some("bob-token"),
        b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n",
    ));
    assert_eq!(bob_task_write.status, 404);
    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/contacts/guarded.vcf",
            Some("bob-token"),
            body,
        ))
        .status,
        404
    );

    let mut revoked = identity();
    assert!(revoked.revoke("alice-key"));
    let mut revoked_app = {
        let mut config = AppConfig::defaults();
        config.space_id = "revoked-space".into();
        App::fake(config).unwrap().with_identity(revoked, 150)
    };
    assert_eq!(
        revoked_app
            .handle(request(
                "GET",
                "/carddav/contacts",
                Some("alice-token"),
                &[]
            ))
            .status,
        401
    );
}

#[test]
fn options_filters_write_methods_for_read_only_collection_scope() {
    let mut app = app();
    let readonly = app.handle(request("OPTIONS", "/caldav/tasks", Some("bob-token"), &[]));
    assert_eq!(readonly.status, 200);
    let allow = readonly
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("allow"))
        .map(|(_, value)| value.as_str())
        .unwrap();
    assert!(allow.contains("GET") && allow.contains("REPORT"));
    assert!(!allow.contains("PUT") && !allow.contains("DELETE"));

    let read_write = app.handle(request(
        "OPTIONS",
        "/carddav/contacts",
        Some("alice-token"),
        &[],
    ));
    assert_eq!(read_write.status, 200);
    let allow = read_write
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("allow"))
        .map(|(_, value)| value.as_str())
        .unwrap();
    assert!(allow.contains("PUT") && allow.contains("DELETE"));
}

#[test]
fn malformed_or_duplicate_identity_headers_fail_closed_without_policy_bypass() {
    let mut app = app();
    let mut duplicate = request("GET", "/carddav/contacts", Some("alice-token"), &[]);
    duplicate
        .headers
        .push(("authorization".into(), "Bearer bob-token".into()));
    assert_eq!(app.handle(duplicate).status, 401);

    let malformed = Request {
        method: "GET".into(),
        path: "/carddav/contacts".into(),
        headers: vec![("Authorization".into(), "Bearer".into())],
        body: vec![],
    };
    assert_eq!(app.handle(malformed).status, 401);
}

#[test]
fn recovered_snapshot_is_attached_to_dav_middleware_without_secret_material() {
    let path = std::env::temp_dir().join(format!(
        "any-cal-identity-middleware-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let snapshot = identity();
    snapshot.save_to(&path).unwrap();
    let mut config = AppConfig::defaults();
    config.space_id = "identity-file-space".into();
    let mut app = App::fake(config)
        .unwrap()
        .with_identity_file(&path, 150)
        .unwrap();

    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/contacts/recovered.vcf",
            Some("alice-token"),
            b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:recovered\r\nFN:Recovered\r\nEND:VCARD\r\n"
        ))
        .status,
        201
    );
    assert_eq!(
        app.handle(request(
            "GET",
            "/carddav/contacts",
            Some("not-the-secret"),
            &[]
        ))
        .status,
        401
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn middleware_rotation_keeps_overlap_then_rejects_old_generation_after_restart() {
    let path = std::env::temp_dir().join(format!(
        "any-cal-identity-rotation-middleware-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut policy = AccessPolicy::new();
    policy.grant_collection(
        principal("alice"),
        CollectionKind::Contacts,
        Operation::Write,
    );
    let mut rotating = IdentityStore::new(policy);
    rotating
        .add_credential(
            CredentialSpec {
                id: "alice-generation-1".into(),
                principal: principal("alice"),
                not_before: 100,
                expires_at: 300,
                capabilities: BTreeSet::new(),
            },
            "synthetic-generation-one",
        )
        .unwrap();
    rotating
        .add_credential(
            CredentialSpec {
                id: "alice-generation-2".into(),
                principal: principal("alice"),
                not_before: 100,
                expires_at: 400,
                capabilities: BTreeSet::new(),
            },
            "synthetic-generation-two",
        )
        .unwrap();
    rotating.save_to(&path).unwrap();

    let mut app = {
        let mut config = AppConfig::defaults();
        config.space_id = "identity-rotation-space".into();
        App::fake(config)
            .unwrap()
            .with_identity_file(&path, 150)
            .unwrap()
    };
    let old_body =
        b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:rotated-old\r\nFN:Rotated Old\r\nEND:VCARD\r\n";
    let new_body =
        b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:rotated-new\r\nFN:Rotated New\r\nEND:VCARD\r\n";
    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/contacts/generation-one.vcf",
            Some("synthetic-generation-one"),
            old_body,
        ))
        .status,
        201
    );
    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/contacts/generation-two.vcf",
            Some("synthetic-generation-two"),
            new_body,
        ))
        .status,
        201
    );

    assert!(rotating.revoke("alice-generation-1"));
    rotating.save_to(&path).unwrap();
    let mut restarted = {
        let mut config = AppConfig::defaults();
        config.space_id = "identity-rotation-space".into();
        App::fake(config)
            .unwrap()
            .with_identity_file(&path, 150)
            .unwrap()
    };
    assert_eq!(
        restarted
            .handle(request(
                "PUT",
                "/carddav/contacts/old-generation.vcf",
                Some("synthetic-generation-one"),
                old_body,
            ))
            .status,
        401
    );
    assert_eq!(
        restarted
            .handle(request(
                "PUT",
                "/carddav/contacts/new-generation.vcf",
                Some("synthetic-generation-two"),
                new_body,
            ))
            .status,
        201
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn auth_audit_events_are_opt_in_bounded_and_secret_safe() {
    let mut config = AppConfig::defaults();
    config.space_id = "audit-space-private-id".into();
    config.emit_auth_events = true;
    let mut app = App::fake(config).unwrap().with_identity(identity(), 150);

    let mut missing = request("GET", "/carddav/contacts/private-resource.vcf", None, &[]);
    missing
        .headers
        .push(("X-Request-Id".into(), "audit-missing".into()));
    assert_eq!(app.handle(missing).status, 401);
    assert_eq!(
        app.handle(request(
            "GET",
            "/carddav/contacts/a.vcf",
            Some("bob-token"),
            &[]
        ))
        .status,
        404
    );
    assert_eq!(
        app.handle(request(
            "PUT",
            "/carddav/contacts/a.vcf",
            Some("alice-token"),
            b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:a\r\nFN:A\r\nEND:VCARD\r\n"
        ))
        .status,
        201
    );

    let events: Vec<_> = app
        .events
        .as_slice()
        .iter()
        .filter(|event| event.kind == "dav.auth")
        .collect();
    assert_eq!(events.len(), 3);
    for event in &events {
        assert!(event.message.len() <= any_cal_observability::MAX_MESSAGE);
        assert!(!event.message.contains("alice-token"));
        assert!(!event.message.contains("bob-token"));
        assert!(!event.message.contains("private-resource"));
        assert!(!event.message.contains("audit-space-private-id"));
        assert!(event.message.contains("outcome="));
        assert!(event.message.contains("scope=contacts"));
    }
    assert_eq!(events[0].request_id.as_deref(), Some("audit-missing"));
}

#[test]
fn auth_audit_events_are_disabled_by_default() {
    let mut app = app();
    assert_eq!(
        app.handle(request("GET", "/carddav/contacts/a.vcf", None, &[]))
            .status,
        401
    );
    assert!(app
        .events
        .as_slice()
        .iter()
        .all(|event| event.kind != "dav.auth"));
}

#[test]
fn auth_audit_emission_is_configuration_controlled() {
    let mut config = AppConfig::defaults();
    assert!(!config.emit_auth_events);
    config
        .apply_env([("ANY_CAL_EMIT_AUTH_EVENTS".into(), "true".into())])
        .unwrap();
    assert!(config.emit_auth_events);
    config
        .set("emit_auth_events", "false")
        .expect("synthetic configuration is valid");
    assert!(!config.emit_auth_events);
}
