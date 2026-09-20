use any_cal_core::{
    CanonicalDocument, Collection, CollectionId, DavKind, FailureMode, MemoryRepository,
    Occurrence, Repository, RepositoryError, ResourceEnvelope, StructuredDocument, WriteCondition,
};
use std::collections::BTreeMap;

fn id<T: TryFrom<&'static str>>(value: &'static str) -> T
where
    <T as TryFrom<&'static str>>::Error: std::fmt::Debug,
{
    value.try_into().unwrap()
}

fn setup() -> (MemoryRepository, CollectionId) {
    let mut repo = MemoryRepository::new();
    let collection = Collection {
        id: id("tasks"),
        name: "Tasks".into(),
    };
    let collection_id = collection.id.clone();
    repo.create_collection(collection).unwrap();
    (repo, collection_id)
}

fn task(
    collection_id: &CollectionId,
    resource: &'static str,
    object: &'static str,
    uid: &'static str,
) -> ResourceEnvelope {
    ResourceEnvelope {
        collection_id: collection_id.clone(),
        resource_id: id(resource),
        kind: DavKind::Task,
        anytype_object_id: id(object),
        dav_uid: id(uid),
        document: CanonicalDocument::new(StructuredDocument {
            fields: BTreeMap::from([("SUMMARY".into(), vec![Occurrence::new("Write tests")])]),
        }),
        revision: 0,
    }
}

#[test]
fn crud_and_archive_keep_identity_and_membership() {
    let (mut repo, collection_id) = setup();
    let created = repo
        .create_resource(
            task(&collection_id, "r1", "o1", "u1"),
            WriteCondition::IfNoneMatch,
        )
        .unwrap();
    assert_eq!(created.envelope.resource_id, id("r1"));
    assert_eq!(repo.list_resources(&collection_id, false).unwrap().len(), 1);
    let archived = repo
        .archive_resource(&id("r1"), WriteCondition::IfMatch(created.etag.clone()))
        .unwrap();
    assert!(archived.archived);
    assert!(repo
        .list_resources(&collection_id, false)
        .unwrap()
        .is_empty());
    assert_eq!(
        repo.list_resources(&collection_id, true).unwrap()[0]
            .envelope
            .dav_uid,
        id("u1")
    );
    let deleted = repo
        .delete_resource(&id("r1"), WriteCondition::Unconditional)
        .unwrap();
    assert!(deleted.archived);
    assert!(repo.get_resource(&id("r1")).unwrap().is_none());
}

#[test]
fn update_increments_revision_and_preserves_stable_ids() {
    let (mut repo, collection_id) = setup();
    let created = repo
        .create_resource(
            task(&collection_id, "r1", "o1", "u1"),
            WriteCondition::Unconditional,
        )
        .unwrap();
    let mut changed = created.envelope.clone();
    changed.document.content.fields.get_mut("SUMMARY").unwrap()[0].value = "Changed".into();
    let updated = repo
        .update_resource(changed, WriteCondition::IfMatch(created.etag.clone()))
        .unwrap();
    assert_eq!(updated.envelope.revision, 1);
    assert_eq!(updated.envelope.anytype_object_id, id("o1"));
    assert_eq!(updated.envelope.dav_uid, id("u1"));
    assert_ne!(updated.etag, created.etag);
    assert_eq!(
        repo.rebuild_indexes().0,
        ["o1".to_string()].into_iter().collect()
    );
}

#[test]
fn stale_conditions_never_overwrite() {
    let (mut repo, collection_id) = setup();
    let created = repo
        .create_resource(
            task(&collection_id, "r1", "o1", "u1"),
            WriteCondition::Unconditional,
        )
        .unwrap();
    let mut changed = created.envelope.clone();
    changed.document.content.fields.get_mut("SUMMARY").unwrap()[0].value = "Changed".into();
    let stale = repo.update_resource(
        changed.clone(),
        WriteCondition::IfMatch(any_cal_core::typed_etag_for_bytes(b"stale")),
    );
    assert!(matches!(
        stale,
        Err(RepositoryError::PreconditionFailed { .. })
    ));
    assert_eq!(
        repo.get_resource(&id("r1")).unwrap().unwrap().envelope,
        created.envelope
    );
    assert!(matches!(
        repo.create_resource(changed, WriteCondition::IfNoneMatch),
        Err(RepositoryError::PreconditionFailed { .. })
    ));
}

#[test]
fn etag_identity_and_stale_conflict_survive_repository_rebuild() {
    let (mut repo, collection_id) = setup();
    let created = repo
        .create_resource(
            task(
                &collection_id,
                "merge-resource",
                "merge-object",
                "merge-uid",
            ),
            WriteCondition::IfNoneMatch,
        )
        .unwrap();
    let mut rebuilt = repo.clone();
    let reopened = rebuilt
        .get_resource(&id("merge-resource"))
        .unwrap()
        .unwrap();
    assert_eq!(reopened.etag, created.etag);
    assert_eq!(reopened.envelope.dav_uid, id("merge-uid"));
    assert_eq!(reopened.envelope.anytype_object_id, id("merge-object"));

    let mut changed = created.envelope.clone();
    changed.document.content.fields.get_mut("SUMMARY").unwrap()[0].value =
        "Concurrent update".into();
    let updated = repo
        .update_resource(changed, WriteCondition::IfMatch(created.etag.clone()))
        .unwrap();
    let stale = repo.update_resource(
        created.envelope,
        WriteCondition::IfMatch(created.etag.clone()),
    );
    assert!(matches!(
        stale,
        Err(RepositoryError::PreconditionFailed {
            expected: Some(_),
            actual: Some(_)
        })
    ));
    let current = repo.get_resource(&id("merge-resource")).unwrap().unwrap();
    assert_eq!(current.etag, updated.etag);
    assert_eq!(
        current.envelope.document.content.fields["SUMMARY"][0].value,
        "Concurrent update"
    );
}

#[test]
fn modified_at_is_durable_and_independent_from_etag_identity() {
    let (mut repo, collection_id) = setup();
    repo.set_clock_seconds(1_700_000_000);
    let created = repo
        .create_resource(
            task(
                &collection_id,
                "metadata-resource",
                "metadata-object",
                "metadata-uid",
            ),
            WriteCondition::IfNoneMatch,
        )
        .unwrap();
    let encoded = serde_json::to_string(&created).unwrap();
    let reopened: any_cal_core::StoredResource = serde_json::from_str(&encoded).unwrap();
    assert_eq!(reopened.modified_at, created.modified_at);
    assert_eq!(reopened.etag, created.etag);
    assert_eq!(reopened.envelope, created.envelope);

    // A backwards wall clock cannot regress the durable second, and changing
    // only archive metadata does not rewrite the representation ETag.
    repo.set_clock_seconds(1_600_000_000);
    let archived = repo
        .archive_resource(
            &id("metadata-resource"),
            WriteCondition::IfMatch(created.etag.clone()),
        )
        .unwrap();
    assert!(archived.modified_at > created.modified_at);
    assert_eq!(archived.etag, created.etag);
}

#[test]
fn duplicate_identities_are_rejected() {
    let (mut repo, collection_id) = setup();
    repo.create_resource(
        task(&collection_id, "r1", "o1", "u1"),
        WriteCondition::Unconditional,
    )
    .unwrap();
    assert!(matches!(
        repo.create_resource(
            task(&collection_id, "r2", "o1", "u2"),
            WriteCondition::Unconditional
        ),
        Err(RepositoryError::IdentityAlreadyExists(_))
    ));
    assert!(matches!(
        repo.create_resource(
            task(&collection_id, "r2", "o2", "u1"),
            WriteCondition::Unconditional
        ),
        Err(RepositoryError::IdentityAlreadyExists(_))
    ));
}

#[test]
fn injected_failures_are_one_shot_and_non_mutating() {
    let (mut repo, collection_id) = setup();
    repo.inject_failure(FailureMode::Timeout);
    assert!(matches!(
        repo.create_resource(
            task(&collection_id, "r1", "o1", "u1"),
            WriteCondition::Unconditional
        ),
        Err(RepositoryError::Timeout)
    ));
    assert!(repo.get_resource(&id("r1")).unwrap().is_none());
    repo.inject_failure(FailureMode::ArchiveFailure);
    repo.create_resource(
        task(&collection_id, "r1", "o1", "u1"),
        WriteCondition::Unconditional,
    )
    .unwrap();
    assert!(matches!(
        repo.archive_resource(&id("r1"), WriteCondition::Unconditional),
        Err(RepositoryError::ArchiveFailure)
    ));
    assert!(!repo.get_resource(&id("r1")).unwrap().unwrap().archived);
    repo.inject_failure(FailureMode::ReadAfterWriteDelay);
    assert!(matches!(
        repo.get_resource(&id("r1")),
        Err(RepositoryError::ReadAfterWriteDelay)
    ));
    assert!(repo.get_resource(&id("r1")).unwrap().is_some());
}

#[test]
fn collection_delete_hard_deletes_resources_and_archived_visibility_is_explicit() {
    let (mut repo, collection_id) = setup();
    repo.create_resource(
        task(&collection_id, "r1", "o1", "u1"),
        WriteCondition::Unconditional,
    )
    .unwrap();
    repo.archive_resource(&id("r1"), WriteCondition::Unconditional)
        .unwrap();
    assert!(repo
        .list_resources(&collection_id, false)
        .unwrap()
        .is_empty());
    assert_eq!(repo.list_resources(&collection_id, true).unwrap().len(), 1);

    repo.delete_collection(&collection_id).unwrap();
    assert!(repo.get_collection(&collection_id).unwrap().is_none());
    assert!(repo.get_resource(&id("r1")).unwrap().is_none());
}
