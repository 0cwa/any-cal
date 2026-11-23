use any_cal_anytype_adapter::{wire, ObjectRecord, TransportError};
use serde_json::json;
use std::collections::BTreeMap;

fn record() -> ObjectRecord {
    ObjectRecord {
        id: "object-1".into(),
        space_id: "space-1".into(),
        name: None,
        properties: vec![
            ("done".into(), "true".into()),
            ("label".into(), "hello".into()),
        ],
        property_formats: BTreeMap::new(),
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
    assert_eq!(objects[0].name.as_deref(), Some("Example"));
    assert!(
        objects[0].body.is_empty(),
        "list responses are summary-only"
    );
    assert_eq!(objects[0].properties, vec![("done".into(), "true".into())]);

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
    assert_eq!(object.name.as_deref(), Some("Example"));
    assert_eq!(object.body, "opaque body");
    assert_eq!(object.properties, vec![("score".into(), "3.5".into())]);
}

#[test]
fn native_name_round_trips_without_becoming_a_property() {
    for name in ["Carol · Personal", "", "  "] {
        let response = json!({"object":{
            "id":"context-1", "name":name, "markdown":"opaque reference",
            "properties":[{"key":"private_note","format":"text","text":"keep"}]
        }});
        let object = wire::decode_object(&response.to_string(), Some("space-1")).unwrap();
        assert_eq!(object.name.as_deref(), Some(name));
        for encoded in [wire::encode_create(&object), wire::encode_update(&object)] {
            let encoded: serde_json::Value = serde_json::from_str(&encoded.unwrap()).unwrap();
            assert_eq!(encoded["name"], name);
            assert_eq!(
                encoded["properties"],
                json!([{"key":"private_note","text":"keep"}])
            );
        }
    }
}

#[test]
fn absent_native_name_is_not_guessed_from_an_opaque_objects_private_property() {
    let response = json!({"object":{
        "id":"context-1", "markdown":"opaque reference",
        "properties":[{"key":"name","format":"text","text":"Private property"}]
    }});
    let object = wire::decode_object(&response.to_string(), Some("space-1")).unwrap();
    assert!(object.name.is_none());
    for encoded in [wire::encode_create(&object), wire::encode_update(&object)] {
        let encoded: serde_json::Value = serde_json::from_str(&encoded.unwrap()).unwrap();
        assert!(encoded.get("name").is_none());
        assert_eq!(
            encoded["properties"],
            json!([{"key":"name","text":"Private property"}])
        );
    }
    let stored = serde_json::to_value(&object).unwrap();
    assert!(stored.get("name").is_none());
    assert!(serde_json::from_value::<ObjectRecord>(stored)
        .unwrap()
        .name
        .is_none());
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
        ]
    );
}

#[test]
fn response_formats_round_trip_select_multi_select_and_object_links() {
    let wrapped = json!({
        "object": {
            "id": "server-object-1",
            "space_id": "space-1",
            "markdown": "{}",
            "properties": [
                {
                    "key": "status",
                    "format": "select",
                    "select": {"id": "tag-id", "key": "important", "name": "Important"}
                },
                {
                    "key": "labels",
                    "format": "multi_select",
                    "multi_select": [
                        {"id": "tag-a", "key": "one", "name": "One"},
                        {"id": "tag-b", "key": "two", "name": "Two"}
                    ]
                },
                {
                    "key": "related",
                    "format": "objects",
                    "objects": ["object-a", "object-b"]
                }
            ]
        }
    });
    let record = wire::decode_object(&wrapped.to_string(), None).unwrap();
    assert_eq!(record.property_formats["status"], "select");
    assert_eq!(record.property_formats["labels"], "multi_select");
    assert_eq!(record.property_formats["related"], "objects");

    let encoded: serde_json::Value =
        serde_json::from_str(&wire::encode_update(&record).unwrap()).unwrap();
    let properties = encoded["properties"].as_array().unwrap();
    let property = |key: &str| {
        properties
            .iter()
            .find(|property| property["key"] == key)
            .unwrap()
    };
    assert_eq!(property("status")["select"], "important");
    assert_eq!(property("labels")["multi_select"], json!(["one", "two"]));
    assert_eq!(
        property("related")["objects"],
        json!(["object-a", "object-b"])
    );
}

#[test]
fn updates_filter_system_properties_without_changing_reads_or_creates() {
    let wrapped = json!({
        "object": {
            "id": "server-object-1",
            "space_id": "space-1",
            "markdown": "private body",
            "properties": [
                {"key": "backlinks", "format": "objects", "objects": ["object-a"]},
                {"key": "CREATOR", "format": "objects", "objects": ["creator-a"]},
                {"key": "created_date", "format": "date", "date": "2026-01-01T00:00:00Z"},
                {"key": "Last_Modified_Date", "format": "date", "date": "2026-01-02T00:00:00Z"},
                {"key": "last_modified_by", "format": "objects", "objects": ["editor-a"]},
                {"key": "links", "format": "objects", "objects": ["object-b"]},
                {"key": "private_note", "format": "text", "text": "true"},
                {"key": "personal_links", "format": "objects", "objects": ["local-object"]}
            ]
        }
    });
    let record = wire::decode_object(&wrapped.to_string(), None).unwrap();
    let original = record.clone();
    assert_eq!(record.properties.len(), 8, "reads retain system metadata");

    let update: serde_json::Value =
        serde_json::from_str(&wire::encode_update(&record).unwrap()).unwrap();
    assert_eq!(
        update["properties"],
        json!([
            {"key": "private_note", "text": "true"},
            {"key": "personal_links", "objects": ["local-object"]}
        ])
    );
    assert_eq!(update["markdown"], "```json\nprivate body\n```");
    assert_eq!(record, original, "encoding does not mutate the read model");

    let create: serde_json::Value =
        serde_json::from_str(&wire::encode_create(&record).unwrap()).unwrap();
    let create_keys: Vec<_> = create["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|property| property["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        create_keys,
        record
            .properties
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>(),
        "the update policy does not change create encoding"
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
    assert_eq!(value["body"], "```json\nopaque body\n```");
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
    assert_eq!(update["markdown"], "```json\nopaque body\n```");
    assert!(update.get("body").is_none());
    assert!(update.get("type_key").is_none());
    assert!(update.get("id").is_none());
    assert!(update.get("space_id").is_none());

    let typed: serde_json::Value = serde_json::from_str(
        &wire::encode_create_with_type(&record(), "anycal_person_context").unwrap(),
    )
    .unwrap();
    assert_eq!(typed["type_key"], "anycal_person_context");
    assert_eq!(
        wire::encode_create_with_type(&record(), "bad type"),
        Err(TransportError::InvalidRequest(
            "object type key is invalid".into()
        ))
    );
}

#[test]
fn markdown_fence_round_trip_preserves_canonical_json() {
    let wrapped = serde_json::json!({
        "object": {
            "id": "server-object-1",
            "space_id": "space-1",
            "markdown": "``` {\"canonical_key\":\"value\"} ```",
            "properties": []
        }
    });
    let object = wire::decode_object(&wrapped.to_string(), None).unwrap();
    assert_eq!(object.body, r#"{"canonical_key":"value"}"#);
    let encoded: serde_json::Value =
        serde_json::from_str(&wire::encode_update(&object).unwrap()).unwrap();
    assert_eq!(
        encoded["markdown"],
        "```json\n{\"canonical_key\":\"value\"}\n```"
    );
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
    let mut record = ObjectRecord {
        id: "dav-uid-1".into(),
        space_id: "space-1".into(),
        name: None,
        properties: vec![("dav_uid".into(), "dav-uid-1".into())],
        property_formats: BTreeMap::new(),
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

    // Keep the legacy property precedence for canonical DAV objects while an
    // explicit native name always wins, including an explicit empty value.
    record
        .properties
        .push(("name".into(), "Property title".into()));
    let encoded: serde_json::Value =
        serde_json::from_str(&wire::encode_update(&record).unwrap()).unwrap();
    assert_eq!(encoded["name"], "Property title");
    for explicit in ["Native title", ""] {
        record.name = Some(explicit.into());
        let encoded: serde_json::Value =
            serde_json::from_str(&wire::encode_update(&record).unwrap()).unwrap();
        assert_eq!(encoded["name"], explicit);
    }
}
