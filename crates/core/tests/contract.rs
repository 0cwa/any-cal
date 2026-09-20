use any_cal_core::{
    etag, AnytypeObjectId, CollectionId, DavKind, DavUid, Occurrence, ResourceEnvelope, ResourceId,
    StructuredDocument,
};
use std::collections::BTreeMap;

fn sample() -> ResourceEnvelope {
    let mut fields = BTreeMap::new();
    fields.insert(
        "TEL".into(),
        vec![
            Occurrence {
                value: "+46 1".into(),
                params: BTreeMap::from([("TYPE".into(), vec!["cell".into(), "voice".into()])]),
            },
            Occurrence::new("+46 2"),
        ],
    );
    fields.insert(
        "X-VENDOR-NOTE".into(),
        vec![Occurrence::new("café\r\nline")],
    );
    ResourceEnvelope {
        collection_id: CollectionId::try_from("contacts").unwrap(),
        resource_id: ResourceId::try_from("obj-1.vcf").unwrap(),
        kind: DavKind::Contact,
        anytype_object_id: AnytypeObjectId::try_from("obj-1").unwrap(),
        dav_uid: DavUid::try_from("uid-1").unwrap(),
        document: any_cal_core::CanonicalDocument::new(StructuredDocument { fields }),
        revision: 7,
    }
}

#[test]
fn repeated_values_and_parameters_round_trip() {
    let original = sample();
    let json = original.canonical_json().unwrap();
    let parsed = ResourceEnvelope::from_json(&json).unwrap();
    assert_eq!(parsed.document.content.fields["TEL"].len(), 2);
    assert_eq!(
        parsed.document.content.fields["TEL"][0].params["TYPE"],
        ["cell", "voice"]
    );
    assert_eq!(
        parsed.document.content.fields["X-VENDOR-NOTE"][0].value,
        "café\nline"
    );
}

#[test]
fn canonicalization_is_stable_and_newlines_are_normalized() {
    let first = sample();
    let mut second = sample();
    second.document.content.fields = BTreeMap::from([
        ("X-VENDOR-NOTE".into(), vec![Occurrence::new("café\nline")]),
        ("TEL".into(), first.document.content.fields["TEL"].clone()),
    ]);
    assert_eq!(
        first.canonical_json().unwrap(),
        second.canonical_json().unwrap()
    );
    assert!(first.semantic_eq(&second));
    assert_eq!(
        first.canonical_json().unwrap(),
        ResourceEnvelope::from_json(&first.canonical_json().unwrap())
            .unwrap()
            .canonical_json()
            .unwrap()
    );
}

#[test]
fn occurrence_order_is_semantically_significant() {
    let mut swapped = sample();
    swapped
        .document
        .content
        .fields
        .get_mut("TEL")
        .unwrap()
        .swap(0, 1);
    assert_ne!(
        sample().canonical_json().unwrap(),
        swapped.canonical_json().unwrap()
    );
    assert!(!sample().semantic_eq(&swapped));
}

#[test]
fn malformed_envelopes_are_rejected() {
    assert!(ResourceEnvelope::from_json("not json").is_err());
    let mut missing = serde_json::to_value(sample()).unwrap();
    missing.as_object_mut().unwrap().remove("dav_uid");
    assert!(ResourceEnvelope::from_json(&missing.to_string()).is_err());
    let mut unsupported = sample();
    unsupported.document.version = 2;
    assert!(unsupported.canonical_json().is_err());
    assert!(CollectionId::try_from("  ").is_err());
    assert!(ResourceId::try_from("").is_err());
    assert!(AnytypeObjectId::try_from("\n").is_err());
    assert!(DavUid::try_from("\t").is_err());
}

#[test]
fn etag_is_deterministic_and_quoted() {
    assert_eq!(etag("hello"), etag("hello"));
    assert!(etag("hello").starts_with('"') && etag("hello").ends_with('"'));
    assert_ne!(etag("hello"), etag("hello!"));
}
