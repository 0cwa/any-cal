use any_cal_anytype_adapter::{FakeAnytypeTransport, TransportError};
use any_cal_app::{App, AppConfig};
use any_cal_dav_server::{Request, Response};

fn config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-must-not-route".into();
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
    config
}

fn request(method: &str, path: &str, body: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: (method == "PUT")
            .then(|| vec![("Content-Type".into(), "text/vcard".into())])
            .unwrap_or_default(),
        body: body.to_vec(),
    }
}

fn put(app: &mut App, path: &str, name: &str) -> Response {
    app.handle(request(
        "PUT",
        path,
        format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:same\r\nFN:{name}\r\nEND:VCARD\r\n").as_bytes(),
    ))
}

fn seeded() -> App {
    let mut app = App::with_transport(config(), FakeAnytypeTransport::new(16)).unwrap();
    assert_eq!(
        put(&mut app, "/carddav/personal/same.vcf", "Personal").status,
        201
    );
    assert_eq!(
        put(&mut app, "/carddav/shared/same.vcf", "Shared").status,
        201
    );
    app
}

#[test]
fn viewer_capability_blocks_writes_before_transport_and_filters_options() {
    let mut app = seeded();
    let (binding_fingerprint, _, _) = app.upstream_binding_identity("shared").unwrap();

    assert!(!app.configure_upstream_capabilities("shared", "stale-binding", true, false));
    assert!(app.configure_upstream_capabilities("shared", &binding_fingerprint, true, false));
    assert_eq!(
        app.upstream_binding_status("shared"),
        Some(("allowed", "denied", "active"))
    );

    assert_eq!(
        app.handle(request("GET", "/carddav/shared/same.vcf", &[]))
            .status,
        200
    );

    let before = app.transport_snapshot();
    let denied = put(&mut app, "/carddav/shared/same.vcf", "Forbidden update");
    assert_eq!(denied.status, 403);
    assert_eq!(denied.body, b"upstream access denied");
    let after = app.transport_snapshot();
    assert_eq!(after.list_calls, before.list_calls);
    assert_eq!(after.update_calls, before.update_calls);
    assert_eq!(after.create_calls, before.create_calls);

    let options = app.handle(request("OPTIONS", "/carddav/shared", &[]));
    assert_eq!(options.status, 200);
    let allow = options
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("allow"))
        .map(|(_, value)| value.as_str())
        .unwrap();
    assert!(allow.contains("GET") && allow.contains("REPORT"));
    assert!(!allow.contains("PUT") && !allow.contains("DELETE"));
}

#[test]
fn upstream_auth_suspends_only_affected_binding_until_exact_reauthorization() {
    let mut app = seeded();
    let (_, account_fingerprint, shared_space) = app.upstream_binding_identity("shared").unwrap();

    app.with_transport_mut(|transport| transport.inject(TransportError::Auth));
    assert_eq!(
        app.handle(request("GET", "/carddav/shared/same.vcf", &[]))
            .status,
        401
    );
    assert_eq!(
        app.upstream_binding_status("shared"),
        Some(("unknown", "allowed", "auth"))
    );

    let after_failure = app.transport_snapshot();
    assert_eq!(
        app.handle(request("GET", "/carddav/shared/same.vcf", &[]))
            .status,
        401
    );
    let after_retry = app.transport_snapshot();
    assert_eq!(after_retry.list_calls, after_failure.list_calls);
    assert_eq!(after_retry.get_calls, after_failure.get_calls);

    assert_eq!(
        app.handle(request("GET", "/carddav/personal/same.vcf", &[]))
            .status,
        200
    );
    assert!(app.transport_snapshot().list_calls > after_retry.list_calls);

    assert!(!app.reauthorize_upstream_domain("shared", "wrong-account", &shared_space));
    assert!(!app.reauthorize_upstream_domain("shared", &account_fingerprint, "wrong-space"));
    assert_eq!(
        app.handle(request("GET", "/carddav/shared/same.vcf", &[]))
            .status,
        401
    );

    assert!(app.reauthorize_upstream_domain("shared", &account_fingerprint, &shared_space));
    assert_eq!(
        app.upstream_binding_status("shared"),
        Some(("unknown", "unknown", "active"))
    );
    assert_eq!(
        app.handle(request("GET", "/carddav/shared/same.vcf", &[]))
            .status,
        200
    );
    assert_eq!(
        app.upstream_binding_status("shared"),
        Some(("allowed", "unknown", "active"))
    );
}

#[test]
fn upstream_forbidden_suspension_is_stable_and_domain_local() {
    let mut app = seeded();
    app.with_transport_mut(|transport| transport.inject(TransportError::Forbidden));

    assert_eq!(
        app.handle(request("GET", "/carddav/shared/same.vcf", &[]))
            .status,
        403
    );
    let calls = app.transport_snapshot().list_calls;
    assert_eq!(
        app.handle(request(
            "REPORT",
            "/carddav/shared",
            b"<addressbook-query/>"
        ))
        .status,
        403
    );
    assert_eq!(app.transport_snapshot().list_calls, calls);
    assert_eq!(
        app.upstream_binding_status("shared"),
        Some(("unknown", "allowed", "forbidden"))
    );

    assert_eq!(
        app.handle(request("GET", "/carddav/personal/same.vcf", &[]))
            .status,
        200
    );
}
