use any_cal_core::{
    BindingLifecycle, DavComponent, DavRoute, DomainBinding, DomainBindings, DomainBindingsError,
    DomainCollection, VisibilityIntent, DOMAIN_BINDINGS_SCHEMA_VERSION,
};

fn binding(
    domain_id: &str,
    label: &str,
    space_id: &str,
    credential_profile_id: &str,
    routes: Vec<DavRoute>,
) -> DomainBinding {
    DomainBinding {
        domain_id: domain_id.into(),
        label: label.into(),
        space_id: space_id.into(),
        credential_profile_id: credential_profile_id.into(),
        routes,
        schema_profile: "default".into(),
        checkpoint_namespace: domain_id.into(),
        visibility: VisibilityIntent::Unknown,
        lifecycle: BindingLifecycle::Configured,
    }
}

#[test]
fn legacy_single_space_is_one_binding_with_contact_and_task_routes() {
    let contract =
        DomainBindings::legacy_single_space("space-legacy", "contacts", "tasks").unwrap();

    assert_eq!(contract.version, DOMAIN_BINDINGS_SCHEMA_VERSION);
    assert_eq!(contract.bindings.len(), 1);
    let legacy = &contract.bindings[0];
    assert_eq!(legacy.domain_id, "legacy-default");
    assert_eq!(legacy.space_id, "space-legacy");
    assert_eq!(legacy.routes.len(), 2);

    let (contact_binding, contact_route) = contract.resolve_route("/carddav/contacts").unwrap();
    assert_eq!(contact_binding.domain_id, "legacy-default");
    assert_eq!(contact_route, &DavRoute::contacts("/carddav/contacts"));

    let (task_binding, task_route) = contract.resolve_route("/caldav/tasks").unwrap();
    assert_eq!(task_binding.domain_id, "legacy-default");
    assert_eq!(task_route, &DavRoute::tasks("/caldav/tasks"));
}

#[test]
fn four_domain_contract_serializes_and_parses_deterministically() {
    let bindings = vec![
        binding(
            "mutual-events",
            "Mutual events",
            "space-events",
            "shared-account",
            vec![DavRoute::events("/dav/calendars/mutual-events")],
        ),
        binding(
            "personal-tasks",
            "Personal tasks",
            "space-personal",
            "personal-account",
            vec![DavRoute::tasks("/dav/tasks/personal")],
        ),
        binding(
            "mutual-friends",
            "Mutual friends",
            "space-friends",
            "shared-account",
            vec![DavRoute::contacts("/dav/contacts/mutual-friends")],
        ),
        binding(
            "personal-contacts",
            "Personal contacts",
            "space-personal",
            "personal-account",
            vec![DavRoute::contacts("/dav/contacts/personal")],
        ),
    ];
    let contract = DomainBindings::new(bindings).unwrap();

    let json = contract.to_json().unwrap();
    let reparsed = DomainBindings::from_json(&json).unwrap();
    assert_eq!(reparsed.to_json().unwrap(), json);
    assert_eq!(
        reparsed
            .bindings
            .iter()
            .map(|binding| binding.domain_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mutual-events",
            "mutual-friends",
            "personal-contacts",
            "personal-tasks"
        ]
    );

    let (binding, _) = reparsed
        .resolve_route("/dav/contacts/mutual-friends")
        .unwrap();
    assert_eq!(binding.space_id, "space-friends");
}

#[test]
fn duplicate_domain_ids_and_routes_fail_closed() {
    let first = binding(
        "personal",
        "Personal",
        "space-a",
        "account-a",
        vec![DavRoute::contacts("/dav/contacts/personal")],
    );
    let duplicate_id = binding(
        "personal",
        "Duplicate",
        "space-b",
        "account-b",
        vec![DavRoute::tasks("/dav/tasks/personal")],
    );
    assert_eq!(
        DomainBindings::new(vec![first.clone(), duplicate_id]).unwrap_err(),
        DomainBindingsError::DuplicateDomainId
    );

    let duplicate_route = binding(
        "shared",
        "Shared",
        "space-b",
        "account-b",
        vec![DavRoute::contacts("/dav/contacts/personal")],
    );
    assert_eq!(
        DomainBindings::new(vec![first, duplicate_route]).unwrap_err(),
        DomainBindingsError::DuplicateRoute
    );
}

#[test]
fn multiple_domains_can_share_space_without_sharing_routes() {
    let contract = DomainBindings::new(vec![
        binding(
            "personal-contacts",
            "Personal contacts",
            "space-personal",
            "account-a",
            vec![DavRoute::contacts("/dav/contacts/personal")],
        ),
        binding(
            "personal-tasks",
            "Personal tasks",
            "space-personal",
            "account-a",
            vec![DavRoute::tasks("/dav/tasks/personal")],
        ),
    ])
    .unwrap();

    assert_eq!(contract.bindings[0].space_id, contract.bindings[1].space_id);
    assert!(contract.resolve_route("/dav/contacts/personal").is_ok());
    assert!(contract.resolve_route("/dav/tasks/personal").is_ok());
    assert_eq!(
        contract.resolve_route("/dav/contacts/missing").unwrap_err(),
        DomainBindingsError::RouteNotFound
    );
}

#[test]
fn binding_fingerprint_changes_with_space_account_and_profile() {
    let base = binding(
        "personal-contacts",
        "Personal contacts",
        "space-a",
        "account-profile-a",
        vec![DavRoute::contacts("/dav/contacts/personal")],
    );

    let original = base.fingerprint("account-fingerprint-a").unwrap();
    assert_eq!(original.len(), 64);
    assert_eq!(original, base.fingerprint("account-fingerprint-a").unwrap());

    let mut moved = base.clone();
    moved.space_id = "space-b".into();
    assert_ne!(
        original,
        moved.fingerprint("account-fingerprint-a").unwrap()
    );

    assert_ne!(original, base.fingerprint("account-fingerprint-b").unwrap());

    let mut reprofiled = base.clone();
    reprofiled.credential_profile_id = "account-profile-b".into();
    assert_ne!(
        original,
        reprofiled.fingerprint("account-fingerprint-a").unwrap()
    );

    let ordered = binding(
        "combined",
        "Combined",
        "space-a",
        "account-profile-a",
        vec![
            DavRoute::contacts("/dav/contacts/personal"),
            DavRoute::tasks("/dav/tasks/personal"),
        ],
    );
    let mut reversed = ordered.clone();
    reversed.routes.reverse();
    assert_eq!(
        ordered.fingerprint("account-fingerprint-a").unwrap(),
        reversed.fingerprint("account-fingerprint-a").unwrap()
    );
}

#[test]
fn malformed_or_unsupported_contracts_are_rejected() {
    assert_eq!(
        DomainBindings::from_json("not-json").unwrap_err(),
        DomainBindingsError::InvalidJson
    );
    assert_eq!(
        DomainBindings::from_json(r#"{"version":1,"bindings":[],"unexpected":true}"#).unwrap_err(),
        DomainBindingsError::InvalidJson
    );

    let mismatched = binding(
        "bad-component",
        "Bad component",
        "space-a",
        "account-a",
        vec![DavRoute {
            collection: DomainCollection::Contacts,
            component: DavComponent::Vtodo,
            path: "/dav/contacts/bad".into(),
        }],
    );
    assert_eq!(
        DomainBindings::new(vec![mismatched]).unwrap_err(),
        DomainBindingsError::ComponentMismatch
    );

    let mut contract = DomainBindings::legacy_single_space("space-a", "contacts", "tasks").unwrap();
    contract.version += 1;
    assert_eq!(
        contract.validate().unwrap_err(),
        DomainBindingsError::UnsupportedVersion
    );

    contract.version = DOMAIN_BINDINGS_SCHEMA_VERSION;
    contract.bindings[0].routes[0].path = "not-absolute".into();
    assert_eq!(
        contract.validate().unwrap_err(),
        DomainBindingsError::InvalidRoute
    );
}
