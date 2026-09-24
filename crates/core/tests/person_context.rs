use any_cal_core::{
    effective_person_context_title, project_person_context_source, reconcile_source_snapshot,
    typed_etag_for_bytes, AnytypeObjectId, CanonicalDocument, CollectionId, DavKind, DavUid,
    ForeignObjectRef, MaterializedReference, ModifiedAt, Occurrence, PersonContextProjectionError,
    ResourceEnvelope, ResourceId, SourceAvailability, SourceSnapshot, StoredResource,
    StructuredDocument, PERSON_CONTEXT_DAV_UID, PERSON_CONTEXT_DISPLAY_NAME,
    PERSON_CONTEXT_EMAILS, PERSON_CONTEXT_ORGANIZATIONS, PERSON_CONTEXT_PHONES,
    PERSON_CONTEXT_TITLE_OVERRIDE,
};
use serde_json::json;
use std::collections::BTreeMap;

fn stored(kind: DavKind, name: &str) -> StoredResource {
    let mut document = StructuredDocument::default();
    document.insert("FN", vec![Occurrence::new(name)]);
    document.insert(
        "EMAIL",
        vec![
            Occurrence::new("carol@example.test"),
            Occurrence::new("carol@work.test"),
        ],
    );
    document.insert(
        "TEL",
        vec![Occurrence::new("+1-555-0100"), Occurrence::new("+1-555-0101")],
    );
    document.insert("ORG", vec![Occurrence::new("Example Org")]);
    document.insert("NOTE", vec![Occurrence::new("canonical note stays source-only")]);

    StoredResource {
        envelope: ResourceEnvelope {
            collection_id: CollectionId::try_from("contacts").unwrap(),
            resource_id: ResourceId::try_from("carol").unwrap(),
            kind,
            anytype_object_id: AnytypeObjectId::try_from("object-carol").unwrap(),
            dav_uid: DavUid::try_from("carol-uid").unwrap(),
            document: CanonicalDocument::new(document),
            revision: 1,
        },
        etag: typed_etag_for_bytes(b"carol"),
        archived: false,
        modified_at: ModifiedAt::UNIX_EPOCH,
    }
}

fn foreign() -> ForeignObjectRef {
    ForeignObjectRef::new(
        "ab".repeat(32),
        "shared-space",
        "object-carol",
        DavKind::Contact,
        Some("carol-uid".into()),
    )
    .unwrap()
}

#[test]
fn contact_projection_is_small_source_owned_and_ignores_private_or_opaque_fields() {
    let projected = project_person_context_source(&stored(DavKind::Contact, "Carol Smith")).unwrap();

    assert_eq!(projected[PERSON_CONTEXT_DISPLAY_NAME], "Carol Smith");
    assert_eq!(
        projected[PERSON_CONTEXT_EMAILS],
        json!(["carol@example.test", "carol@work.test"])
    );
    assert_eq!(
        projected[PERSON_CONTEXT_PHONES],
        json!(["+1-555-0100", "+1-555-0101"])
    );
    assert_eq!(
        projected[PERSON_CONTEXT_ORGANIZATIONS],
        json!(["Example Org"])
    );
    assert_eq!(projected[PERSON_CONTEXT_DAV_UID], "carol-uid");
    assert!(!projected.contains_key("NOTE"));
    assert!(!projected.contains_key("private_notes"));
}

#[test]
fn projection_is_bounded_and_rejects_non_contacts() {
    let mut value = stored(DavKind::Contact, &"N".repeat(2_000));
    value.envelope.document.content.fields.insert(
        "EMAIL".into(),
        (0..40)
            .map(|index| Occurrence::new(format!("person-{index}@example.test")))
            .collect(),
    );
    let projected = project_person_context_source(&value).unwrap();
    assert_eq!(
        projected[PERSON_CONTEXT_DISPLAY_NAME]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        1_024
    );
    assert_eq!(
        projected[PERSON_CONTEXT_EMAILS].as_array().unwrap().len(),
        32
    );

    assert_eq!(
        project_person_context_source(&stored(DavKind::Task, "Not a contact")).unwrap_err(),
        PersonContextProjectionError::NotContact
    );
}

#[test]
fn source_rename_updates_default_title_but_never_overwrites_local_title_override() {
    let initial_stored = stored(DavKind::Contact, "Carol Old");
    let initial = SourceSnapshot::new(
        foreign(),
        project_person_context_source(&initial_stored).unwrap(),
        SourceAvailability::Available,
    )
    .unwrap();

    let plain = MaterializedReference::new(&initial).unwrap();
    assert_eq!(
        effective_person_context_title(&plain).as_deref(),
        Some("Carol Old")
    );

    let updated_stored = stored(DavKind::Contact, "Carol New");
    let updated = SourceSnapshot::new(
        foreign(),
        project_person_context_source(&updated_stored).unwrap(),
        SourceAvailability::Available,
    )
    .unwrap();

    let refreshed = reconcile_source_snapshot(Some(&plain), &updated).unwrap();
    assert_eq!(
        effective_person_context_title(&refreshed.reference).as_deref(),
        Some("Carol New")
    );

    let mut customized = plain;
    customized.user_fields = BTreeMap::from([(
        PERSON_CONTEXT_TITLE_OVERRIDE.into(),
        json!("Carol · Private"),
    )]);
    let refreshed = reconcile_source_snapshot(Some(&customized), &updated).unwrap();
    assert_eq!(
        effective_person_context_title(&refreshed.reference).as_deref(),
        Some("Carol · Private")
    );
    assert_eq!(
        refreshed.reference.source_fields[PERSON_CONTEXT_DISPLAY_NAME],
        "Carol New"
    );
    assert_eq!(
        refreshed.reference.user_fields[PERSON_CONTEXT_TITLE_OVERRIDE],
        "Carol · Private"
    );
}


#[test]
fn two_principals_keep_independent_private_facets_for_the_same_shared_contact() {
    let initial_stored = stored(DavKind::Contact, "Carol Shared");
    let initial = SourceSnapshot::new(
        foreign(),
        project_person_context_source(&initial_stored).unwrap(),
        SourceAvailability::Available,
    )
    .unwrap();

    let mut alice = MaterializedReference::new(&initial).unwrap();
    alice
        .user_fields
        .insert("private_notes".into(), json!("Alice note"));
    alice
        .user_fields
        .insert(PERSON_CONTEXT_TITLE_OVERRIDE.into(), json!("Carol · Alice"));

    let mut bob = MaterializedReference::new(&initial).unwrap();
    bob.user_fields
        .insert("private_notes".into(), json!("Bob note"));

    let updated_stored = stored(DavKind::Contact, "Carol Renamed");
    let updated = SourceSnapshot::new(
        foreign(),
        project_person_context_source(&updated_stored).unwrap(),
        SourceAvailability::Available,
    )
    .unwrap();

    let alice = reconcile_source_snapshot(Some(&alice), &updated)
        .unwrap()
        .reference;
    let bob = reconcile_source_snapshot(Some(&bob), &updated)
        .unwrap()
        .reference;

    assert_eq!(alice.source_fields, bob.source_fields);
    assert_eq!(
        alice.source_fields[PERSON_CONTEXT_DISPLAY_NAME],
        "Carol Renamed"
    );
    assert_eq!(alice.user_fields["private_notes"], "Alice note");
    assert_eq!(bob.user_fields["private_notes"], "Bob note");
    assert_eq!(
        effective_person_context_title(&alice).as_deref(),
        Some("Carol · Alice")
    );
    assert_eq!(
        effective_person_context_title(&bob).as_deref(),
        Some("Carol Renamed")
    );
    assert_ne!(alice.user_fields, bob.user_fields);
}
