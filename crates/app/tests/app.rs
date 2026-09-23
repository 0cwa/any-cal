use any_cal_anytype_adapter::{FakeAnytypeTransport, ObjectRecord, TransportError};
use any_cal_app::{parse_cli, App, AppConfig};
use any_cal_core::{
    AnytypeObjectId, CanonicalDocument, CollectionId, DavKind, DavUid, ResourceEnvelope,
    ResourceId, StructuredDocument,
};
use any_cal_dav_server::Request;
use any_cal_observability::{AuditEventWriter, ReconciliationReport};
use any_cal_sync::CommitFault;
use std::collections::BTreeMap;
use std::fs;

#[test]
fn app_recovery_wrappers_persist_artifacts_and_emit_events() {
    let root = std::env::temp_dir().join(format!("any-cal-app-recovery-{}", std::process::id()));
    let source = root.join("source");
    let backup = root.join("backup");
    let destination = root.join("destination");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, b"checkpoint-v1").unwrap();
    let mut app = App::fake({
        let mut config = AppConfig::defaults();
        config.space_id = "recovery-space".into();
        config
    })
    .unwrap();
    app.backup_artifact(&source, &backup).unwrap();
    fs::write(&source, b"checkpoint-v2").unwrap();
    app.restore_artifact(&backup, &destination).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"checkpoint-v1");
    assert!(app
        .events
        .as_slice()
        .iter()
        .any(|event| event.kind == "recovery.backup"));
    assert!(app
        .events
        .as_slice()
        .iter()
        .any(|event| event.kind == "recovery.restore"));
    assert!(app
        .events
        .as_slice()
        .iter()
        .any(|event| event.correlation_id.as_deref() == Some("recovery-backup")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn authenticated_sync_admin_export_restore_is_bounded_and_atomic() {
    let root = std::env::temp_dir().join(format!("any-cal-app-admin-sync-{}", std::process::id()));
    let checkpoint = root.join("checkpoint.json");
    let export = root.join("checkpoint.export");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let mut config = AppConfig::defaults();
    config.space_id = "admin-sync-space".into();
    config.auth_credential = Some("operator-fixture-token".into());
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());
    let mut app = App::with_transport(config, FakeAnytypeTransport::new(2)).unwrap();

    let unauthorized = app.handle(Request {
        method: "GET".into(),
        path: "/admin/sync/capabilities".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(unauthorized.status, 401);

    let capabilities = app.handle(Request {
        method: "GET".into(),
        path: "/admin/sync/capabilities".into(),
        headers: vec![(
            "Authorization".into(),
            "Bearer operator-fixture-token".into(),
        )],
        body: vec![],
    });
    assert_eq!(capabilities.status, 200);
    assert!(String::from_utf8_lossy(&capabilities.body).contains("any-cal.sync-export"));

    let created = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/admin-export.vcf".into(),
        headers: vec![
            (
                "Authorization".into(),
                "Bearer operator-fixture-token".into(),
            ),
            ("Content-Type".into(), "text/vcard".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:admin-export\r\nFN:First\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(created.status, 201);
    let exported = app.handle(Request {
        method: "POST".into(),
        path: "/admin/sync/export".into(),
        headers: vec![(
            "Authorization".into(),
            "Bearer operator-fixture-token".into(),
        )],
        body: vec![],
    });
    assert_eq!(exported.status, 200);
    let exported_body = String::from_utf8_lossy(&exported.body);
    assert!(exported_body.contains("\"operation\":\"export\""));
    assert!(!exported_body.contains("admin-sync-space"));
    assert!(export.exists());
    let good_export = fs::read(&export).unwrap();

    let second = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/admin-export-2.vcf".into(),
        headers: vec![
            (
                "Authorization".into(),
                "Bearer operator-fixture-token".into(),
            ),
            ("Content-Type".into(), "text/vcard".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:admin-export-2\r\nFN:Second\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(second.status, 201);
    assert_eq!(app.sync_state().unwrap().observed.len(), 2);

    fs::write(
        &export,
        br#"{"format":"any-cal.sync-export","export_version":1}"#,
    )
    .unwrap();
    let rejected = app.handle(Request {
        method: "POST".into(),
        path: "/admin/sync/restore".into(),
        headers: vec![(
            "Authorization".into(),
            "Bearer operator-fixture-token".into(),
        )],
        body: vec![],
    });
    assert_eq!(rejected.status, 500);
    assert_eq!(app.sync_state().unwrap().observed.len(), 2);
    fs::write(&export, good_export).unwrap();

    let restored = app.handle(Request {
        method: "POST".into(),
        path: "/admin/sync/restore".into(),
        headers: vec![(
            "Authorization".into(),
            "Bearer operator-fixture-token".into(),
        )],
        body: vec![],
    });
    assert_eq!(restored.status, 200);
    assert_eq!(app.sync_state().unwrap().observed.len(), 1);
    assert!(app
        .events
        .as_slice()
        .iter()
        .all(|event| !event.message.contains("admin-export")));

    let malformed = app.handle(Request {
        method: "POST".into(),
        path: "/admin/sync/restore".into(),
        headers: vec![(
            "Authorization".into(),
            "Bearer operator-fixture-token".into(),
        )],
        body: b"unexpected".to_vec(),
    });
    assert_eq!(malformed.status, 400);
    assert_eq!(app.sync_state().unwrap().observed.len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::remove_file(&export).unwrap();
        let sentinel = root.join("sentinel");
        fs::write(&sentinel, b"unchanged").unwrap();
        symlink(&sentinel, &export).unwrap();
        let symlink_restore = app.handle(Request {
            method: "POST".into(),
            path: "/admin/sync/restore".into(),
            headers: vec![(
                "Authorization".into(),
                "Bearer operator-fixture-token".into(),
            )],
            body: vec![],
        });
        assert_eq!(symlink_restore.status, 500);
        assert_eq!(app.sync_state().unwrap().observed.len(), 1);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sync_admin_operations_are_persisted_as_bounded_audit_events() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-app-admin-audit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let audit = root.join("audit");
    let checkpoint = root.join("checkpoint.json");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let mut config = AppConfig::defaults();
    config.space_id = "admin-audit-space".into();
    config.auth_credential = Some("operator-fixture-token".into());
    config.audit_directory = Some(audit.to_string_lossy().into_owned());
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());
    let mut app = App::fake(config).unwrap();

    let request = |method: &str, path: &str, body: &[u8], authorized: bool| Request {
        method: method.into(),
        path: path.into(),
        headers: [
            authorized.then(|| {
                (
                    "Authorization".into(),
                    "Bearer operator-fixture-token".into(),
                )
            }),
            Some(("X-Request-Id".into(), "admin-audit-correlation".into())),
        ]
        .into_iter()
        .flatten()
        .collect(),
        body: body.to_vec(),
    };

    assert_eq!(
        app.handle(request("GET", "/admin/sync/capabilities", &[], true))
            .status,
        200
    );
    assert_eq!(
        app.handle(request("POST", "/admin/sync/export", &[], true))
            .status,
        200
    );
    assert_eq!(
        app.handle(request("POST", "/admin/sync/restore", b"unexpected", true))
            .status,
        400
    );
    assert_eq!(
        app.handle(request("GET", "/admin/sync/capabilities", &[], false))
            .status,
        401
    );
    let audit_text = fs::read_to_string(audit.join("active.jsonl")).unwrap();
    assert!(audit_text.contains("\"kind\":\"recovery.admin\""));
    assert!(audit_text.contains("operation=capabilities status=ok"));
    assert!(audit_text.contains("operation=export status=ok"));
    assert!(audit_text.contains("operation=restore status=request_body_not_allowed"));
    assert!(audit_text.contains("operation=denied"));
    assert!(audit_text.contains("admin-audit-correlation"));
    assert!(!audit_text.contains("operator-fixture-token"));
    assert!(!audit_text.contains("checkpoint.json"));
    assert!(!audit_text.contains("/tmp/"));
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_admin_audit_readback_is_authenticated_bounded_and_redacted() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-admin-audit-readback-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut config = AppConfig::defaults();
    config.space_id = "readback-private-space".into();
    config.auth_credential = Some("readback-operator-secret".into());
    config.audit_directory = Some(root.to_string_lossy().into_owned());
    config.audit_required = true;
    let mut app = App::fake(config).unwrap();
    let request = |path: &str, token: Option<&str>, body: &[u8]| Request {
        method: "GET".into(),
        path: path.into(),
        headers: token
            .map(|token| vec![("Authorization".into(), format!("Bearer {token}"))])
            .unwrap_or_default(),
        body: body.to_vec(),
    };

    let denied = app.handle(request("/admin/sync/audit", Some("wrong-secret"), &[]));
    assert_eq!(denied.status, 401);
    assert!(!String::from_utf8_lossy(&denied.body).contains("readback-operator-secret"));

    let capability = app.handle(request(
        "/admin/sync/capabilities",
        Some("readback-operator-secret"),
        &[],
    ));
    assert_eq!(capability.status, 200);
    let response = app.handle(request(
        "/admin/sync/audit?limit=64",
        Some("readback-operator-secret"),
        &[],
    ));
    assert_eq!(response.status, 200);
    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("\"status\":\"ok\""));
    assert!(body.contains("\"operation\":\"capabilities\""));
    assert!(body.contains("\"correlation_digest\":\"sha256:"));
    assert!(!body.contains("readback-private-space"));
    assert!(!body.contains("readback-operator-secret"));
    assert!(!body.contains("message"));

    for path in [
        "/admin/sync/audit?limit=0",
        "/admin/sync/audit?limit=65",
        "/admin/sync/audit?unsupported=1",
        "/admin/sync/audit?limit=1&limit=1",
    ] {
        let malformed = app.handle(request(path, Some("readback-operator-secret"), &[]));
        assert_eq!(malformed.status, 400, "{path}");
        assert!(String::from_utf8_lossy(&malformed.body).len() < 128);
    }
    let with_body = app.handle(request(
        "/admin/sync/audit",
        Some("readback-operator-secret"),
        b"not-allowed",
    ));
    assert_eq!(with_body.status, 400);

    fs::write(root.join("active.jsonl"), b"{not-json}\n").unwrap();
    let unavailable = app.handle(request(
        "/admin/sync/audit",
        Some("readback-operator-secret"),
        &[],
    ));
    assert_eq!(unavailable.status, 503);
    assert!(!String::from_utf8_lossy(&unavailable.body).contains("not-json"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sync_admin_health_contract_is_correlated_and_separates_health_auth() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-app-admin-health-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();

    let mut config = AppConfig::defaults();
    config.space_id = "admin-health-space".into();
    config.auth_credential = Some("operator-token".into());
    config.local_auth_credential = Some("health-token".into());
    config.sync_checkpoint = Some(root.join("checkpoint.json").to_string_lossy().into_owned());
    let mut app = App::fake(config).unwrap();

    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![
            ("Authorization".into(), "Bearer health-token".into()),
            ("X-Request-ID".into(), "health-1".into()),
        ],
        body: vec![],
    });
    assert_eq!(health.status, 200);
    assert!(health
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("x-request-id") && value == "health-1"));
    assert!(String::from_utf8_lossy(&health.body).contains("\"sync_export_ready\":true"));

    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![
            ("Authorization".into(), "Bearer health-token".into()),
            ("X-Request-ID".into(), "ready-1".into()),
        ],
        body: vec![],
    });
    assert_eq!(ready.status, 200);
    assert!(ready
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("x-request-id") && value == "ready-1"));

    let denied_admin = app.handle(Request {
        method: "GET".into(),
        path: "/admin/sync/capabilities".into(),
        headers: vec![
            ("Authorization".into(), "Bearer health-token".into()),
            ("X-Request-ID".into(), "admin-denied".into()),
        ],
        body: vec![],
    });
    assert_eq!(denied_admin.status, 401);
    assert_eq!(denied_admin.body, b"unauthorized");
    assert!(denied_admin
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("cache-control") && value == "no-store"));
    assert!(denied_admin
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("x-request-id") && value == "admin-denied"));

    let malformed_id = app.handle(Request {
        method: "POST".into(),
        path: "/admin/sync/export".into(),
        headers: vec![
            ("Authorization".into(), "Bearer operator-token".into()),
            ("X-Request-ID".into(), "unsafe id\r\n".into()),
        ],
        body: b"unexpected".to_vec(),
    });
    assert_eq!(malformed_id.status, 400);
    assert!(!malformed_id
        .headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("x-request-id")));
    assert!(!String::from_utf8_lossy(&malformed_id.body).contains("checkpoint"));

    let method = app.handle(Request {
        method: "DELETE".into(),
        path: "/admin/sync/capabilities".into(),
        headers: vec![
            ("Authorization".into(), "Bearer operator-token".into()),
            ("X-Request-ID".into(), "method-1".into()),
        ],
        body: vec![],
    });
    assert_eq!(method.status, 405);
    assert!(method
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("allow") && value == "GET, POST"));
    assert!(method
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("x-request-id") && value == "method-1"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn observability_hooks_correlate_requests_and_reconciliation() {
    let mut config = AppConfig::defaults();
    config.space_id = "observability-space".into();
    let mut app = App::fake(config).unwrap();
    let response = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![
            ("X-Request-ID".into(), "req-test".into()),
            ("X-Sync-ID".into(), "sync-test".into()),
        ],
        body: vec![],
    });
    assert_eq!(response.status, 200);
    app.record_reconciliation(ReconciliationReport::new("sync-test"));
    assert!(app
        .events
        .as_slice()
        .iter()
        .any(|event| event.request_id.as_deref() == Some("req-test")));
    assert!(app
        .events
        .as_slice()
        .iter()
        .any(|event| event.sync_id.as_deref() == Some("sync-test")));
    let status = app.handle(Request {
        method: "GET".into(),
        path: "/status".into(),
        headers: vec![],
        body: vec![],
    });
    let status = String::from_utf8(status.body).unwrap();
    assert!(
        status.contains("\"last_error\":null")
            && status.contains("\"recovery\":\"sync-checkpoint\"")
    );
}

#[test]
fn protocol_error_events_preserve_conflict_category() {
    let mut config = AppConfig::defaults();
    config.space_id = "category-space".into();
    let mut app = App::fake(config).unwrap();
    let response = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/category.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:one\r\nFN:One\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(response.status, 201);
    let response = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/category.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:two\r\nFN:Two\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(response.status, 409);
    assert!(app
        .events
        .as_slice()
        .iter()
        .any(|event| { event.category == Some(any_cal_observability::ErrorCategory::Conflict) }));
}

#[test]
fn stale_contact_write_returns_typed_conflict_receipt_without_body_leakage() {
    let mut config = AppConfig::defaults();
    config.space_id = "conflict-receipt-space".into();
    let mut app = App::fake(config).unwrap();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:receipt-contact\r\nFN:First\r\nEND:VCARD\r\n";
    let created = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/receipt.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: body.to_vec(),
    });
    assert_eq!(created.status, 201);
    let stale = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/receipt.vcf".into(),
        headers: vec![
            ("If-Match".into(), "\"stale-etag\"".into()),
            ("Content-Type".into(), "text/vcard".into()),
            ("X-Request-ID".into(), "synthetic-conflict-request".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:receipt-contact\r\nFN:Second\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(stale.status, 412);
    let event = app
        .events
        .as_slice()
        .iter()
        .rev()
        .find(|event| event.kind == "dav.request")
        .unwrap();
    assert_eq!(
        event.category,
        Some(any_cal_observability::ErrorCategory::Conflict)
    );
    assert_eq!(
        event.request_id.as_deref(),
        Some("synthetic-conflict-request")
    );
    assert_eq!(event.message, "status=412");
    assert!(!event.message.contains("receipt-contact"));
    assert_eq!(
        app.handle(Request {
            method: "GET".into(),
            path: "/carddav/contacts/receipt.vcf".into(),
            headers: vec![],
            body: vec![],
        })
        .status,
        200
    );
}

struct NoopHttp;
impl any_cal_anytype_adapter::HttpExchange for NoopHttp {
    fn exchange(&mut self, _request: &[u8]) -> Result<Vec<u8>, TransportError> {
        panic!("HTTP exchange should not run during health/check construction")
    }
}

struct UnavailableHttp;
impl any_cal_anytype_adapter::HttpExchange for UnavailableHttp {
    fn exchange(&mut self, _request: &[u8]) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::Unavailable)
    }
}

fn seeded_transport() -> FakeAnytypeTransport {
    let mut transport = FakeAnytypeTransport::new(2);
    for (id, collection, kind) in [
        ("seed-contact", "contacts", DavKind::Contact),
        ("seed-task", "tasks", DavKind::Task),
    ] {
        let envelope = ResourceEnvelope {
            collection_id: CollectionId::try_from(collection).unwrap(),
            resource_id: ResourceId::try_from(id).unwrap(),
            kind,
            anytype_object_id: AnytypeObjectId::try_from(id).unwrap(),
            dav_uid: DavUid::try_from(id).unwrap(),
            document: CanonicalDocument::new(StructuredDocument::default()),
            revision: 0,
        };
        transport.objects.insert(
            id.into(),
            ObjectRecord {
                id: id.into(),
                space_id: "space".into(),
                properties: vec![],
                property_formats: BTreeMap::new(),
                body: envelope.canonical_json().unwrap(),
                archived: false,
                revision: 0,
            },
        );
    }
    transport
}

#[test]
fn durable_checkpoint_tracks_dav_write_and_reopens() {
    let checkpoint =
        std::env::temp_dir().join(format!("any-cal-app-sync-{}.json", std::process::id()));
    let _ = fs::remove_file(&checkpoint);
    let _ = fs::remove_file(checkpoint.with_extension("bak"));
    let mut config = AppConfig::defaults();
    config.space_id = "space".into();
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());
    let mut app = App::with_transport(config.clone(), FakeAnytypeTransport::new(2)).unwrap();
    let response = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/checkpoint.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:checkpoint\r\nFN:Checkpoint\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert!(app.sync_state().is_some_and(|state| state
        .observed
        .values()
        .any(|item| item.dav_uid == "checkpoint")));
    let remote = app.transport_snapshot();
    drop(app);
    let mut reopened = App::with_transport(config, remote).unwrap();
    assert!(reopened.sync_state().is_some_and(|state| state
        .observed
        .values()
        .any(|item| item.dav_uid == "checkpoint")));
    let deleted = reopened.handle(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/checkpoint.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(deleted.status, 204);
    assert!(reopened.sync_state().is_some_and(|state| state
        .tombstones
        .values()
        .any(|item| item.dav_uid == "checkpoint")));
    let _ = fs::remove_file(&checkpoint);
    let _ = fs::remove_file(checkpoint.with_extension("bak"));
}

#[test]
fn checkpoint_failure_does_not_publish_in_memory_state() {
    let checkpoint = std::env::temp_dir().join(format!(
        "any-cal-app-sync-fault-{}.json",
        std::process::id()
    ));
    let _ = fs::remove_file(&checkpoint);
    let mut config = AppConfig::defaults();
    config.space_id = "space".into();
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());
    let mut app = App::with_transport(config, FakeAnytypeTransport::new(2)).unwrap();
    app.inject_checkpoint_fault(CommitFault::BeforeWrite);
    let response = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/fault.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:fault\r\nFN:Fault\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(
        response.status,
        500,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert!(app
        .sync_state()
        .is_some_and(|state| !state.observed.values().any(|item| item.dav_uid == "fault")));
    let _ = fs::remove_file(&checkpoint);
}

#[test]
fn http_mode_uses_scripted_transport_for_safe_health_and_check() {
    let mut config = AppConfig::defaults();
    config.space_id = "space".into();
    config.token = Some("super-secret".into());
    config.transport_mode = "http".into();
    let mut app = any_cal_app::AppGeneric::<any_cal_anytype_adapter::HttpAnytypeTransport>::
        http_with_exchange(config, Box::new(NoopHttp))
        .unwrap();
    assert_eq!(app.config.transport_mode, "http");
    let check = app.check();
    assert!(check.contains("endpoint=http://"));
    assert!(check.contains("token_configured=true"));
    assert!(!check.contains("super-secret"));
    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let health = String::from_utf8(health.body).unwrap();
    assert!(health.contains("\"transport\":\"http\""));
    assert!(health.contains("\"upstream\":{\"mode\":\"http\",\"configured\":true,\"tested\":false,\"ready\":false,\"status\":\"not_tested\""));
    assert!(!health.contains("super-secret"));
}

#[test]
fn http_readiness_requires_a_successful_upstream_probe() {
    let mut config = AppConfig::defaults();
    config.space_id = "space".into();
    let mut app = any_cal_app::AppGeneric::<any_cal_anytype_adapter::HttpAnytypeTransport>::
        http_with_exchange(config, Box::new(UnavailableHttp))
        .unwrap();
    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 503);
    assert_eq!(
        ready.body,
        b"{\"status\":\"unavailable\",\"ready\":false,\"reason\":\"upstream\"}"
    );
    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let body = String::from_utf8(health.body).unwrap();
    assert!(body.contains("\"status\":\"unavailable\""));
    assert!(body.contains(
        "\"tested\":true,\"ready\":false,\"status\":\"unavailable\",\"error\":\"unavailable\""
    ));
}

#[test]
fn http_mode_rejects_missing_endpoint_and_accepts_https() {
    let mut missing = AppConfig::defaults();
    missing.space_id = "space".into();
    missing.endpoint.clear();
    assert!(
        any_cal_app::AppGeneric::<any_cal_anytype_adapter::HttpAnytypeTransport>::http(missing)
            .is_err()
    );
    let mut https = AppConfig::defaults();
    https.space_id = "space".into();
    https.endpoint = "https://example.test".into();
    assert!(
        any_cal_app::AppGeneric::<any_cal_anytype_adapter::HttpAnytypeTransport>::http(https)
            .is_ok()
    );
}

#[test]
fn lan_policy_authenticates_and_rate_limits_without_secret_leakage() {
    let mut config = AppConfig::defaults();
    config.space_id = "lan-space".into();
    config.listen_address = "127.0.0.1:8080".into();
    assert!(App::fake(config.clone()).is_ok());
    config.reverse_proxy_tls = true;
    config.auth_credential = Some("credential-secret".into());
    config.rate_limit_per_minute = 2;
    let mut app = App::fake(config).unwrap();
    let unauthorized = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(unauthorized.status, 401);
    assert_eq!(
        unauthorized.headers,
        vec![
            ("Content-Type".into(), "text/plain; charset=utf-8".into()),
            (
                "WWW-Authenticate".into(),
                "Bearer realm=any-cal, Basic realm=any-cal".into(),
            ),
        ]
    );
    let wrong = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![("Authorization".into(), "Bearer wrong".into())],
        body: vec![],
    });
    assert_eq!(wrong.status, 401);
    let duplicate = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![
            ("Authorization".into(), "Bearer credential-secret".into()),
            ("authorization".into(), "Basic credential-secret".into()),
        ],
        body: vec![],
    });
    assert_eq!(duplicate.status, 401);
    let headers = vec![("Authorization".into(), "Bearer credential-secret".into())];
    let ok = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: headers.clone(),
        body: vec![],
    });
    assert_eq!(ok.status, 200);
    let basic = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![("Authorization".into(), "Basic credential-secret".into())],
        body: vec![],
    });
    assert_eq!(basic.status, 200);
    let limited = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers,
        body: vec![],
    });
    assert_eq!(limited.status, 429);
    assert!(!String::from_utf8_lossy(&limited.body).contains("credential-secret"));
}

#[test]
fn unauthenticated_dav_requests_hide_resource_existence_and_do_not_consume_rate_budget() {
    let mut config = AppConfig::defaults();
    config.space_id = "auth-boundary-space".into();
    config.auth_credential = Some("auth-boundary-secret".into());
    config.rate_limit_per_minute = 1;
    let mut app = App::fake(config).unwrap();

    let request = |path: &str, authorization: Option<&str>| Request {
        method: "GET".into(),
        path: path.into(),
        headers: authorization
            .map(|value| vec![("Authorization".into(), value.into())])
            .unwrap_or_default(),
        body: vec![],
    };
    let missing = app.handle(request("/carddav/does-not-exist", None));
    let known = app.handle(request("/carddav/contacts", Some("Bearer wrong")));
    assert_eq!(missing.status, 401);
    assert_eq!(known.status, 401);
    assert_eq!(missing.headers, known.headers);
    assert!(!String::from_utf8_lossy(&missing.body).contains("does-not-exist"));

    // Failed authentication does not consume the single authenticated slot.
    let authorized = app.handle(request(
        "/carddav/contacts",
        Some("Bearer auth-boundary-secret"),
    ));
    assert_eq!(authorized.status, 405);
}

#[test]
fn local_health_credential_is_an_alternate_runtime_only_secret() {
    let mut config = AppConfig::defaults();
    config.space_id = "health-space".into();
    config.reverse_proxy_tls = true;
    config.auth_credential = Some("dav-secret".into());
    config.local_auth_credential = Some("gui-health-secret".into());
    let mut app = App::fake(config).unwrap();

    let local = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![("Authorization".into(), "Bearer gui-health-secret".into())],
        body: vec![],
    });
    assert_eq!(local.status, 200);

    let dav = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts".into(),
        headers: vec![("Authorization".into(), "Bearer gui-health-secret".into())],
        body: vec![],
    });
    assert_eq!(dav.status, 401);

    let (_, config) = parse_cli(
        vec![
            "any-cal".into(),
            "check".into(),
            "--space-id".into(),
            "health-space".into(),
        ],
        AppConfig::defaults(),
    )
    .unwrap();
    let mut config = config;
    config
        .apply_env([(
            String::from("ANY_CAL_LOCAL_AUTH"),
            String::from("runtime-only"),
        )])
        .unwrap();
    assert_eq!(
        config.local_auth_credential.as_deref(),
        Some("runtime-only")
    );
}

#[test]
fn local_health_credential_alone_fails_closed_and_cannot_authorize_dav() {
    let mut config = AppConfig::defaults();
    config.space_id = "health-only-space".into();
    config.local_auth_credential = Some("health-only-secret".into());
    let mut app = App::fake(config).unwrap();

    for headers in [
        vec![],
        vec![("Authorization".into(), "Bearer wrong".into())],
    ] {
        let response = app.handle(Request {
            method: "GET".into(),
            path: "/health".into(),
            headers,
            body: vec![],
        });
        assert_eq!(response.status, 401);
    }

    let correct = app.handle(Request {
        method: "GET".into(),
        path: "/status".into(),
        headers: vec![("Authorization".into(), "Bearer health-only-secret".into())],
        body: vec![],
    });
    assert_eq!(correct.status, 200);

    let dav = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts".into(),
        headers: vec![("Authorization".into(), "Bearer health-only-secret".into())],
        body: vec![],
    });
    assert_eq!(dav.status, 405);
}

#[test]
fn reverse_proxy_tls_rejects_plaintext_non_loopback_binding() {
    let mut config = AppConfig::defaults();
    config.space_id = "lan-space".into();
    config.listen_address = "0.0.0.0:8080".into();
    config.allow_lan = true;
    config.reverse_proxy_tls = true;
    config.auth_credential = Some("credential-secret".into());
    let error = match App::fake(config) {
        Ok(_) => panic!("non-loopback proxy backend must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        any_cal_app::ConfigError::Invalid(message)
            if message.contains("requires a loopback listen_address")
    ));
}

#[test]
fn lan_scope_does_not_follow_request_controlled_space() {
    let mut config = AppConfig::defaults();
    config.space_id = "configured-space".into();
    config.auth_credential = Some("scope-secret".into());
    let mut app = App::fake(config).unwrap();
    let response = app.handle(Request {
        method: "GET".into(),
        path: "/spaces/other-space/carddav/contacts/person.vcf".into(),
        headers: vec![("Authorization".into(), "bEaReR   scope-secret".into())],
        body: vec![],
    });
    assert_eq!(response.status, 404);
}

#[test]
fn lan_policy_supports_ipv6_and_cli_security_overrides() {
    let mut config = AppConfig::defaults();
    config.space_id = "ipv6-space".into();
    config.listen_address = "[::1]:8080".into();
    assert!(App::fake(config).is_ok());

    let (_, config) = parse_cli(
        vec![
            "any-cal".into(),
            "check".into(),
            "--space-id".into(),
            "lan-space".into(),
            "--listen".into(),
            "0.0.0.0:8080".into(),
            "--allow-lan".into(),
            "--reverse-proxy-tls".into(),
            "--rate-limit-per-minute".into(),
            "7".into(),
            "--max-connections".into(),
            "3".into(),
        ],
        AppConfig::defaults(),
    )
    .unwrap();
    assert!(config.allow_lan && config.reverse_proxy_tls);
    assert_eq!(config.rate_limit_per_minute, 7);
    assert_eq!(config.max_connections, 3);
    let mut config = config;
    config.auth_credential = Some("env-or-file-secret".into());
    assert!(App::fake(config.clone()).is_err());
    config.listen_address = "127.0.0.1:8080".into();
    assert!(App::fake(config).is_ok());
}

#[test]
fn config_precedence_and_secret_safe_check() {
    let mut c = AppConfig::defaults();
    c.apply_env([(String::from("ANY_CAL_SPACE_ID"), String::from("env-space"))])
        .unwrap();
    let (_, c) = parse_cli(
        vec![
            "any-cal".into(),
            "check".into(),
            "--space-id".into(),
            "cli-space".into(),
            "--token".into(),
            "secret".into(),
        ],
        c,
    )
    .unwrap();
    let app = App::fake(c).unwrap();
    let output = app.check();
    assert!(output.contains("space_configured=true") && output.contains("token_configured=true"));
    assert!(!output.contains("secret"));
}

#[test]
fn file_config_is_loaded_and_cli_can_override_it() {
    let path = std::env::temp_dir().join(format!("any-cal-app-{}.conf", std::process::id()));
    std::fs::write(
        &path,
        "space_id=file-space\nlisten_address=127.0.0.1:9999\n",
    )
    .unwrap();
    let c = AppConfig::from_file(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let (_, c) = parse_cli(
        vec![
            "any-cal".into(),
            "check".into(),
            "--space-id".into(),
            "cli-space".into(),
        ],
        c,
    )
    .unwrap();
    assert_eq!(c.space_id, "cli-space");
    assert_eq!(c.listen_address, "127.0.0.1:9999");
}

#[test]
fn config_precedence_is_defaults_then_file_then_env_then_cli() {
    let path = std::env::temp_dir().join(format!(
        "any-cal-precedence-{}-{}.conf",
        std::process::id(),
        "synthetic"
    ));
    std::fs::write(
        &path,
        "space_id=file-space\nemit_auth_events=true\nlisten_address=127.0.0.1:9998\n",
    )
    .unwrap();
    let mut config = AppConfig::from_file(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(config.emit_auth_events);

    config
        .apply_env([
            ("ANY_CAL_SPACE_ID".into(), "environment-space".into()),
            ("ANY_CAL_EMIT_AUTH_EVENTS".into(), "false".into()),
        ])
        .unwrap();
    assert_eq!(config.space_id, "environment-space");
    assert!(!config.emit_auth_events);

    let (_, config) = parse_cli(
        [
            "any-cal",
            "check",
            "--space-id",
            "cli-space",
            "--emit-auth-events",
        ]
        .into_iter()
        .map(String::from),
        config,
    )
    .unwrap();
    assert_eq!(config.space_id, "cli-space");
    assert!(config.emit_auth_events);
    assert_eq!(config.listen_address, "127.0.0.1:9998");

    let (_, config) = parse_cli(
        ["any-cal", "check", "--no-emit-auth-events"]
            .into_iter()
            .map(String::from),
        config,
    )
    .unwrap();
    assert!(!config.emit_auth_events);
}

#[test]
fn invalid_configuration_diagnostics_are_bounded_and_secret_safe() {
    let sentinel = "config-diagnostic-secret-4f91";
    let path = std::env::temp_dir().join(format!(
        "any-cal-invalid-{}-{}.conf",
        std::process::id(),
        sentinel
    ));
    std::fs::write(&path, format!("unknown-{sentinel}=true\n")).unwrap();
    let error = AppConfig::from_file(&path).unwrap_err();
    std::fs::remove_file(&path).unwrap();
    let diagnostic = format!("{error:?}");
    assert_eq!(diagnostic, "Invalid(\"unknown configuration key\")");
    assert!(!diagnostic.contains(sentinel));

    let error = AppConfig::from_file(std::path::Path::new(&format!(
        "/tmp/missing-{sentinel}.conf"
    )))
    .unwrap_err();
    let diagnostic = format!("{error:?}");
    assert_eq!(diagnostic, "Io(\"configuration file could not be read\")");
    assert!(!diagnostic.contains(sentinel));

    let error = parse_cli(
        ["any-cal", "--unknown-option", sentinel]
            .into_iter()
            .map(String::from),
        AppConfig::defaults(),
    )
    .unwrap_err();
    let diagnostic = format!("{error:?}");
    assert_eq!(diagnostic, "Invalid(\"unknown command-line option\")");
    assert!(!diagnostic.contains(sentinel));

    let error = AppConfig::defaults()
        .apply_env([("ANY_CAL_EMIT_AUTH_EVENTS".into(), sentinel.into())])
        .unwrap_err();
    let diagnostic = format!("{error:?}");
    assert_eq!(diagnostic, "Invalid(\"expected boolean\")");
    assert!(!diagnostic.contains(sentinel));
}

#[test]
fn malformed_file_diagnostic_does_not_echo_secret_material() {
    let path = std::env::temp_dir().join(format!(
        "any-cal-malformed-secret-config-{}.conf",
        std::process::id()
    ));
    let secret = "fixture-file-secret-7e8f";
    std::fs::write(&path, format!("token={secret}\nmalformed-{secret}\n")).unwrap();
    let error = any_cal_app::AppConfig::from_file(&path).unwrap_err();
    std::fs::remove_file(&path).unwrap();
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains(secret));
    assert!(diagnostic.contains("invalid configuration line"));
}

#[test]
fn check_redacts_endpoint_userinfo_and_query_material() {
    let mut config = AppConfig::defaults();
    config.space_id = "diagnostic-space".into();
    config.endpoint =
        "https://user:fixture-uri-secret@example.test/api?token=fixture-query-secret#fragment"
            .into();
    let app = App::fake(config).unwrap();
    let output = app.check();
    assert!(output.contains("endpoint=https://example.test/api"));
    assert!(!output.contains("fixture-uri-secret"));
    assert!(!output.contains("fixture-query-secret"));
    assert!(!output.contains("fragment"));
}

#[test]
fn configured_empty_credentials_fail_closed() {
    let mut config = AppConfig::defaults();
    config.space_id = "empty-credential-space".into();
    let configure_cases: [fn(&mut AppConfig); 3] = [
        |config: &mut AppConfig| config.token = Some(String::new()),
        |config: &mut AppConfig| config.auth_credential = Some(String::new()),
        |config: &mut AppConfig| config.local_auth_credential = Some(String::new()),
    ];
    for configure in configure_cases {
        let mut candidate = config.clone();
        configure(&mut candidate);
        assert!(matches!(
            App::fake(candidate),
            Err(any_cal_app::ConfigError::Invalid(message))
                if message.contains("must not be empty")
        ));
    }
}

#[test]
fn fake_app_constructs_actual_dav_wiring_and_validates_required_space() {
    assert_eq!(AppConfig::defaults().transport_mode, "http");
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    let app = App::fake(c).unwrap();
    assert!(app.check().starts_with("ok endpoint="));
    assert!(app.check().contains("transport=fake upstream_tested=true"));
    assert!(App::fake(AppConfig::defaults()).is_err());
}

#[test]
fn health_is_structured_and_secret_safe() {
    let mut c = AppConfig::defaults();
    c.space_id = "space\"quoted".into();
    c.token = Some("do-not-leak".into());
    let mut app = App::fake(c).unwrap();
    let response = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let body = String::from_utf8(response.body).unwrap();
    assert_eq!(response.status, 200);
    assert!(body.contains("\"transport\":\"fake\""));
    assert!(body.contains(
        "\"upstream\":{\"mode\":\"fake\",\"configured\":true,\"tested\":true,\"ready\":true"
    ));
    assert!(body.contains("\"space_configured\":true"));
    assert!(!body.contains("do-not-leak"));
}

#[test]
fn health_responses_are_uncacheable_typed_and_correlated_without_echoing_input() {
    let mut config = AppConfig::defaults();
    config.space_id = "health-contract-space".into();
    let mut app = App::fake(config).unwrap();
    let request_id = "request-secret-".to_string() + &"x".repeat(512);
    let response = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![("X-Request-ID".into(), request_id.clone())],
        body: vec![],
    });
    assert_eq!(response.status, 200);
    assert_eq!(
        response
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str()),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        response
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("cache-control"))
            .map(|(_, value)| value.as_str()),
        Some("no-store, no-cache, max-age=0")
    );
    assert!(response
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("pragma") && value == "no-cache"));
    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("\"status\":\"healthy\",\"ready\":true"));
    assert!(!body.contains(&request_id));
    let event = app.events.as_slice().last().expect("health event");
    assert!(event
        .request_id
        .as_ref()
        .is_some_and(|id| id.chars().count() <= 129));
}

#[test]
fn health_state_is_truthful_when_required_audit_is_unavailable() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-readiness-contract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = AppConfig::defaults();
    config.space_id = "readiness-contract-space".into();
    config.audit_directory = Some(root.to_string_lossy().into_owned());
    config.audit_required = true;
    let mut app = App::fake(config).unwrap();
    fs::write(root.join("active.jsonl"), b"{not-json}\n").unwrap();
    let _ = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/missing.vcf".into(),
        headers: vec![],
        body: vec![],
    });

    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(health.status, 200);
    let body = String::from_utf8(health.body).unwrap();
    assert!(body.contains("\"status\":\"unavailable\",\"ready\":false"));
    assert!(health.headers.iter().any(
        |(key, value)| key.eq_ignore_ascii_case("cache-control") && value.contains("no-store")
    ));

    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 503);
    assert_eq!(ready.body, b"{\"status\":\"unavailable\",\"ready\":false}");
    assert!(ready
        .headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("content-type")
            && value == "application/json; charset=utf-8"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn health_paths_reject_unsupported_methods_truthfully() {
    let mut config = AppConfig::defaults();
    config.space_id = "method-contract-space".into();
    let mut app = App::fake(config).unwrap();
    for path in ["/health", "/ready", "/status"] {
        let response = app.handle(Request {
            method: "POST".into(),
            path: path.into(),
            headers: vec![],
            body: vec![],
        });
        assert_eq!(response.status, 405, "{path}");
        assert!(response.body.len() < 256);
        assert!(response
            .headers
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("allow") && value == "GET"));
    }
}

#[test]
fn audit_health_visibility_and_readiness_are_bounded() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-audit-health-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = AppConfig::defaults();
    config.space_id = "audit-health-space".into();
    config.audit_directory = Some(root.to_string_lossy().into_owned());
    config.audit_required = true;
    config.expose_audit_health = true;
    let mut app = App::fake(config).unwrap();

    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let health = String::from_utf8(health.body).unwrap();
    assert!(health.contains("\"audit\":{\"enabled\":true"));
    assert!(health.contains("\"state\":\"healthy\""));
    assert!(health.contains("\"checkpoint_ready\":true"));
    assert!(!health.contains("audit-health-space"));
    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 200);
    let mut hidden_config = AppConfig::defaults();
    hidden_config.space_id = "audit-hidden-space".into();
    hidden_config.audit_directory =
        Some(root.with_extension("hidden").to_string_lossy().into_owned());
    let mut hidden = App::fake(hidden_config).unwrap();
    let hidden_health = hidden.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    assert!(!String::from_utf8(hidden_health.body)
        .unwrap()
        .contains("\"audit\":"));
    let _ = fs::remove_dir_all(root.with_extension("hidden"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn required_audit_failure_fails_closed_and_reports_recovery() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-audit-failure-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = AppConfig::defaults();
    config.space_id = "audit-failure-space".into();
    config.audit_directory = Some(root.to_string_lossy().into_owned());
    config.audit_required = true;
    config.expose_audit_health = true;
    let mut app = App::fake(config).unwrap();
    fs::write(root.join("active.jsonl"), b"{not-json}\n").unwrap();

    let failed = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/missing.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(failed.status, 503);
    assert_eq!(failed.body, b"audit unavailable");
    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let health = String::from_utf8(health.body).unwrap();
    assert!(health.contains("\"state\":\"unavailable\""));
    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 503);

    fs::remove_file(root.join("active.jsonl")).unwrap();
    let _ = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/missing.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let health = String::from_utf8(health.body).unwrap();
    assert!(health.contains("\"state\":\"recovering\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn required_audit_configuration_fails_before_startup_and_optional_mode_is_ready() {
    let mut required = AppConfig::defaults();
    required.space_id = "required-audit-config-space".into();
    required.audit_required = true;
    let error = match App::fake(required) {
        Ok(_) => panic!("required audit needs an explicit directory"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        any_cal_app::ConfigError::Invalid(message)
            if message == "audit_required requires audit_directory"
    ));

    let mut optional = AppConfig::defaults();
    optional.space_id = "optional-audit-config-space".into();
    let mut app = App::fake(optional).unwrap();
    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 200);
    assert_eq!(ready.body, b"{\"status\":\"healthy\",\"ready\":true,\"space_configured\":true,\"contacts_collection_configured\":true,\"tasks_collection_configured\":true,\"transport\":\"fake\",\"upstream\":{\"mode\":\"fake\",\"configured\":true,\"tested\":true,\"ready\":true,\"status\":\"ready\",\"error\":null},\"cache\":\"rebuildable\",\"events\":0,\"failures\":0,\"last_error\":null,\"recovery\":\"sync-checkpoint\" }".to_vec());
}

#[test]
fn stale_audit_lock_is_refused_then_exact_path_recovery_allows_restart() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-audit-stale-lock-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(".writer.lock"), b"pid=4294967294\n").unwrap();

    let mut locked = AppConfig::defaults();
    locked.space_id = "stale-lock-space".into();
    locked.audit_required = true;
    locked.expose_audit_health = true;
    locked.audit_directory = Some(root.to_string_lossy().into_owned());
    let error = match App::fake(locked.clone()) {
        Ok(_) => panic!("stale lock must not be silently removed"),
        Err(error) => error,
    };
    assert!(matches!(error, any_cal_app::ConfigError::Io(message) if message.contains("locked")));

    AuditEventWriter::recover_stale_lock(&root).unwrap();
    let mut recovered = App::fake(locked).unwrap();
    let ready = recovered.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 200);
    drop(recovered);
    assert!(!root.join(".writer.lock").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restart_preserves_readiness_and_checkpoint_before_audit_export() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-audit-restart-order-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let audit = root.join("audit");
    let checkpoint = root.join("sync.json");
    fs::create_dir_all(&root).unwrap();
    let mut config = AppConfig::defaults();
    config.space_id = "restart-order-space".into();
    config.audit_required = true;
    config.expose_audit_health = true;
    config.audit_directory = Some(audit.to_string_lossy().into_owned());
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());

    let mut app = App::fake(config.clone()).unwrap();
    let written = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/order.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:order\r\nFN:Order\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(written.status, 201);
    assert!(checkpoint.exists());
    let exported = fs::read_to_string(audit.join("active.jsonl")).unwrap();
    assert!(exported.contains("\"kind\":\"dav.request\""));
    assert!(!exported.contains("restart-order-space"));
    drop(app);
    assert!(!audit.join(".writer.lock").exists());

    let mut reopened = App::fake(config).unwrap();
    let ready = reopened.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 200);
    let health = reopened.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let health = String::from_utf8(health.body).unwrap();
    assert!(health.contains("\"state\":\"healthy\""));
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn optional_audit_degradation_keeps_service_ready_and_recovers() {
    let root = std::env::temp_dir().join(format!(
        "any-cal-audit-optional-recovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = AppConfig::defaults();
    config.space_id = "optional-recovery-space".into();
    config.audit_directory = Some(root.to_string_lossy().into_owned());
    config.expose_audit_health = true;
    let mut app = App::fake(config).unwrap();
    fs::write(root.join("active.jsonl"), b"{not-json}\n").unwrap();

    let degraded = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/missing.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(degraded.status, 404);
    let health = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    assert!(String::from_utf8(health.body)
        .unwrap()
        .contains("\"state\":\"unavailable\""));
    let ready = app.handle(Request {
        method: "GET".into(),
        path: "/ready".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(ready.status, 200);

    fs::remove_file(root.join("active.jsonl")).unwrap();
    let _ = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/missing.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    let recovered = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let recovered = String::from_utf8(recovered.body).unwrap();
    assert!(recovered.contains("\"state\":\"recovering\""));
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn diagnostics_do_not_echo_anytype_identifier_or_bearer_sentinels() {
    let mut c = AppConfig::defaults();
    c.space_id = "anytype-object-raw-sentinel-7f3d".into();
    c.contacts_collection = "contacts-raw-sentinel-2a91".into();
    c.tasks_collection = "tasks-raw-sentinel-9c44".into();
    c.token = Some("Bearer raw-bearer-sentinel-4e82".into());
    let mut app = App::fake(c).unwrap();

    let check = app.check();
    assert!(check.contains("space_configured=true"));
    assert!(!check.contains("raw-sentinel"));
    assert!(!check.contains("raw-bearer-sentinel"));

    let response = app.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    let health = String::from_utf8(response.body).unwrap();
    assert_eq!(response.status, 200);
    assert!(health.contains("\"space_configured\":true"));
    assert!(health.contains("\"contacts_collection_configured\":true"));
    assert!(health.contains("\"tasks_collection_configured\":true"));
    assert!(!health.contains("raw-sentinel"));
    assert!(!health.contains("raw-bearer-sentinel"));
}

#[test]
fn invalid_endpoint_api_and_listen_values_fail_before_startup() {
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    c.endpoint = "file:///secret".into();
    assert!(App::fake(c).is_err());
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    c.api_version = "old".into();
    assert!(App::fake(c).is_err());
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    c.listen_address = "not-an-address".into();
    assert!(App::fake(c).is_err());
}

#[test]
fn restart_reopens_from_remote_state_with_stable_etag_and_archive() {
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    let mut first = App::with_transport(c.clone(), seeded_transport()).unwrap();
    let put = first.handle_primary_dav(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/reopen.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\nUID:reopen\nFN:Reopen\nEND:VCARD\n".to_vec(),
    });
    assert_eq!(put.status, 201);
    let etag = put
        .headers
        .iter()
        .find(|(key, _)| key == "ETag")
        .unwrap()
        .1
        .clone();
    let remote = first.transport_snapshot();
    let mut second = App::with_transport(c, remote).unwrap();
    let get = second.handle_primary_dav(Request {
        method: "GET".into(),
        path: "/carddav/contacts/reopen.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(get.status, 200);
    assert_eq!(
        get.headers.iter().find(|(key, _)| key == "ETag").unwrap().1,
        etag
    );
    let report = second.handle_primary_dav(Request {
        method: "REPORT".into(),
        path: "/carddav/contacts".into(),
        headers: vec![],
        body: b"<addressbook-query><prop><getetag/></prop></addressbook-query>".to_vec(),
    });
    assert_eq!(report.status, 207);
    assert!(String::from_utf8(report.body)
        .unwrap()
        .contains("reopen.vcf"));
    let archived = second.handle_primary_dav(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/reopen.vcf".into(),
        headers: vec![("If-Match".into(), etag)],
        body: vec![],
    });
    assert_eq!(archived.status, 204);
    let mut reopened =
        App::with_transport(second.config.clone(), second.transport_snapshot()).unwrap();
    assert_eq!(
        reopened
            .handle_primary_dav(Request {
                method: "GET".into(),
                path: "/carddav/contacts/reopen.vcf".into(),
                headers: vec![],
                body: vec![],
            })
            .status,
        404
    );
    let health = second.handle(Request {
        method: "GET".into(),
        path: "/health".into(),
        headers: vec![],
        body: vec![],
    });
    assert!(String::from_utf8(health.body)
        .unwrap()
        .contains("rebuildable"));
}

#[test]
fn malformed_write_and_transport_failure_do_not_mutate_remote_state() {
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    let mut app = App::with_transport(c, seeded_transport()).unwrap();
    let malformed = app.handle_primary_dav(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/bad.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"not-vcard".to_vec(),
    });
    assert_eq!(malformed.status, 400);
    assert!(!app.transport_snapshot().objects.contains_key("bad"));
    app.with_transport_mut(|transport| {
        transport.inject(any_cal_anytype_adapter::TransportError::Timeout)
    });
    let failed = app.handle_primary_dav(Request {
        method: "GET".into(),
        path: "/carddav/contacts/seed-contact.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    // A backend timeout must not be mistaken for a missing resource.
    assert_eq!(failed.status, 408);
    assert!(app
        .transport_snapshot()
        .objects
        .contains_key("seed-contact"));
}

#[test]
fn fault_matrix_preserves_timeout_protocol_and_recovery_categories() {
    let mut config = AppConfig::defaults();
    config.space_id = "fault-matrix".into();
    let checkpoint =
        std::env::temp_dir().join(format!("any-cal-fault-matrix-{}.json", std::process::id()));
    let _ = fs::remove_file(&checkpoint);
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());
    let mut app = App::with_transport(config, seeded_transport()).unwrap();

    let malformed = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/malformed.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"not-vcard".to_vec(),
    });
    assert_eq!(malformed.status, 400);
    assert_eq!(
        app.events
            .as_slice()
            .last()
            .and_then(|event| event.category.clone()),
        Some(any_cal_observability::ErrorCategory::Protocol)
    );

    app.with_transport_mut(|transport| {
        transport.inject(any_cal_anytype_adapter::TransportError::Timeout)
    });
    let timeout = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/seed-contact.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(timeout.status, 408);
    assert_eq!(
        app.events
            .as_slice()
            .last()
            .and_then(|event| event.category.clone()),
        Some(any_cal_observability::ErrorCategory::Timeout)
    );

    app.inject_checkpoint_fault(CommitFault::BeforeWrite);
    let recovery = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/recovery.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:recovery\r\nFN:Recovery\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(recovery.status, 500);
    assert_eq!(
        app.events
            .as_slice()
            .last()
            .and_then(|event| event.category.clone()),
        Some(any_cal_observability::ErrorCategory::Recovery)
    );
    assert!(app.events.as_slice().len() <= any_cal_observability::MAX_RETAINED_EVENTS);
    let _ = fs::remove_file(&checkpoint);
    let _ = fs::remove_file(checkpoint.with_extension("bak"));
}

#[test]
fn actual_app_dav_seam_supports_conditional_contact_lifecycle() {
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    // A remote object establishes the collection in the rebuildable adapter
    // cache; the DAV write itself still goes through the application seam.
    let envelope = ResourceEnvelope {
        collection_id: CollectionId::try_from("contacts").unwrap(),
        resource_id: ResourceId::try_from("bootstrap").unwrap(),
        kind: DavKind::Contact,
        anytype_object_id: AnytypeObjectId::try_from("bootstrap").unwrap(),
        dav_uid: DavUid::try_from("bootstrap").unwrap(),
        document: CanonicalDocument::new(StructuredDocument::default()),
        revision: 0,
    };
    let mut transport = FakeAnytypeTransport::new(1);
    transport.objects.insert(
        "bootstrap".into(),
        ObjectRecord {
            id: "bootstrap".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: envelope.canonical_json().unwrap(),
            archived: false,
            revision: 0,
        },
    );
    let mut app = App::with_transport(c, transport).unwrap();
    let body = b"BEGIN:VCARD\nVERSION:4.0\nUID:person\nFN:Alice\nEND:VCARD\n".to_vec();
    let put = app.handle_primary_dav(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/person.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: body.clone(),
    });
    assert_eq!(put.status, 201);
    let etag = put
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
        .unwrap()
        .1
        .clone();
    let get = app.handle_primary_dav(Request {
        method: "GET".into(),
        path: "/carddav/contacts/person.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(get.status, 200);
    assert_eq!(
        String::from_utf8(get.body).unwrap(),
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Alice\r\nUID:person\r\nEND:VCARD\r\n"
    );
    let bad = app.handle_primary_dav(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/person.vcf".into(),
        headers: vec![
            ("If-Match".into(), "\"stale\"".into()),
            ("Content-Type".into(), "text/vcard".into()),
        ],
        body: body.clone(),
    });
    assert_eq!(bad.status, 412);
    let deleted = app.handle_primary_dav(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/person.vcf".into(),
        headers: vec![("If-Match".into(), etag)],
        body: vec![],
    });
    assert_eq!(deleted.status, 204);
    assert_eq!(
        app.handle_primary_dav(Request {
            method: "GET".into(),
            path: "/carddav/contacts/person.vcf".into(),
            headers: vec![],
            body: vec![]
        })
        .status,
        404
    );
}

#[test]
fn actual_app_dav_seam_supports_vtodo_task_lifecycle() {
    let mut c = AppConfig::defaults();
    c.space_id = "space".into();
    let envelope = ResourceEnvelope {
        collection_id: CollectionId::try_from("tasks").unwrap(),
        resource_id: ResourceId::try_from("bootstrap-task").unwrap(),
        kind: DavKind::Task,
        anytype_object_id: AnytypeObjectId::try_from("bootstrap-task").unwrap(),
        dav_uid: DavUid::try_from("bootstrap-task").unwrap(),
        document: CanonicalDocument::new(StructuredDocument::default()),
        revision: 0,
    };
    let mut transport = FakeAnytypeTransport::new(1);
    transport.objects.insert(
        "bootstrap-task".into(),
        ObjectRecord {
            id: "bootstrap-task".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: envelope.canonical_json().unwrap(),
            archived: false,
            revision: 0,
        },
    );
    let mut app = App::with_transport(c, transport).unwrap();
    let body=b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:task-1\r\nSUMMARY:Ship\r\nEND:VTODO\r\nEND:VCALENDAR\r\n".to_vec();
    let put = app.handle_primary_dav(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/task-1.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body,
    });
    assert_eq!(put.status, 201);
    assert_eq!(
        app.handle_primary_dav(Request {
            method: "GET".into(),
            path: "/caldav/tasks/task-1.ics".into(),
            headers: vec![],
            body: vec![]
        })
        .status,
        200
    );
}

#[test]
fn app_listener_exposes_caldav_home_discovery() {
    let mut config = AppConfig::defaults();
    config.space_id = "space".into();
    let mut app = App::fake(config).unwrap();
    let redirect = app.handle(Request {
        method: "GET".into(),
        path: "/.well-known/caldav".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(redirect.status, 302);
    let response = app.handle(Request {
        method: "PROPFIND".into(),
        path: "/caldav/".into(),
        headers: vec![("Depth".into(), "1".into())],
        body: b"<allprop/>".to_vec(),
    });
    let xml = String::from_utf8(response.body).unwrap();
    assert_eq!(response.status, 207);
    assert!(xml.contains("calendar-home-set"));
    assert!(xml.contains("/caldav/tasks"));
    let collection = app.handle(Request {
        method: "PROPFIND".into(),
        path: "/caldav/tasks/".into(),
        headers: vec![("Depth".into(), "0".into())],
        body: b"<allprop/>".to_vec(),
    });
    assert_eq!(collection.status, 207);
}
