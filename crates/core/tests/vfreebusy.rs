use any_cal_core::vfreebusy::{FreeBusyType, VFreeBusy, VFreeBusyError};

const SAMPLE: &str = concat!(
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
    "BEGIN:VFREEBUSY\r\n",
    "DTSTART;TZID=Europe/Stockholm:20261025T090000\r\n",
    "DTEND:20261025T180000Z\r\n",
    "ORGANIZER:mailto:planner@example.test\r\n",
    "URL:https://example.test/freebusy\r\n",
    "FREEBUSY;FBTYPE=BUSY:20261025T100000Z/20261025T110000Z,20261025T103000Z/20261025T120000Z\r\n",
    "FREEBUSY;FBTYPE=BUSY-TENTATIVE:20261025T130000/20261025T140000\r\n",
    "END:VFREEBUSY\r\nEND:VCALENDAR\r\n"
);

#[test]
fn wire_round_trip_is_normalized_and_metadata_is_retained() {
    let parsed = VFreeBusy::parse(SAMPLE).unwrap();
    assert_eq!(parsed.dtstart.as_deref(), Some("20261025090000"));
    assert_eq!(parsed.tzid.as_deref(), Some("Europe/Stockholm"));
    assert_eq!(parsed.periods.len(), 3);
    assert!(matches!(parsed.periods[0].kind, FreeBusyType::Busy));
    let wire = parsed.serialize().unwrap();
    assert!(wire.contains(
        "FREEBUSY;FBTYPE=BUSY:20261025100000/20261025110000,20261025103000/20261025120000\r\n"
    ));
    assert_eq!(wire, parsed.serialize().unwrap());
    assert_eq!(
        parsed.etag().unwrap(),
        VFreeBusy::parse(&wire).unwrap().etag().unwrap()
    );
}

#[test]
fn projection_clips_merges_per_type_and_is_bounded() {
    let parsed = VFreeBusy::parse(SAMPLE).unwrap();
    let projected = parsed
        .project("20261025103000", "20261025133000", 4)
        .unwrap();
    assert_eq!(projected.periods.len(), 2);
    assert_eq!(projected.periods[0].interval.start, "20261025103000");
    assert_eq!(projected.periods[0].interval.end, "20261025120000");
    assert!(matches!(
        parsed.project("20261025103000", "20261025133000", 1),
        Err(VFreeBusyError::TooManyIntervals)
    ));
}

#[test]
fn malformed_empty_and_size_limits_fail_closed() {
    let empty =
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VFREEBUSY\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n";
    assert!(VFreeBusy::parse(empty).unwrap().periods.is_empty());
    assert!(matches!(
        VFreeBusy::parse(
            "BEGIN:VCALENDAR\nBEGIN:VFREEBUSY\nFREEBUSY:bad\nEND:VFREEBUSY\nEND:VCALENDAR\n"
        ),
        Err(VFreeBusyError::Malformed(_))
    ));
    assert!(matches!(
        VFreeBusy::parse_bounded(empty, 8, 10),
        Err(VFreeBusyError::TooLarge)
    ));
    let parsed = VFreeBusy::parse(SAMPLE).unwrap();
    assert!(matches!(
        parsed.serialize_bounded(8),
        Err(VFreeBusyError::TooLarge)
    ));
}

#[test]
fn duration_periods_are_not_silently_reinterpreted() {
    let input = "BEGIN:VCALENDAR\nBEGIN:VFREEBUSY\nFREEBUSY:20261025T100000Z/PT1H\nEND:VFREEBUSY\nEND:VCALENDAR\n";
    assert!(matches!(
        VFreeBusy::parse(input),
        Err(VFreeBusyError::Malformed(_))
    ));
}
