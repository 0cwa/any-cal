use any_cal_core::freebusy::{project, FreeBusyError, FreeBusyInterval};

fn interval(start: &str, end: &str) -> FreeBusyInterval {
    FreeBusyInterval::new(start, end).unwrap()
}

#[test]
fn projection_clips_subtracts_overlaps_and_orders_deterministically() {
    let intervals = [
        interval("20261025T110000", "20261025T130000"),
        interval("20261025T100000Z", "20261025T120000Z"),
        interval("20261025", "20261026"),
    ];
    let exclusions = [interval("20261025T113000", "20261025T120000")];
    let projected = project(
        &intervals,
        "20261025T100000",
        "20261025T140000Z",
        &exclusions,
    )
    .unwrap();
    assert_eq!(
        projected,
        vec![
            interval("20261025T100000", "20261025T113000"),
            interval("20261025T120000", "20261025T140000"),
        ]
    );
}

#[test]
fn projection_handles_empty_and_half_open_boundaries() {
    let intervals = [interval("20261025T100000", "20261025T110000")];
    assert!(
        project(&intervals, "20261025T110000", "20261025T120000", &[])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        project(&intervals, "20261025T103000", "20261025T110000", &[]).unwrap(),
        vec![interval("20261025T103000", "20261025T110000")]
    );
}

#[test]
fn date_only_dst_boundary_and_malformed_values_are_explicit() {
    // The DST transition is retained as literal local wall-clock data; this
    // layer does not claim to resolve Europe/Stockholm rules.
    let dst = interval("20261025T013000", "20261025T023000");
    assert_eq!(dst.start, "20261025013000");
    assert_eq!(dst.end, "20261025023000");
    assert_eq!(interval("20261025", "20261026").start, "20261025000000");
    assert!(matches!(
        FreeBusyInterval::new("20261032T010000", "20261032T020000"),
        Err(FreeBusyError::InvalidTime(_))
    ));
    assert!(matches!(
        FreeBusyInterval::new("20261025T020000", "20261025T020000"),
        Err(FreeBusyError::ReversedInterval)
    ));
    assert!(matches!(
        project(&[], "20261026", "20261025", &[]),
        Err(FreeBusyError::InvalidWindow)
    ));
}
