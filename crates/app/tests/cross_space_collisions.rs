use any_cal_anytype_adapter::{AmbiguousMutation, FakeAnytypeTransport};
use any_cal_app::{App, AppConfig};
use any_cal_core::{Repository, ResourceId, WriteCondition};
use any_cal_dav_server::{Request, Response};

fn multi_domain_config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-scalar-must-not-be-used".into();
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
                {"collection":"contacts","component":"vcard","path":"/carddav/personal"},
                {"collection":"tasks","component":"vtodo","path":"/caldav/personal"}
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
                {"collection":"contacts","component":"vcard","path":"/carddav/shared"},
                {"collection":"tasks","component":"vtodo","path":"/caldav/shared"}
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

fn request(method: &str, path: &str, content_type: Option<&str>, body: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: content_type
            .map(|value| vec![("Content-Type".into(), value.into())])
            .unwrap_or_default(),
        body: body.to_vec(),
    }
}

fn contact(path: &str, name: &str) -> Request {
    request(
        "PUT",
        path,
        Some("text/vcard"),
        format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:same-contact\r\nFN:{name}\r\nEND:VCARD\r\n")
            .as_bytes(),
    )
}

fn task(path: &str, summary: &str) -> Request {
    request(
        "PUT",
        path,
        Some("text/calendar"),
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:same-task\r\nSUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
        .as_bytes(),
    )
}

fn get(app: &mut App, path: &str) -> Response {
    app.handle(request("GET", path, None, &[]))
}

#[test]
fn same_contact_identity_and_ambiguous_create_remain_space_qualified() {
    let mut app =
        App::with_transport(multi_domain_config(), FakeAnytypeTransport::new(100)).unwrap();

    assert_eq!(
        app.handle(contact(
            "/carddav/personal/same-contact.vcf",
            "Personal Alice"
        ))
        .status,
        201
    );

    // The second Space uses the exact same DAV UID, resource ID and provisional
    // Anytype object ID. Force an ambiguous transport result so reconciliation
    // must search only the binding-selected Space rather than adopting A's row.
    app.server
        .repository
        .transport
        .timeout_after(AmbiguousMutation::Create);
    assert_eq!(
        app.handle(contact("/carddav/shared/same-contact.vcf", "Shared Bob"))
            .status,
        201
    );

    let transport = &app.server.repository.transport;
    assert!(transport
        .objects
        .contains_in_space("space-a", "same-contact"));
    assert!(transport
        .objects
        .contains_in_space("space-b", "same-contact"));
    assert!(!transport
        .objects
        .contains_in_space("legacy-scalar-must-not-be-used", "same-contact"));
    assert_eq!(transport.create_calls, 2);
    assert_eq!(app.server.repository.metrics.ambiguous_mutations, 1);
    assert_eq!(app.server.repository.metrics.reconciled_mutations, 1);

    let personal = get(&mut app, "/carddav/personal/same-contact.vcf");
    assert_eq!(personal.status, 200);
    assert!(String::from_utf8(personal.body)
        .unwrap()
        .contains("FN:Personal Alice"));

    let shared = get(&mut app, "/carddav/shared/same-contact.vcf");
    assert_eq!(shared.status, 200);
    assert!(String::from_utf8(shared.body)
        .unwrap()
        .contains("FN:Shared Bob"));

    let personal_list = app.handle(request(
        "REPORT",
        "/carddav/personal",
        None,
        b"<addressbook-query><prop><getetag/></prop></addressbook-query>",
    ));
    assert_eq!(personal_list.status, 207);
    let personal_list = String::from_utf8(personal_list.body).unwrap();
    assert!(personal_list.contains("/carddav/personal/same-contact.vcf"));
    assert!(!personal_list.contains("/carddav/shared/"));

    let shared_list = app.handle(request(
        "REPORT",
        "/carddav/shared",
        None,
        b"<addressbook-query><prop><getetag/></prop></addressbook-query>",
    ));
    assert_eq!(shared_list.status, 207);
    let shared_list = String::from_utf8(shared_list.body).unwrap();
    assert!(shared_list.contains("/carddav/shared/same-contact.vcf"));
    assert!(!shared_list.contains("/carddav/personal/"));
}

#[test]
fn update_and_archive_of_same_task_identity_do_not_cross_spaces() {
    let mut app =
        App::with_transport(multi_domain_config(), FakeAnytypeTransport::new(100)).unwrap();

    assert_eq!(
        app.handle(task("/caldav/personal/same-task.ics", "Personal task"))
            .status,
        201
    );
    assert_eq!(
        app.handle(task("/caldav/shared/same-task.ics", "Shared task"))
            .status,
        201
    );

    assert_eq!(
        app.handle(task(
            "/caldav/personal/same-task.ics",
            "Personal task updated"
        ))
        .status,
        204
    );
    let personal = get(&mut app, "/caldav/personal/same-task.ics");
    assert_eq!(personal.status, 200);
    assert!(String::from_utf8(personal.body)
        .unwrap()
        .contains("SUMMARY:Personal task updated"));
    let shared = get(&mut app, "/caldav/shared/same-task.ics");
    assert_eq!(shared.status, 200);
    assert!(String::from_utf8(shared.body)
        .unwrap()
        .contains("SUMMARY:Shared task"));

    assert_eq!(
        app.handle(request("DELETE", "/caldav/shared/same-task.ics", None, &[]))
            .status,
        204
    );
    assert_eq!(get(&mut app, "/caldav/shared/same-task.ics").status, 404);
    assert_eq!(get(&mut app, "/caldav/personal/same-task.ics").status, 200);

    let transport = &app.server.repository.transport;
    assert!(
        !transport
            .objects
            .get_in_space("space-a", "same-task")
            .unwrap()
            .archived
    );
    assert!(
        transport
            .objects
            .get_in_space("space-b", "same-task")
            .unwrap()
            .archived
    );
}

#[test]
fn missing_resource_in_other_space_and_unknown_routes_do_not_mutate_active_binding() {
    let mut app =
        App::with_transport(multi_domain_config(), FakeAnytypeTransport::new(100)).unwrap();

    assert_eq!(
        app.handle(contact(
            "/carddav/personal/same-contact.vcf",
            "Only in personal"
        ))
        .status,
        201
    );
    assert_eq!(
        get(&mut app, "/carddav/shared/same-contact.vcf").status,
        404
    );

    // The miss in B must not be interpreted as a deletion/tombstone of A, and
    // switching back must recover A's binding-scoped cache/remote view.
    let personal = get(&mut app, "/carddav/personal/same-contact.vcf");
    assert_eq!(personal.status, 200);
    assert!(String::from_utf8(personal.body)
        .unwrap()
        .contains("FN:Only in personal"));
    assert!(
        !app.server
            .repository
            .transport
            .objects
            .get_in_space("space-a", "same-contact")
            .unwrap()
            .archived
    );

    // A DAV-looking path that is not present in the binding contract must fail
    // closed instead of being interpreted through whichever domain is active.
    assert_eq!(
        get(&mut app, "/carddav/not-configured/same-contact.vcf").status,
        404
    );
    assert_eq!(
        get(&mut app, "/caldav/not-configured/same-task.ics").status,
        404
    );
    assert_eq!(
        get(&mut app, "/carddav/personal/same-contact.vcf").status,
        200
    );
}

#[test]
fn hard_delete_after_route_selection_remains_in_selected_space() {
    let mut app =
        App::with_transport(multi_domain_config(), FakeAnytypeTransport::new(100)).unwrap();

    assert_eq!(
        app.handle(contact(
            "/carddav/personal/same-contact.vcf",
            "Personal survives"
        ))
        .status,
        201
    );
    assert_eq!(
        app.handle(contact(
            "/carddav/shared/same-contact.vcf",
            "Shared is deleted"
        ))
        .status,
        201
    );

    // Select the shared binding through the public route before exercising the
    // repository's hard-delete operation. The binding-scoped repository must
    // remove only the object in space-b even though space-a has the same ID.
    assert_eq!(get(&mut app, "/carddav/shared/same-contact.vcf").status, 200);
    app.server
        .repository
        .delete_resource(
            &ResourceId::try_from("same-contact").unwrap(),
            WriteCondition::Unconditional,
        )
        .unwrap();

    assert!(app
        .server
        .repository
        .transport
        .objects
        .get_in_space("space-b", "same-contact")
        .is_none());
    assert!(app
        .server
        .repository
        .transport
        .objects
        .get_in_space("space-a", "same-contact")
        .is_some());
    assert_eq!(get(&mut app, "/carddav/personal/same-contact.vcf").status, 200);
}
