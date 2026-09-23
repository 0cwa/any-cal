use any_cal_core::{
    apply_destination_mutation, reconcile_source_snapshot, CompositionError, CompositionProfile,
    DavKind, DestinationMutation, DestinationMutationResult, FieldOwnership, ForeignObjectRef,
    MaterializedReference, ProjectionPolicy, RefreshKind, SourceAvailability, SourceSnapshot,
    VisibilityIntent,
};
use serde_json::json;
use std::collections::BTreeMap;

fn account() -> String {
    "ab".repeat(32)
}

fn foreign(space: &str, object: &str, uid: Option<&str>) -> ForeignObjectRef {
    ForeignObjectRef::new(
        account(),
        space,
        object,
        DavKind::Contact,
        uid.map(str::to_owned),
    )
    .unwrap()
}

fn fields(entries: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn foreign_identity_is_account_space_object_not_uid_or_kind() {
    let a = foreign("space-a", "same-object", Some("uid-a"));
    let b = foreign("space-b", "same-object", Some("uid-a"));
    assert_ne!(a.identity_fingerprint().unwrap(), b.identity_fingerprint().unwrap());

    let renamed_uid = foreign("space-a", "same-object", Some("uid-renamed"));
    assert_eq!(
        a.identity_fingerprint().unwrap(),
        renamed_uid.identity_fingerprint().unwrap()
    );

    let task = ForeignObjectRef::new(
        account(),
        "space-a",
        "same-object",
        DavKind::Task,
        Some("different-correlation".into()),
    )
    .unwrap();
    assert_eq!(
        a.identity_fingerprint().unwrap(),
        task.identity_fingerprint().unwrap()
    );
}

#[test]
fn foreign_reference_normalizes_fingerprint_and_rejects_ambiguous_identity() {
    let uppercase = account().to_ascii_uppercase();
    let reference = ForeignObjectRef::new(
        uppercase,
        "space-a",
        "object-a",
        DavKind::Contact,
        None,
    )
    .unwrap();
    assert_eq!(reference.upstream_account_fingerprint, account());

    assert_eq!(
        ForeignObjectRef::new(
            "not-a-fingerprint",
            "space-a",
            "object-a",
            DavKind::Contact,
            None,
        )
        .unwrap_err(),
        CompositionError::InvalidFingerprint
    );
    assert_eq!(
        ForeignObjectRef::new(
            account(),
            " ",
            "object-a",
            DavKind::Contact,
            None,
        )
        .unwrap_err(),
        CompositionError::InvalidField("source_space_id")
    );
}

#[test]
fn composition_profile_is_deterministic_separate_and_privacy_gated() {
    let profile = CompositionProfile::new(
        "alice",
        "home",
        vec!["work".into(), "family".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap();
    assert_eq!(profile.source_domain_ids, vec!["family", "work"]);

    let same = CompositionProfile::new(
        "alice",
        "home",
        vec!["family".into(), "work".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap();
    assert_eq!(profile.to_json().unwrap(), same.to_json().unwrap());
    assert_eq!(profile.fingerprint().unwrap(), same.fingerprint().unwrap());

    assert!(profile
        .authorize_automatic_projection("work", VisibilityIntent::Private)
        .is_ok());
    assert_eq!(
        profile
            .authorize_automatic_projection("work", VisibilityIntent::Shared)
            .unwrap_err(),
        CompositionError::DestinationNotPrivate
    );
    assert_eq!(
        profile
            .authorize_automatic_projection("work", VisibilityIntent::Unknown)
            .unwrap_err(),
        CompositionError::DestinationNotPrivate
    );
    assert_eq!(
        profile
            .authorize_automatic_projection("unconfigured", VisibilityIntent::Private)
            .unwrap_err(),
        CompositionError::SourceDomainNotAllowed
    );

    assert_eq!(
        CompositionProfile::new(
            "alice",
            "home",
            vec!["work".into(), "work".into()],
            ProjectionPolicy::PrivateDestinationOnly,
        )
        .unwrap_err(),
        CompositionError::DuplicateSourceDomain
    );
    assert_eq!(
        CompositionProfile::new(
            "alice",
            "home",
            vec!["home".into()],
            ProjectionPolicy::PrivateDestinationOnly,
        )
        .unwrap_err(),
        CompositionError::DestinationInSources
    );
}

#[test]
fn source_refresh_replaces_source_fields_but_preserves_private_user_fields() {
    let initial = SourceSnapshot::new(
        foreign("shared", "carol", Some("carol-uid")),
        fields(&[
            ("display_name", json!("Carol Old")),
            ("email", json!("old@example.test")),
        ]),
        SourceAvailability::Available,
    )
    .unwrap();
    let created = reconcile_source_snapshot(None, &initial).unwrap();
    assert_eq!(created.kind, RefreshKind::Created);

    let with_notes = match apply_destination_mutation(
        &created.reference,
        DestinationMutation::ReplaceUserFields(fields(&[
            ("private_notes", json!("Met at conference")),
            ("private_tags", json!(["vip"])),
        ])),
    )
    .unwrap()
    {
        DestinationMutationResult::Updated(reference) => reference,
        other => panic!("unexpected mutation result: {other:?}"),
    };

    assert_eq!(
        with_notes.field_ownership("display_name"),
        Some(FieldOwnership::Source)
    );
    assert_eq!(
        with_notes.field_ownership("private_notes"),
        Some(FieldOwnership::DestinationUser)
    );

    let refreshed = SourceSnapshot::new(
        foreign("shared", "carol", Some("carol-new-uid")),
        fields(&[
            ("display_name", json!("Carol New")),
            ("email", json!("new@example.test")),
        ]),
        SourceAvailability::Available,
    )
    .unwrap();
    let merged = reconcile_source_snapshot(Some(&with_notes), &refreshed).unwrap();
    assert_eq!(merged.kind, RefreshKind::SourceRefreshed);
    assert_eq!(merged.reference.source_fields["display_name"], "Carol New");
    assert_eq!(
        merged.reference.user_fields["private_notes"],
        "Met at conference"
    );
    assert_eq!(merged.reference.user_fields["private_tags"], json!(["vip"]));
    assert_eq!(
        merged.reference.foreign.identity_fingerprint().unwrap(),
        with_notes.foreign.identity_fingerprint().unwrap()
    );
}

#[test]
fn revocation_retains_last_known_source_and_private_content_then_reauthorizes() {
    let initial = SourceSnapshot::new(
        foreign("shared", "carol", None),
        fields(&[("display_name", json!("Carol"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let mut reference = reconcile_source_snapshot(None, &initial).unwrap().reference;
    reference = match apply_destination_mutation(
        &reference,
        DestinationMutation::ReplaceUserFields(fields(&[(
            "private_notes",
            json!("Do not disclose"),
        )])),
    )
    .unwrap()
    {
        DestinationMutationResult::Updated(reference) => reference,
        _ => unreachable!(),
    };

    let unavailable = SourceSnapshot::new(
        foreign("shared", "carol", None),
        BTreeMap::new(),
        SourceAvailability::Unavailable,
    )
    .unwrap();
    let revoked = reconcile_source_snapshot(Some(&reference), &unavailable).unwrap();
    assert_eq!(revoked.kind, RefreshKind::SourceUnavailable);
    assert!(revoked.reference.source_is_stale());
    assert_eq!(revoked.reference.source_fields["display_name"], "Carol");
    assert_eq!(
        revoked.reference.user_fields["private_notes"],
        "Do not disclose"
    );

    let restored = SourceSnapshot::new(
        foreign("shared", "carol", None),
        fields(&[("display_name", json!("Carol Restored"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let reauthorized =
        reconcile_source_snapshot(Some(&revoked.reference), &restored).unwrap();
    assert_eq!(reauthorized.kind, RefreshKind::Reauthorized);
    assert!(!reauthorized.reference.source_is_stale());
    assert_eq!(
        reauthorized.reference.source_fields["display_name"],
        "Carol Restored"
    );
    assert_eq!(
        reauthorized.reference.user_fields["private_notes"],
        "Do not disclose"
    );
}

#[test]
fn archive_and_delete_never_erase_destination_user_fields() {
    let initial = SourceSnapshot::new(
        foreign("shared", "carol", None),
        fields(&[("display_name", json!("Carol"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let mut reference = MaterializedReference::new(&initial).unwrap();
    reference = match apply_destination_mutation(
        &reference,
        DestinationMutation::ReplaceUserFields(fields(&[(
            "private_notes",
            json!("Keep forever"),
        )])),
    )
    .unwrap()
    {
        DestinationMutationResult::Updated(reference) => reference,
        _ => unreachable!(),
    };

    let archived = SourceSnapshot::new(
        foreign("shared", "carol", None),
        BTreeMap::new(),
        SourceAvailability::Archived,
    )
    .unwrap();
    let archived = reconcile_source_snapshot(Some(&reference), &archived).unwrap();
    assert_eq!(archived.kind, RefreshKind::SourceArchived);
    assert_eq!(archived.reference.source_fields["display_name"], "Carol");
    assert_eq!(
        archived.reference.user_fields["private_notes"],
        "Keep forever"
    );

    let deleted = SourceSnapshot::new(
        foreign("shared", "carol", None),
        BTreeMap::new(),
        SourceAvailability::Deleted,
    )
    .unwrap();
    let deleted = reconcile_source_snapshot(Some(&archived.reference), &deleted).unwrap();
    assert_eq!(deleted.kind, RefreshKind::SourceDeleted);
    assert_eq!(deleted.reference.source_fields["display_name"], "Carol");
    assert_eq!(
        deleted.reference.user_fields["private_notes"],
        "Keep forever"
    );
}

#[test]
fn destination_mutation_cannot_overwrite_source_owned_fields() {
    let source = SourceSnapshot::new(
        foreign("shared", "carol", None),
        fields(&[("display_name", json!("Carol"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let reference = MaterializedReference::new(&source).unwrap();

    assert_eq!(
        apply_destination_mutation(
            &reference,
            DestinationMutation::ReplaceUserFields(fields(&[(
                "display_name",
                json!("Private override")
            )])),
        )
        .unwrap_err(),
        CompositionError::FieldOwnershipConflict("display_name".into())
    );
}

#[test]
fn destination_delete_removes_only_materialized_reference_intent() {
    let source = SourceSnapshot::new(
        foreign("shared", "carol", None),
        fields(&[("display_name", json!("Carol"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let reference = MaterializedReference::new(&source).unwrap();
    let identity = reference.foreign.identity_fingerprint().unwrap();

    let deleted =
        apply_destination_mutation(&reference, DestinationMutation::DeleteReference).unwrap();
    assert_eq!(
        deleted,
        DestinationMutationResult::Deleted(any_cal_core::DestinationDeletion {
            foreign_identity_fingerprint: identity,
        })
    );

    // The original source snapshot is unchanged and no source mutation is
    // representable in the deletion result.
    assert_eq!(source.source_fields["display_name"], "Carol");
}

#[test]
fn source_identity_mismatch_fails_closed() {
    let source_a = SourceSnapshot::new(
        foreign("space-a", "same", None),
        fields(&[("display_name", json!("A"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let reference = MaterializedReference::new(&source_a).unwrap();

    let source_b = SourceSnapshot::new(
        foreign("space-b", "same", None),
        fields(&[("display_name", json!("B"))]),
        SourceAvailability::Available,
    )
    .unwrap();

    assert_eq!(
        reconcile_source_snapshot(Some(&reference), &source_b).unwrap_err(),
        CompositionError::IdentityMismatch
    );
}

#[test]
fn materialized_reference_serialization_is_canonical_even_for_nested_json() {
    let source = SourceSnapshot::new(
        foreign("shared", "carol", Some("uid")),
        fields(&[(
            "snapshot",
            json!({
                "z": 1,
                "a": {
                    "two": 2,
                    "one": 1
                }
            }),
        )]),
        SourceAvailability::Available,
    )
    .unwrap();
    let reference = MaterializedReference::new(&source).unwrap();
    let json = reference.canonical_json().unwrap();
    let round_trip = MaterializedReference::from_json(&json).unwrap();
    assert_eq!(reference, round_trip);

    let a = json.find("\"a\"").unwrap();
    let z = json.find("\"z\"").unwrap();
    assert!(a < z);
    let one = json.find("\"one\"").unwrap();
    let two = json.find("\"two\"").unwrap();
    assert!(one < two);
}

#[test]
fn malformed_or_ambiguous_materialized_state_is_rejected() {
    let source = SourceSnapshot::new(
        foreign("shared", "carol", None),
        fields(&[("display_name", json!("Carol"))]),
        SourceAvailability::Available,
    )
    .unwrap();
    let mut reference = MaterializedReference::new(&source).unwrap();
    reference
        .user_fields
        .insert("display_name".into(), json!("collision"));
    assert_eq!(
        reference.validate().unwrap_err(),
        CompositionError::FieldOwnershipConflict("display_name".into())
    );

    let unsupported = format!(
        r#"{{"version":99,"foreign":{},"source_fields":{{}},"user_fields":{{}},"source_availability":"available"}}"#,
        source.foreign.canonical_json().unwrap()
    );
    assert_eq!(
        MaterializedReference::from_json(&unsupported).unwrap_err(),
        CompositionError::UnsupportedVersion
    );
}
