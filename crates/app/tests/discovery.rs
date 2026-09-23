use any_cal_anytype_adapter::{
    AnytypeDiscoveryTransport, AnytypeTransport, DiscoveredMember, DiscoveredProperty,
    DiscoveredSpace, DiscoveredTag, DiscoveredType, DiscoveredView, FakeAnytypeTransport,
    ObjectRecord, Page, TransportError,
};
use any_cal_app::discovery::{DiscoveryError, RequirementState};
use any_cal_app::{AppConfig, AppWithTransport};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct DiscoveryTransport {
    inner: FakeAnytypeTransport,
    discovered_spaces: Vec<String>,
}

impl DiscoveryTransport {
    fn new() -> Self {
        Self {
            inner: FakeAnytypeTransport::new(100),
            discovered_spaces: Vec::new(),
        }
    }
}

impl AnytypeTransport for DiscoveryTransport {
    fn list_objects(
        &mut self,
        space_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<ObjectRecord>, TransportError> {
        self.inner.list_objects(space_id, cursor)
    }

    fn get_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.get_object(space_id, object_id)
    }

    fn create_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
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

impl AnytypeDiscoveryTransport for DiscoveryTransport {
    fn list_spaces(
        &mut self,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredSpace>, TransportError> {
        Ok(Page {
            data: vec![DiscoveredSpace {
                id: "space-a".into(),
                name: "Personal".into(),
                extra: BTreeMap::new(),
            }],
            next_offset: None,
        })
    }

    fn list_types(
        &mut self,
        space_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredType>, TransportError> {
        self.discovered_spaces.push(space_id.into());
        Ok(Page {
            data: vec![
                DiscoveredType {
                    id: "type-task".into(),
                    key: "task".into(),
                    name: "Task".into(),
                    layout: Some("task".into()),
                    extra: BTreeMap::new(),
                },
                DiscoveredType {
                    id: "type-person".into(),
                    key: "person".into(),
                    name: "Person".into(),
                    layout: None,
                    extra: BTreeMap::new(),
                },
            ],
            next_offset: None,
        })
    }

    fn list_properties(
        &mut self,
        space_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredProperty>, TransportError> {
        self.discovered_spaces.push(space_id.into());
        Ok(Page {
            data: vec![
                DiscoveredProperty {
                    id: "property-description".into(),
                    key: Some("description".into()),
                    name: "Description".into(),
                    format: "text".into(),
                    extra: BTreeMap::new(),
                },
                DiscoveredProperty {
                    id: "property-done".into(),
                    key: Some("done".into()),
                    name: "Done".into(),
                    format: "checkbox".into(),
                    extra: BTreeMap::new(),
                },
                DiscoveredProperty {
                    id: "property-status".into(),
                    key: Some("status".into()),
                    name: "Status".into(),
                    format: "select".into(),
                    extra: BTreeMap::new(),
                },
            ],
            next_offset: None,
        })
    }

    fn list_members(
        &mut self,
        space_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredMember>, TransportError> {
        self.discovered_spaces.push(space_id.into());
        Ok(Page {
            data: vec![DiscoveredMember {
                profile_id: "profile-viewer".into(),
                name: "Viewer".into(),
                global_name: None,
                status: "active".into(),
                role: "Viewer".into(),
                extra: BTreeMap::new(),
            }],
            next_offset: None,
        })
    }

    fn list_tags(
        &mut self,
        space_id: &str,
        property_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredTag>, TransportError> {
        self.discovered_spaces.push(space_id.into());
        assert_eq!(property_id, "property-status");
        Ok(Page {
            data: vec![DiscoveredTag {
                id: "tag-open".into(),
                name: "Open".into(),
                color: Some("blue".into()),
                extra: BTreeMap::new(),
            }],
            next_offset: None,
        })
    }

    fn list_views(
        &mut self,
        space_id: &str,
        list_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredView>, TransportError> {
        self.discovered_spaces.push(space_id.into());
        assert_eq!(list_id, "trusted-list");
        Ok(Page {
            data: vec![DiscoveredView {
                id: "view-table".into(),
                name: "Table".into(),
                layout: Some("table".into()),
                extra: BTreeMap::new(),
            }],
            next_offset: None,
        })
    }
}

fn config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-scalar-must-not-be-used".into();
    config.credential_profile_id = "primary".into();
    config.domain_bindings_json = Some(
        r#"{
          "version":1,
          "bindings":[{
            "domain_id":"personal",
            "label":"Personal",
            "space_id":"space-a",
            "credential_profile_id":"primary",
            "routes":[
              {"collection":"contacts","component":"vcard","path":"/carddav/personal"}
            ],
            "schema_profile":"default",
            "checkpoint_namespace":"personal",
            "visibility":"private",
            "lifecycle":"configured"
          }]
        }"#
        .into(),
    );
    config
}

#[test]
fn capability_discovery_is_binding_scoped_read_only_and_non_authoritative() {
    let mut app =
        AppWithTransport::with_transport(config(), DiscoveryTransport::new()).unwrap();

    let snapshot = app.discover_domain_capabilities("personal").unwrap();
    assert_eq!(snapshot.domain_id, "personal");
    assert_eq!(snapshot.space_id, "space-a");
    assert_eq!(snapshot.schema_profile, "default");
    assert!(snapshot.body_only_available);
    assert!(!snapshot.schema_ready);
    assert_eq!(snapshot.members[0].role, "Viewer");
    assert_eq!(
        snapshot.tags_by_property_id["property-status"][0].name,
        "Open"
    );
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .find(|item| item.key == "done")
            .unwrap()
            .state,
        RequirementState::Present
    );
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .find(|item| item.key == "event_reference")
            .unwrap()
            .state,
        RequirementState::Missing
    );

    // Merely seeing a Viewer member is not enough to identify the gateway
    // account, so discovery must not silently turn member metadata into auth.
    assert_eq!(
        app.upstream_binding_status("personal"),
        Some(("unknown", "unknown", "active"))
    );

    let views = app.discover_list_views("personal", "trusted-list").unwrap();
    assert_eq!(views[0].id, "view-table");

    let transport = app.transport_snapshot();
    assert!(transport
        .discovered_spaces
        .iter()
        .all(|space| space == "space-a"));
    assert_eq!(transport.inner.create_calls, 0);
    assert_eq!(transport.inner.update_calls, 0);
    assert_eq!(transport.inner.archive_calls, 0);
    assert_eq!(transport.inner.delete_calls, 0);

    assert_eq!(
        app.discover_domain_capabilities("not-configured").unwrap_err(),
        DiscoveryError::UnknownDomain
    );
}
