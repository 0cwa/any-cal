use any_cal_anytype_adapter::{
    AnytypeDiscovery, AnytypeTransport, HttpAnytypeTransport, TransportError, API_VERSION,
};
use any_cal_core::RepositoryError;
use rcgen::generate_simple_self_signed;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

struct Scripted {
    expected: VecDeque<String>,
    responses: VecDeque<Vec<u8>>,
}
impl any_cal_anytype_adapter::HttpExchange for Scripted {
    fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        let text = String::from_utf8(request.to_vec()).unwrap();
        let first = text.lines().next().unwrap().to_string();
        assert_eq!(first, self.expected.pop_front().unwrap());
        assert!(text.contains("Anytype-Version: 2025-11-08"));
        assert!(text.contains("Authorization: Bearer secret"));
        let declared = text
            .lines()
            .find(|line| line.starts_with("Content-Length:"))
            .unwrap()
            .split_once(':')
            .unwrap()
            .1
            .trim()
            .parse::<usize>()
            .unwrap();
        let body = text.split_once("\r\n\r\n").unwrap().1.as_bytes();
        assert_eq!(declared, body.len());
        if first.starts_with("POST ") {
            assert!(text.contains("\"type_key\":\"page\""));
            assert!(text.contains("\"body\""));
            assert!(text.contains("\"key\""));
        } else if first.starts_with("PATCH ") {
            assert!(text.contains("\"markdown\""));
            assert!(!text.contains("\"type_key\""));
        }
        Ok(self.responses.pop_front().unwrap())
    }
}

fn json_response(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn response_with_headers(status: u16, headers: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status} Test\r\n{headers}\r\n\r\n{body}").into_bytes()
}

#[derive(Clone)]
struct SingleResponse(Vec<u8>);
impl any_cal_anytype_adapter::HttpExchange for SingleResponse {
    fn exchange(&mut self, _request: &[u8]) -> Result<Vec<u8>, TransportError> {
        Ok(self.0.clone())
    }
}

struct FailingExchange;
impl any_cal_anytype_adapter::HttpExchange for FailingExchange {
    fn exchange(&mut self, _request: &[u8]) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::Timeout)
    }
}

fn object_record() -> any_cal_anytype_adapter::ObjectRecord {
    any_cal_anytype_adapter::ObjectRecord {
        id: "obj".into(),
        space_id: "space".into(),
        properties: vec![("description".into(), "Example".into())],
        property_formats: BTreeMap::new(),
        body: "{}".into(),
        archived: false,
        revision: 1,
    }
}

fn object_json() -> String {
    serde_json::json!({
        "object": {
            "id": "obj",
            "space_id": "space",
            "type": {"key": "page", "name": "Page"},
            "name": "Example",
            "markdown": "{}",
            "properties": [{"key": "description", "name": "Description", "format": "text", "text": "Example"}],
            "archived": false
        }
    })
    .to_string()
}

#[test]
fn http_transport_executes_framed_paginated_request() {
    let body = r#"{"data":[],"next_offset":null}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let scripted = Scripted {
        expected: ["GET /v1/spaces/space/objects?offset=next&limit=100 HTTP/1.1".into()]
            .into_iter()
            .collect(),
        responses: [response].into_iter().collect(),
    };
    let mut transport =
        HttpAnytypeTransport::new("http://127.0.0.1:1", "2025-11-08", Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(scripted));
    let page = transport.list_objects("space", Some("next")).unwrap();
    assert!(page.data.is_empty());
    assert!(page.next_offset.is_none());
}

#[test]
fn scripted_crud_uses_exact_paths_and_framed_json_bodies() {
    let body = object_json();
    let scripted = Scripted {
        expected: [
            "POST /v1/spaces/space/objects HTTP/1.1",
            "GET /v1/spaces/space/objects/obj HTTP/1.1",
            "PATCH /v1/spaces/space/objects/obj HTTP/1.1",
            "DELETE /v1/spaces/space/objects/obj HTTP/1.1",
            "DELETE /v1/spaces/space/objects/obj HTTP/1.1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        responses: (0..5).map(|_| json_response(200, &body)).collect(),
    };
    let mut transport =
        HttpAnytypeTransport::new("http://127.0.0.1:1", "2025-11-08", Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(scripted));
    let object = object_record();
    assert_eq!(transport.create_object(object.clone()).unwrap().id, "obj");
    assert_eq!(transport.get_object("space", "obj").unwrap().id, "obj");
    let updated = transport.update_object(object.clone()).unwrap();
    assert_eq!(updated.id, "obj");
    assert_eq!(updated.body, "{}");
    assert_eq!(transport.archive_object("space", "obj").unwrap().id, "obj");
    assert_eq!(transport.delete_object("space", "obj").unwrap().id, "obj");
}

#[test]
fn scripted_statuses_remain_typed_and_bad_framing_is_rejected() {
    for (status, expected) in [
        (401, TransportError::Auth),
        (403, TransportError::Forbidden),
        (404, TransportError::NotFound),
        (409, TransportError::Conflict),
        (429, TransportError::RateLimited),
        (500, TransportError::Unavailable),
        (504, TransportError::Timeout),
    ] {
        let mut transport = HttpAnytypeTransport::new("http://host", "v", None)
            .unwrap()
            .with_exchange(Box::new(SingleResponse(json_response(status, "{}"))));
        assert_eq!(transport.list_objects("s", None).unwrap_err(), expected);
    }

    let malformed = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}";
    let mut transport = HttpAnytypeTransport::new("http://host", "v", None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(malformed.to_vec())));
    assert_eq!(
        transport.list_objects("s", None),
        Err(TransportError::Malformed)
    );

    let truncated =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 3\r\n\r\n{}";
    let mut transport = HttpAnytypeTransport::new("http://host", "v", None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(truncated.to_vec())));
    assert_eq!(
        transport.list_objects("s", None),
        Err(TransportError::Malformed)
    );

    let mut transport = HttpAnytypeTransport::new("http://host", "v", None)
        .unwrap()
        .with_exchange(Box::new(FailingExchange));
    assert_eq!(
        transport.list_objects("s", None),
        Err(TransportError::Timeout)
    );

    let mut transport = HttpAnytypeTransport::new("http://host", "v", None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(json_response(200, "{}"))));
    transport.max_body = 1;
    assert_eq!(
        transport.list_objects("s", None),
        Err(TransportError::Malformed)
    );
}

#[test]
fn keep_alive_content_length_is_complete_without_eof() {
    let body = r#"{"data":[],"next_offset":null}"#;
    let response = response_with_headers(
        200,
        &format!(
            "Content-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: {}",
            body.len()
        ),
        body,
    );
    let mut transport = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(response)));
    assert!(transport
        .list_objects("space", None)
        .unwrap()
        .data
        .is_empty());
}

#[test]
fn chunked_keep_alive_response_is_decoded_and_bounded() {
    let body = r#"{"data":[],"next_offset":null}"#;
    let split = body.len() / 2;
    let chunked = format!(
        "{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\nX-Test: bounded\r\n\r\n",
        split,
        &body[..split],
        body.len() - split,
        &body[split..]
    );
    let response = response_with_headers(
        200,
        "Content-Type: application/json\r\nConnection: keep-alive\r\nTransfer-Encoding: chunked",
        &chunked,
    );
    let mut transport = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(response)));
    assert!(transport
        .list_objects("space", None)
        .unwrap()
        .data
        .is_empty());
}

#[test]
fn close_delimited_response_is_supported_only_with_connection_close() {
    let body = r#"{"data":[],"next_offset":null}"#;
    let response = response_with_headers(
        200,
        "Content-Type: application/json\r\nConnection: close",
        body,
    );
    let mut transport = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(response)));
    assert!(transport
        .list_objects("space", None)
        .unwrap()
        .data
        .is_empty());

    let response = response_with_headers(
        200,
        "Content-Type: application/json\r\nConnection: keep-alive",
        body,
    );
    let mut transport = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(response)));
    assert_eq!(
        transport.list_objects("space", None),
        Err(TransportError::Malformed)
    );
}

#[test]
fn typed_remote_errors_survive_repository_mapping() {
    assert_eq!(
        any_cal_anytype_adapter::AnytypeRepository::<
            any_cal_anytype_adapter::FakeAnytypeTransport,
        >::map_transport_error(TransportError::Auth),
        RepositoryError::Auth
    );
    assert_eq!(
        any_cal_anytype_adapter::AnytypeRepository::<
            any_cal_anytype_adapter::FakeAnytypeTransport,
        >::map_transport_error(TransportError::Forbidden),
        RepositoryError::Forbidden
    );
    assert_eq!(
        any_cal_anytype_adapter::AnytypeRepository::<
            any_cal_anytype_adapter::FakeAnytypeTransport,
        >::map_transport_error(TransportError::RateLimited),
        RepositoryError::RateLimited
    );
    assert_eq!(
        any_cal_anytype_adapter::AnytypeRepository::<
            any_cal_anytype_adapter::FakeAnytypeTransport,
        >::map_transport_error(TransportError::Unavailable),
        RepositoryError::Unavailable
    );
}

#[test]
fn https_is_accepted_only_with_a_valid_authority() {
    assert!(HttpAnytypeTransport::new("https://example.test", "v", None).is_ok());
    assert!(HttpAnytypeTransport::new("https://[::1]:8443", "v", None).is_ok());
    assert!(matches!(
        HttpAnytypeTransport::new("https://bad host", "v", None),
        Err(TransportError::InvalidRequest(_))
    ));
}

fn self_signed_tls_listener() -> (String, thread::JoinHandle<()>) {
    let certificate = generate_simple_self_signed(["wrong.example".to_owned()]).unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(certificate.signing_key.serialize_der().into()),
        )
        .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let thread = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut request = [0; 1024];
        let _ = std::io::Read::read(&mut stream, &mut request);
    });
    (endpoint, thread)
}

#[test]
#[ignore = "requires local socket permission; run in the host integration harness"]
fn https_rejects_untrusted_and_wrong_hostname_certificates_without_leaking_token() {
    let (endpoint, server) = self_signed_tls_listener();
    let mut transport =
        HttpAnytypeTransport::new(endpoint, API_VERSION, Some("secret-token".into())).unwrap();
    transport.timeout = std::time::Duration::from_secs(2);
    assert_eq!(
        transport.list_objects("space", None),
        Err(TransportError::Unavailable)
    );
    server.join().unwrap();
}

#[test]
fn official_offset_and_path_encoding_are_used() {
    let page = r#"{"data":[],"next_offset":25}"#;
    let scripted = Scripted {
        expected: ["GET /v1/spaces/space%20id/objects?offset=10&limit=100 HTTP/1.1".into()]
            .into_iter()
            .collect(),
        responses: [json_response(200, page)].into_iter().collect(),
    };
    let mut transport =
        HttpAnytypeTransport::new("http://host/base/", API_VERSION, Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(scripted));
    let page = transport.list_objects("space id", Some("10")).unwrap();
    assert_eq!(page.next_offset.as_deref(), Some("25"));
}

#[test]
fn path_segments_are_percent_encoded() {
    let body = object_json();
    let scripted = Scripted {
        expected: ["GET /v1/spaces/s%2Fpace/objects/o%20bj HTTP/1.1".into()]
            .into_iter()
            .collect(),
        responses: [json_response(200, &body)].into_iter().collect(),
    };
    let mut transport =
        HttpAnytypeTransport::new("http://host", API_VERSION, Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(scripted));
    assert_eq!(transport.get_object("s/pace", "o bj").unwrap().id, "obj");
}

#[test]
fn header_values_reject_control_characters() {
    assert!(matches!(
        HttpAnytypeTransport::new("http://host", "2025-11-08\r\nX-Evil: yes", None),
        Err(TransportError::InvalidRequest(_))
    ));
    assert!(matches!(
        HttpAnytypeTransport::new("http://host", API_VERSION, Some("secret\nforged".into())),
        Err(TransportError::InvalidRequest(_))
    ));
}

#[test]
fn endpoint_rejects_nul_and_del_but_accepts_valid_host() {
    assert!(matches!(
        HttpAnytypeTransport::new("http://host\0", API_VERSION, None),
        Err(TransportError::InvalidRequest(_))
    ));
    assert!(matches!(
        HttpAnytypeTransport::new("http://host\u{7f}", API_VERSION, None),
        Err(TransportError::InvalidRequest(_))
    ));
    assert!(HttpAnytypeTransport::new("http://127.0.0.1:31012", API_VERSION, None).is_ok());
}

#[test]
fn archive_and_delete_accept_successful_no_content() {
    let response = response_with_headers(204, "Content-Length: 0", "");
    let mut archive = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(response.clone())));
    let archived = archive.archive_object("space", "obj").unwrap();
    assert_eq!(archived.id, "obj");
    assert_eq!(archived.space_id, "space");
    assert!(archived.archived);

    let mut delete = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(response)));
    let deleted = delete.delete_object("space", "obj").unwrap();
    assert_eq!(deleted.id, "obj");
    assert_eq!(deleted.space_id, "space");
    assert!(deleted.archived);
}

#[test]
fn malformed_json_and_oversized_pages_fail_without_a_second_request() {
    let mut transport =
        HttpAnytypeTransport::new("http://host", API_VERSION, Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(SingleResponse(json_response(200, "{not-json}"))));
    assert_eq!(
        transport.list_objects("space", None),
        Err(TransportError::Malformed)
    );

    let mut transport =
        HttpAnytypeTransport::new("http://host", API_VERSION, Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(SingleResponse(json_response(200, "{}"))));
    transport.max_body = 1;
    assert_eq!(
        transport.list_objects("space", None),
        Err(TransportError::Malformed)
    );
}

#[test]
fn pagination_advances_offsets_deterministically() {
    let page = r#"{"data":[],"next_offset":100}"#;
    let last = r#"{"data":[],"next_offset":null}"#;
    let scripted = Scripted {
        expected: [
            "GET /v1/spaces/space/objects?offset=0&limit=100 HTTP/1.1",
            "GET /v1/spaces/space/objects?offset=100&limit=100 HTTP/1.1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        responses: [json_response(200, page), json_response(200, last)]
            .into_iter()
            .collect(),
    };
    let mut transport =
        HttpAnytypeTransport::new("http://host", API_VERSION, Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(scripted));
    let first = transport.list_objects("space", Some("0")).unwrap();
    assert_eq!(first.next_offset.as_deref(), Some("100"));
    let second = transport
        .list_objects("space", first.next_offset.as_deref())
        .unwrap();
    assert!(second.next_offset.is_none());
}


#[test]
fn stable_v1_discovery_uses_read_only_space_scoped_endpoints() {
    let scripted = Scripted {
        expected: [
            "GET /v1/spaces/space%20id/types?offset=0&limit=100 HTTP/1.1",
            "GET /v1/spaces/space%20id/properties?offset=100&limit=100 HTTP/1.1",
            "GET /v1/spaces/space%20id/members?offset=0&limit=100 HTTP/1.1",
            "GET /v1/spaces/space%20id/properties/status%2Fproperty/tags?offset=0&limit=100 HTTP/1.1",
            "GET /v1/spaces/space%20id/lists/list%2Fid/views?offset=0&limit=100 HTTP/1.1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        responses: [
            json_response(
                200,
                r#"{"data":[{"id":"type-id","key":"task","name":"Task","layout":"task","icon":{"emoji":"✅"}}],"pagination":{"has_more":true,"offset":0,"limit":100,"total":101}}"#,
            ),
            json_response(
                200,
                r#"{"data":[{"id":"property-id","key":"due-date","name":"Due date","format":"date","object":"property"}],"pagination":{"has_more":false,"offset":100,"limit":100,"total":101}}"#,
            ),
            json_response(
                200,
                r#"{"data":[{"profile_id":"profile-id","name":"Alice","network_id":"network-id","global_name":"alice.any","status":"active","role":"Viewer"}],"pagination":{"has_more":false,"offset":0,"limit":100,"total":1}}"#,
            ),
            json_response(
                200,
                r#"{"data":[{"id":"tag-id","key":"in-progress","name":"In progress","color":"yellow","object":"tag"}],"pagination":{"has_more":false,"offset":0,"limit":100,"total":1}}"#,
            ),
            json_response(
                200,
                r#"{"data":[{"id":"view-id","name":"Today","layout":"table","filters":[{"property_key":"due-date"}],"sorts":[{"property_key":"name"}]}],"pagination":{"has_more":false,"offset":0,"limit":100,"total":1}}"#,
            ),
        ]
        .into_iter()
        .collect(),
    };
    let mut transport =
        HttpAnytypeTransport::new("http://host", API_VERSION, Some("secret".into()))
            .unwrap()
            .with_exchange(Box::new(scripted));

    let types = transport.list_types("space id", None).unwrap();
    assert_eq!(types.data[0].id, "type-id");
    assert_eq!(types.data[0].key, "task");
    assert_eq!(types.data[0].layout, "task");
    assert_eq!(types.next_offset.as_deref(), Some("100"));

    let properties = transport
        .list_properties("space id", types.next_offset.as_deref())
        .unwrap();
    assert_eq!(properties.data[0].id, "property-id");
    assert_eq!(properties.data[0].key, "due-date");
    assert_eq!(properties.data[0].format, "date");
    assert!(properties.next_offset.is_none());

    let members = transport.list_members("space id", None).unwrap();
    assert_eq!(members.data[0].profile_id, "profile-id");
    assert_eq!(members.data[0].network_id.as_deref(), Some("network-id"));
    assert_eq!(members.data[0].role, "Viewer");
    assert_eq!(members.data[0].status, "active");

    let tags = transport
        .list_tags("space id", "status/property", None)
        .unwrap();
    assert_eq!(tags.data[0].id, "tag-id");
    assert_eq!(tags.data[0].key, "in-progress");
    assert_eq!(tags.data[0].color, "yellow");

    let views = transport.list_views("space id", "list/id", None).unwrap();
    assert_eq!(views.data[0].id, "view-id");
    assert_eq!(views.data[0].layout, "table");
    assert!(views.data[0].filters.is_array());
    assert!(views.data[0].sorts.is_array());
}

#[test]
fn discovery_rejects_unbounded_or_malformed_offsets_before_http() {
    let mut transport = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(FailingExchange));
    assert!(matches!(
        transport.list_types("space", Some("next")),
        Err(TransportError::InvalidRequest(_))
    ));
}

#[test]
fn discovery_rejects_schema_records_without_stable_ids_or_formats() {
    let body = r#"{"data":[{"id":"property-id","name":"Due date","format":"date"}],"pagination":{"has_more":false,"offset":0,"limit":100,"total":1}}"#;
    let mut transport = HttpAnytypeTransport::new("http://host", API_VERSION, None)
        .unwrap()
        .with_exchange(Box::new(SingleResponse(json_response(200, body))));
    assert_eq!(
        transport.list_properties("space", None),
        Err(TransportError::Malformed)
    );
}
