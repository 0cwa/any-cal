use any_cal_core::vfreebusy::VFreeBusy;
use any_cal_core::{
    ical::Calendar, CanonicalDocument, DavKind, DavUid, MemoryRepository, Occurrence, Repository,
    ResourceEnvelope, ResourceId, StructuredDocument, WriteCondition,
};
use any_cal_dav_server::{DavServer, Request};
use std::collections::BTreeMap;

const SAMPLE: &str = concat!(
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
    "BEGIN:VFREEBUSY\r\n",
    "DTSTART:20261025T090000Z\r\n",
    "DTEND:20261025T180000Z\r\n",
    "FREEBUSY;FBTYPE=BUSY:20261025T100000Z/20261025T110000Z,",
    "20261025T103000Z/20261025T120000Z\r\n",
    "FREEBUSY;FBTYPE=FREE:20261025T130000Z/20261025T140000Z\r\n",
    "END:VFREEBUSY\r\nEND:VCALENDAR\r\n"
);

fn request(method: &str, path: &str, body: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: vec![],
        body: body.to_vec(),
    }
}

#[test]
fn vfreebusy_projection_and_repository_rebuild_are_stable() {
    let parsed = VFreeBusy::parse(SAMPLE).unwrap();
    let projected = parsed
        .project("20261025T103000Z", "20261025T150000Z", 8)
        .unwrap();
    assert_eq!(projected.periods.len(), 2);
    assert_eq!(projected.periods[0].interval.start, "20261025103000");
    assert_eq!(projected.periods[0].interval.end, "20261025120000");

    let calendar = Calendar::parse(&parsed.serialize().unwrap()).unwrap();
    let collection: any_cal_core::CollectionId = "freebusy".try_into().unwrap();
    let envelope = ResourceEnvelope {
        collection_id: collection.clone(),
        resource_id: ResourceId::try_from("availability").unwrap(),
        kind: DavKind::Event,
        anytype_object_id: "availability-object".try_into().unwrap(),
        dav_uid: DavUid::try_from("availability-uid").unwrap(),
        document: CanonicalDocument {
            version: 1,
            content: StructuredDocument {
                fields: BTreeMap::from([("UID".into(), vec![Occurrence::new("availability-uid")])]),
            },
            opaque_calendar: Some(calendar),
        },
        revision: 0,
    };
    let mut repository = MemoryRepository::new();
    repository
        .create_collection(any_cal_core::Collection {
            id: collection.clone(),
            name: "Synthetic availability".into(),
        })
        .unwrap();
    let stored = repository
        .create_resource(envelope, WriteCondition::IfNoneMatch)
        .unwrap();
    let etag = stored.etag.clone();
    let mut rebuilt = repository.clone();
    let reopened = rebuilt
        .get_resource(&ResourceId::try_from("availability").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(reopened.etag, etag);
    assert_eq!(reopened.modified_at, stored.modified_at);
    let reopened_calendar = reopened.envelope.document.opaque_calendar.unwrap();
    let reopened_wire = reopened_calendar.serialize();
    assert_eq!(VFreeBusy::parse(&reopened_wire).unwrap(), parsed);
}

#[test]
fn vfreebusy_dav_boundaries_are_truthful_and_non_mutating() {
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let put = Request {
        method: "PUT".into(),
        path: "/caldav/tasks/availability.ics".into(),
        headers: vec![("Content-Type".into(), "text/calendar".into())],
        body: SAMPLE.as_bytes().to_vec(),
    };
    // VFREEBUSY is not a VTODO resource in the current task-only calendar
    // profile.  Reject it rather than persisting a resource that cannot be
    // represented as a task or advertising unsupported scheduling semantics.
    assert_eq!(server.handle(put).status, 400);
    assert!(server
        .repository
        .list_resources(&"tasks".try_into().unwrap(), false)
        .unwrap()
        .is_empty());

    let unsupported = server.handle(request(
        "REPORT",
        "/caldav/tasks",
        br#"<free-busy-query><time-range start="20261025T090000Z" end="20261025T180000Z"/></free-busy-query>"#,
    ));
    assert_eq!(unsupported.status, 400);

    let empty = server.handle(request(
        "REPORT",
        "/caldav/tasks",
        br#"<calendar-query><prop><calendar-data/></prop><filter><comp-filter name="VTODO"/></filter></calendar-query>"#,
    ));
    assert_eq!(empty.status, 207);
    assert_eq!(
        String::from_utf8(empty.body).unwrap(),
        "<multistatus xmlns=\"DAV:\"></multistatus>"
    );

    let malformed = server.handle(request("REPORT", "/caldav/tasks", b"<broken"));
    assert_eq!(malformed.status, 400);
}
