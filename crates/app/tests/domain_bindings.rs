use any_cal_app::{App, AppConfig};
use any_cal_anytype_adapter::FakeAnytypeTransport;
use any_cal_dav_server::Request;
use any_cal_core::{DavRoute, VisibilityIntent};

#[test]
fn scalar_app_config_projects_to_one_legacy_domain_binding() {
    let mut config = AppConfig::defaults();
    config.space_id = "space-legacy".into();
    config.contacts_collection = "friends".into();
    config.tasks_collection = "todo".into();

    let contract = config.domain_bindings().unwrap();
    assert_eq!(contract.bindings.len(), 1);

    let binding = &contract.bindings[0];
    assert_eq!(binding.domain_id, "legacy-default");
    assert_eq!(binding.space_id, "space-legacy");
    assert_eq!(binding.visibility, VisibilityIntent::Unknown);
    assert_eq!(
        binding.routes,
        vec![
            DavRoute::contacts("/carddav/friends"),
            DavRoute::tasks("/caldav/todo")
        ]
    );
}
#[test]
fn invalid_scalar_config_does_not_produce_domain_bindings() {
    let config = AppConfig::defaults();
    assert!(config.domain_bindings().is_err());
}

#[test]
fn sync_scope_is_non_secret_and_changes_with_upstream_context() {
    let mut config = AppConfig::defaults();
    config.space_id = "space-a".into();
    config.token = Some("synthetic-token-a".into());

    let base = config.sync_scope("custom").unwrap();
    assert_eq!(base.domain_id, "legacy-default");
    assert_eq!(base.space_id, "space-a");
    assert_eq!(base.account_fingerprint.len(), 64);
    assert_eq!(base.endpoint_fingerprint.len(), 64);
    assert!(!base.account_fingerprint.contains("synthetic-token-a"));

    let mut moved = config.clone();
    moved.space_id = "space-b".into();
    assert_ne!(base, moved.sync_scope("custom").unwrap());

    let mut endpoint = config.clone();
    endpoint.endpoint = "http://127.0.0.1:31013".into();
    let endpoint_scope = endpoint.sync_scope("custom").unwrap();
    assert_ne!(base, endpoint_scope);
    assert_ne!(
        base.endpoint_fingerprint,
        endpoint_scope.endpoint_fingerprint
    );
    assert_ne!(base.account_fingerprint, endpoint_scope.account_fingerprint);
    assert_ne!(base.binding_fingerprint, endpoint_scope.binding_fingerprint);

    let mut rotated = config;
    rotated.token = Some("synthetic-token-b".into());
    let rotated_scope = rotated.sync_scope("custom").unwrap();
    assert_ne!(base, rotated_scope);
    assert_ne!(base.account_fingerprint, rotated_scope.account_fingerprint);
    assert_ne!(base.binding_fingerprint, rotated_scope.binding_fingerprint);
}


#[test]
fn explicit_multi_domain_routes_keep_same_resource_identity_space_qualified() {
    let mut config = AppConfig::defaults();
    // Deliberately poison the legacy scalar. Explicit bindings must be the
    // runtime source of truth and must never fall back to this Space.
    config.space_id = "scalar-space-must-not-be-used".into();
    config.credential_profile_id = "primary".into();
    config.domain_bindings_json = Some(
        r#"{
          "version":1,
          "bindings":[
            {
              "domain_id":"personal",
              "label":"Personal",
              "space_id":"space-a",
              "credential_profile_id":"primary",
              "routes":[
                {"collection":"contacts","component":"vcard","path":"/carddav/personal"}
              ],
              "schema_profile":"default",
              "checkpoint_namespace":"personal",
              "visibility":"private",
              "lifecycle":"configured"
            },
            {
              "domain_id":"shared",
              "label":"Shared",
              "space_id":"space-b",
              "credential_profile_id":"primary",
              "routes":[
                {"collection":"contacts","component":"vcard","path":"/carddav/shared"}
              ],
              "schema_profile":"default",
              "checkpoint_namespace":"shared",
              "visibility":"shared",
              "lifecycle":"configured"
            }
          ]
        }"#
        .into(),
    );

    let contract = config.domain_bindings().unwrap();
    assert_eq!(contract.bindings.len(), 2);
    assert!(contract
        .bindings
        .iter()
        .all(|binding| binding.space_id != "scalar-space-must-not-be-used"));

    let mut app = App::with_transport(config, FakeAnytypeTransport::new(100)).unwrap();

    let personal = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/personal/same.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:same\r\nFN:Personal Alice\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(personal.status, 201);

    let shared = app.handle(Request {
        method: "PUT".into(),
        path: "/carddav/shared/same.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:same\r\nFN:Shared Bob\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(shared.status, 201);

    let objects = app.server.repository.transport.objects.values().collect::<Vec<_>>();
    assert_eq!(objects.len(), 2);
    assert!(objects.iter().any(|object| object.space_id == "space-a"));
    assert!(objects.iter().any(|object| object.space_id == "space-b"));
    assert!(objects
        .iter()
        .all(|object| object.space_id != "scalar-space-must-not-be-used"));

    let personal_get = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/personal/same.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(personal_get.status, 200);
    assert!(String::from_utf8(personal_get.body)
        .unwrap()
        .contains("FN:Personal Alice"));

    let shared_get = app.handle(Request {
        method: "GET".into(),
        path: "/carddav/shared/same.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(shared_get.status, 200);
    assert!(String::from_utf8(shared_get.body)
        .unwrap()
        .contains("FN:Shared Bob"));
    assert_eq!(app.server.repository.binding.space_id, "space-b");
}

#[test]
fn explicit_multi_domain_rejects_unavailable_credential_profiles() {
    let mut config = AppConfig::defaults();
    config.credential_profile_id = "primary".into();
    config.domain_bindings_json = Some(
        r#"{
          "version":1,
          "bindings":[{
            "domain_id":"other-account",
            "label":"Other account",
            "space_id":"space-a",
            "credential_profile_id":"secondary",
            "routes":[
              {"collection":"contacts","component":"vcard","path":"/carddav/other"}
            ],
            "schema_profile":"default",
            "checkpoint_namespace":"other-account",
            "visibility":"unknown",
            "lifecycle":"configured"
          }]
        }"#
        .into(),
    );
    assert!(config.validate().is_err());
}
