use any_cal_app::identity::{
    AccessDecision, AccessPolicy, AuthOutcome, Capability, CollectionKind, CredentialSpec,
    IdentityStore, Operation, PrincipalId,
};
use std::collections::BTreeSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn principal(value: &str) -> PrincipalId {
    PrincipalId::new(value).unwrap()
}

fn spec(id: &str, owner: &str, not_before: i64, expires_at: i64) -> CredentialSpec {
    CredentialSpec {
        id: id.into(),
        principal: principal(owner),
        not_before,
        expires_at,
        capabilities: BTreeSet::new(),
    }
}

#[test]
fn synthetic_credentials_have_deterministic_expiry_skew_and_revocation_outcomes() {
    let mut store = IdentityStore::new(AccessPolicy::new());
    store
        .add_credential(spec("alice-key", "alice", 100, 200), "alice-token")
        .unwrap();

    assert_eq!(
        store.authenticate_outcome("wrong-token", 150),
        AuthOutcome::Invalid
    );
    assert_eq!(
        store.authenticate_outcome("alice-token", 99),
        AuthOutcome::NotYetValid
    );
    assert_eq!(
        store.authenticate_outcome("alice-token", 100),
        AuthOutcome::Authenticated
    );
    assert_eq!(
        store.authenticate_outcome("alice-token", 199),
        AuthOutcome::Authenticated
    );
    assert_eq!(
        store.authenticate_outcome("alice-token", 200),
        AuthOutcome::Expired
    );
    assert!(store.revoke("alice-key"));
    assert_eq!(
        store.authenticate_outcome("alice-token", 150),
        AuthOutcome::Revoked
    );
    assert_eq!(
        store.authenticate_outcome("bad\n-token", 150),
        AuthOutcome::Invalid
    );
}

#[test]
fn collection_and_resource_acl_are_principal_scoped_and_non_leaky() {
    let alice = principal("alice");
    let bob = principal("bob");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(alice.clone(), CollectionKind::Contacts, Operation::Read);
    policy.grant_collection(alice.clone(), CollectionKind::Contacts, Operation::Write);
    policy.grant_collection(bob.clone(), CollectionKind::Tasks, Operation::Read);
    policy.grant_resource(
        bob.clone(),
        CollectionKind::Contacts,
        "shared.vcf",
        Operation::Read,
    );

    assert_eq!(
        policy.authorize(&alice, CollectionKind::Contacts, None, Operation::Read),
        AccessDecision::Allowed
    );
    assert_eq!(
        policy.authorize(
            &alice,
            CollectionKind::Contacts,
            Some("private.vcf"),
            Operation::Read
        ),
        AccessDecision::Allowed
    );
    assert_eq!(
        policy.authorize(&bob, CollectionKind::Contacts, None, Operation::Read),
        AccessDecision::Forbidden
    );
    assert_eq!(
        policy.authorize(
            &bob,
            CollectionKind::Contacts,
            Some("private.vcf"),
            Operation::Read
        ),
        AccessDecision::NotFound
    );
    assert_eq!(
        policy.authorize(
            &bob,
            CollectionKind::Contacts,
            Some("private.vcf"),
            Operation::Write
        ),
        AccessDecision::NotFound
    );
    assert_eq!(
        policy.authorize(
            &bob,
            CollectionKind::Contacts,
            Some("shared.vcf"),
            Operation::Read
        ),
        AccessDecision::Allowed
    );
}

#[test]
fn domain_scoped_acl_keeps_identical_collections_and_resources_isolated() {
    let alice = principal("alice");
    let mut policy = AccessPolicy::new();
    assert!(policy.grant_domain_collection(
        alice.clone(),
        "personal",
        CollectionKind::Contacts,
        Operation::Read,
    ));
    assert!(policy.grant_domain_collection(
        alice.clone(),
        "personal",
        CollectionKind::Contacts,
        Operation::Write,
    ));
    assert!(policy.grant_domain_collection(
        alice.clone(),
        "shared",
        CollectionKind::Contacts,
        Operation::Read,
    ));
    assert!(policy.grant_domain_resource(
        alice.clone(),
        "shared",
        CollectionKind::Tasks,
        "same-id.ics",
        Operation::Read,
    ));
    assert!(!policy.grant_domain_collection(
        alice.clone(),
        "bad domain",
        CollectionKind::Contacts,
        Operation::Read,
    ));

    assert_eq!(
        policy.authorize_domain(
            &alice,
            "personal",
            CollectionKind::Contacts,
            Some("same-id.vcf"),
            Operation::Write,
        ),
        AccessDecision::Allowed
    );
    assert_eq!(
        policy.authorize_domain(
            &alice,
            "shared",
            CollectionKind::Contacts,
            Some("same-id.vcf"),
            Operation::Write,
        ),
        AccessDecision::NotFound
    );
    assert_eq!(
        policy.authorize_domain(
            &alice,
            "shared",
            CollectionKind::Contacts,
            Some("same-id.vcf"),
            Operation::Read,
        ),
        AccessDecision::Allowed
    );
    assert_eq!(
        policy.authorize_domain(
            &alice,
            "personal",
            CollectionKind::Tasks,
            Some("same-id.ics"),
            Operation::Read,
        ),
        AccessDecision::NotFound
    );
    assert_eq!(
        policy.authorize_domain(
            &alice,
            "shared",
            CollectionKind::Tasks,
            Some("same-id.ics"),
            Operation::Read,
        ),
        AccessDecision::Allowed
    );
    assert_eq!(
        policy.capabilities_for_domain(&alice, "personal"),
        BTreeSet::from([Capability::ReadContacts, Capability::WriteContacts])
    );
    assert_eq!(
        policy.capabilities_for_domain(&alice, "shared"),
        BTreeSet::from([Capability::ReadContacts])
    );
}

#[test]
fn capability_discovery_is_authenticated_and_excludes_resource_ids() {
    let alice = principal("alice");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(alice.clone(), CollectionKind::Contacts, Operation::Read);
    policy.grant_collection(alice.clone(), CollectionKind::Contacts, Operation::Write);
    policy.grant_resource(alice, CollectionKind::Tasks, "secret.ics", Operation::Read);
    let mut store = IdentityStore::new(policy);
    let mut alice_credential = spec("alice-key", "alice", 0, 100);
    alice_credential.capabilities = BTreeSet::from([
        Capability::ReadContacts,
        Capability::WriteContacts,
        Capability::ReadTasks,
    ]);
    store
        .add_credential(alice_credential, "alice-token")
        .unwrap();

    assert!(store.capabilities("wrong-token", 1).is_empty());
    assert_eq!(
        store.capabilities("alice-token", 1),
        BTreeSet::from([Capability::ReadContacts, Capability::WriteContacts])
    );
}

#[test]
fn cloned_snapshot_preserves_decisions_across_restart_without_secret_metadata() {
    let alice = principal("alice");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(alice, CollectionKind::Tasks, Operation::Read);
    let mut before = IdentityStore::new(policy);
    before
        .add_credential(spec("alice-key", "alice", 10, 20), "alice-token")
        .unwrap();
    let after = before.clone();

    for now in [9, 10, 19, 20] {
        assert_eq!(
            before.authenticate_outcome("alice-token", now),
            after.authenticate_outcome("alice-token", now)
        );
    }
    assert_eq!(before, after);
    let debug = format!("{after:?}");
    assert!(!debug.contains("alice-token"));
}

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "any-cal-identity-{label}-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn digest_only_snapshot_is_atomic_private_and_restart_safe() {
    let path = temp_path("persist");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(
        principal("alice"),
        CollectionKind::Contacts,
        Operation::Read,
    );
    let mut before = IdentityStore::new(policy);
    before
        .add_credential(spec("alice-key", "alice", 10, 20), "synthetic-secret")
        .unwrap();
    before.save_to(&path).unwrap();

    let bytes = fs::read(&path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("synthetic-secret"));
    assert!(text.contains("token_digest_hex"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let after = IdentityStore::load_from(&path).unwrap();
    assert_eq!(
        after.authenticate_outcome("synthetic-secret", 9),
        AuthOutcome::NotYetValid
    );
    assert_eq!(
        after.authenticate_outcome("synthetic-secret", 10),
        AuthOutcome::Authenticated
    );
    assert_eq!(
        after.authenticate_outcome("synthetic-secret", 20),
        AuthOutcome::Expired
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn revoked_state_and_acl_survive_restart_and_malformed_state_fails_closed() {
    let path = temp_path("revoke");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(principal("alice"), CollectionKind::Tasks, Operation::Read);
    let mut before = IdentityStore::new(policy);
    before
        .add_credential(spec("alice-key", "alice", 0, 100), "synthetic-secret")
        .unwrap();
    assert!(before.revoke("alice-key"));
    before.save_to(&path).unwrap();
    let after = IdentityStore::load_from(&path).unwrap();
    assert_eq!(
        after.authenticate_outcome("synthetic-secret", 50),
        AuthOutcome::Revoked
    );
    assert_eq!(
        after.policy.authorize(
            &principal("alice"),
            CollectionKind::Tasks,
            None,
            Operation::Read
        ),
        AccessDecision::Allowed
    );

    fs::write(&path, b"{\"version\":1,\"credentials\":[").unwrap();
    let error = IdentityStore::load_from(&path).unwrap_err();
    assert!(matches!(
        error,
        any_cal_app::identity::IdentityError::Corrupt(_)
    ));
    fs::write(
        &path,
        b"{\"version\":3,\"credentials\":[],\"collections\":[],\"resources\":[]}",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(matches!(
            IdentityStore::load_from(&path),
            Err(any_cal_app::identity::IdentityError::UnsafePath)
        ));
        permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&path, permissions).unwrap();
    }
    assert!(matches!(
        IdentityStore::load_from(&path),
        Err(any_cal_app::identity::IdentityError::Corrupt(_))
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn synthetic_rotation_overlaps_generations_then_revokes_only_the_old_one() {
    let path = temp_path("rotation");
    let mut policy = AccessPolicy::new();
    policy.grant_collection(
        principal("alice"),
        CollectionKind::Contacts,
        Operation::Read,
    );
    let mut before = IdentityStore::new(policy);
    before
        .add_credential(
            spec("alice-generation-1", "alice", 100, 300),
            "synthetic-generation-one",
        )
        .unwrap();

    // Rotation is additive: the replacement generation is introduced before
    // the old generation is revoked, giving operators an explicit overlap
    // window for clients to switch credentials.
    before
        .add_credential(
            spec("alice-generation-2", "alice", 100, 400),
            "synthetic-generation-two",
        )
        .unwrap();
    assert_eq!(
        before.authenticate_outcome("synthetic-generation-one", 150),
        AuthOutcome::Authenticated
    );
    assert_eq!(
        before.authenticate_outcome("synthetic-generation-two", 150),
        AuthOutcome::Authenticated
    );

    // A duplicate create/replay is rejected without changing the active
    // generation, while an unknown revoke is a harmless no-op.
    assert_eq!(
        before
            .add_credential(
                spec("alice-generation-2", "alice", 100, 400),
                "synthetic-generation-two-replay",
            )
            .unwrap_err(),
        any_cal_app::identity::IdentityError::DuplicateCredential
    );
    assert!(!before.revoke("alice-generation-unknown"));

    before.save_to(&path).unwrap();
    let mut after = IdentityStore::load_from(&path).unwrap();
    assert_eq!(
        after.authenticate_outcome("synthetic-generation-one", 150),
        AuthOutcome::Authenticated
    );
    assert_eq!(
        after.authenticate_outcome("synthetic-generation-two", 150),
        AuthOutcome::Authenticated
    );

    assert!(after.revoke("alice-generation-1"));
    // Replaying the same revoke is idempotent and cannot revoke generation 2.
    assert!(after.revoke("alice-generation-1"));
    assert_eq!(
        after.authenticate_outcome("synthetic-generation-one", 150),
        AuthOutcome::Revoked
    );
    assert_eq!(
        after.authenticate_outcome("synthetic-generation-two", 150),
        AuthOutcome::Authenticated
    );

    after.save_to(&path).unwrap();
    let restarted = IdentityStore::load_from(&path).unwrap();
    assert_eq!(
        restarted.authenticate_outcome("synthetic-generation-one", 150),
        AuthOutcome::Revoked
    );
    assert_eq!(
        restarted.authenticate_outcome("synthetic-generation-two", 150),
        AuthOutcome::Authenticated
    );
    let debug = format!("{restarted:?}");
    assert!(!debug.contains("synthetic-generation-one"));
    assert!(!debug.contains("synthetic-generation-two"));
    fs::remove_file(path).unwrap();
}
