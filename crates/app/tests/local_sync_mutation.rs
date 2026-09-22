use any_cal_anytype_adapter::FakeAnytypeTransport;
use any_cal_app::{App, AppConfig};
use any_cal_core::{typed_etag_for_bytes, ResourceId};
use any_cal_dav_server::Request;
use any_cal_sync::{
    classify, ConflictPolicy, ObservedResource, OperationKind, PendingOperation, Reconciliation,
    SyncStore,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "any-cal-{label}-{}-{nonce}.json",
        std::process::id()
    ))
}

fn remove_checkpoint(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("bak"));
    let _ = fs::remove_file(path.with_extension("lock"));
}

fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: headers
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect(),
        body: body.into(),
    }
}

fn header(response: &any_cal_dav_server::Response, name: &str) -> String {
    response
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
        .expect("response header")
}

const FIRST: &[u8] =
    b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:local-mutation\r\nFN:First\r\nEND:VCARD\r\n";
const SECOND: &[u8] =
    b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:local-mutation\r\nFN:Second\r\nEND:VCARD\r\n";

#[test]
fn dav_mutation_checkpoint_tombstone_and_restart_are_one_local_flow() {
    let checkpoint = temporary_path("dav-local-flow");
    let mut config = AppConfig::defaults();
    config.space_id = "local-flow-space".into();
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());

    let mut app = App::with_transport(config.clone(), FakeAnytypeTransport::new(2)).unwrap();
    let created = app.handle(request(
        "PUT",
        "/carddav/contacts/local-mutation.vcf",
        &[
            ("Content-Type", "text/vcard"),
            ("X-Request-ID", "local-create"),
        ],
        FIRST,
    ));
    assert_eq!(created.status, 201);
    let first_etag = header(&created, "ETag");
    let generation_after_create = app.sync_state().unwrap().generation;
    assert_eq!(app.sync_state().unwrap().observed.len(), 1);

    let updated = app.handle(request(
        "PUT",
        "/carddav/contacts/local-mutation.vcf",
        &[
            ("Content-Type", "text/vcard"),
            ("If-Match", &first_etag),
            ("X-Request-ID", "local-update"),
        ],
        SECOND,
    ));
    assert_eq!(updated.status, 204);
    let second_etag = header(&updated, "ETag");
    assert_ne!(first_etag, second_etag);
    assert!(app.sync_state().unwrap().generation > generation_after_create);

    let stale_delete = app.handle(request(
        "DELETE",
        "/carddav/contacts/local-mutation.vcf",
        &[
            ("If-Match", &first_etag),
            ("X-Request-ID", "local-stale-delete"),
        ],
        &[],
    ));
    assert_eq!(stale_delete.status, 412);

    let stale = app.handle(request(
        "PUT",
        "/carddav/contacts/local-mutation.vcf",
        &[
            ("Content-Type", "text/vcard"),
            ("If-Match", &first_etag),
            ("X-Request-ID", "local-stale"),
        ],
        FIRST,
    ));
    assert_eq!(stale.status, 412);
    let state_after_stale = app.sync_state().unwrap().clone();
    let current = app.handle(request(
        "GET",
        "/carddav/contacts/local-mutation.vcf",
        &[],
        &[],
    ));
    assert_eq!(current.status, 200);
    assert_eq!(header(&current, "ETag"), second_etag);
    assert!(String::from_utf8_lossy(&current.body).contains("Second"));
    assert_eq!(
        app.sync_state().unwrap().generation,
        state_after_stale.generation
    );

    let deleted = app.handle(request(
        "DELETE",
        "/carddav/contacts/local-mutation.vcf",
        &[("If-Match", &second_etag), ("X-Request-ID", "local-delete")],
        &[],
    ));
    assert_eq!(deleted.status, 204);
    let state = app.sync_state().unwrap();
    assert!(state.observed.is_empty());
    assert!(state
        .tombstones
        .values()
        .any(|tombstone| tombstone.dav_uid == "local-mutation"));
    let remote = app.server.repository.transport.clone();
    drop(app);

    let mut reopened = App::with_transport(config, remote).unwrap();
    assert!(reopened
        .sync_state()
        .unwrap()
        .tombstones
        .values()
        .any(|tombstone| {
            tombstone.resource_id == ResourceId::try_from("local-mutation").unwrap()
        }));
    assert_eq!(
        reopened
            .handle(request(
                "GET",
                "/carddav/contacts/local-mutation.vcf",
                &[],
                &[],
            ))
            .status,
        404
    );
    assert!(reopened
        .events
        .as_slice()
        .iter()
        .all(|event| !event.message.contains("local-mutation")));
    drop(reopened);
    remove_checkpoint(&checkpoint);
}

#[test]
fn local_observation_and_pending_replay_are_idempotent_and_conflict_safe() {
    let checkpoint = temporary_path("dav-local-observation");
    let id = ResourceId::try_from("observed-resource").unwrap();
    let observed = ObservedResource {
        resource_id: id.clone(),
        anytype_object_id: "object-local".into(),
        dav_uid: "uid-local".into(),
        revision: 4,
        etag: typed_etag_for_bytes(b"v4").as_str().into(),
        modified_at: 4,
        archived: false,
    };
    let mut store = SyncStore::open(&checkpoint).unwrap();
    store.replace_observed([observed.clone()]).unwrap();
    let operation = PendingOperation {
        operation_id: "operation-local-1".into(),
        kind: OperationKind::Update,
        resource_id: id.clone(),
        expected_etag: Some(observed.etag.clone()),
        expected_revision: Some(observed.revision),
        envelope: None,
        attempts: 0,
    };
    store.enqueue(operation.clone()).unwrap();
    store.enqueue(operation).unwrap();
    assert_eq!(store.pending().count(), 1);

    let remote = ObservedResource {
        revision: 5,
        etag: typed_etag_for_bytes(b"v5").as_str().into(),
        ..observed.clone()
    };
    assert_eq!(
        classify(Some(&observed), Some(&remote), true),
        Reconciliation::Conflict
    );
    assert_eq!(
        any_cal_sync::classify_with_policy(
            Some(&observed),
            Some(&remote),
            true,
            ConflictPolicy::LaterWriteWins,
        ),
        Reconciliation::RemoteChanged
    );

    store
        .mark_applied("operation-local-1", Some(remote.clone()))
        .unwrap();
    store.replace_observed(std::iter::empty()).unwrap();
    assert!(store.state().observed.is_empty());
    assert!(store.state().tombstones.contains_key(&id));
    drop(store);

    let mut reopened = SyncStore::open(&checkpoint).unwrap();
    assert!(reopened.state().tombstones.contains_key(&id));
    assert!(reopened.replace_observed([remote.clone()]).is_err());
    reopened
        .replace_observed_explicit_resurrection([remote])
        .unwrap();
    assert!(reopened.state().tombstones.is_empty());
    assert!(reopened.state().observed.contains_key(&id));
    drop(reopened);
    remove_checkpoint(&checkpoint);
}

#[test]
fn app_checkpoint_rejects_space_endpoint_and_token_context_changes() {
    let checkpoint = temporary_path("scoped-startup");
    let mut config = AppConfig::defaults();
    config.space_id = "scope-a".into();
    config.token = Some("synthetic-token-a".into());
    config.sync_checkpoint = Some(checkpoint.to_string_lossy().into_owned());

    {
        let mut app =
            App::with_transport(config.clone(), FakeAnytypeTransport::new(2)).unwrap();
        let created = app.handle(request(
            "PUT",
            "/carddav/contacts/scoped.vcf",
            &[("Content-Type", "text/vcard")],
            b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:scoped\r\nFN:Scoped\r\nEND:VCARD\r\n",
        ));
        assert_eq!(created.status, 201);
    }

    assert!(App::with_transport(config.clone(), FakeAnytypeTransport::new(2)).is_ok());

    let mut moved = config.clone();
    moved.space_id = "scope-b".into();
    assert!(App::with_transport(moved, FakeAnytypeTransport::new(2)).is_err());

    let mut endpoint = config.clone();
    endpoint.endpoint = "http://127.0.0.1:31013".into();
    assert!(App::with_transport(endpoint, FakeAnytypeTransport::new(2)).is_err());

    let mut rotated = config;
    rotated.token = Some("synthetic-token-b".into());
    assert!(App::with_transport(rotated, FakeAnytypeTransport::new(2)).is_err());

    remove_checkpoint(&checkpoint);
}

