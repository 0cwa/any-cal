use any_cal_app::composition::{CompositionAuthorizationError, CompositionSourceScope};
use any_cal_app::identity::{
    AccessPolicy, Capability, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::{App, AppConfig};
use any_cal_core::{CompositionError, CompositionProfile, ProjectionPolicy};
use std::collections::BTreeSet;

fn principal(value: &str) -> PrincipalId {
    PrincipalId::new(value).unwrap()
}

fn config(destination_visibility: &str) -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-must-not-compose".into();
    config.credential_profile_id = "primary".into();
    config.domain_bindings_json = Some(format!(
        r#"{{
              "version":1,
              "bindings":[
                {{
                  "domain_id":"home",
                  "label":"Home",
                  "space_id":"space-home",
                  "credential_profile_id":"primary",
                  "routes":[
                    {{"collection":"contacts","component":"vcard","path":"/carddav/home"}}
                  ],
                  "schema_profile":"default",
                  "checkpoint_namespace":"home",
                  "visibility":"{destination_visibility}",
                  "lifecycle":"configured"
                }},
                {{
                  "domain_id":"shared",
                  "label":"Shared",
                  "space_id":"space-shared",
                  "credential_profile_id":"primary",
                  "routes":[
                    {{"collection":"contacts","component":"vcard","path":"/carddav/shared"}}
                  ],
                  "schema_profile":"default",
                  "checkpoint_namespace":"shared",
                  "visibility":"shared",
                  "lifecycle":"configured"
                }},
                {{
                  "domain_id":"tasks",
                  "label":"Tasks",
                  "space_id":"space-tasks",
                  "credential_profile_id":"primary",
                  "routes":[
                    {{"collection":"tasks","component":"vtodo","path":"/caldav/tasks"}}
                  ],
                  "schema_profile":"default",
                  "checkpoint_namespace":"tasks",
                  "visibility":"shared",
                  "lifecycle":"configured"
                }}
              ]
            }}"#
    ));
    config
}

fn profile(principal_id: &str) -> CompositionProfile {
    CompositionProfile::new(
        principal_id,
        "home",
        vec!["shared".into(), "tasks".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap()
}

fn identity(
    source_collection_grant: bool,
    source_resource: Option<&str>,
    destination_grant: bool,
    capabilities: BTreeSet<Capability>,
) -> IdentityStore {
    let alice = principal("alice");
    let mut policy = AccessPolicy::new();
    if source_collection_grant {
        assert!(policy.grant_domain_collection(
            alice.clone(),
            "shared",
            CollectionKind::Contacts,
            Operation::Read,
        ));
    }
    if let Some(resource_id) = source_resource {
        assert!(policy.grant_domain_resource(
            alice.clone(),
            "shared",
            CollectionKind::Contacts,
            resource_id,
            Operation::Read,
        ));
    }
    assert!(policy.grant_domain_collection(
        alice.clone(),
        "tasks",
        CollectionKind::Tasks,
        Operation::Read,
    ));
    if destination_grant {
        assert!(policy.grant_domain_collection(
            alice.clone(),
            "home",
            CollectionKind::Composition,
            Operation::Write,
        ));
    }

    let mut store = IdentityStore::new(policy);
    store
        .add_credential(
            CredentialSpec {
                id: "alice-key".into(),
                principal: alice,
                not_before: 100,
                expires_at: 200,
                capabilities,
            },
            "alice-token",
        )
        .unwrap();
    store
}

fn app(store: IdentityStore) -> App {
    App::fake(config("private"))
        .unwrap()
        .with_identity(store, 150)
}

#[test]
fn composition_requires_source_read_and_distinct_destination_composition_write() {
    let store = identity(true, None, true, BTreeSet::new());
    let app = app(store);
    let scopes = [
        CompositionSourceScope::collection("tasks", CollectionKind::Tasks),
        CompositionSourceScope::collection("shared", CollectionKind::Contacts),
    ];

    let plan = app
        .authorize_composition(&profile("alice"), "alice-token", &scopes)
        .unwrap();

    assert_eq!(plan.principal_id, "alice");
    assert_eq!(plan.destination_domain_id, "home");
    assert_eq!(plan.sources.len(), 2);
    assert_eq!(plan.sources[0].domain_id, "shared");
    assert_eq!(plan.sources[1].domain_id, "tasks");
    assert!(!plan.destination_binding_fingerprint.is_empty());
    assert!(plan
        .sources
        .iter()
        .all(|source| !source.binding_fingerprint.is_empty()));
}

#[test]
fn resource_only_source_grant_composes_only_that_resource() {
    let store = identity(false, Some("carol.vcf"), true, BTreeSet::new());
    let app = app(store);

    let allowed = [CompositionSourceScope::resource(
        "shared",
        CollectionKind::Contacts,
        "carol.vcf",
    )];
    assert!(app
        .authorize_composition(&profile("alice"), "alice-token", &allowed)
        .is_ok());

    let denied = [CompositionSourceScope::resource(
        "shared",
        CollectionKind::Contacts,
        "other.vcf",
    )];
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &denied)
            .unwrap_err(),
        CompositionAuthorizationError::SourceReadDenied("shared".into())
    );
}

#[test]
fn destination_dav_write_does_not_substitute_for_composition_write() {
    let alice = principal("alice");
    let mut policy = AccessPolicy::new();
    assert!(policy.grant_domain_collection(
        alice.clone(),
        "shared",
        CollectionKind::Contacts,
        Operation::Read,
    ));
    // A canonical Contact write grant is intentionally insufficient.
    assert!(policy.grant_domain_collection(
        alice.clone(),
        "home",
        CollectionKind::Contacts,
        Operation::Write,
    ));
    let mut store = IdentityStore::new(policy);
    store
        .add_credential(
            CredentialSpec {
                id: "alice-key".into(),
                principal: alice,
                not_before: 100,
                expires_at: 200,
                capabilities: BTreeSet::new(),
            },
            "alice-token",
        )
        .unwrap();
    let app = app(store);

    assert_eq!(
        app.authorize_composition(
            &profile("alice"),
            "alice-token",
            &[CompositionSourceScope::collection(
                "shared",
                CollectionKind::Contacts,
            )],
        )
        .unwrap_err(),
        CompositionAuthorizationError::DestinationWriteDenied
    );
}

#[test]
fn credential_restrictions_intersect_with_composition_policy() {
    let source_only = BTreeSet::from([Capability::ReadContacts]);
    let source_app = app(identity(true, None, true, source_only));
    let scope = [CompositionSourceScope::collection(
        "shared",
        CollectionKind::Contacts,
    )];
    assert_eq!(
        source_app
            .authorize_composition(&profile("alice"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::DestinationCredentialDenied
    );

    let destination_only = BTreeSet::from([Capability::WriteComposition]);
    let destination_app = app(identity(true, None, true, destination_only));
    assert_eq!(
        destination_app
            .authorize_composition(&profile("alice"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::SourceCredentialDenied("shared".into())
    );
}

#[test]
fn composition_fails_closed_for_wrong_principal_shared_destination_and_unknown_source() {
    let store = identity(true, None, true, BTreeSet::new());
    let app = app(store.clone());
    let scope = [CompositionSourceScope::collection(
        "shared",
        CollectionKind::Contacts,
    )];

    assert_eq!(
        app.authorize_composition(&profile("bob"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::PrincipalMismatch
    );

    let shared_destination_app = App::fake(config("shared"))
        .unwrap()
        .with_identity(store, 150);
    assert_eq!(
        shared_destination_app
            .authorize_composition(&profile("alice"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::InvalidProfile(CompositionError::DestinationNotPrivate)
    );

    let guessed = [CompositionSourceScope::collection(
        "space-shared",
        CollectionKind::Contacts,
    )];
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &guessed)
            .unwrap_err(),
        CompositionAuthorizationError::InvalidProfile(CompositionError::SourceDomainNotAllowed)
    );
}

#[test]
fn planner_rejects_empty_duplicate_invalid_and_noncanonical_source_scopes() {
    let app = app(identity(true, None, true, BTreeSet::new()));
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &[])
            .unwrap_err(),
        CompositionAuthorizationError::EmptySources
    );

    let duplicated = [
        CompositionSourceScope::collection("shared", CollectionKind::Contacts),
        CompositionSourceScope::collection("shared", CollectionKind::Contacts),
    ];
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &duplicated)
            .unwrap_err(),
        CompositionAuthorizationError::DuplicateSourceScope
    );

    assert_eq!(
        app.authorize_composition(
            &profile("alice"),
            "alice-token",
            &[CompositionSourceScope::resource(
                "shared",
                CollectionKind::Contacts,
                " ",
            )],
        )
        .unwrap_err(),
        CompositionAuthorizationError::InvalidResourceId
    );

    assert_eq!(
        app.authorize_composition(
            &profile("alice"),
            "alice-token",
            &[CompositionSourceScope::collection(
                "shared",
                CollectionKind::Composition,
            )],
        )
        .unwrap_err(),
        CompositionAuthorizationError::SourceCollectionUnsupported
    );

    // Shared has no tasks route even though the profile allows the domain.
    assert_eq!(
        app.authorize_composition(
            &profile("alice"),
            "alice-token",
            &[CompositionSourceScope::collection(
                "shared",
                CollectionKind::Tasks,
            )],
        )
        .unwrap_err(),
        CompositionAuthorizationError::SourceCollectionUnavailable("shared".into())
    );
}

#[test]
fn authentication_lifecycle_is_checked_before_composition_authorization() {
    let store = identity(true, None, true, BTreeSet::new());
    let mut app = app(store);
    let scope = [CompositionSourceScope::collection(
        "shared",
        CollectionKind::Contacts,
    )];

    assert_eq!(
        app.authorize_composition(&profile("alice"), "wrong-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::AuthenticationFailed(
            any_cal_app::identity::AuthOutcome::Invalid
        )
    );

    app.set_identity_now(200);
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::AuthenticationFailed(
            any_cal_app::identity::AuthOutcome::Expired
        )
    );
}

#[test]
fn composition_respects_binding_local_upstream_read_and_write_state() {
    let mut app = app(identity(true, None, true, BTreeSet::new()));
    let scope = [CompositionSourceScope::collection(
        "shared",
        CollectionKind::Contacts,
    )];

    let (shared_binding, _, _) = app.upstream_binding_identity("shared").unwrap();
    assert!(app.configure_upstream_capabilities("shared", &shared_binding, false, true));
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::SourceUpstreamDenied {
            domain_id: "shared".into(),
            status: 403,
        }
    );

    let (home_binding, _, _) = app.upstream_binding_identity("home").unwrap();
    assert!(app.configure_upstream_capabilities("shared", &shared_binding, true, true));
    assert!(app.configure_upstream_capabilities("home", &home_binding, true, false));
    assert_eq!(
        app.authorize_composition(&profile("alice"), "alice-token", &scope)
            .unwrap_err(),
        CompositionAuthorizationError::DestinationUpstreamDenied(403)
    );

    assert!(app.configure_upstream_capabilities("home", &home_binding, true, true));
    assert!(app
        .authorize_composition(&profile("alice"), "alice-token", &scope)
        .is_ok());
}
