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
