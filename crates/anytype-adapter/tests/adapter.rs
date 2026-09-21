use any_cal_anytype_adapter::{
    projected_anytype_properties, AmbiguousMutation, AnytypeRepository, AnytypeTransport,
    ConflictPolicy, FakeAnytypeTransport, ObjectLocks, ObjectRecord, TransportError,
};
use any_cal_core::{
    AnytypeObjectId, CanonicalDocument, CollectionId, DavKind, DavUid, Repository,
    ResourceEnvelope, ResourceId, StructuredDocument,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn object(id: &str, space: &str) -> ObjectRecord {
    ObjectRecord {
        id: id.into(),
        space_id: space.into(),
        properties: vec![("Name".into(), id.into())],
        property_formats: BTreeMap::new(),
        body: format!("line one\n{id}"),
        archived: false,
        revision: 1,
    }
}

#[derive(Clone, Debug)]
struct ServerAssignedTransport {
    inner: FakeAnytypeTransport,
    next_id: usize,
}

impl ServerAssignedTransport {
    fn new() -> Self {
        Self {
            inner: FakeAnytypeTransport::new(10),
            next_id: 1,
        }
    }

    fn timeout_after_create(mut self) -> Self {
        self.inner.timeout_after(AmbiguousMutation::Create);
        self
    }
}

impl AnytypeTransport for ServerAssignedTransport {
    fn list_objects(
        &mut self,
        space_id: &str,
        cursor: Option<&str>,
    ) -> Result<any_cal_anytype_adapter::Page<ObjectRecord>, TransportError> {
        let mut page = self.inner.list_objects(space_id, cursor)?;
        // Match the API's summary list response: the full markdown body is
        // available from GET, not from the list item.
        for object in &mut page.data {
            object.body.clear();
        }
        Ok(page)
    }

    fn get_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.get_object(space_id, object_id)
    }

    fn create_object(&mut self, mut object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        object.id = format!("server-object-{}", self.next_id);
        self.next_id += 1;
        self.inner.create_object(object)
    }

    fn update_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        self.inner.update_object(object)
    }

    fn archive_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.archive_object(space_id, object_id)
    }

    fn delete_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.delete_object(space_id, object_id)
    }
}

#[test]
fn fixtures_are_sanitized_and_fake_paginates_per_space() {
    let page_fixture = include_str!("../../../fixtures/anytype-api/list-objects-page.json");
    assert!(page_fixture.contains("space-demo") && page_fixture.contains("pagination"));
    let mut fake = FakeAnytypeTransport::new(1);
    fake.create_object(object("a", "s1")).unwrap();
    fake.create_object(object("b", "s1")).unwrap();
    fake.create_object(object("other", "s2")).unwrap();
    let first = fake.list_objects("s1", None).unwrap();
    assert_eq!(first.data.len(), 1);
    let second = fake
        .list_objects("s1", first.next_offset.as_deref())
        .unwrap();
    assert_eq!(second.data.len(), 1);
    assert!(second.next_offset.is_none());
}

#[test]
fn fake_crud_archive_delete_and_typed_failures_are_executable() {
    let mut fake = FakeAnytypeTransport::new(10);
    let created = fake.create_object(object("id", "space")).unwrap();
    assert_eq!(fake.get_object("space", "id").unwrap(), created);
    let mut updated = created.clone();
    updated.body = "utf-8: café\nmultiline".into();
    updated.revision = 2;
    assert_eq!(fake.update_object(updated.clone()).unwrap(), updated);
    assert!(fake.archive_object("space", "id").unwrap().archived);
    fake.inject(TransportError::RateLimited);
    assert_eq!(
        fake.get_object("space", "id"),
        Err(TransportError::RateLimited)
    );
    assert_eq!(fake.delete_object("space", "id").unwrap().id, "id");
    assert_eq!(
        fake.get_object("space", "id"),
        Err(TransportError::NotFound)
    );
}

fn envelope(collection: &str, id: &str, kind: DavKind) -> ResourceEnvelope {
    ResourceEnvelope {
        collection_id: CollectionId::try_from(collection).unwrap(),
        resource_id: ResourceId::try_from(id).unwrap(),
        kind,
        anytype_object_id: AnytypeObjectId::try_from(format!("obj-{id}")).unwrap(),
        dav_uid: DavUid::try_from(format!("uid-{id}")).unwrap(),
        document: CanonicalDocument::new(StructuredDocument {
            fields: BTreeMap::new(),
        }),
        revision: 4,
    }
}

#[test]
fn repository_reads_hydrate_empty_cache_with_pages_and_etags() {
    let first = envelope("contacts", "c1", DavKind::Contact);
    let second = envelope("tasks", "t1", DavKind::Task);
    let mut fake = FakeAnytypeTransport::new(1);
    for e in [&first, &second] {
        fake.objects.insert(
            e.anytype_object_id.to_string(),
            ObjectRecord {
                id: e.anytype_object_id.to_string(),
                space_id: "space".into(),
                properties: vec![],
                property_formats: BTreeMap::new(),
                body: e.canonical_json().unwrap(),
                archived: false,
                revision: e.revision,
            },
        );
    }
    let mut repo = AnytypeRepository::new(fake, "space");
    assert_eq!(repo.list_collections().unwrap().len(), 2);
    assert!(repo
        .get_collection(&CollectionId::try_from("contacts").unwrap())
        .unwrap()
        .is_some());
    let resources = repo
        .list_resources(&CollectionId::try_from("contacts").unwrap(), false)
        .unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].envelope.dav_uid, first.dav_uid);
    assert_eq!(resources[0].envelope.revision, 4);
    assert!(!resources[0].etag.as_str().is_empty());
    let etag = resources[0].etag.clone();
    assert!(repo
        .get_resource(&ResourceId::try_from("c1").unwrap())
        .unwrap()
        .is_some());
    assert!(repo.transport.list_calls >= 3);
    assert_eq!(
        repo.get_resource(&ResourceId::try_from("c1").unwrap())
            .unwrap()
            .unwrap()
            .etag,
        etag
    );
}

#[test]
fn server_id_is_separate_from_dav_uid_across_reload_update_and_delete() {
    let input = envelope("contacts", "contact-1", DavKind::Contact);
    let mut repo = AnytypeRepository::new(ServerAssignedTransport::new(), "space");
    let created = repo
        .create_resource(input.clone(), any_cal_core::WriteCondition::Unconditional)
        .unwrap();

    assert_eq!(created.envelope.dav_uid.as_str(), "uid-contact-1");
    assert_eq!(
        created.envelope.anytype_object_id.as_str(),
        "server-object-1"
    );
    assert_ne!(
        created.envelope.dav_uid.as_str(),
        created.envelope.anytype_object_id.as_str()
    );
    assert!(
        repo.transport
            .inner
            .objects
            .get("server-object-1")
            .unwrap()
            .properties
            .is_empty(),
        "body-only mode must not require a custom Anytype property schema"
    );

    // Reopening with only the remote transport must hydrate the canonical
    // envelope from GET after the summary-only list response.
    let transport = repo.transport.clone();
    drop(repo);
    let mut reopened = AnytypeRepository::new(transport, "space");
    let loaded = reopened
        .get_resource(&ResourceId::try_from("contact-1").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.envelope.anytype_object_id.as_str(),
        "server-object-1"
    );
    assert_eq!(loaded.envelope.dav_uid.as_str(), "uid-contact-1");

    let mut changed = loaded.envelope.clone();
    changed
        .document
        .content
        .fields
        .insert("FN".into(), vec![any_cal_core::Occurrence::new("Updated")]);
    let updated = reopened
        .update_resource(changed, any_cal_core::WriteCondition::Unconditional)
        .unwrap();
    assert_eq!(
        updated.envelope.anytype_object_id.as_str(),
        "server-object-1"
    );
    assert_eq!(updated.envelope.dav_uid.as_str(), "uid-contact-1");

    reopened
        .delete_resource(
            &ResourceId::try_from("contact-1").unwrap(),
            any_cal_core::WriteCondition::Unconditional,
        )
        .unwrap();
    assert!(!reopened
        .transport
        .inner
        .objects
        .contains_key("server-object-1"));
}

#[test]
fn ambiguous_create_reconciles_by_dav_markers_when_server_id_is_unknown() {
    let input = envelope("contacts", "contact-timeout", DavKind::Contact);
    let mut repo = AnytypeRepository::new(
        ServerAssignedTransport::new().timeout_after_create(),
        "space",
    );
    let created = repo
        .create_resource(input, any_cal_core::WriteCondition::Unconditional)
        .unwrap();

    assert_eq!(
        created.envelope.anytype_object_id.as_str(),
        "server-object-1"
    );
    assert_eq!(created.envelope.dav_uid.as_str(), "uid-contact-timeout");
    assert_eq!(repo.metrics.ambiguous_mutations, 1);
    assert_eq!(repo.metrics.reconciled_mutations, 1);
    assert_eq!(repo.receipts[0].outcome, "reconciled");
}

#[test]
fn property_projection_has_stable_identity_kind_and_occurrence_keys() {
    let mut e = envelope("contacts", "c1", DavKind::Contact);
    e.document.content.fields.insert(
        "TEL".into(),
        vec![
            any_cal_core::Occurrence::new("+1-555-0100"),
            any_cal_core::Occurrence::new("+1-555-0101"),
        ],
    );
    assert_eq!(
        projected_anytype_properties(&e),
        vec![
            ("dav_uid".into(), "uid-c1".into()),
            ("dav_kind".into(), "Contact".into()),
            ("dav_property_tel_0".into(), "+1-555-0100".into()),
            ("dav_property_tel_1".into(), "+1-555-0101".into()),
        ]
    );
}

#[test]
fn update_carries_unknown_anytype_properties_forward() {
    let e = envelope("contacts", "c1", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.objects.insert(
        e.anytype_object_id.to_string(),
        ObjectRecord {
            id: e.anytype_object_id.to_string(),
            space_id: "space".into(),
            properties: vec![
                ("custom_label".into(), "keep-me".into()),
                ("creator".into(), "system-user".into()),
                ("last_modified_date".into(), "system-date".into()),
            ],
            property_formats: BTreeMap::new(),
            body: e.canonical_json().unwrap(),
            archived: false,
            revision: e.revision,
        },
    );
    let mut repo = AnytypeRepository::new(fake, "space");
    repo.list_collections().unwrap();
    repo.update_resource(e.clone(), any_cal_core::WriteCondition::Unconditional)
        .unwrap();
    let remote = repo
        .transport
        .objects
        .get(e.anytype_object_id.as_str())
        .unwrap();
    assert!(remote
        .properties
        .contains(&("custom_label".into(), "keep-me".into())));
    assert!(!remote
        .properties
        .iter()
        .any(|(key, _)| key == "creator" || key == "last_modified_date"));
}

#[test]
fn refresh_is_atomic_and_replaces_stale_rows() {
    let original = envelope("contacts", "c1", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(1);
    fake.objects.insert(
        "obj-c1".into(),
        ObjectRecord {
            id: "obj-c1".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: original.canonical_json().unwrap(),
            archived: false,
            revision: original.revision,
        },
    );
    let mut repo = AnytypeRepository::new(fake, "space");
    repo.list_collections().unwrap();
    let before = repo.get_resource(&original.resource_id).unwrap().unwrap();

    let mut changed = original.clone();
    changed.revision = 5;
    changed
        .document
        .content
        .fields
        .insert("FN".into(), vec![any_cal_core::Occurrence::new("Changed")]);
    repo.transport.objects.insert(
        "obj-c1".into(),
        ObjectRecord {
            id: "obj-c1".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: changed.canonical_json().unwrap(),
            archived: false,
            revision: changed.revision,
        },
    );
    let after = repo.get_resource(&original.resource_id).unwrap().unwrap();
    assert_eq!(after.envelope.revision, 5);
    assert_ne!(after.etag, before.etag);

    // A malformed later page must leave the already-populated cache intact.
    repo.transport.objects.insert(
        "obj-bad".into(),
        ObjectRecord {
            id: "obj-bad".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: r#"{"resource_id":"bad","document":{}}"#.into(),
            archived: false,
            revision: 1,
        },
    );
    let stable = repo
        .cache
        .get_resource(&original.resource_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        repo.list_collections(),
        Err(any_cal_core::RepositoryError::InvalidEnvelope(_))
    ));
    let remains = repo
        .cache
        .get_resource(&original.resource_id)
        .unwrap()
        .unwrap();
    assert_eq!(remains, stable);
}

#[test]
fn malformed_remote_envelope_is_an_explicit_error() {
    let mut fake = FakeAnytypeTransport::new(10);
    fake.objects.insert(
        "bad".into(),
        ObjectRecord {
            id: "bad".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: r#"{"dav_uid":"bad"}"#.into(),
            archived: false,
            revision: 1,
        },
    );
    let mut repo = AnytypeRepository::new(fake, "space");
    assert!(matches!(
        repo.list_collections(),
        Err(any_cal_core::RepositoryError::InvalidEnvelope(_))
    ));
}

#[test]
fn timeout_after_create_is_reconciled_without_a_blind_retry() {
    let e = envelope("contacts", "c-timeout", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.timeout_after(AmbiguousMutation::Create);
    let mut repo = AnytypeRepository::new(fake, "space");
    repo.create_resource(e, any_cal_core::WriteCondition::Unconditional)
        .unwrap();
    assert_eq!(repo.transport.create_calls, 1);
    assert_eq!(repo.metrics.ambiguous_mutations, 1);
    assert_eq!(repo.metrics.reconciled_mutations, 1);
    assert_eq!(repo.receipts.len(), 1);
    assert_eq!(repo.receipts[0].outcome, "reconciled");
}

#[test]
fn archive_is_confirmed_and_omitted_from_normal_relist() {
    let e = envelope("contacts", "c-archive", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.create_object(ObjectRecord {
        id: e.anytype_object_id.to_string(),
        space_id: "space".into(),
        properties: projected_anytype_properties(&e),
        property_formats: BTreeMap::new(),
        body: e.canonical_json().unwrap(),
        archived: false,
        revision: e.revision,
    })
    .unwrap();
    let mut repo = AnytypeRepository::new(fake, "space");
    repo.archive_resource(&e.resource_id, any_cal_core::WriteCondition::Unconditional)
        .unwrap();
    assert_eq!(repo.metrics.archive_confirmations, 1);
    assert!(repo
        .list_resources(&e.collection_id, false)
        .unwrap()
        .is_empty());
    assert!(repo
        .transport
        .objects
        .get_in_space("space", e.anytype_object_id.as_str())
        .unwrap()
        .archived);
}

#[test]
fn object_lock_order_is_canonical_and_receipts_do_not_store_secrets() {
    let order = ObjectLocks::canonical_order(&["b".into(), "a".into(), "b".into()]);
    assert_eq!(order, vec!["a", "b"]);
    let locks = ObjectLocks::default();
    assert_eq!(locks.with_locks(&["b".into(), "a".into()], || 7), 7);
    let receipt = any_cal_anytype_adapter::OperationReceipt {
        operation_id: "update:o1".into(),
        object_id: "o1".into(),
        kind: any_cal_anytype_adapter::OperationKind::Update,
        outcome: "applied".into(),
        reconciliation_reads: 0,
    };
    assert!(!receipt.redacted_summary().contains("token"));
    assert!(!receipt.redacted_summary().contains("password"));
    assert_eq!(
        ConflictPolicy::LaterWriteWins,
        ConflictPolicy::LaterWriteWins
    );
}

#[test]
fn object_lock_serializes_same_object_across_threads() {
    let locks = ObjectLocks::default();
    let active = Arc::new(Mutex::new(0_u32));
    let maximum = Arc::new(Mutex::new(0_u32));
    let mut workers = Vec::new();

    for _ in 0..8 {
        let locks = locks.clone();
        let active = active.clone();
        let maximum = maximum.clone();
        workers.push(thread::spawn(move || {
            locks.with_lock("same-object", || {
                let current = {
                    let mut count = active.lock().unwrap();
                    *count += 1;
                    *count
                };
                {
                    let mut peak = maximum.lock().unwrap();
                    *peak = (*peak).max(current);
                }
                thread::sleep(Duration::from_millis(1));
                *active.lock().unwrap() -= 1;
            });
        }));
    }

    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(*maximum.lock().unwrap(), 1);
    assert_eq!(*active.lock().unwrap(), 0);
}

#[test]
fn repository_projects_properties_and_hides_archived_or_wrong_kind_objects() {
    let mut contact = envelope("contacts", "c1", DavKind::Contact);
    contact.document.content.fields.insert(
        "FN".into(),
        vec![any_cal_core::Occurrence::new("Ada & Grace")],
    );
    let archived = envelope("contacts", "old", DavKind::Contact);
    let event = envelope("events", "e1", DavKind::Event);
    let mut fake = FakeAnytypeTransport::new(10);
    for (e, archived) in [(&contact, false), (&archived, true), (&event, false)] {
        fake.objects.insert(
            e.anytype_object_id.to_string(),
            ObjectRecord {
                id: e.anytype_object_id.to_string(),
                space_id: "space".into(),
                properties: if e.resource_id.as_str() == "c1" {
                    vec![
                        ("dav_uid".into(), e.dav_uid.to_string()),
                        ("dav_kind".into(), "Contact".into()),
                        ("dav_property_fn_0".into(), "Ada & Grace".into()),
                        ("custom_label".into(), "kept".into()),
                    ]
                } else {
                    vec![]
                },
                property_formats: BTreeMap::new(),
                body: e.canonical_json().unwrap(),
                archived,
                revision: e.revision,
            },
        );
    }
    let mut repo = AnytypeRepository::new(fake, "space");
    let resources = repo
        .list_resources(&CollectionId::try_from("contacts").unwrap(), false)
        .unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].envelope.dav_uid, contact.dav_uid);
    assert!(repo
        .get_resource(&ResourceId::try_from("old").unwrap())
        .unwrap()
        .is_none());
    assert!(repo
        .get_resource(&ResourceId::try_from("e1").unwrap())
        .unwrap()
        .is_none());
}

#[test]
fn unrelated_anytype_pages_are_ignored_during_hydration() {
    let contact = envelope("contacts", "c1", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.objects.insert(
        "ordinary-note".into(),
        ObjectRecord {
            id: "ordinary-note".into(),
            space_id: "space".into(),
            properties: vec![("title".into(), "A normal Anytype page".into())],
            property_formats: BTreeMap::from([("title".into(), "text".into())]),
            body: "This is not an Any-Cal envelope".into(),
            archived: false,
            revision: 0,
        },
    );
    fake.objects.insert(
        "obj-c1".into(),
        ObjectRecord {
            id: "obj-c1".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: contact.canonical_json().unwrap(),
            archived: false,
            revision: 0,
        },
    );
    let mut repo = AnytypeRepository::new(fake, "space");
    let resources = repo
        .list_resources(&CollectionId::try_from("contacts").unwrap(), false)
        .unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].envelope.resource_id, contact.resource_id);
}

#[test]
fn delayed_read_is_typed_through_repository() {
    let e = envelope("contacts", "c1", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.objects.insert(
        e.anytype_object_id.to_string(),
        ObjectRecord {
            id: e.anytype_object_id.to_string(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: e.canonical_json().unwrap(),
            archived: false,
            revision: e.revision,
        },
    );
    fake.delay_next_read = true;
    let mut repo = AnytypeRepository::new(fake, "space");
    assert_eq!(
        repo.list_collections(),
        Err(any_cal_core::RepositoryError::ReadAfterWriteDelay)
    );
}

#[test]
fn server_id_mismatch_is_adopted_when_dav_identity_matches() {
    let e = envelope("contacts", "c1", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.objects.insert(
        "wrong-object-id".into(),
        ObjectRecord {
            id: "wrong-object-id".into(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: e.canonical_json().unwrap(),
            archived: false,
            revision: e.revision,
        },
    );
    let mut repo = AnytypeRepository::new(fake, "space");
    assert_eq!(repo.list_collections().unwrap().len(), 1);
    let loaded = repo
        .get_resource(&e.resource_id)
        .unwrap()
        .expect("canonical DAV identity remains discoverable");
    assert_eq!(
        loaded.envelope.anytype_object_id.as_str(),
        "wrong-object-id"
    );
}

#[test]
fn delayed_visibility_is_distinct_from_not_found() {
    let e = envelope("contacts", "delayed", DavKind::Contact);
    let mut fake = FakeAnytypeTransport::new(10);
    fake.objects.insert(
        e.anytype_object_id.to_string(),
        ObjectRecord {
            id: e.anytype_object_id.to_string(),
            space_id: "space".into(),
            properties: vec![],
            property_formats: BTreeMap::new(),
            body: e.canonical_json().unwrap(),
            archived: false,
            revision: e.revision,
        },
    );
    fake.delayed = true;
    fake.delay_next_read = true;
    let mut repo = AnytypeRepository::new(fake, "space");
    assert!(matches!(
        repo.list_resources(&CollectionId::try_from("contacts").unwrap(), false),
        Err(any_cal_core::RepositoryError::ReadAfterWriteDelay)
    ));
    assert!(matches!(
        repo.get_resource(&ResourceId::try_from("missing").unwrap()),
        Ok(None)
    ));
}

#[test]
fn transport_error_categories_are_explicit() {
    let errors = [
        (TransportError::Auth, "auth"),
        (TransportError::Forbidden, "forbidden"),
        (TransportError::RateLimited, "rate_limited"),
        (TransportError::Unavailable, "unavailable"),
        (TransportError::NotFound, "not_found"),
        (TransportError::Conflict, "conflict"),
        (TransportError::Timeout, "timeout"),
        (TransportError::Malformed, "malformed"),
        (TransportError::DelayedVisibility, "delayed_visibility"),
    ];
    for (error, category) in errors {
        assert_eq!(error.category(), category);
    }
    assert_eq!(
        AnytypeRepository::<FakeAnytypeTransport>::map_transport_error(
            TransportError::DelayedVisibility
        ),
        any_cal_core::RepositoryError::ReadAfterWriteDelay
    );
}
