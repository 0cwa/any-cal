use any_cal_anytype_adapter::{wire, ObjectRecord, TransportError};
use serde_json::json;

fn record() -> ObjectRecord {
    ObjectRecord {
        id: "object-1".into(),
        space_id: "space-1".into(),
        properties: vec![
            ("done".into(), "true".into()),
            ("label".into(), "hello".into()),
        ],
        body: "opaque body".into(),
        archived: false,
        revision: 7,
    }
}

#[test]
fn live_shaped_list_and_single_wrappers_convert_deterministically() {
    let list = json!({
        "data": [{
            "id": "server-object-1",
            "space_id": "space-1",
            "type": {"key": "page", "name": "Page"},
            "name": "Example",
            "snippet": "opaque body",
            "properties": [{"key": "done", "name": "Done", "format": "checkbox", "checkbox": true, "vendor": "kept"}],
            "server_meta": {"opaque": 1}
        }],
        "next_offset": 100,
        "trace_id": "redacted"
    });
    let (objects, next) = wire::decode_list(&list.to_string(), Some("space-1")).unwrap();
    assert_eq!(next.as_deref(), Some("100"));
    assert_eq!(objects[0].space_id, "space-1");
    assert_eq!(objects[0].id, "server-object-1");
    assert!(
        objects[0].body.is_empty(),
        "list responses are summary-only"
    );
    assert_eq!(
        objects[0].properties,
        vec![
            ("done".into(), "true".into()),
            ("name".into(), "Example".into())
        ]
    );

    let wrapped = json!({
        "object": {
            "id": "server-object-1",
            "space_id": "space-1",
            "type": {"key": "page", "name": "Page"},
            "icon": {"emoji": "🙂", "format": "emoji"},
            "name": "Example",
            "markdown": "opaque body",
            "properties": [{"key": "score", "name": "Score", "format": "number", "number": 3.5}],
            "unknown": ["preserved by DTO"]
        },
        "request_meta": {"opaque": true}
    });
    let dto: wire::WireObjectResponse = serde_json::from_value(wrapped.clone()).unwrap();
    assert_eq!(dto.extra["request_meta"], json!({"opaque": true}));
    assert_eq!(dto.object.extra["unknown"], json!(["preserved by DTO"]));
    assert_eq!(
        serde_json::to_value(dto).unwrap()["request_meta"],
        json!({"opaque": true})
    );
    let object = wire::decode_object(&wrapped.to_string(), None).unwrap();
    assert_eq!(object.id, "server-object-1");
    assert_eq!(object.body, "opaque body");
    assert_eq!(
        object.properties,
        vec![
            ("score".into(), "3.5".into()),
            ("name".into(), "Example".into())
        ]
    );
}

#[test]
fn source_backed_object_response_fixture_preserves_typed_values() {
    let object = wire::decode_object(include_str!("fixtures/object-response.json"), None).unwrap();
    assert_eq!(object.id, "server-object-1");
    assert_eq!(object.space_id, "space-1");
    assert_eq!(object.body, "{\"canonical\":true}");
    assert_eq!(
        object.properties,
        vec![
            ("dav_uid".into(), "dav-uid-1".into()),
            ("done".into(), "false".into()),
            ("score".into(), "3.5".into()),
            ("related".into(), "[\"server-object-2\"]".into()),
            ("name".into(), "Ada Lovelace".into()),
        ]
    );
}

#[test]
fn official_pagination_metadata_becomes_the_transport_offset() {
    let list = json!({
        "data": [{
            "id": "server-object-1",
            "space_id": "space-1",
            "properties": [],
            "archived": false
        }],
        "pagination": {"has_more": true, "offset": 0, "limit": 1, "total": 2}
    });
    let (_, next) = wire::decode_list(&list.to_string(), Some("space-1")).unwrap();
    assert_eq!(next.as_deref(), Some("1"));
}

#[test]
fn create_and_update_dtos_have_pinned_fields_and_typed_property_shape() {
    let value: serde_json::Value =
        serde_json::from_str(&wire::encode_create(&record()).unwrap()).unwrap();
    assert_eq!(value["type_key"], "page");
    assert_eq!(value["body"], "opaque body");
    assert!(value.get("space_id").is_none());
    assert!(value.get("id").is_none());
    assert_eq!(value["properties"][0]["key"], "done");
    assert_eq!(value["properties"][0]["checkbox"], true);
    assert!(value["properties"]
        .as_array()
        .unwrap()
        .iter()
        .all(|property| property.get("key").is_some()));

    let update: serde_json::Value =
        serde_json::from_str(&wire::encode_update(&record()).unwrap()).unwrap();
    assert_eq!(update["markdown"], "opaque body");
    assert!(update.get("body").is_none());
    assert!(update.get("type_key").is_none());
    assert!(update.get("id").is_none());
    assert!(update.get("space_id").is_none());
}

#[test]
fn missing_required_identity_and_malformed_shapes_fail_closed() {
    assert_eq!(
        wire::decode_object(r#"{"object":{"body":"missing id"}}"#, Some("space")),
        Err(TransportError::Malformed)
    );
    assert_eq!(
        wire::decode_list(r#"{"data":[{"id":"x","properties":{}}]}"#, Some("space")),
        Err(TransportError::Malformed)
    );
    assert_eq!(
        wire::encode_create(&ObjectRecord {
            id: String::new(),
            ..record()
        }),
        Err(TransportError::InvalidRequest(
            "object id must not be empty".into()
        ))
    );
}

#[test]
fn property_link_fixture_uses_tagged_values_not_the_rejected_bare_shape() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/property-values.json")).unwrap();
    assert_eq!(fixture["typed_property_links"][0]["key"], "description");
    assert_eq!(fixture["typed_property_links"][0]["text"], "hello");
    assert_eq!(fixture["typed_property_links"][1]["checkbox"], true);
    assert_eq!(fixture["response_properties"][0]["format"], "text");

    let encoded: serde_json::Value = serde_json::from_str(
        &wire::encode_create(&ObjectRecord {
            properties: vec![
                ("description".into(), "hello".into()),
                ("done".into(), "true".into()),
                ("relation".into(), "object-a,object-b".into()),
            ],
            ..record()
        })
        .unwrap(),
    )
    .unwrap();
    for property in encoded["properties"].as_array().unwrap() {
        assert!(property["key"].is_string());
        assert!(property.get("value").is_none());
    }
}

#[test]
fn create_uses_the_dav_display_name_without_turning_it_into_a_property() {
    let record = ObjectRecord {
        id: "dav-uid-1".into(),
        space_id: "space-1".into(),
        properties: vec![("dav_uid".into(), "dav-uid-1".into())],
        body: r#"{"collection_id":"contacts","resource_id":"contact-1","kind":"contact","anytype_object_id":"dav-uid-1","dav_uid":"dav-uid-1","document":{"version":1,"content":{"fields":{"FN":[{"value":"Ada Lovelace"}]} }},"revision":0}"#.into(),
        archived: false,
        revision: 0,
    };
    let encoded: serde_json::Value =
        serde_json::from_str(&wire::encode_create(&record).unwrap()).unwrap();
    assert_eq!(encoded["name"], "Ada Lovelace");
    assert!(encoded["properties"]
        .as_array()
        .unwrap()
        .iter()
        .all(|property| property["key"] != "name"));
}
