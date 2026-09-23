use any_cal_anytype_adapter::{FakeAnytypeTransport, ObjectRecord, TransportError};
use any_cal_app::{App, AppConfig};
use any_cal_core::{
    AnytypeObjectId, BridgeRequest, CanonicalDocument, CollectionId, DavKind, DavUid, Occurrence,
    ResourceEnvelope, ResourceId, StructuredDocument, BRIDGE_SCHEMA_VERSION,
};
use any_cal_dav_server::Request;
use serde_json::Value;
use std::collections::BTreeMap;

fn envelope(collection: &str, resource: &str, object: &str, kind: DavKind) -> ResourceEnvelope {
    let mut document = StructuredDocument::default();
    document.insert("FN", vec![Occurrence::new("Android contact")]);
    ResourceEnvelope {
        collection_id: CollectionId::try_from(collection).unwrap(),
        resource_id: ResourceId::try_from(resource).unwrap(),
        kind,
        anytype_object_id: AnytypeObjectId::try_from(object).unwrap(),
        dav_uid: DavUid::try_from(format!("uid:{object}")).unwrap(),
        document: CanonicalDocument::new(document),
        revision: 1,
    }
}

fn request(body: Vec<u8>, token: Option<&str>) -> Request {
    Request {
        method: "POST".into(),
        path: "/android/sync".into(),
        headers: token
            .map(|token| vec![("Authorization".into(), format!("Bearer {token}"))])
            .unwrap_or_default(),
        body,
    }
}

fn app_config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "android-space".into();
    config.auth_credential = Some("android-test-credential".into());
    config
}

fn bridge_request(
    resources: Vec<ResourceEnvelope>,
    tombstones: Vec<any_cal_core::BridgeTombstone>,
) -> Vec<u8> {
    BridgeRequest {
        schema_version: BRIDGE_SCHEMA_VERSION,
        account_name: "android-account".into(),
        account_type: "org.anycal.android".into(),
        authority: "com.android.contacts".into(),
        checkpoint: None,
        resources,
        tombstones,
    }
    .canonical_json()
    .unwrap()
    .into_bytes()
}

#[test]
fn android_pull_returns_canonical_contact_payload_and_checkpoint() {
    let contact = envelope(
        "contacts",
        "server-contact",
        "object-contact",
        DavKind::Contact,
    );
    let mut transport = FakeAnytypeTransport::new(2);
    transport.objects.insert(
        "object-contact".into(),
        ObjectRecord {
            id: "object-contact".into(),
            space_id: "android-space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: contact.canonical_json().unwrap(),
            archived: false,
            revision: 1,
        },
    );
    let mut app = App::with_transport(app_config(), transport).unwrap();
    let response = app.handle(request(
        bridge_request(Vec::new(), Vec::new()),
        Some("android-test-credential"),
    ));
    assert_eq!(response.status, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"], Value::Null);
    assert_eq!(body["resources"].as_array().unwrap().len(), 1);
    assert_eq!(body["resources"][0]["kind"], "contact");
    assert_eq!(body["decisions"][0]["decision"], "upsert");
    assert!(body["checkpoint"]["cursor"]
        .as_str()
        .unwrap()
        .starts_with("android:"));
    assert!(!String::from_utf8(response.body)
        .unwrap()
        .contains("NOT_LINKED"));
}

#[test]
fn android_push_updates_anytype_and_archives_with_stable_decisions() {
    let mut app = App::with_transport(app_config(), FakeAnytypeTransport::new(2)).unwrap();
    let contact = envelope(
        "contacts",
        "contact:object-new",
        "object-new",
        DavKind::Contact,
    );
    let created = app.handle(request(
        bridge_request(vec![contact.clone()], Vec::new()),
        Some("android-test-credential"),
    ));
    assert_eq!(created.status, 200);
    let created_body: Value = serde_json::from_slice(&created.body).unwrap();
    assert_eq!(created_body["error"], Value::Null);
    assert_eq!(
        created_body["decisions"][0]["resource_id"],
        "contact:object-new"
    );
    assert_eq!(created_body["decisions"][0]["decision"], "upsert");
    assert!(app
        .transport_snapshot()
        .objects
        .get("object-new")
        .is_some_and(|object| !object.archived));

    let tombstone = any_cal_core::BridgeTombstone {
        resource_id: ResourceId::try_from("contact:object-new").unwrap(),
        canonical_id: "object-new".into(),
        revision: 2,
    };
    let archived = app.handle(request(
        bridge_request(Vec::new(), vec![tombstone]),
        Some("android-test-credential"),
    ));
    assert_eq!(archived.status, 200);
    let archived_body: Value = serde_json::from_slice(&archived.body).unwrap();
    assert_eq!(archived_body["error"], Value::Null);
    assert_eq!(archived_body["decisions"][0]["decision"], "archive");
    assert_eq!(archived_body["tombstones"][0]["canonical_id"], "object-new");
    assert!(app
        .transport_snapshot()
        .objects
        .get("object-new")
        .is_some_and(|object| object.archived));
}

#[test]
fn android_sync_requires_credential_and_fails_closed_on_upstream_errors() {
    let body = bridge_request(Vec::new(), Vec::new());
    let mut app = App::with_transport(app_config(), FakeAnytypeTransport::new(2)).unwrap();
    assert_eq!(app.handle(request(body.clone(), None)).status, 401);

    let mut transport = FakeAnytypeTransport::new(2);
    transport.inject(TransportError::Unavailable);
    let mut unavailable = App::with_transport(app_config(), transport).unwrap();
    let response = unavailable.handle(request(body, Some("android-test-credential")));
    assert_eq!(response.status, 503);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "transport_unavailable");
    assert_eq!(body["resources"].as_array().unwrap().len(), 0);
    assert_eq!(body["tombstones"].as_array().unwrap().len(), 0);
}
