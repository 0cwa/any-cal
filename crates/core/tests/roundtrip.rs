use any_cal_core::{vcard, vtodo};

#[test]
fn contact_golden_round_trip_preserves_repeated_and_unknown_fields() {
    let input = include_str!("fixtures/contact.vcf");
    let parsed = vcard::parse(input).unwrap();
    assert_eq!(parsed.fields.fields["TEL"].len(), 2);
    assert_eq!(
        parsed.fields.fields["TEL"][0].params["TYPE"],
        ["cell", "voice"]
    );
    assert_eq!(parsed.fields.fields["NOTE"][0].value, "line one\nline two");
    for field in vcard::NAMED_FIELDS {
        assert!(!parsed.values(field).is_empty(), "missing {field}");
    }
    assert_eq!(parsed.metadata.fields["VERSION"][0].value, "4.0");
    let mut projected = parsed.clone();
    let mut update = parsed.projection();
    update.get_mut("FN").unwrap()[0].value = "Renamed".into();
    projected.apply_projection(&update);
    assert_eq!(projected.values("FN")[0].value, "Renamed");
    assert_eq!(projected.values("TEL"), parsed.values("TEL"));
    assert_eq!(
        projected.fields.fields["X-CLIENT-FOO"],
        parsed.fields.fields["X-CLIENT-FOO"]
    );
    assert_eq!(projected.metadata, parsed.metadata);
    let reparsed = vcard::parse(&vcard::serialize(&parsed)).unwrap();
    assert_eq!(parsed, reparsed);
}

#[test]
fn vtodo_golden_round_trip_preserves_opaque_fields() {
    let input = include_str!("fixtures/task.ics");
    let parsed = vtodo::parse(input).unwrap();
    assert_eq!(
        parsed.fields.fields["RELATED-TO"][0].params["RELTYPE"],
        ["PARENT"]
    );
    assert_eq!(
        parsed.fields.fields["X-TASKSORG-FOO"][0].value,
        "opaque\nvalue"
    );
    for field in vtodo::NAMED_FIELDS {
        assert!(!parsed.values(field).is_empty(), "missing {field}");
    }
    assert_eq!(
        parsed.calendar_metadata.fields["PRODID"][0].value,
        "-//Any-Cal Fixture//EN"
    );
    assert_eq!(
        parsed.calendar_metadata.fields["PRODID"][0].params["X-ORG"],
        ["example"]
    );
    assert_eq!(
        parsed.calendar_metadata.fields["X-CALENDAR-META"][0].params["X-LABEL"],
        ["one", "two"]
    );
    let projection = parsed.projection();
    for field in vtodo::NAMED_FIELDS {
        assert!(
            projection.contains_key(*field),
            "projection missing {field}"
        );
    }
    let mut updated = parsed.clone();
    let mut changed = projection;
    changed.get_mut("SUMMARY").unwrap()[0].value = "Updated".into();
    updated.apply_projection(&changed);
    assert_eq!(updated.values("SUMMARY")[0].value, "Updated");
    assert_eq!(
        updated.values("X-TASKSORG-FOO"),
        parsed.values("X-TASKSORG-FOO")
    );
    assert_eq!(updated.calendar_metadata, parsed.calendar_metadata);
    assert_eq!(parsed, vtodo::parse(&vtodo::serialize(&parsed)).unwrap());
}

#[test]
fn vcard_version_is_retained_as_metadata_but_normalized_on_wire() {
    let parsed = vcard::parse("BEGIN:VCARD\nVERSION:3.0\nUID:v\nFN:Name\nEND:VCARD\n").unwrap();
    assert_eq!(parsed.metadata.fields["VERSION"][0].value, "3.0");
    let wire = vcard::serialize(&parsed);
    assert!(wire.contains("VERSION:4.0\r\n"));
    assert!(!wire.contains("VERSION:3.0"));
}

#[test]
fn malformed_input_is_rejected_without_a_replacement_value() {
    let valid = vcard::parse("BEGIN:VCARD\nVERSION:4.0\nUID:1\nFN:A\nEND:VCARD\n").unwrap();
    let result = vcard::parse("BEGIN:VCARD\nFN:broken\nNO_COLON\nEND:VCARD\n");
    assert!(result.is_err());
    assert_eq!(valid.fields.fields["FN"][0].value, "A");
    assert!(vtodo::parse("BEGIN:VCALENDAR\nBEGIN:VTODO\nSUMMARY:x\nEND:VCALENDAR\n").is_err());
}

#[test]
fn recurring_vtodo_preserves_time_forms_attendees_alarm_and_opaque_fields() {
    let input = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "PRODID:-//Any-Cal//EN\r\n",
        "BEGIN:VTODO\r\nUID:recurring-1\r\nSUMMARY:Planning\r\n",
        "DTSTART;TZID=Europe/Stockholm:20261025T090000\r\n",
        "DUE:20261025T080000Z\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4\r\n",
        "EXDATE;TZID=Europe/Stockholm:20261102T090000,20261109T090000\r\n",
        "RDATE:20261201T090000\r\n",
        "ATTENDEE;CN=\"Doe, Jane\";ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED:mailto:jane@example.test\r\n",
        "ORGANIZER;CN=\"Planner; Team\":mailto:planner@example.test\r\n",
        "STATUS:NEEDS-ACTION\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=START:-PT15M\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\n",
        "END:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let parsed = vtodo::parse(input).unwrap();
    assert_eq!(
        parsed.values("RRULE")[0].value,
        "FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4"
    );
    assert_eq!(
        parsed.values("EXDATE")[0].params["TZID"],
        ["Europe/Stockholm"]
    );
    assert_eq!(parsed.values("ATTENDEE")[0].params["CN"], ["Doe, Jane"]);
    assert_eq!(
        parsed.values("ORGANIZER")[0].params["CN"],
        ["Planner; Team"]
    );
    assert_eq!(parsed.values("TRIGGER")[0].params["RELATED"], ["START"]);
    assert_eq!(parsed.values("DTSTART")[0].value, "20261025T090000");
    assert_eq!(parsed.values("DUE")[0].value, "20261025T080000Z");
    assert_eq!(parsed, vtodo::parse(&vtodo::serialize(&parsed)).unwrap());
}

#[test]
fn malformed_quoted_parameter_is_rejected() {
    let input = "BEGIN:VCALENDAR\nBEGIN:VTODO\nUID:x\nATTENDEE;CN=\"unterminated:mailto:x@example.test\nEND:VTODO\nEND:VCALENDAR\n";
    assert!(vtodo::parse(input).is_err());
}

#[test]
fn vtodo_scheduling_projection_updates_editable_status_without_dropping_attendees() {
    let input = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VTODO\r\nUID:scheduling-task\r\nSUMMARY:Review\r\n",
        "DTSTAMP:20260920T120000Z\r\nSEQUENCE:3\r\nSTATUS:NEEDS-ACTION\r\n",
        "ORGANIZER;CN=\"Planner; Team\":mailto:planner@example.test\r\n",
        "ATTENDEE;CN=\"Doe, Jane\";RSVP=TRUE;PARTSTAT=NEEDS-ACTION;ROLE=REQ-PARTICIPANT;CUTYPE=INDIVIDUAL:mailto:jane@example.test\r\n",
        "END:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let parsed = vtodo::parse(input).unwrap();
    let mut projected = parsed.clone();
    let mut projection = parsed.projection();
    projection.get_mut("STATUS").unwrap()[0].value = "COMPLETED".into();
    projected.apply_projection(&projection);
    assert_eq!(projected.values("STATUS")[0].value, "COMPLETED");
    assert_eq!(projected.values("ATTENDEE"), parsed.values("ATTENDEE"));
    assert_eq!(projected.values("ORGANIZER"), parsed.values("ORGANIZER"));
    assert_eq!(
        projected.values("ATTENDEE")[0].params["PARTSTAT"],
        ["NEEDS-ACTION"]
    );
    let reparsed = vtodo::parse(&vtodo::serialize(&projected)).unwrap();
    assert_eq!(reparsed.values("STATUS")[0].value, "COMPLETED");
    assert_eq!(reparsed.values("ATTENDEE"), parsed.values("ATTENDEE"));
    assert_eq!(reparsed.values("ORGANIZER"), parsed.values("ORGANIZER"));
}

#[test]
fn rich_vcard_properties_round_trip_and_partial_projection_preserves_opaque_data() {
    let input = concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\n",
        "UID:contact-1\r\n",
        "FN:Zoë \\\u{1F30D}\r\n",
        "N:van\\;der\\,Meer;Zoë;;;\r\n",
        "ADR;TYPE=home;LABEL=\"Line 1; Line 2\":;;Main St\\, 1;Stockholm;;111 22;Sweden\r\n",
        "ITEM1.EMAIL;TYPE=work;PREF=1:zoe@example.test\r\n",
        "EMAIL;TYPE=home;LABEL=\"personal, mailbox\":zoe@home.test\r\n",
        "TEL;TYPE=cell,voice;VALUE=text:+46\\;70\\;123\\;456\r\n",
        "NOTE:line one\\nline two;still one value\r\n",
        "X-VENDOR-FOO;X-LABEL=\"a\\\\b\":café\r\n",
        "END:VCARD\r\n"
    );
    let parsed = vcard::parse(input).unwrap();
    assert_eq!(parsed.values("N")[0].value, "van;der,Meer;Zoë;;;");
    assert_eq!(parsed.values("ADR")[0].params["LABEL"], ["Line 1; Line 2"]);
    assert_eq!(parsed.values("ITEM1.EMAIL")[0].params["TYPE"], ["work"]);
    assert_eq!(parsed.values("TEL")[0].value, "+46;70;123;456");
    assert_eq!(
        parsed.values("NOTE")[0].value,
        "line one\nline two;still one value"
    );
    assert_eq!(parsed.values("X-VENDOR-FOO")[0].params["X-LABEL"], ["a\\b"]);

    let reparsed = vcard::parse(&vcard::serialize(&parsed)).unwrap();
    assert_eq!(parsed, reparsed);

    let mut partial = parsed.clone();
    partial.apply_projection(&std::collections::BTreeMap::from([(
        "FN".to_owned(),
        vec![any_cal_core::Occurrence::new("Renamed")],
    )]));
    assert_eq!(partial.values("FN")[0].value, "Renamed");
    assert_eq!(partial.values("EMAIL"), parsed.values("EMAIL"));
    assert_eq!(partial.values("ITEM1.EMAIL"), parsed.values("ITEM1.EMAIL"));
    assert_eq!(
        partial.values("X-VENDOR-FOO"),
        parsed.values("X-VENDOR-FOO")
    );
}

#[test]
fn partial_projection_preserves_omitted_repeated_values_and_replaces_explicit_group() {
    let input = concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\n",
        "UID:merge-contact\r\nFN:Original\r\n",
        "TEL;TYPE=cell:+46-1\r\n",
        "TEL;TYPE=work:+46-2\r\n",
        "EMAIL;TYPE=home:home@example.test\r\n",
        "EMAIL;TYPE=work:work@example.test\r\n",
        "X-PRIVATE:opaque\r\nEND:VCARD\r\n"
    );
    let parsed = vcard::parse(input).unwrap();

    // An update that addresses only FN must not accidentally drop any
    // repeated or opaque values.
    let mut fn_only = parsed.clone();
    fn_only.apply_projection(&std::collections::BTreeMap::from([(
        "FN".to_owned(),
        vec![any_cal_core::Occurrence::new("Changed")],
    )]));
    assert_eq!(fn_only.values("FN")[0].value, "Changed");
    assert_eq!(fn_only.values("TEL"), parsed.values("TEL"));
    assert_eq!(fn_only.values("EMAIL"), parsed.values("EMAIL"));
    assert_eq!(fn_only.values("X-PRIVATE"), parsed.values("X-PRIVATE"));

    // An explicitly addressed repeated property is a complete replacement of
    // that property group, with input order and labels retained exactly.
    let replacement = vec![
        any_cal_core::Occurrence {
            value: "+46-9".into(),
            params: std::collections::BTreeMap::from([(
                "TYPE".into(),
                vec!["home".into(), "voice".into()],
            )]),
        },
        any_cal_core::Occurrence {
            value: "+46-10".into(),
            params: std::collections::BTreeMap::from([("TYPE".into(), vec!["work".into()])]),
        },
    ];
    let mut replaced = fn_only.clone();
    replaced.apply_projection(&std::collections::BTreeMap::from([(
        "TEL".to_owned(),
        replacement.clone(),
    )]));
    assert_eq!(replaced.values("TEL"), replacement.as_slice());
    assert_eq!(replaced.values("EMAIL"), parsed.values("EMAIL"));
    assert_eq!(replaced.values("X-PRIVATE"), parsed.values("X-PRIVATE"));
    assert_eq!(replaced.values("UID"), parsed.values("UID"));
    assert_eq!(replaced, {
        let mut again = fn_only;
        again.apply_projection(&std::collections::BTreeMap::from([(
            "TEL".to_owned(),
            replacement,
        )]));
        again
    });
}
