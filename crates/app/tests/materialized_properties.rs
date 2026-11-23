//! Sanitized HTTP fixtures for optional materialized-reference metadata.
use any_cal_anytype_adapter::{
    HttpAnytypeTransport, HttpExchange, TransportError, ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_PERSON_CONTEXT_TYPE_KEY, ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY, ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY, ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
    DEFAULT_OBJECT_TYPE_KEY,
};
use any_cal_app::composition::CompositionSourceScope;
use any_cal_app::composition_orchestration::{
    CompositionOrchestrationError, CompositionSchemaError, MaterializedReferenceRecord,
    MetadataProjection,
};
use any_cal_app::identity::{
    AccessPolicy, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::{App, AppConfig, AppGeneric};
use any_cal_core::{
    project_person_context_source, CompositionProfile, ProjectionPolicy, RefreshKind,
    ResourceEnvelope,
};
use any_cal_dav_server::Request;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

const MANAGED_KEYS: [&str; 8] = [
    ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY,
    ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
    ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
];

enum Reply {
    Json(Value),
    Destination,
    SaveDestination { timeout: bool },
    Timeout,
}

#[derive(Default)]
struct HttpScript {
    steps: VecDeque<(String, Reply)>,
    requests: Vec<(String, Value)>,
    destination: Option<Value>,
}

struct ScriptedHttp(Arc<Mutex<HttpScript>>);

impl HttpExchange for ScriptedHttp {
    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        let (headers, body) = std::str::from_utf8(request)
            .unwrap()
            .split_once("\r\n\r\n")
            .unwrap();
        let request_line = headers.lines().next().unwrap();
        let mut script = self.0.lock().unwrap();
        let (expected, reply) = script.steps.pop_front().expect("unexpected HTTP request");
        assert_eq!(request_line, expected);
        assert!(headers.contains("Anytype-Version: 2025-11-08"));
        assert!(headers.contains("Authorization: Bearer fixture-upstream-token"));
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length:"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        assert_eq!(content_length, body.len());
        let request_body = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(body).unwrap()
        };
        script
            .requests
            .push((request_line.to_owned(), request_body.clone()));
        let response = match reply {
            Reply::Json(value) => value,
            Reply::Destination => json!({"object": script.destination.as_ref().unwrap()}),
            Reply::Timeout => return Err(TransportError::Timeout),
            Reply::SaveDestination { timeout } => {
                // Model PATCH's per-property behavior: omitted server/private
                // fields remain present in the next GET response.
                let mut destination = script.destination.take().unwrap_or_else(|| {
                    json!({
                        "id":"context-1", "type":{"key":request_body["type_key"]},
                        "properties":[]
                    })
                });
                destination["markdown"] = request_body
                    .get("markdown")
                    .or_else(|| request_body.get("body"))
                    .unwrap()
                    .clone();
                let properties = destination["properties"].as_array_mut().unwrap();
                for property in request_body["properties"].as_array().unwrap() {
                    let mut property = property.clone();
                    let format = ["text", "multi_select", "objects"]
                        .into_iter()
                        .find(|format| property.get(*format).is_some())
                        .expect("fixture supports the native formats used by this test");
                    property["format"] = json!(format);
                    properties.retain(|existing| existing["key"] != property["key"]);
                    properties.push(property);
                }
                script.destination = Some(destination.clone());
                if timeout {
                    return Err(TransportError::Timeout);
                }
                json!({"object":destination})
            }
        }
        .to_string();
        Ok(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{response}",
            response.len()
        )
        .into_bytes())
    }
}

fn config() -> AppConfig {
    let mut config = AppConfig::defaults();
    config.space_id = "legacy-must-not-compose".into();
    config.credential_profile_id = "primary".into();
    config.token = Some("fixture-upstream-token".into());
    config.domain_bindings_json = Some(
        json!({"version":1,"bindings":[
            {
                "domain_id":"home", "label":"Home", "space_id":"space-home",
                "credential_profile_id":"primary", "schema_profile":"default",
                "checkpoint_namespace":"home", "visibility":"private",
                "lifecycle":"configured",
                "routes":[{"collection":"contacts","component":"vcard","path":"/carddav/home"}]
            },
            {
                "domain_id":"shared", "label":"Shared", "space_id":"space-shared",
                "credential_profile_id":"primary", "schema_profile":"default",
                "checkpoint_namespace":"shared", "visibility":"shared",
                "lifecycle":"configured",
                "routes":[{"collection":"contacts","component":"vcard","path":"/carddav/shared"}]
            }
        ]})
        .to_string(),
    );
    config
}

fn identity() -> IdentityStore {
    let alice = PrincipalId::new("alice").unwrap();
    let mut policy = AccessPolicy::new();
    for (domain, collection, operation) in [
        ("shared", CollectionKind::Contacts, Operation::Read),
        ("home", CollectionKind::Composition, Operation::Write),
    ] {
        assert!(policy.grant_domain_collection(alice.clone(), domain, collection, operation));
    }
    let mut identity = IdentityStore::new(policy);
    identity
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
    identity
}

fn profile() -> CompositionProfile {
    CompositionProfile::new(
        "alice",
        "home",
        vec!["shared".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap()
}

fn scopes() -> [CompositionSourceScope; 1] {
    [CompositionSourceScope::resource(
        "shared",
        CollectionKind::Contacts,
        "carol",
    )]
}

fn person_types() -> Vec<Value> {
    vec![json!({
        "id":"type-person-context", "key":ANYCAL_PERSON_CONTEXT_TYPE_KEY,
        "name":"A renamed display label"
    })]
}

fn managed_schema() -> Vec<Value> {
    MANAGED_KEYS
        .iter()
        .enumerate()
        .map(|(index, key)| {
            json!({
                "id":format!("property-{index}"), "key":key,
                "name":format!("Unrelated label {index}"), "format":"text"
            })
        })
        .collect()
}

struct Fixture {
    script: Arc<Mutex<HttpScript>>,
    source: ResourceEnvelope,
}

impl Fixture {
    fn new() -> Self {
        // Only the canonical source is seeded with the existing fixture API.
        // Every destination create/read/update below crosses the HTTP encoder.
        let mut source_app = App::fake(config()).unwrap();
        assert_eq!(source_app.handle(Request {
            method:"PUT".into(), path:"/carddav/shared/carol.vcf".into(),
            headers:vec![("Content-Type".into(),"text/vcard".into())],
            body:b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:carol-uid\r\nFN:Carol Smith\r\nEMAIL:carol@example.test\r\nEND:VCARD\r\n".to_vec(),
        }).status, 201);
        let source = source_app
            .transport_snapshot()
            .objects
            .values()
            .find(|object| object.space_id == "space-shared")
            .map(|object| ResourceEnvelope::from_json(&object.body).unwrap())
            .unwrap();
        Self {
            script: Arc::new(Mutex::new(HttpScript::default())),
            source,
        }
    }

    fn app(&self) -> AppGeneric<HttpAnytypeTransport> {
        AppGeneric::http_with_exchange(config(), Box::new(ScriptedHttp(Arc::clone(&self.script))))
            .unwrap()
            .with_identity(identity(), 150)
    }

    fn discovery(&self, types: Vec<Value>, properties: Vec<Value>) {
        let mut script = self.script.lock().unwrap();
        for (path, data) in [
            ("/v1/spaces", vec![json!({"id":"space-home","name":"Home"})]),
            ("/v1/spaces/space-home/types", types),
            ("/v1/spaces/space-home/properties", properties),
            ("/v1/spaces/space-home/members", Vec::new()),
        ] {
            script.steps.push_back((
                format!("GET {path}?offset=0&limit=100 HTTP/1.1"),
                Reply::Json(json!({"data":data})),
            ));
        }
    }

    fn materialization(&self, create: bool) {
        let mut script = self.script.lock().unwrap();
        script.steps.push_back((
            "GET /v1/spaces/space-home/objects HTTP/1.1".into(),
            Reply::Json(if create {
                json!({"data":[]})
            } else {
                json!({"data":[{"id":"context-1"}]})
            }),
        ));
        if !create {
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                Reply::Destination,
            ));
        }
        let source_id = self.source.anytype_object_id.to_string();
        script.steps.push_back((
            "GET /v1/spaces/space-shared/objects HTTP/1.1".into(),
            Reply::Json(json!({"data":[{"id":source_id}]})),
        ));
        script.steps.push_back((
            format!("GET /v1/spaces/space-shared/objects/{source_id} HTTP/1.1"),
            Reply::Json(json!({"object":{
                "id":source_id,
                "markdown":format!("```json\n{}\n```", self.source.canonical_json().unwrap()),
                "properties":[]
            }})),
        ));
        script.steps.push_back((
            if create {
                "POST /v1/spaces/space-home/objects HTTP/1.1"
            } else {
                "PATCH /v1/spaces/space-home/objects/context-1 HTTP/1.1"
            }
            .into(),
            Reply::SaveDestination { timeout: false },
        ));
    }
}

fn record_from_markdown(markdown: &Value) -> MaterializedReferenceRecord {
    MaterializedReferenceRecord::from_json(
        markdown
            .as_str()
            .unwrap()
            .strip_prefix("```json\n")
            .unwrap()
            .strip_suffix("\n```")
            .unwrap(),
    )
    .unwrap()
}

fn materialize_typed(
    app: &mut AppGeneric<HttpAnytypeTransport>,
    type_key: &str,
) -> Result<MetadataProjection, CompositionOrchestrationError> {
    app.materialize_references_with_type(&profile(), "alice-token", &scopes(), type_key, |stored| {
        project_person_context_source(stored).unwrap()
    })
    .map(|result| result[0].metadata_projection)
}

#[test]
fn default_page_without_discovery_writes_body_only() {
    let fixture = Fixture::new();
    fixture.materialization(true);
    let mut app = fixture.app();
    assert_eq!(
        materialize_typed(&mut app, DEFAULT_OBJECT_TYPE_KEY).unwrap(),
        MetadataProjection::BodyOnly
    );
    let script = fixture.script.lock().unwrap();
    assert!(script.steps.is_empty());
    let create = &script.requests.last().unwrap().1;
    assert_eq!(create["type_key"], DEFAULT_OBJECT_TYPE_KEY);
    assert_eq!(create["properties"], json!([]));
    assert_eq!(
        record_from_markdown(&create["body"])
            .reference
            .foreign
            .dav_uid
            .as_deref(),
        Some("carol-uid")
    );
}

#[test]
fn missing_optional_key_keeps_typed_creation_body_only() {
    let fixture = Fixture::new();
    let mut properties = managed_schema();
    properties.retain(|property| property["key"] != ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
    fixture.discovery(person_types(), properties);
    fixture.materialization(true);
    let mut app = fixture.app();
    app.discover_domain_capabilities("home").unwrap();
    assert_eq!(
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY).unwrap(),
        MetadataProjection::BodyOnly
    );
    let script = fixture.script.lock().unwrap();
    assert!(script.steps.is_empty());
    let create = &script.requests.last().unwrap().1;
    assert_eq!(create["type_key"], ANYCAL_PERSON_CONTEXT_TYPE_KEY);
    assert_eq!(create["properties"], json!([]));
    assert_eq!(
        record_from_markdown(&create["body"])
            .reference
            .foreign
            .source_space_id,
        "space-shared"
    );
}

#[test]
fn typed_creation_requires_discovery_before_any_http_request() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    assert_eq!(
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY),
        Err(CompositionOrchestrationError::DestinationSchema(
            CompositionSchemaError::SnapshotRequired
        ))
    );
    assert!(fixture.script.lock().unwrap().requests.is_empty());
}

#[test]
fn invalid_destination_schema_fails_before_object_reads_or_writes() {
    let mut wrong_format = managed_schema();
    wrong_format[0]["format"] = json!("number");
    let mut duplicate_key = managed_schema();
    let mut duplicate = duplicate_key[0].clone();
    duplicate["id"] = json!("another-property");
    duplicate_key.push(duplicate);
    let mut duplicate_type = person_types();
    let mut duplicate = duplicate_type[0].clone();
    duplicate["id"] = json!("another-type");
    duplicate_type.push(duplicate);
    for (types, properties, expected) in [
        (
            Vec::new(),
            managed_schema(),
            CompositionSchemaError::RequiredTypeMissing,
        ),
        (
            duplicate_type,
            managed_schema(),
            CompositionSchemaError::RequiredTypeAmbiguous,
        ),
        (
            person_types(),
            wrong_format,
            CompositionSchemaError::PropertyFormatMismatch(MANAGED_KEYS[0]),
        ),
        (
            person_types(),
            duplicate_key,
            CompositionSchemaError::DuplicatePropertyKey(MANAGED_KEYS[0]),
        ),
    ] {
        let fixture = Fixture::new();
        fixture.discovery(types, properties);
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        assert_eq!(
            materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY),
            Err(CompositionOrchestrationError::DestinationSchema(expected))
        );
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        assert_eq!(
            script.requests.len(),
            4,
            "only discovery GETs may reach Anytype"
        );
        assert!(script.destination.is_none());
    }
}

#[test]
fn conflicting_native_metadata_from_get_is_never_overwritten() {
    for (duplicate, expected) in [
        (
            false,
            CompositionSchemaError::ExistingPropertyFormatMismatch(
                ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
            ),
        ),
        (
            true,
            CompositionSchemaError::DuplicateObjectProperty(ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY),
        ),
    ] {
        let fixture = Fixture::new();
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(true);
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY).unwrap();
        let before = {
            let mut script = fixture.script.lock().unwrap();
            let destination = script.destination.as_mut().unwrap();
            let properties = destination["properties"].as_array_mut().unwrap();
            if duplicate {
                properties.push(json!({
                    "key":ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
                    "format":"text", "text":"conflicting-uid"
                }));
            } else {
                properties.retain(|property| property["key"] != ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY);
                properties.push(json!({
                    "key":ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
                    "format":"objects", "objects":[{"id":"private-local-object"}]
                }));
            }
            destination.clone()
        };
        fixture.discovery(person_types(), managed_schema());
        {
            let mut script = fixture.script.lock().unwrap();
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects HTTP/1.1".into(),
                Reply::Json(json!({"data":[{"id":"context-1"}]})),
            ));
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                Reply::Destination,
            ));
        }
        let mut reopened = fixture.app();
        reopened.discover_domain_capabilities("home").unwrap();
        assert_eq!(
            materialize_typed(&mut reopened, ANYCAL_PERSON_CONTEXT_TYPE_KEY),
            Err(CompositionOrchestrationError::DestinationSchema(expected))
        );
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        assert!(!script
            .requests
            .iter()
            .any(|(line, _)| line.starts_with("PATCH ")));
        assert_eq!(script.destination.as_ref().unwrap(), &before);
    }
}

#[test]
fn patch_timeout_requires_native_metadata_readback_even_when_body_is_unchanged() {
    for applied in [false, true] {
        let fixture = Fixture::new();
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(true);
        let mut app = fixture.app();
        app.discover_domain_capabilities("home").unwrap();
        materialize_typed(&mut app, ANYCAL_PERSON_CONTEXT_TYPE_KEY).unwrap();
        let unchanged_markdown = {
            let mut script = fixture.script.lock().unwrap();
            let destination = script.destination.as_mut().unwrap();
            let stale = destination["properties"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|property| property["key"] == ANYCAL_SOURCE_STATUS_PROPERTY_KEY)
                .unwrap();
            stale["text"] = json!("unavailable");
            destination["markdown"].clone()
        };
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(false);
        {
            let mut script = fixture.script.lock().unwrap();
            script.steps.back_mut().unwrap().1 = if applied {
                Reply::SaveDestination { timeout: true }
            } else {
                Reply::Timeout
            };
            script.steps.push_back((
                "GET /v1/spaces/space-home/objects/context-1 HTTP/1.1".into(),
                Reply::Destination,
            ));
        }
        let mut reopened = fixture.app();
        reopened.discover_domain_capabilities("home").unwrap();
        let result = materialize_typed(&mut reopened, ANYCAL_PERSON_CONTEXT_TYPE_KEY);
        if applied {
            result.unwrap();
        } else {
            assert_eq!(
                result,
                Err(CompositionOrchestrationError::DestinationTransport(
                    TransportError::Timeout
                ))
            );
        }
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        let patch = &script
            .requests
            .iter()
            .find(|(line, _)| line.starts_with("PATCH "))
            .unwrap()
            .1;
        assert_eq!(patch["markdown"], unchanged_markdown);
        assert!(patch["properties"]
            .as_array()
            .unwrap()
            .iter()
            .any(|property| {
                property["key"] == ANYCAL_SOURCE_STATUS_PROPERTY_KEY
                    && property["text"] == "available"
            }));
        let observed = script.destination.as_ref().unwrap()["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|property| property["key"] == ANYCAL_SOURCE_STATUS_PROPERTY_KEY)
            .unwrap();
        assert_eq!(
            observed["text"],
            if applied { "available" } else { "unavailable" }
        );
    }
}

#[test]
fn native_metadata_round_trips_with_private_fields_across_http_refresh_and_reopen() {
    let mut fixture = Fixture::new();
    fixture.discovery(person_types(), managed_schema());
    fixture.materialization(true);
    let mut app = fixture.app();
    app.discover_domain_capabilities("home").unwrap();
    let created = app
        .materialize_person_contexts(&profile(), "alice-token", &scopes())
        .unwrap();
    assert!(created[0].created);
    assert_eq!(created[0].metadata_projection, MetadataProjection::Native);
    let original_id = created[0].destination_object_id.clone();
    let foreign_identity = created[0].reference.foreign.identity_fingerprint().unwrap();
    let private_properties = json!([
        {"key":"private_notes","format":"text","text":"Private café note"},
        {"key":"private_tags","format":"multi_select","multi_select":[{"key":"follow-up","name":"Follow up"}]},
        {"key":"private_relations","format":"objects","objects":[{"id":"home-project","name":"Private project"}]}
    ]);
    {
        let mut script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        let create = &script.requests.last().unwrap().1;
        assert_eq!(create["type_key"], ANYCAL_PERSON_CONTEXT_TYPE_KEY);
        assert_eq!(
            create["properties"].as_array().unwrap().len(),
            MANAGED_KEYS.len()
        );
        for key in MANAGED_KEYS {
            assert!(create["properties"]
                .as_array()
                .unwrap()
                .iter()
                .any(|property| property["key"] == key && property["text"].is_string()));
        }
        let destination = script.destination.as_mut().unwrap();
        let mut record = record_from_markdown(&destination["markdown"]);
        record.reference.user_fields =
            BTreeMap::from([("private_notes".into(), json!("Alice only"))]);
        destination["markdown"] = json!(format!(
            "```json\n{}\n```",
            record.canonical_json().unwrap()
        ));
        destination["properties"]
            .as_array_mut()
            .unwrap()
            .extend(private_properties.as_array().unwrap().iter().cloned());
        destination["properties"].as_array_mut().unwrap().extend([
            json!({"key":"creator","format":"objects","objects":[{"id":"alice"}]}),
            json!({"key":"created_date","format":"date","date":"2026-09-23T10:00:00Z"}),
            json!({"key":"links","format":"objects","objects":[{"id":"home-project"}]}),
        ]);
    }
    fixture
        .source
        .document
        .content
        .fields
        .get_mut("FN")
        .unwrap()[0]
        .value = "Carol Updated".into();

    // Restart before refresh: recover identity and private data from HTTP GET,
    // and require fresh binding-scoped schema discovery for the new runtime.
    for expected_kind in [RefreshKind::SourceRefreshed, RefreshKind::Unchanged] {
        fixture.discovery(person_types(), managed_schema());
        fixture.materialization(false);
        let mut reopened = fixture.app();
        reopened.discover_domain_capabilities("home").unwrap();
        let refreshed = reopened
            .materialize_person_contexts(&profile(), "alice-token", &scopes())
            .unwrap();
        assert!(!refreshed[0].created);
        assert_eq!(refreshed[0].metadata_projection, MetadataProjection::Native);
        assert_eq!(refreshed[0].destination_object_id, original_id);
        assert_eq!(refreshed[0].refresh_kind, expected_kind);
        assert_eq!(
            refreshed[0].reference.source_fields["display_name"],
            "Carol Updated"
        );
        assert_eq!(
            refreshed[0].reference.user_fields["private_notes"],
            "Alice only"
        );
        let script = fixture.script.lock().unwrap();
        assert!(script.steps.is_empty());
        let patch = &script.requests.last().unwrap().1;
        assert!(patch.get("type_key").is_none());
        let properties = patch["properties"]
            .as_array()
            .unwrap()
            .iter()
            .map(|property| (property["key"].as_str().unwrap(), property))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(properties.len(), MANAGED_KEYS.len() + 3);
        assert_eq!(
            properties[ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY]["text"],
            foreign_identity
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY]["text"],
            "space-shared"
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY]["text"],
            fixture.source.anytype_object_id.to_string()
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY]["text"],
            "carol-uid"
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_KIND_PROPERTY_KEY]["text"],
            "contact"
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_STATUS_PROPERTY_KEY]["text"],
            "available"
        );
        assert_eq!(properties["private_notes"]["text"], "Private café note");
        assert_eq!(
            properties["private_tags"]["multi_select"],
            json!(["follow-up"])
        );
        assert_eq!(
            properties["private_relations"]["objects"],
            json!(["home-project"])
        );
        assert!(!properties.contains_key("creator"));
        assert!(!properties.contains_key("created_date"));
        assert!(!properties.contains_key("links"));
        let persisted = record_from_markdown(&patch["markdown"]);
        assert_eq!(persisted.reference, refreshed[0].reference);
        assert_eq!(
            properties[ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY]["text"],
            persisted.profile_fingerprint
        );
        assert_eq!(
            properties[ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY]["text"],
            persisted.reference.foreign.upstream_account_fingerprint
        );
    }
    let script = fixture.script.lock().unwrap();
    assert_eq!(
        script
            .requests
            .iter()
            .filter(|(line, _)| line.starts_with("POST "))
            .count(),
        1
    );
    assert_eq!(
        script
            .requests
            .iter()
            .filter(|(line, _)| line.starts_with("PATCH "))
            .count(),
        2
    );
}
