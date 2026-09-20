use any_cal_core::etag_for_bytes;
use any_cal_core::ical::{Calendar, Entry};

const MIXED: &str = concat!(
    "BEGIN:VCALENDAR\r\n",
    "VERSION:2.0\r\n",
    "PRODID:-//Any-Cal//EN\r\n",
    "BEGIN:VTIMEZONE\r\n",
    "TZID:Europe/Stockholm\r\n",
    "BEGIN:STANDARD\r\n",
    "DTSTART:20261025T010000\r\n",
    "TZOFFSETFROM:+0200\r\n",
    "TZOFFSETTO:+0100\r\n",
    "X-TZ-OPAQUE:keep\r\n",
    "END:STANDARD\r\n",
    "END:VTIMEZONE\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:event-1\r\n",
    "SUMMARY:Planning\r\n",
    "BEGIN:VALARM\r\n",
    "ACTION:DISPLAY\r\n",
    "TRIGGER;RELATED=START:-PT15M\r\n",
    "END:VALARM\r\n",
    "X-UNKNOWN-COMPONENT-PROP:preserve\r\n",
    "END:VEVENT\r\n",
    "BEGIN:VTODO\r\n",
    "UID:task-1\r\n",
    "SUMMARY:Follow up\r\n",
    "END:VTODO\r\n",
    "END:VCALENDAR\r\n"
);

#[test]
fn mixed_components_preserve_ownership_order_and_unknowns() {
    let calendar = Calendar::parse(MIXED).unwrap();
    assert_eq!(calendar.root().name, "VCALENDAR");
    let children: Vec<_> = calendar
        .root()
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Component(component) => Some(component.name.as_str()),
            Entry::Property(_) => None,
        })
        .collect();
    assert_eq!(children, ["VTIMEZONE", "VEVENT", "VTODO"]);
    let event = calendar.root().components_named("VEVENT").next().unwrap();
    assert_eq!(
        event.properties_named("UID").next().unwrap().value,
        "event-1"
    );
    assert_eq!(event.components_named("VALARM").count(), 1);
    assert_eq!(
        event
            .properties_named("X-UNKNOWN-COMPONENT-PROP")
            .next()
            .unwrap()
            .value,
        "preserve"
    );
    let zone = calendar
        .root()
        .components_named("VTIMEZONE")
        .next()
        .unwrap();
    assert_eq!(zone.components_named("STANDARD").count(), 1);
    assert_eq!(
        zone.components_named("STANDARD")
            .next()
            .unwrap()
            .properties_named("X-TZ-OPAQUE")
            .next()
            .unwrap()
            .value,
        "keep"
    );
}

#[test]
fn mixed_components_round_trip_has_stable_canonical_form_and_etag() {
    let first = Calendar::parse(MIXED).unwrap().serialize();
    let second = Calendar::parse(&first).unwrap().serialize();
    assert_eq!(first, second);
    assert_eq!(
        etag_for_bytes(first.as_bytes()),
        etag_for_bytes(second.as_bytes())
    );
    assert!(first.contains("BEGIN:VALARM\r\nACTION:DISPLAY\r\n"));
}

#[test]
fn malformed_boundaries_and_multiple_roots_are_rejected() {
    for input in [
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nEND:VTODO\nEND:VCALENDAR\n",
        "BEGIN:VCALENDAR\nEND:VEVENT\n",
        "BEGIN:VCALENDAR\nEND:VCALENDAR\nBEGIN:VCALENDAR\nEND:VCALENDAR\n",
        "BEGIN:VEVENT\nUID:x\nEND:VEVENT\n",
        "VERSION:2.0\n",
    ] {
        assert!(
            Calendar::parse(input).is_err(),
            "accepted malformed input: {input:?}"
        );
    }
}

#[test]
fn repeated_properties_keep_occurrence_order_and_parameters() {
    let input = concat!(
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\n",
        "ATTENDEE;CN=\"Doe, Jane\";ROLE=REQ-PARTICIPANT:mailto:jane@example.test\n",
        "ATTENDEE;CN=\"Smith; Alex\":mailto:alex@example.test\n",
        "END:VEVENT\nEND:VCALENDAR\n"
    );
    let event = Calendar::parse(input).unwrap();
    let event = event.root().components_named("VEVENT").next().unwrap();
    let attendees: Vec<_> = event.properties_named("ATTENDEE").collect();
    assert_eq!(attendees.len(), 2);
    assert_eq!(attendees[0].params["CN"], ["Doe, Jane"]);
    assert_eq!(attendees[1].params["CN"], ["Smith; Alex"]);
}

#[test]
fn alarm_and_timezone_values_round_trip_at_dst_boundary() {
    let input = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VTIMEZONE\r\nTZID:Europe/Stockholm\r\n",
        "BEGIN:STANDARD\r\nDTSTART:20261025T010000\r\n",
        "TZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\n",
        "END:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:dst\r\n",
        "DTSTART;TZID=Europe/Stockholm:20261025T013000\r\n",
        "DTEND;TZID=Europe/Stockholm:20261025T023000\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\n",
        "TRIGGER;RELATED=START:-PT15M\r\nREPEAT:2\r\nDURATION:PT5M\r\n",
        "DESCRIPTION:Before\\,transition\r\nEND:VALARM\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let calendar = Calendar::parse(input).unwrap();
    let event = calendar.root().components_named("VEVENT").next().unwrap();
    let alarm = event.components_named("VALARM").next().unwrap();
    assert_eq!(
        alarm.properties_named("ACTION").next().unwrap().value,
        "DISPLAY"
    );
    assert_eq!(
        alarm.properties_named("TRIGGER").next().unwrap().params["RELATED"],
        ["START"]
    );
    assert_eq!(alarm.properties_named("REPEAT").next().unwrap().value, "2");
    assert_eq!(
        calendar.serialize(),
        Calendar::parse(&calendar.serialize()).unwrap().serialize()
    );
    assert!(calendar
        .serialize()
        .contains("TZID=Europe/Stockholm:20261025T023000"));
}

#[test]
fn malformed_alarm_parameters_and_boundaries_are_rejected() {
    for input in [
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nBEGIN:VALARM\nTRIGGER;RELATED=\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nBEGIN:VALARM\nACTION\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n",
    ] {
        assert!(Calendar::parse(input).is_err(), "accepted malformed alarm: {input:?}");
    }
}

#[test]
fn scheduling_properties_round_trip_and_malformed_parameters_fail_closed() {
    let input = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VEVENT\r\nUID:scheduling-event\r\n",
        "DTSTAMP:20260920T120000Z\r\nSEQUENCE:7\r\nSTATUS:CONFIRMED\r\n",
        "ORGANIZER;CN=\"Planner; Team\";ROLE=CHAIR:mailto:planner@example.test\r\n",
        "ATTENDEE;CN=\"Doe, Jane\";RSVP=TRUE;PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT;CUTYPE=INDIVIDUAL:mailto:jane@example.test\r\n",
        "ATTENDEE;CN=\"Smith; Alex\";RSVP=FALSE;PARTSTAT=DECLINED;ROLE=OPT-PARTICIPANT;CUTYPE=GROUP:mailto:team@example.test\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let parsed = Calendar::parse(input).unwrap();
    let event = parsed.root().components_named("VEVENT").next().unwrap();
    assert_eq!(event.properties_named("ATTENDEE").count(), 2);
    assert_eq!(
        event.properties_named("ORGANIZER").next().unwrap().params["ROLE"],
        ["CHAIR"]
    );
    assert_eq!(
        event.properties_named("ATTENDEE").next().unwrap().params["RSVP"],
        ["TRUE"]
    );
    assert_eq!(
        event.properties_named("SEQUENCE").next().unwrap().value,
        "7"
    );
    assert_eq!(
        event.properties_named("DTSTAMP").next().unwrap().value,
        "20260920T120000Z"
    );
    let serialized = parsed.serialize();
    assert_eq!(
        serialized,
        Calendar::parse(&serialized).unwrap().serialize()
    );

    for malformed in [
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nATTENDEE;CN=\"unterminated:mailto:x@example.test\nEND:VEVENT\nEND:VCALENDAR\n",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nATTENDEE;PARTSTAT=:mailto:x@example.test\nEND:VEVENT\nEND:VCALENDAR\n",
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nATTENDEE;RSVP=TRUE,\"\":mailto:x@example.test\nEND:VEVENT\nEND:VCALENDAR\n",
    ] {
        assert!(Calendar::parse(malformed).is_err(), "accepted malformed scheduling input: {malformed:?}");
    }
}
