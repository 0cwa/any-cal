use any_cal_app::AppConfig;
use any_cal_core::{DavRoute, VisibilityIntent};

#[test]
fn scalar_app_config_projects_to_one_legacy_domain_binding() {
    let mut config = AppConfig::defaults();
    config.space_id = "space-legacy".into();
    config.contacts_collection = "friends".into();
    config.tasks_collection = "todo".into();

    let contract = config.domain_bindings().unwrap();
    assert_eq!(contract.bindings.len(), 1);

    let binding = &contract.bindings[0];
    assert_eq!(binding.domain_id, "legacy-default");
    assert_eq!(binding.space_id, "space-legacy");
    assert_eq!(binding.visibility, VisibilityIntent::Unknown);
    assert_eq!(
        binding.routes,
        vec![
            DavRoute::contacts("/carddav/friends"),
            DavRoute::tasks("/caldav/todo")
        ]
    );
}

#[test]
fn invalid_scalar_config_does_not_produce_domain_bindings() {
    let config = AppConfig::defaults();
    assert!(config.domain_bindings().is_err());
}

#[test]
fn sync_scope_is_non_secret_and_changes_with_upstream_context() {
    let mut config = AppConfig::defaults();
    config.space_id = "space-a".into();
    config.token = Some("synthetic-token-a".into());

    let base = config.sync_scope("custom").unwrap();
    assert_eq!(base.domain_id, "legacy-default");
    assert_eq!(base.space_id, "space-a");
    assert_eq!(base.account_fingerprint.len(), 64);
    assert_eq!(base.endpoint_fingerprint.len(), 64);
    assert!(!base.account_fingerprint.contains("synthetic-token-a"));

    let mut moved = config.clone();
    moved.space_id = "space-b".into();
    assert_ne!(base, moved.sync_scope("custom").unwrap());

    let mut endpoint = config.clone();
    endpoint.endpoint = "http://127.0.0.1:31013".into();
    assert_ne!(base, endpoint.sync_scope("custom").unwrap());

    let mut rotated = config;
    rotated.token = Some("synthetic-token-b".into());
    assert_ne!(base, rotated.sync_scope("custom").unwrap());
}

