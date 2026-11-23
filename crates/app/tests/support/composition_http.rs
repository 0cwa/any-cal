//! Shared sanitized HTTP fixture for composition integration tests.
use any_cal_anytype_adapter::{
    HttpAnytypeTransport, HttpExchange, TransportError, ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_PERSON_CONTEXT_TYPE_KEY, ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY, ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY, ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY, ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
};
use any_cal_app::composition::CompositionSourceScope;
use any_cal_app::composition_orchestration::MaterializedReferenceRecord;
use any_cal_app::identity::{
    AccessPolicy, CollectionKind, CredentialSpec, IdentityStore, Operation, PrincipalId,
};
use any_cal_app::{App, AppConfig, AppGeneric};
use any_cal_core::{CompositionProfile, ProjectionPolicy, ResourceEnvelope};
use any_cal_dav_server::Request;
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

pub const MANAGED_KEYS: [&str; 8] = [
    ANYCAL_PROFILE_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_FOREIGN_IDENTITY_PROPERTY_KEY,
    ANYCAL_SOURCE_ACCOUNT_FINGERPRINT_PROPERTY_KEY,
    ANYCAL_SOURCE_SPACE_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_OBJECT_ID_PROPERTY_KEY,
    ANYCAL_SOURCE_KIND_PROPERTY_KEY,
    ANYCAL_SOURCE_STATUS_PROPERTY_KEY,
    ANYCAL_SOURCE_DAV_UID_PROPERTY_KEY,
];

pub enum Reply {
    Json(Value),
    Destination,
    SaveDestination { timeout: bool },
    SaveDestinationIgnoringName,
    Timeout,
}

#[derive(Default)]
pub struct HttpScript {
    pub steps: VecDeque<(String, Reply)>,
    pub requests: Vec<(String, Value)>,
    pub destination: Option<Value>,
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
                let destination = save_destination(&mut script, &request_body, true);
                if timeout {
                    return Err(TransportError::Timeout);
                }
                json!({"object":destination})
            }
            Reply::SaveDestinationIgnoringName => {
                // A matching body and properties must not confirm a write
                // when the independently stored native title did not change.
                save_destination(&mut script, &request_body, false);
                return Err(TransportError::Timeout);
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

fn save_destination(script: &mut HttpScript, request_body: &Value, apply_name: bool) -> Value {
    // Model PATCH's per-field behavior: omitted server/private fields remain
    // present in the next GET response, while an explicit empty name is saved.
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
    if apply_name {
        if let Some(name) = request_body.get("name") {
            destination["name"] = name.clone();
        }
    }
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
    destination
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

pub fn profile() -> CompositionProfile {
    CompositionProfile::new(
        "alice",
        "home",
        vec!["shared".into()],
        ProjectionPolicy::PrivateDestinationOnly,
    )
    .unwrap()
}

pub fn scopes() -> [CompositionSourceScope; 1] {
    [CompositionSourceScope::resource(
        "shared",
        CollectionKind::Contacts,
        "carol",
    )]
}

pub fn person_types() -> Vec<Value> {
    vec![json!({
        "id":"type-person-context", "key":ANYCAL_PERSON_CONTEXT_TYPE_KEY,
        "name":"A renamed display label"
    })]
}

pub fn managed_schema() -> Vec<Value> {
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

pub struct Fixture {
    pub script: Arc<Mutex<HttpScript>>,
    pub source: ResourceEnvelope,
}

impl Fixture {
    pub fn new() -> Self {
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

    pub fn app(&self) -> AppGeneric<HttpAnytypeTransport> {
        AppGeneric::http_with_exchange(config(), Box::new(ScriptedHttp(Arc::clone(&self.script))))
            .unwrap()
            .with_identity(identity(), 150)
    }

    pub fn discovery(&self, types: Vec<Value>, properties: Vec<Value>) {
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

    pub fn materialization(&self, create: bool) {
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

pub fn record_from_markdown(markdown: &Value) -> MaterializedReferenceRecord {
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
