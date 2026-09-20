use any_cal_core::{
    typed_etag_for_bytes, AnytypeObjectId, CanonicalDocument, DavKind, DavUid, FailureMode,
    MemoryRepository, Occurrence, Repository, ResourceEnvelope, ResourceId, StructuredDocument,
    WriteCondition,
};
use any_cal_dav_server::{DavServer, Request};
use std::collections::BTreeMap;

fn req(method: &str, path: &str, body: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: vec![],
        body: body.into(),
    }
}

fn response_header<'a>(response: &'a any_cal_dav_server::Response, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn contact_put(
    server: &mut DavServer<MemoryRepository>,
    name: &str,
    fn_value: &str,
) -> any_cal_dav_server::Response {
    server.handle(Request {
        method: "PUT".into(),
        path: format!("/carddav/contacts/{name}.vcf"),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{name}\r\nFN:{fn_value}\r\nEND:VCARD\r\n")
            .into_bytes(),
    })
}

#[test]
fn unsupported_lock_tokens_fail_closed_without_mutation() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let created = contact_put(&mut server, "lock-boundary", "Original");
    assert_eq!(created.status, 201);
    let original_etag = response_header(&created, "ETag").unwrap().to_owned();

    for token in [
        "(<opaquelocktoken:synthetic-valid>)",
        "(<opaquelocktoken:synthetic-expired>)",
        "(<opaquelocktoken:synthetic-invalid>)",
    ] {
        let rejected = server.handle(Request {
            method: "PUT".into(),
            path: "/carddav/contacts/lock-boundary.vcf".into(),
            headers: vec![
                ("Content-Type".into(), "text/vcard".into()),
                ("If".into(), token.into()),
            ],
            body:
                b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:lock-boundary\r\nFN:Rejected\r\nEND:VCARD\r\n"
                    .to_vec(),
        });
        assert_eq!(rejected.status, 412, "lock condition {token}");
        assert_eq!(rejected.body, b"precondition failed");
        assert!(!String::from_utf8_lossy(&rejected.body).contains("synthetic"));

        let current = server.handle(req("GET", "/carddav/contacts/lock-boundary.vcf", b""));
        assert_eq!(current.status, 200);
        assert_eq!(
            response_header(&current, "ETag"),
            Some(original_etag.as_str())
        );
        assert!(String::from_utf8_lossy(&current.body).contains("Original"));
    }

    let delete_rejected = server.handle(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/lock-boundary.vcf".into(),
        headers: vec![("If".into(), "(<opaquelocktoken:synthetic-valid>)".into())],
        body: vec![],
    });
    assert_eq!(delete_rejected.status, 412);
    let still_present = server.handle(req("GET", "/carddav/contacts/lock-boundary.vcf", b""));
    assert_eq!(still_present.status, 200);
    assert_eq!(
        response_header(&still_present, "ETag"),
        Some(original_etag.as_str())
    );
}

#[test]
fn lock_methods_are_truthfully_unsupported_and_stale_etags_are_atomic() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let created = contact_put(&mut server, "conditional-order", "First");
    assert_eq!(created.status, 201);
    let first_etag = response_header(&created, "ETag").unwrap().to_owned();

    for method in ["LOCK", "UNLOCK"] {
        let response = server.handle(Request {
            method: method.into(),
            path: "/carddav/contacts/conditional-order.vcf".into(),
            headers: vec![("Lock-Token".into(), "<opaquelocktoken:synthetic>".into())],
            body: b"<lockinfo/>".to_vec(),
        });
        assert_eq!(response.status, 405, "{method} must remain unsupported");
        assert_eq!(
            response_header(&response, "Allow"),
            Some("OPTIONS, PROPFIND, REPORT, GET, HEAD, PUT, DELETE")
        );
        assert!(!String::from_utf8_lossy(&response.body).contains("synthetic"));
    }

    let second = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/conditional-order.vcf".into(),
        headers: vec![
            ("Content-Type".into(), "text/vcard".into()),
            ("If-Match".into(), first_etag.clone()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:conditional-order\r\nFN:Second\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(second.status, 204);
    let second_etag = response_header(&second, "ETag").unwrap().to_owned();
    assert_ne!(second_etag, first_etag);

    let stale = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/conditional-order.vcf".into(),
        headers: vec![
            ("Content-Type".into(), "text/vcard".into()),
            ("If-Match".into(), first_etag),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:conditional-order\r\nFN:Stale\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(stale.status, 412);
    let current = server.handle(req("GET", "/carddav/contacts/conditional-order.vcf", b""));
    assert_eq!(current.status, 200);
    assert_eq!(
        response_header(&current, "ETag"),
        Some(second_etag.as_str())
    );
    assert!(String::from_utf8_lossy(&current.body).contains("Second"));
}

#[test]
fn error_contract_is_bounded_correlated_and_nonleaky() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let cases = [
        ("PATCH", "/carddav/contacts", b"".as_slice(), 405),
        ("GET", "/carddav/contacts/missing-secret.vcf", b"", 404),
        ("PUT", "/carddav/contacts/media.vcf", b"not-a-vcard", 415),
        ("REPORT", "/carddav/contacts", b"<broken", 400),
    ];
    for (method, path, body, status) in cases {
        let response = server.handle(Request {
            method: method.into(),
            path: path.into(),
            headers: vec![
                ("Content-Type".into(), "application/octet-stream".into()),
                ("X-Request-ID".into(), "dav-contract-1".into()),
            ],
            body: body.to_vec(),
        });
        assert_eq!(response.status, status, "{method} {path}");
        assert_eq!(
            response_header(&response, "X-Request-ID"),
            Some("dav-contract-1")
        );
        assert_eq!(
            response_header(&response, "Cache-Control"),
            Some("no-store")
        );
        assert!(response_header(&response, "Content-Type")
            .is_some_and(|value| value.starts_with("text/plain")));
        assert!(response.body.len() < 256);
        let body = String::from_utf8_lossy(&response.body);
        assert!(!body.contains(path));
        assert!(!body.contains("secret"));
    }
    let response = server.handle(req("PATCH", "/carddav/contacts", b""));
    assert_eq!(response.status, 405);
    assert_eq!(
        response_header(&response, "Allow"),
        Some("OPTIONS, PROPFIND, REPORT, GET, HEAD, PUT, DELETE")
    );
}

#[test]
fn backend_failures_and_preconditions_keep_truthful_statuses() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    server.repository.inject_failure(FailureMode::Timeout);
    let timeout = server.handle(req("GET", "/carddav/contacts/missing.vcf", b""));
    assert_eq!(timeout.status, 408);
    assert_eq!(response_header(&timeout, "Cache-Control"), Some("no-store"));
    assert_eq!(timeout.body, b"request timeout");

    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/contract.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:contract\r\nFN:Contract\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(created.status, 201);
    let stale = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/contract.vcf".into(),
        headers: vec![
            ("Content-Type".into(), "text/vcard".into()),
            ("If-Match".into(), "\"stale\"".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:contract\r\nFN:Changed\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(stale.status, 412);
    assert_eq!(stale.body, b"precondition failed");
    assert_eq!(response_header(&stale, "Cache-Control"), Some("no-store"));
}

#[test]
fn android_discovery_well_known_and_principal_paths_are_safe() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let cal = server.handle(req("GET", "/.well-known/caldav", b""));
    assert_eq!(cal.status, 302);
    assert!(cal
        .headers
        .iter()
        .any(|(k, v)| k == "Location" && v == "/caldav/"));
    let card = server.handle(req("GET", "/.well-known/carddav", b""));
    assert_eq!(card.status, 302);
    assert!(card
        .headers
        .iter()
        .any(|(k, v)| k == "Location" && v == "/carddav/"));
    let root = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/".into(),
        headers: vec![("Depth".into(), "1".into())],
        body: b"<allprop/>".to_vec(),
    });
    let xml = String::from_utf8(root.body).unwrap();
    assert_eq!(root.status, 207);
    assert!(xml.contains("current-user-principal") && xml.contains("calendar-home-set"));
}

#[test]
fn discovery_uses_validated_proxy_origin_when_available() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let request = |path: &str| Request {
        method: "PROPFIND".into(),
        path: path.into(),
        headers: vec![
            ("Host".into(), "dav.example.test:8443".into()),
            ("X-Forwarded-Proto".into(), "https".into()),
            ("Depth".into(), "0".into()),
        ],
        body: b"<allprop/>".to_vec(),
    };
    let principal = server.handle(request("/principals/users/default"));
    let xml = String::from_utf8(principal.body).unwrap();
    assert!(xml.contains("<href>https://dav.example.test:8443/principals/users/default</href>"));
    assert!(xml.contains(
        "<calendar-home-set xmlns=\"urn:ietf:params:xml:ns:caldav\"><d:href xmlns:d=\"DAV:\">https://dav.example.test:8443/caldav/</d:href>"
    ));

    let root = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/".into(),
        headers: vec![
            ("Host".into(), "dav.example.test:8443".into()),
            ("X-Forwarded-Proto".into(), "https".into()),
            ("Depth".into(), "0".into()),
        ],
        body: b"<allprop/>".to_vec(),
    });
    let xml = String::from_utf8(root.body).unwrap();
    assert!(xml.contains(
        "<addressbook-home-set xmlns=\"urn:ietf:params:xml:ns:carddav\"><d:href xmlns:d=\"DAV:\">https://dav.example.test:8443/carddav/</d:href>"
    ));

    let collection = server.handle(request("/caldav/tasks/"));
    let xml = String::from_utf8(collection.body).unwrap();
    assert!(xml.contains("<href>https://dav.example.test:8443/caldav/tasks</href>"));
    assert!(xml.contains(
        "<calendar-home-set><d:href xmlns:d=\"DAV:\">https://dav.example.test:8443/caldav/</d:href>"
    ));

    let malformed = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/".into(),
        headers: vec![("Host".into(), "evil.example/redirect".into())],
        body: b"<allprop/>".to_vec(),
    });
    let xml = String::from_utf8(malformed.body).unwrap();
    assert!(xml.contains("<href>/principals/users/default</href>"));
    assert!(!xml.contains("evil.example"));

    let malformed_port = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/".into(),
        headers: vec![("Host".into(), "dav.example.test:not-a-port".into())],
        body: b"<allprop/>".to_vec(),
    });
    let xml = String::from_utf8(malformed_port.body).unwrap();
    assert!(xml.contains("<href>/</href>"));
    assert!(!xml.contains("dav.example.test"));
}

#[test]
fn vtodo_timezone_forms_are_preserved_without_silent_conversion() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    for (name, value) in [
        ("utc", "20260830T120000Z"),
        ("floating", "20260830T120000"),
        ("tzid", "20260830T120000"),
    ] {
        let body = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:{name}\r\nDTSTART{}:{}\r\nDUE{}:{}\r\nCOMPLETED{}:{}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n", if name == "tzid" { ";TZID=Europe/Stockholm" } else { "" }, value, if name == "tzid" { ";TZID=Europe/Stockholm" } else { "" }, value, if name == "tzid" { ";TZID=Europe/Stockholm" } else { "" }, value);
        let response = server.handle(Request {
            method: "PUT".into(),
            path: format!("/caldav/tasks/{name}.ics"),
            headers: vec![("Content-Type".into(), "text/calendar".into())],
            body: body.into_bytes(),
        });
        assert_eq!(response.status, 201);
    }
    for (name, expected) in [
        ("utc", "DTSTART:20260830T120000Z"),
        ("floating", "DTSTART:20260830T120000"),
        ("tzid", "DTSTART;TZID=Europe/Stockholm:20260830T120000"),
    ] {
        let fetched = server.handle(req("GET", &format!("/caldav/tasks/{name}.ics"), b""));
        let text = String::from_utf8(fetched.body).unwrap();
        assert!(text.contains(expected));
        assert!(text.contains(&expected.replacen("DTSTART", "DUE", 1)));
        assert!(text.contains(&expected.replacen("DTSTART", "COMPLETED", 1)));
        if name == "floating" {
            assert!(!text.contains("TZID=") && !text.contains("T120000Z"));
        }
    }
}

#[test]
fn recurring_vtodo_attendees_alarms_and_etags_are_deterministic() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let body = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VTODO\r\nUID:recurrence-contract\r\nSUMMARY:Planning\r\n",
        "DTSTART;TZID=Europe/Stockholm:20261025T090000\r\n",
        "DUE:20261025T080000Z\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4\r\n",
        "EXDATE;TZID=Europe/Stockholm:20261102T090000,20261109T090000\r\n",
        "RDATE:20261201T090000\r\n",
        "ATTENDEE;CN=\"Doe, Jane\";PARTSTAT=ACCEPTED:mailto:jane@example.test\r\n",
        "ORGANIZER;CN=\"Planner; Team\":mailto:planner@example.test\r\n",
        "STATUS:NEEDS-ACTION\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=START:-PT15M\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\n",
        "END:VTODO\r\nEND:VCALENDAR\r\n"
    )
    .to_owned();
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/recurrence-contract.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar; charset=utf-8".into())],
        body: body.as_bytes().to_vec(),
    });
    assert_eq!(created.status, 201);
    let etag = created
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("etag"))
        .map(|(_, value)| value.clone())
        .unwrap();
    let fetched = server.handle(req("GET", "/caldav/tasks/recurrence-contract.ics", b""));
    assert_eq!(fetched.status, 200);
    let text = String::from_utf8(fetched.body).unwrap();
    for expected in [
        "DTSTART;TZID=Europe/Stockholm:20261025T090000",
        "DUE:20261025T080000Z",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4",
        "EXDATE;TZID=Europe/Stockholm:20261102T090000,20261109T090000",
        "RDATE:20261201T090000",
        "ATTENDEE;CN=\"Doe, Jane\";PARTSTAT=ACCEPTED:mailto:jane@example.test",
        "ORGANIZER;CN=\"Planner; Team\":mailto:planner@example.test",
        "TRIGGER;RELATED=START:-PT15M",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(
        fetched
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("etag"))
            .map(|(_, value)| value),
        Some(&etag)
    );

    let updated = body.replace("SUMMARY:Planning", "SUMMARY:Planning updated");
    let update = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/recurrence-contract.ics".into(),
        headers: vec![
            ("Content-Type".into(), "text/calendar".into()),
            ("If-Match".into(), etag.clone()),
        ],
        body: updated.as_bytes().to_vec(),
    });
    assert_eq!(update.status, 204);
    let new_etag = update
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("etag"))
        .map(|(_, value)| value.clone())
        .unwrap();
    assert_ne!(new_etag, etag);
    let stale = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/recurrence-contract.ics".into(),
        headers: vec![
            ("Content-Type".into(), "text/calendar".into()),
            ("If-Match".into(), etag),
        ],
        body: body.as_bytes().to_vec(),
    });
    assert_eq!(stale.status, 412);
}

#[test]
fn root_principal_depth_one_narrow_props_is_bounded_noop() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let response = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/principals/users/default".into(),
        headers: vec![("Depth".into(), "0".into())],
        body: b"<prop><current-user-principal/></prop>".to_vec(),
    });
    let xml = String::from_utf8(response.body).unwrap();
    assert_eq!(response.status, 207);
    assert!(xml.contains("current-user-principal"));
    assert!(!xml.contains("calendar-home-set"));
    let rejected = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/".into(),
        headers: vec![("Depth".into(), "infinity".into())],
        body: b"<allprop/>".to_vec(),
    });
    assert_eq!(rejected.status, 400);
    let depth_one = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/".into(),
        headers: vec![("Depth".into(), "1".into())],
        body: b"<prop><calendar-home-set/></prop>".to_vec(),
    });
    let xml = String::from_utf8(depth_one.body).unwrap();
    assert_eq!(depth_one.status, 207);
    assert!(xml.contains("calendar-home-set"));
    assert!(!xml.contains("addressbook-home-set"));
}

#[test]
fn slash_terminated_home_set_collections_are_discoverable() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    for (path, marker) in [
        ("/caldav/tasks/", "calendar-home-set"),
        ("/carddav/contacts/", "addressbook-home-set"),
    ] {
        let response = server.handle(Request {
            method: "PROPFIND".into(),
            path: path.into(),
            headers: vec![("Depth".into(), "0".into())],
            body: b"<allprop/>".to_vec(),
        });
        let xml = String::from_utf8(response.body).unwrap();
        assert_eq!(response.status, 207, "{path}");
        assert!(xml.contains(marker), "{path}: {marker}");
        assert!(
            xml.contains("resourcetype"),
            "{path}: collection properties"
        );
    }
}

#[test]
fn caldav_root_home_set_is_discoverable_after_well_known_redirect() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let redirect = server.handle(req("GET", "/.well-known/caldav", b""));
    assert_eq!(redirect.status, 302);
    assert!(redirect
        .headers
        .iter()
        .any(|(key, value)| key == "Location" && value == "/caldav/"));

    let response = server.handle(Request {
        method: "PROPFIND".into(),
        path: "/caldav/".into(),
        headers: vec![("Depth".into(), "1".into())],
        body: b"<allprop/>".to_vec(),
    });
    let xml = String::from_utf8(response.body).unwrap();
    assert_eq!(response.status, 207);
    assert!(xml.contains("calendar-home-set"));
    assert!(xml.contains("/caldav/"));
    assert!(xml.contains("/caldav/tasks"));
}

fn seed() -> DavServer<MemoryRepository> {
    let mut server = DavServer::new(MemoryRepository::new());
    let contact_fields = BTreeMap::from([
        ("UID".into(), vec![Occurrence::new("uid<&>")]),
        ("FN".into(), vec![Occurrence::new("Alpha <One> & Co")]),
        (
            "TEL".into(),
            vec![Occurrence {
                value: "+461234".into(),
                params: BTreeMap::from([("TYPE".into(), vec!["cell".into()])]),
            }],
        ),
        ("X-OPAQUE".into(), vec![Occurrence::new("kept")]),
    ]);
    let task_fields = |uid: &str, summary: &str, status: &str, due: &str| {
        BTreeMap::from([
            ("UID".into(), vec![Occurrence::new(uid)]),
            ("SUMMARY".into(), vec![Occurrence::new(summary)]),
            ("STATUS".into(), vec![Occurrence::new(status)]),
            ("DUE".into(), vec![Occurrence::new(due)]),
        ])
    };
    let contact = ResourceEnvelope {
        collection_id: server.contacts.clone(),
        resource_id: ResourceId::try_from("c&1").unwrap(),
        kind: DavKind::Contact,
        anytype_object_id: AnytypeObjectId::try_from("o-c1").unwrap(),
        dav_uid: DavUid::try_from("uid<&>").unwrap(),
        document: CanonicalDocument::new(StructuredDocument {
            fields: contact_fields,
        }),
        revision: 0,
    };
    server
        .repository
        .create_resource(contact, WriteCondition::Unconditional)
        .unwrap();
    for (id, uid, summary, status, due) in [
        ("t1", "u-t1", "Today", "NEEDS-ACTION", "20260115"),
        ("t2", "u-t2", "Later", "COMPLETED", "20260315"),
    ] {
        let task = ResourceEnvelope {
            collection_id: server.tasks.clone(),
            resource_id: ResourceId::try_from(id).unwrap(),
            kind: DavKind::Task,
            anytype_object_id: AnytypeObjectId::try_from(format!("o-{id}")).unwrap(),
            dav_uid: DavUid::try_from(uid).unwrap(),
            document: CanonicalDocument::new(StructuredDocument {
                fields: task_fields(uid, summary, status, due),
            }),
            revision: 0,
        };
        server
            .repository
            .create_resource(task, WriteCondition::Unconditional)
            .unwrap();
    }
    server
}

#[test]
fn backend_read_failures_are_not_collapsed_to_not_found() {
    let mut server = seed();
    server.repository.inject_failure(FailureMode::Timeout);
    let get = server.handle(req("GET", "/carddav/contacts/c&1.vcf", b""));
    assert_eq!(get.status, 408);

    server.repository.inject_failure(FailureMode::Timeout);
    let delete = server.handle(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/c&1.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(delete.status, 408);
}

#[test]
fn populated_carddav_query_filters_and_selects_properties() {
    let mut server = seed();
    let body = br#"<addressbook-query><prop><getetag/><address-data/></prop><filter><prop-filter name="FN"><text-match>alpha</text-match></prop-filter></filter></addressbook-query>"#;
    let response = server.handle(req("REPORT", "/carddav/contacts", body));
    assert_eq!(response.status, 207);
    assert_eq!(
        response
            .headers
            .iter()
            .find(|(k, _)| k == "Content-Type")
            .unwrap()
            .1,
        "application/xml"
    );
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("/carddav/contacts/c&amp;1.vcf"));
    assert!(xml.contains("address-data") && xml.contains("getetag"));
    assert!(xml.contains("Alpha &lt;One&gt; &amp; Co"));
    assert!(!xml.contains("/carddav/contacts/c2.vcf"));
}

#[test]
fn carddav_multiget_hrefs_and_requested_property_are_honored() {
    let mut server = seed();
    let body = br#"<addressbook-multiget><prop><getetag/></prop><href>/carddav/contacts/c&amp;1.vcf</href><href>/carddav/contacts/missing.vcf</href></addressbook-multiget>"#;
    let response = server.handle(req("REPORT", "/carddav/contacts", body));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("c&amp;1.vcf") && xml.contains("getetag"));
    assert!(
        !xml.contains("address-data")
            && xml.contains("missing.vcf")
            && xml.contains("404 Not Found")
    );
}

#[test]
fn malformed_property_reports_and_media_types_are_fail_closed() {
    let mut server = seed();
    for body in [
        b"<addressbook-query><prop>".as_slice(),
        b"<calendar-query><filter><prop-filter name=\"STATUS\"></filter></calendar-query>"
            .as_slice(),
    ] {
        let response = server.handle(req("REPORT", "/carddav/contacts", body));
        assert_eq!(response.status, 400);
    }
    let response = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/new.vcf".into(),
        headers: vec![("Content-Type".into(), "application/octet-stream".into())],
        body: b"BEGIN:VCARD\nUID:new\nFN:New\nEND:VCARD\n".to_vec(),
    });
    assert_eq!(response.status, 415);
    assert!(server
        .repository
        .get_resource(&ResourceId::try_from("new").unwrap())
        .unwrap()
        .is_none());
}

#[test]
fn stale_conditional_delete_does_not_mutate_resource() {
    let mut server = seed();
    let get = server.handle(req("GET", "/carddav/contacts/c&1.vcf", b""));
    let etag = get
        .headers
        .iter()
        .find(|(key, _)| key == "ETag")
        .unwrap()
        .1
        .clone();
    let stale = server.handle(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/c&1.vcf".into(),
        headers: vec![("If-Match".into(), "\"stale\"".into())],
        body: vec![],
    });
    assert_eq!(stale.status, 412);
    let still_there = server.handle(req("GET", "/carddav/contacts/c&1.vcf", b""));
    assert_eq!(still_there.status, 200);
    assert_eq!(
        still_there
            .headers
            .iter()
            .find(|(key, _)| key == "ETag")
            .unwrap()
            .1,
        etag
    );
}

#[test]
fn populated_caldav_queries_filter_status_and_time_and_multiget_is_empty_for_absence() {
    let mut server = seed();
    let query = br#"<calendar-query><prop><calendar-data/></prop><filter><comp-filter name="VTODO"><prop-filter name="STATUS"><text-match>needs-action</text-match></prop-filter><prop-filter name="DUE"><time-range start="20260101" end="20260131"></time-range></prop-filter></comp-filter></filter></calendar-query>"#;
    let response = server.handle(req("REPORT", "/caldav/tasks", query));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("t1.ics") && xml.contains("calendar-data"));
    assert!(!xml.contains("t2.ics") && !xml.contains("getetag"));

    let multiget = br#"<calendar-multiget><prop><getetag/></prop><href>/caldav/tasks/nope.ics</href></calendar-multiget>"#;
    let empty = server.handle(req("REPORT", "/caldav/tasks", multiget));
    assert_eq!(empty.status, 207);
    let empty_xml = String::from_utf8(empty.body).unwrap();
    assert!(empty_xml.contains("nope.ics") && empty_xml.contains("404 Not Found"));
}

#[test]
fn caldav_time_ranges_use_inclusive_start_exclusive_end_and_normalize_utc_floating_values() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    server.repository.set_clock_seconds(1_800_000_000);
    for (id, uid, due) in [
        ("range-start", "range-start", "20260101T000000Z"),
        ("range-middle", "range-middle", "20260115T120000"),
        ("range-end", "range-end", "20260131T000000Z"),
    ] {
        let body = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:{id}\r\nDUE:{due}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        );
        let response = server.handle(Request {
            method: "PUT".into(),
            path: format!("/caldav/tasks/{id}.ics"),
            headers: vec![("Content-Type".into(), "text/calendar".into())],
            body: body.into_bytes(),
        });
        assert_eq!(response.status, 201);
    }

    let query = br#"<calendar-query><prop><getetag/></prop><filter><comp-filter name="VTODO"><prop-filter name="DUE"><time-range start="20260101T000000Z" end="20260131T000000Z"/></prop-filter></comp-filter></filter></calendar-query>"#;
    let response = server.handle(req("REPORT", "/caldav/tasks", query));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("range-start.ics") && xml.contains("range-middle.ics"));
    assert!(!xml.contains("range-end.ics"));

    let exact_floating = br#"<calendar-query><prop><getetag/></prop><filter><comp-filter name="VTODO"><prop-filter name="DUE"><time-range start="20260115T120000" end="20260115T120001Z"/></prop-filter></comp-filter></filter></calendar-query>"#;
    let response = server.handle(req("REPORT", "/caldav/tasks", exact_floating));
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("range-middle.ics"));
    assert!(!xml.contains("range-start.ics") && !xml.contains("range-end.ics"));
}

#[test]
fn malformed_or_unsupported_time_ranges_fail_closed_and_empty_ranges_are_deterministic() {
    let mut server = seed();
    for query in [
        br#"<calendar-query><filter><comp-filter name="VTODO"><prop-filter name="DUE"><time-range start="20260132" end="20260201"/></prop-filter></comp-filter></filter></calendar-query>"#.as_slice(),
        br#"<calendar-query><filter><comp-filter name="VTODO"><prop-filter name="DUE"><time-range start="20260201" end="20260101"/></prop-filter></comp-filter></filter></calendar-query>"#.as_slice(),
    ] {
        assert_eq!(
            server
                .handle(req("REPORT", "/caldav/tasks", query))
                .status,
            400
        );
    }

    let empty = br#"<calendar-query><prop><getetag/></prop><filter><comp-filter name="VTODO"><prop-filter name="DUE"><time-range start="20270101" end="20270201"/></prop-filter></comp-filter></filter></calendar-query>"#;
    let first = server.handle(req("REPORT", "/caldav/tasks", empty));
    let second = server.handle(req("REPORT", "/caldav/tasks", empty));
    assert_eq!(first.status, 207);
    assert_eq!(first.body, second.body);
    let xml = String::from_utf8(first.body).unwrap();
    assert!(!xml.contains(".ics"));

    let carddav_range = br#"<addressbook-query><prop><getetag/></prop><filter><prop-filter name="FN"><time-range start="20260101" end="20260201"/></prop-filter></filter></addressbook-query>"#;
    assert_eq!(
        server
            .handle(req("REPORT", "/carddav/contacts", carddav_range))
            .status,
        400
    );
}

#[test]
fn report_date_filter_uses_component_dates_not_modified_at_and_preserves_etag_ordering() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    server.repository.set_clock_seconds(1_800_000_000);
    let body = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:stable-date\r\nSUMMARY:Stable\r\nDUE:20260115\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/stable-date.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: body.to_vec(),
    });
    let etag = response_header(&created, "ETag").unwrap().to_owned();
    let modified = response_header(&created, "Last-Modified")
        .unwrap()
        .to_owned();
    server.repository.set_clock_seconds(1_900_000_000);
    let updated = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/stable-date.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: body.to_vec(),
    });
    assert_eq!(updated.status, 204);
    let updated_etag = response_header(&updated, "ETag").unwrap().to_owned();
    assert_ne!(updated_etag, etag);
    assert_ne!(
        response_header(&updated, "Last-Modified"),
        Some(modified.as_str())
    );

    let query = br#"<calendar-query><prop><getetag/></prop><filter><comp-filter name="VTODO"><prop-filter name="DUE"><time-range start="20260101" end="20260201"/></prop-filter></comp-filter></filter></calendar-query>"#;
    let report = server.handle(req("REPORT", "/caldav/tasks", query));
    let xml = String::from_utf8(report.body).unwrap();
    assert!(xml.contains("stable-date.ics") && xml.contains(&updated_etag));
}

#[test]
fn scheduling_attendees_round_trip_and_invalid_update_does_not_mutate() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    server.repository.set_clock_seconds(1_800_000_000);
    let body = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VTODO\r\nUID:scheduling-contract\r\nSUMMARY:Review\r\n",
        "DTSTAMP:20260920T120000Z\r\nSEQUENCE:7\r\nSTATUS:NEEDS-ACTION\r\n",
        "ORGANIZER;CN=\"Planner; Team\";ROLE=CHAIR:mailto:planner@example.test\r\n",
        "ATTENDEE;CN=\"Doe, Jane\";RSVP=TRUE;PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT;CUTYPE=INDIVIDUAL:mailto:jane@example.test\r\n",
        "ATTENDEE;CN=\"Smith; Alex\";RSVP=FALSE;PARTSTAT=DECLINED;ROLE=OPT-PARTICIPANT;CUTYPE=GROUP:mailto:team@example.test\r\n",
        "END:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/scheduling-contract.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: body.as_bytes().to_vec(),
    });
    assert_eq!(created.status, 201);
    let original_etag = response_header(&created, "ETag").unwrap().to_owned();
    let original_modified = response_header(&created, "Last-Modified")
        .unwrap()
        .to_owned();
    let fetched = server.handle(req("GET", "/caldav/tasks/scheduling-contract.ics", b""));
    assert_eq!(fetched.status, 200);
    let text = String::from_utf8(fetched.body).unwrap();
    for expected in [
        "DTSTAMP:20260920T120000Z",
        "SEQUENCE:7",
        "STATUS:NEEDS-ACTION",
        "ORGANIZER;CN=\"Planner; Team\";ROLE=CHAIR:mailto:planner@example.test",
        "ATTENDEE;CN=\"Doe, Jane\";CUTYPE=INDIVIDUAL;PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT;RSVP=TRUE:mailto:jane@example.test",
        "ATTENDEE;CN=\"Smith; Alex\";CUTYPE=GROUP;PARTSTAT=DECLINED;ROLE=OPT-PARTICIPANT;RSVP=FALSE:mailto:team@example.test",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }

    let malformed = body.replace(
        "SUMMARY:Review",
        "SUMMARY:Rejected\r\nATTENDEE;CN=\"unterminated:mailto:bad@example.test",
    );
    let rejected = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/scheduling-contract.ics".into(),
        headers: vec![
            ("Content-Type".into(), "text/calendar".into()),
            ("If-Match".into(), original_etag.clone()),
        ],
        body: malformed.into_bytes(),
    });
    assert_eq!(rejected.status, 400);
    let unchanged = server.handle(req("GET", "/caldav/tasks/scheduling-contract.ics", b""));
    assert_eq!(unchanged.status, 200);
    assert_eq!(
        response_header(&unchanged, "ETag"),
        Some(original_etag.as_str())
    );
    assert_eq!(
        response_header(&unchanged, "Last-Modified"),
        Some(original_modified.as_str())
    );
    assert!(String::from_utf8(unchanged.body)
        .unwrap()
        .contains("SUMMARY:Review"));
}

#[test]
fn namespaced_query_reports_are_accepted_and_preserve_requested_properties() {
    let mut server = seed();
    let body = br#"<c:addressbook-query xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:d="DAV:"><d:prop><d:getetag/><c:address-data/></d:prop><c:filter><c:prop-filter name="FN"><c:text-match>alpha</c:text-match></c:prop-filter></c:filter></c:addressbook-query>"#;
    let response = server.handle(req("REPORT", "/carddav/contacts", body));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("c&amp;1.vcf"));
    assert!(xml.contains("address-data") && xml.contains("getetag"));
    assert!(!xml.contains("c2.vcf"));
}

#[test]
fn empty_property_filters_mean_property_presence() {
    let mut server = seed();
    let body = br#"<addressbook-query><prop><getetag/></prop><filter><prop-filter name="FN"/></filter></addressbook-query>"#;
    let response = server.handle(req("REPORT", "/carddav/contacts", body));
    assert_eq!(response.status, 207);
    assert!(String::from_utf8(response.body)
        .unwrap()
        .contains("c&amp;1.vcf"));
}

#[test]
fn carddav_filters_match_uid_and_repeated_labelled_values_deterministically() {
    let mut server = seed();
    for (resource_id, uid, name, phones) in [
        (
            "c2",
            "uid-two",
            "Beta Two",
            vec![
                Occurrence {
                    value: "+46001".into(),
                    params: BTreeMap::from([(String::from("TYPE"), vec![String::from("home")])]),
                },
                Occurrence {
                    value: "+46002".into(),
                    params: BTreeMap::from([(String::from("TYPE"), vec![String::from("work")])]),
                },
            ],
        ),
        (
            "c0",
            "uid-zero",
            "Gamma Zero",
            vec![Occurrence::new("+46000")],
        ),
    ] {
        let mut fields = BTreeMap::from([
            ("UID".into(), vec![Occurrence::new(uid)]),
            ("FN".into(), vec![Occurrence::new(name)]),
            ("TEL".into(), phones),
        ]);
        fields.insert("CATEGORIES".into(), vec![Occurrence::new("crm")]);
        server
            .repository
            .create_resource(
                ResourceEnvelope {
                    collection_id: server.contacts.clone(),
                    resource_id: ResourceId::try_from(resource_id).unwrap(),
                    kind: DavKind::Contact,
                    anytype_object_id: AnytypeObjectId::try_from(format!("o-{resource_id}"))
                        .unwrap(),
                    dav_uid: DavUid::try_from(uid).unwrap(),
                    document: CanonicalDocument::new(StructuredDocument { fields }),
                    revision: 0,
                },
                WriteCondition::Unconditional,
            )
            .unwrap();
    }

    let repeated = br#"<addressbook-query><prop><getetag/><address-data/></prop><filter><prop-filter name="TEL"><text-match>46002</text-match></prop-filter></filter></addressbook-query>"#;
    let response = server.handle(req("REPORT", "/carddav/contacts", repeated));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("c2.vcf"));
    assert!(!xml.contains("c0.vcf") && !xml.contains("c&amp;1.vcf"));
    assert!(xml.contains("+46001") && xml.contains("+46002"));

    let uid = br#"<addressbook-query><prop><getetag/></prop><filter><prop-filter name="UID"><text-match>uid-two</text-match></prop-filter></filter></addressbook-query>"#;
    let response = server.handle(req("REPORT", "/carddav/contacts", uid));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("c2.vcf"));
    assert!(!xml.contains("c0.vcf") && !xml.contains("c&amp;1.vcf"));

    let all = server.handle(req(
        "REPORT",
        "/carddav/contacts",
        b"<addressbook-query><prop><getetag/></prop></addressbook-query>",
    ));
    let xml = String::from_utf8(all.body).unwrap();
    assert!(xml.find("c&amp;1.vcf").unwrap() < xml.find("c0.vcf").unwrap());
    assert!(xml.find("c0.vcf").unwrap() < xml.find("c2.vcf").unwrap());
}

#[test]
fn empty_carddav_filter_is_rejected_without_broadening_query() {
    let mut server = seed();
    for body in [
        br#"<addressbook-query><prop><getetag/></prop><filter/></addressbook-query>"#.as_slice(),
        br#"<addressbook-query><prop><getetag/></prop><filter></filter></addressbook-query>"#
            .as_slice(),
    ] {
        let response = server.handle(req("REPORT", "/carddav/contacts", body));
        assert_eq!(response.status, 400);
    }
}

#[test]
fn report_hrefs_are_stably_ordered_by_resource_id() {
    let mut server = seed();
    let response = server.handle(req(
        "REPORT",
        "/caldav/tasks",
        b"<calendar-query><prop><getetag/></prop></calendar-query>",
    ));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.find("t1.ics").unwrap() < xml.find("t2.ics").unwrap());
}

#[test]
fn report_projects_dav_metadata_and_mixed_resource_statuses_truthfully() {
    let mut server = seed();
    let archived = ResourceId::try_from("t2").unwrap();
    server
        .repository
        .archive_resource(&archived, WriteCondition::Unconditional)
        .unwrap();

    let query = br#"<calendar-multiget><prop><getetag/><getlastmodified/><getcontenttype/><getcontentlength/></prop><href>/caldav/tasks/t1.ics</href><href>/caldav/tasks/t2.ics</href><href>/caldav/tasks/missing.ics</href></calendar-multiget>"#;
    let response = server.handle(req("REPORT", "/caldav/tasks", query));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("t1.ics"));
    assert!(xml.contains("<getetag>"));
    assert!(xml.contains("<getlastmodified>"));
    assert!(xml.contains("<getcontenttype>text/calendar</getcontenttype>"));
    assert!(xml.contains("<getcontentlength>"));
    assert!(xml.contains("t2.ics") && xml.contains("missing.ics"));
    assert!(xml.matches("404 Not Found").count() == 2);
    assert!(xml.find("t1.ics").unwrap() < xml.find("t2.ics").unwrap());
    assert!(xml.find("t2.ics").unwrap() < xml.find("missing.ics").unwrap());
}

#[test]
fn report_malformed_repository_state_is_not_projected_as_an_empty_success() {
    let mut server = seed();
    server
        .repository
        .inject_failure(FailureMode::MalformedState);
    let response = server.handle(req(
        "REPORT",
        "/caldav/tasks",
        b"<calendar-query><prop><getetag/></prop></calendar-query>",
    ));
    assert_eq!(response.status, 500);
    assert!(String::from_utf8(response.body)
        .unwrap()
        .contains("internal server error"));
}

#[test]
fn calendar_component_filters_are_honored_for_opaque_trees() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let body = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VTODO\r\nUID:todo-filter\r\nSUMMARY:Keep\r\nEND:VTODO\r\n",
        "END:VCALENDAR\r\n"
    );
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/todo-filter.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: body.as_bytes().to_vec(),
    });
    assert_eq!(created.status, 201);
    let todo = br#"<c:calendar-query xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop xmlns:d="DAV:"><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VTODO"/></c:filter></c:calendar-query>"#;
    let event = br#"<c:calendar-query xmlns:c="urn:ietf:params:xml:ns:caldav"><c:filter><c:comp-filter name="VEVENT"/></c:filter></c:calendar-query>"#;
    assert_eq!(
        server.handle(req("REPORT", "/caldav/tasks", todo)).status,
        207
    );
    let todo_xml =
        String::from_utf8(server.handle(req("REPORT", "/caldav/tasks", todo)).body).unwrap();
    assert!(todo_xml.contains("todo-filter.ics"));
    let event_xml =
        String::from_utf8(server.handle(req("REPORT", "/caldav/tasks", event)).body).unwrap();
    assert!(!event_xml.contains("todo-filter.ics"));
}

#[test]
fn propfind_depth_and_property_selection_are_scoped_and_capabilities_are_truthful() {
    let mut server = seed();
    let mut collection = req(
        "PROPFIND",
        "/carddav/contacts",
        b"<prop><displayname/><getetag/></prop>",
    );
    collection.headers.push(("Depth".into(), "1".into()));
    let response = server.handle(collection);
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("displayname") && xml.contains("c&amp;1.vcf"));
    assert!(!xml.contains("address-data") && !xml.contains("getcontentlength"));

    let mut resource = req("PROPFIND", "/carddav/contacts/c&1.vcf", b"<allprop/>");
    resource.headers.push(("Depth".into(), "0".into()));
    let resource_response = server.handle(resource);
    assert_eq!(resource_response.status, 207);
    let resource_xml = String::from_utf8(resource_response.body).unwrap();
    assert!(resource_xml.contains("getetag") && resource_xml.contains("getcontenttype"));
    assert!(!resource_xml.contains("/carddav/contacts/c&1.vcf"));

    let card_options = server.handle(req("OPTIONS", "/carddav/contacts", b""));
    let card_dav = card_options
        .headers
        .iter()
        .find(|(k, _)| k == "DAV")
        .unwrap()
        .1
        .clone();
    assert!(card_dav.contains("addressbook") && !card_dav.contains("calendar-access"));
    assert_eq!(response_header(&card_options, "Accept"), Some("text/vcard"));
    assert_eq!(
        response_header(&card_options, "Content-Type"),
        Some("application/xml; charset=utf-8")
    );
    assert_eq!(
        response_header(&card_options, "Cache-Control"),
        Some("no-store")
    );
    let task_options = server.handle(req("OPTIONS", "/caldav/tasks", b""));
    let task_dav = task_options
        .headers
        .iter()
        .find(|(k, _)| k == "DAV")
        .unwrap()
        .1
        .clone();
    assert!(task_dav.contains("calendar-access") && !task_dav.contains("addressbook"));
    assert_eq!(
        response_header(&task_options, "Accept"),
        Some("text/calendar")
    );
    assert_eq!(server.handle(req("OPTIONS", "/unrelated", b"")).status, 404);
}

#[test]
fn propfind_property_namespaces_propname_and_unknowns_are_bounded() {
    let mut server = seed();

    // Prefixes are client-selected; a namespaced explicit request must still
    // select the DAV properties advertised by the resource.
    let explicit = server.handle(req(
        "PROPFIND",
        "/carddav/contacts/c&1.vcf",
        br#"<d:prop xmlns:d="DAV:"><d:getetag/><d:getcontenttype/><d:not-supported/></d:prop>"#,
    ));
    assert_eq!(explicit.status, 207);
    let explicit_xml = String::from_utf8(explicit.body).unwrap();
    assert!(explicit_xml.contains("<getetag>") && explicit_xml.contains("<getcontenttype>"));
    assert!(explicit_xml.contains("<not-supported/>") && explicit_xml.contains("404 Not Found"));
    assert_eq!(explicit_xml.matches("<propstat>").count(), 2);

    // propname returns the supported property vocabulary as empty elements,
    // rather than accidentally treating the request as an empty <prop>.
    let names = server.handle(req(
        "PROPFIND",
        "/carddav/contacts",
        br#"<d:propname xmlns:d="DAV:"/>"#,
    ));
    assert_eq!(names.status, 207);
    let names_xml = String::from_utf8(names.body).unwrap();
    for property in [
        "resourcetype",
        "displayname",
        "current-user-principal",
        "addressbook-home-set",
        "d:supported-report-set",
    ] {
        assert!(
            names_xml.contains(&format!("<{property}/>")),
            "missing {property}: {names_xml}"
        );
    }
    assert!(!names_xml.contains("<getetag/>") && !names_xml.contains("404 Not Found"));
}

#[test]
fn report_explicit_unknown_property_gets_per_resource_not_found() {
    let mut server = seed();
    let response = server.handle(req(
        "REPORT",
        "/carddav/contacts",
        br#"<c:addressbook-query xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop xmlns:d="DAV:"><d:getetag/><d:vendor-only/></d:prop></c:addressbook-query>"#,
    ));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("<getetag>") && xml.contains("<vendor-only />"));
    assert!(xml.contains("404 Not Found"));
}

#[test]
fn allprop_advertises_only_supported_reports_with_structured_elements() {
    let mut server = seed();
    let card = server.handle(req("PROPFIND", "/carddav/contacts", b"<allprop/>"));
    let card_xml = String::from_utf8(card.body).unwrap();
    assert_eq!(card_xml.matches("<d:report>").count(), 2);
    assert!(
        card_xml.contains("<c:addressbook-query/>")
            && card_xml.contains("<c:addressbook-multiget/>")
    );
    assert!(!card_xml.contains("calendar-query") && !card_xml.contains("sync-collection"));
    let task = server.handle(req("PROPFIND", "/caldav/tasks", b"<allprop/>"));
    let task_xml = String::from_utf8(task.body).unwrap();
    assert_eq!(task_xml.matches("<d:report>").count(), 2);
    assert!(
        task_xml.contains("<c:calendar-query/>") && task_xml.contains("<c:calendar-multiget/>")
    );
    assert!(!task_xml.contains("VEVENT") && !task_xml.contains("sync-collection"));
}

#[test]
fn mixed_calendar_components_survive_repository_envelope_and_report() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let body = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Any-Cal//EN\r\n",
        "BEGIN:VTIMEZONE\r\nTZID:Europe/Stockholm\r\n",
        "BEGIN:STANDARD\r\nDTSTART:20261025T010000\r\n",
        "X-TZ-OPAQUE:keep\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n",
        "BEGIN:VEVENT\r\nUID:event-opaque\r\nSUMMARY:Planning\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n",
        "X-EVENT-OPAQUE:keep\r\nEND:VEVENT\r\n",
        "BEGIN:VTODO\r\nUID:task-opaque\r\nSUMMARY:Follow up\r\n",
        "END:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/caldav/tasks/task-opaque.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: body.as_bytes().to_vec(),
    });
    assert_eq!(created.status, 201);

    // The production repository envelope, rather than a test-only parser,
    // owns the opaque tree used for subsequent DAV reads.
    let rows = server
        .repository
        .list_resources(&server.tasks.clone(), false)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let envelope = &rows[0].envelope;
    let canonical = envelope.canonical_json().unwrap();
    assert_eq!(
        rows[0].etag,
        typed_etag_for_bytes(canonical.as_bytes()),
        "repository ETag must include the persisted component tree"
    );
    let tree = envelope.document.opaque_calendar.as_ref().unwrap();
    assert_eq!(
        tree.root()
            .entries
            .iter()
            .filter_map(|entry| match entry {
                any_cal_core::ical::Entry::Component(component) => Some(component.name.as_str()),
                any_cal_core::ical::Entry::Property(_) => None,
            })
            .collect::<Vec<_>>(),
        ["VTIMEZONE", "VEVENT", "VTODO"]
    );

    let fetched = server.handle(req("GET", "/caldav/tasks/task-opaque.ics", b""));
    let fetched_body = String::from_utf8(fetched.body).unwrap();
    for marker in [
        "BEGIN:VTIMEZONE",
        "X-TZ-OPAQUE:keep",
        "BEGIN:VEVENT",
        "BEGIN:VALARM",
        "X-EVENT-OPAQUE:keep",
        "BEGIN:VTODO",
    ] {
        assert!(fetched_body.contains(marker), "missing {marker}");
    }

    let report = server.handle(req(
        "REPORT",
        "/caldav/tasks",
        b"<calendar-query><prop><calendar-data/><getetag/></prop></calendar-query>",
    ));
    assert_eq!(report.status, 207);
    let report_body = String::from_utf8(report.body).unwrap();
    assert!(report_body.contains("<multistatus"));
    assert!(report_body.contains("<calendar-data"));
    assert!(report_body.contains("BEGIN:VALARM") && report_body.contains("BEGIN:VEVENT"));
    let serialized_tree = tree.serialize();

    let deleted = server.handle(Request {
        method: "DELETE".into(),
        path: "/caldav/tasks/task-opaque.ics".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(deleted.status, 204);
    let archived = server
        .repository
        .list_resources(&server.tasks.clone(), true)
        .unwrap();
    assert!(archived[0].archived);
    assert_eq!(
        archived[0]
            .envelope
            .document
            .opaque_calendar
            .as_ref()
            .unwrap()
            .serialize(),
        serialized_tree
    );
}

#[test]
fn tasks_collection_advertises_vtodo_without_claiming_vevent_support() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let response = server.handle(req("PROPFIND", "/caldav/tasks", b"<allprop/>"));
    assert_eq!(response.status, 207);
    let xml = String::from_utf8(response.body).unwrap();
    assert!(xml.contains("<comp name=\"VTODO\"/>"));
    assert!(!xml.contains("<comp name=\"VEVENT\"/>"));
}

#[test]
fn lifecycle_put_get_conditional_update_and_archive() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let vcard = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:life-1\r\nFN:Lifecycle\r\nEND:VCARD\r\n";
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/life.vcf".into(),
        headers: vec![
            ("If-None-Match".into(), "*".into()),
            ("Content-Type".into(), "text/vcard; charset=utf-8".into()),
        ],
        body: vcard.to_vec(),
    });
    assert_eq!(created.status, 201);
    let etag = created
        .headers
        .iter()
        .find(|(k, _)| k == "ETag")
        .unwrap()
        .1
        .clone();
    assert!(created
        .headers
        .iter()
        .any(|(k, v)| k == "Location" && v.ends_with("life.vcf")));
    let fetched = server.handle(req("GET", "/carddav/contacts/life.vcf", b""));
    assert_eq!(fetched.status, 200);
    assert!(String::from_utf8(fetched.body)
        .unwrap()
        .contains("UID:life-1"));
    let duplicate = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/life.vcf".into(),
        headers: vec![
            ("If-None-Match".into(), "*".into()),
            ("Content-Type".into(), "text/vcard".into()),
        ],
        body: vcard.to_vec(),
    });
    assert_eq!(duplicate.status, 412);
    let updated = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/life.vcf".into(),
        headers: vec![
            ("If-Match".into(), etag.clone()),
            ("Content-Type".into(), "text/vcard".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:life-1\r\nFN:Updated\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(updated.status, 204);
    assert!(
        server
            .handle(Request {
                method: "PUT".into(),
                path: "/carddav/contacts/life.vcf".into(),
                headers: vec![
                    ("If-Match".into(), etag),
                    ("Content-Type".into(), "text/vcard".into())
                ],
                body: vcard.to_vec()
            })
            .status
            == 412
    );
    assert_eq!(
        server
            .handle(Request {
                method: "DELETE".into(),
                path: "/carddav/contacts/life.vcf".into(),
                headers: vec![],
                body: vec![]
            })
            .status,
        204
    );
    assert_eq!(
        server
            .handle(req("GET", "/carddav/contacts/life.vcf", b""))
            .status,
        404
    );
}

#[test]
fn put_keeps_server_generated_anytype_id_separate_from_dav_uid() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let resource_id = ResourceId::try_from("remote-identity").unwrap();
    server
        .repository
        .create_resource(
            ResourceEnvelope {
                collection_id: server.contacts.clone(),
                resource_id: resource_id.clone(),
                kind: DavKind::Contact,
                anytype_object_id: AnytypeObjectId::try_from("anytype-server-id").unwrap(),
                dav_uid: DavUid::try_from("stable-dav-uid").unwrap(),
                document: CanonicalDocument::new(StructuredDocument {
                    fields: BTreeMap::from([
                        ("UID".into(), vec![Occurrence::new("stable-dav-uid")]),
                        ("FN".into(), vec![Occurrence::new("Before")]),
                    ]),
                }),
                revision: 0,
            },
            WriteCondition::Unconditional,
        )
        .unwrap();

    let updated = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/remote-identity.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:stable-dav-uid\r\nFN:After\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(updated.status, 204);
    let stored = server
        .repository
        .get_resource(&resource_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.envelope.anytype_object_id.as_str(),
        "anytype-server-id"
    );
    assert_eq!(stored.envelope.dav_uid.as_str(), "stable-dav-uid");
    assert_eq!(
        stored.envelope.document.content.fields["FN"][0].value,
        "After"
    );

    let uid_change = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/remote-identity.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:changed-dav-uid\r\nFN:Rejected\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(uid_change.status, 409);
}

#[test]
fn if_match_wildcard_updates_and_deletes_existing_resources() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:wildcard-1\r\nFN:Before\r\nEND:VCARD\r\n";
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/wildcard.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: body.to_vec(),
    });
    assert_eq!(created.status, 201);

    let updated = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/wildcard.vcf".into(),
        headers: vec![
            ("If-Match".into(), "*".into()),
            ("Content-Type".into(), "text/vcard".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:wildcard-1\r\nFN:After\r\nEND:VCARD\r\n".to_vec(),
    });
    assert_eq!(updated.status, 204);
    assert!(String::from_utf8(
        server
            .handle(req("GET", "/carddav/contacts/wildcard.vcf", b""))
            .body,
    )
    .unwrap()
    .contains("FN:After"));

    let deleted = server.handle(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/wildcard.vcf".into(),
        headers: vec![("If-Match".into(), "*".into())],
        body: vec![],
    });
    assert_eq!(deleted.status, 204);
    assert_eq!(
        server
            .handle(req("GET", "/carddav/contacts/wildcard.vcf", b""))
            .status,
        404
    );
}

#[test]
fn head_and_etag_preconditions_match_get_without_mutating() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:conditional-read\r\nFN:Before\r\nEND:VCARD\r\n";
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/conditional-read.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: body.to_vec(),
    });
    assert_eq!(created.status, 201);
    let etag = response_header(&created, "ETag").unwrap().to_owned();

    let head = server.handle(Request {
        method: "HEAD".into(),
        path: "/carddav/contacts/conditional-read.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert_eq!(response_header(&head, "ETag"), Some(etag.as_str()));

    let not_modified = server.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/conditional-read.vcf".into(),
        headers: vec![("If-None-Match".into(), etag.clone())],
        body: vec![],
    });
    assert_eq!(not_modified.status, 304);
    assert!(not_modified.body.is_empty());
    assert_eq!(response_header(&not_modified, "ETag"), Some(etag.as_str()));

    let stale_head = server.handle(Request {
        method: "HEAD".into(),
        path: "/carddav/contacts/conditional-read.vcf".into(),
        headers: vec![("If-Match".into(), "\"stale\"".into())],
        body: vec![],
    });
    assert_eq!(stale_head.status, 412);
    assert!(stale_head.body.is_empty());

    let unchanged = server.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/conditional-read.vcf".into(),
        headers: vec![],
        body: vec![],
    });
    assert_eq!(unchanged.status, 200);
    assert!(String::from_utf8_lossy(&unchanged.body).contains("FN:Before"));
}

#[test]
fn date_preconditions_are_second_precision_and_etag_precedence_is_explicit() {
    let mut repository = MemoryRepository::new();
    repository.set_clock_seconds(1_700_000_000);
    let mut server = DavServer::try_new(repository).unwrap();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:date-conditional\r\nFN:Before\r\nEND:VCARD\r\n";
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/date-conditional.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: body.to_vec(),
    });
    assert_eq!(created.status, 201);
    let modified = response_header(&created, "Last-Modified")
        .unwrap()
        .to_owned();
    let etag = response_header(&created, "ETag").unwrap().to_owned();

    let not_modified = server.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/date-conditional.vcf".into(),
        headers: vec![("If-Modified-Since".into(), modified.clone())],
        body: vec![],
    });
    assert_eq!(not_modified.status, 304);
    assert_eq!(
        response_header(&not_modified, "Last-Modified"),
        Some(modified.as_str())
    );

    // Presence of If-None-Match makes the date condition secondary.  A
    // non-matching ETag therefore returns the representation even with a
    // future date.
    let etag_wins = server.handle(Request {
        method: "GET".into(),
        path: "/carddav/contacts/date-conditional.vcf".into(),
        headers: vec![
            ("If-None-Match".into(), "\"stale\"".into()),
            (
                "If-Modified-Since".into(),
                "Wed, 21 Oct 2099 07:28:00 GMT".into(),
            ),
        ],
        body: vec![],
    });
    assert_eq!(etag_wins.status, 200);
    assert_eq!(response_header(&etag_wins, "ETag"), Some(etag.as_str()));

    let stale_write = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/date-conditional.vcf".into(),
        headers: vec![
            ("Content-Type".into(), "text/vcard".into()),
            (
                "If-Unmodified-Since".into(),
                "Thu, 01 Jan 1970 00:00:00 GMT".into(),
            ),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:date-conditional\r\nFN:Rejected\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(stale_write.status, 412);
    assert!(String::from_utf8_lossy(
        &server
            .handle(req("GET", "/carddav/contacts/date-conditional.vcf", b""))
            .body
    )
    .contains("FN:Before"));

    // A malformed date is ignored, while a backwards wall clock is clamped
    // to a strictly newer whole-second value.
    server.repository.set_clock_seconds(1_600_000_000);
    let updated = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/date-conditional.vcf".into(),
        headers: vec![
            ("Content-Type".into(), "text/vcard".into()),
            ("If-Unmodified-Since".into(), "not-a-date".into()),
        ],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:date-conditional\r\nFN:After\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(updated.status, 204);
    assert_ne!(response_header(&updated, "Last-Modified"), None);
}

#[test]
fn date_precondition_matrix_covers_head_delete_report_and_clock_skew() {
    let mut repository = MemoryRepository::new();
    repository.set_clock_seconds(1_700_000_000);
    let mut server = DavServer::try_new(repository).unwrap();
    let path = "/carddav/contacts/date-matrix.vcf";
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:date-matrix\r\nFN:Before\r\nEND:VCARD\r\n";
    let created = server.handle(Request {
        method: "PUT".into(),
        path: path.into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: body.to_vec(),
    });
    assert_eq!(created.status, 201);
    let modified = response_header(&created, "Last-Modified")
        .expect("created resources expose durable Last-Modified")
        .to_owned();

    // Safe retrieval treats an exact or future date as not modified, while
    // an older, malformed, or invalid calendar date returns the resource.
    for date in [modified.as_str(), "Thu, 01 Jan 2099 00:00:00 GMT"] {
        let response = server.handle(Request {
            method: "HEAD".into(),
            path: path.into(),
            headers: vec![("If-Modified-Since".into(), date.into())],
            body: vec![],
        });
        assert_eq!(response.status, 304, "date={date}");
        assert!(response.body.is_empty());
    }
    for date in [
        "Wed, 21 Oct 2015 07:28:00 GMT",
        "not-a-date",
        "Wed, 31 Feb 2099 07:28:00 GMT",
    ] {
        let response = server.handle(Request {
            method: "HEAD".into(),
            path: path.into(),
            headers: vec![("If-Modified-Since".into(), date.into())],
            body: vec![],
        });
        assert_eq!(response.status, 200, "date={date}");
        assert!(response.body.is_empty());
        assert_eq!(
            response_header(&response, "Last-Modified"),
            Some(modified.as_str())
        );
    }

    // A stale date rejects DELETE before archive; a future date permits it.
    let rejected = server.handle(Request {
        method: "DELETE".into(),
        path: path.into(),
        headers: vec![(
            "If-Unmodified-Since".into(),
            "Thu, 01 Jan 1970 00:00:00 GMT".into(),
        )],
        body: vec![],
    });
    assert_eq!(rejected.status, 412);
    assert_eq!(
        server.handle(req("GET", path, b"")).status,
        200,
        "rejected DELETE must not archive the resource"
    );
    let deleted = server.handle(Request {
        method: "DELETE".into(),
        path: path.into(),
        headers: vec![(
            "If-Unmodified-Since".into(),
            "Thu, 01 Jan 2099 00:00:00 GMT".into(),
        )],
        body: vec![],
    });
    assert_eq!(deleted.status, 204);
    assert_eq!(server.handle(req("GET", path, b"")).status, 404);

    // REPORT is a collection query and has no representation-level date
    // validator.  Date headers must not turn a successful report into a 304
    // or mutate the collection; this remains true for a malformed date.
    let report = server.handle(Request {
        method: "REPORT".into(),
        path: "/carddav/contacts".into(),
        headers: vec![(
            "If-Modified-Since".into(),
            "Thu, 01 Jan 2099 00:00:00 GMT".into(),
        )],
        body: b"<addressbook-query><prop><getetag/></prop></addressbook-query>".to_vec(),
    });
    assert_eq!(report.status, 207);
    assert!(String::from_utf8_lossy(&report.body).contains("multistatus"));
    let malformed_report = server.handle(Request {
        method: "REPORT".into(),
        path: "/carddav/contacts".into(),
        headers: vec![("If-Unmodified-Since".into(), "not-a-date".into())],
        body: b"<addressbook-query><prop><getetag/></prop></addressbook-query>".to_vec(),
    });
    assert_eq!(malformed_report.status, 207);
}

#[test]
fn delete_if_none_match_wildcard_rejects_without_mutation() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let created = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/delete-conditional.vcf".into(),
        headers: vec![("Content-Type".into(), "text/vcard".into())],
        body: b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:delete-conditional\r\nFN:Keep\r\nEND:VCARD\r\n"
            .to_vec(),
    });
    assert_eq!(created.status, 201);
    let rejected = server.handle(Request {
        method: "DELETE".into(),
        path: "/carddav/contacts/delete-conditional.vcf".into(),
        headers: vec![("If-None-Match".into(), "*".into())],
        body: vec![],
    });
    assert_eq!(rejected.status, 412);
    assert_eq!(
        server
            .handle(req("GET", "/carddav/contacts/delete-conditional.vcf", b""))
            .status,
        200
    );
}

#[test]
fn put_rejects_missing_or_wrong_content_type_without_mutation() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let body = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:type-1\r\nFN:Type\r\nEND:VCARD\r\n";
    let missing = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/type.vcf".into(),
        headers: vec![],
        body: body.to_vec(),
    });
    assert_eq!(missing.status, 415);
    let wrong = server.handle(Request {
        method: "PUT".into(),
        path: "/carddav/contacts/type.vcf".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: body.to_vec(),
    });
    assert_eq!(wrong.status, 415);
    let collection = server.tasks.clone();
    assert!(server
        .repository
        .list_resources(&collection, false)
        .unwrap()
        .is_empty());
}

#[test]
fn direct_protocol_matrix_executes_without_transport() {
    let mut server = DavServer::new(MemoryRepository::new());
    let options = server.handle(req("OPTIONS", "/carddav/contacts", b""));
    assert_eq!(options.status, 200);
    let discovery = server.handle(req("PROPFIND", "/carddav/contacts", b""));
    assert_eq!(discovery.status, 207);
    let card_report = server.handle(req("REPORT", "/carddav/contacts", b"<addressbook-query/>"));
    assert_eq!(card_report.status, 207);
    assert!(String::from_utf8(card_report.body)
        .unwrap()
        .contains("multistatus"));
    let task_report = server.handle(req("REPORT", "/caldav/tasks", b"<calendar-query/>"));
    assert_eq!(task_report.status, 207);
    assert!(
        server
            .handle(req("REPORT", "/carddav/contacts", b"<calendar-query/>"))
            .status
            == 400
    );
}

#[test]
fn malformed_report_is_rejected_without_repository_mutation() {
    let mut server = DavServer::new(MemoryRepository::new());
    assert_eq!(
        server
            .handle(req("REPORT", "/caldav/tasks", b"not xml"))
            .status,
        400
    );
    let tasks = server.tasks.clone();
    assert!(server
        .repository
        .list_resources(&tasks, false)
        .unwrap()
        .is_empty());
}
