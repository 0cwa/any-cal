use any_cal_core::{MemoryRepository, Repository};
use any_cal_dav_server::{DavServer, Request, Response};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Step {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    expected_status: u16,
    expectations: Vec<String>,
}
type NormalizedResponse = (u16, Vec<(String, String)>, Vec<u8>);

fn parse_profile(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut current: Option<Step> = None;
    for line in text.lines() {
        let line = line.trim_end();
        let first = line.split_whitespace().next().unwrap_or("");
        if ["OPTIONS", "PROPFIND", "PUT", "GET", "REPORT", "DELETE"].contains(&first) {
            if let Some(step) = current.take() {
                steps.push(step);
            }
            let (request_line, expected_status) = if let Some((left, right)) = line.split_once("=>")
            {
                (
                    left.trim_end(),
                    right
                        .trim()
                        .parse()
                        .map_err(|_| "bad expected status")
                        .unwrap(),
                )
            } else {
                (line, 200)
            };
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap().to_string();
            let path = parts
                .next()
                .ok_or_else(|| format!("missing path: {line}"))?
                .to_string();
            let headers = request_line
                .split_once('(')
                .and_then(|(_, x)| x.strip_suffix(')'))
                .map(|h| {
                    h.split(';')
                        .filter_map(|p| {
                            p.trim()
                                .split_once(':')
                                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            current = Some(Step {
                method,
                path,
                headers,
                body: Vec::new(),
                expected_status,
                expectations: Vec::new(),
            });
        } else if let Some(expectation) = line.strip_prefix("EXPECT-HEADER ") {
            current
                .as_mut()
                .ok_or_else(|| "expectation without request".to_owned())?
                .expectations
                .push(format!("HEADER {expectation}"));
        } else if let Some(expectation) = line.strip_prefix("EXPECT ") {
            current
                .as_mut()
                .ok_or_else(|| "expectation without request".to_owned())?
                .expectations
                .push(expectation.to_owned());
        } else if line.starts_with('#') || line.is_empty() {
            continue;
        } else if let Some(step) = current.as_mut() {
            step.body.extend_from_slice(line.as_bytes());
            step.body.push(b'\n');
        } else {
            return Err(format!("content without request: {line}"));
        }
    }
    if let Some(step) = current {
        steps.push(step);
    }
    if steps.is_empty() {
        return Err("empty profile".into());
    }
    Ok(steps)
}

fn normalize(response: &Response) -> NormalizedResponse {
    let mut headers: Vec<_> = response
        .headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();
    headers.sort();
    (
        response.status,
        headers,
        String::from_utf8_lossy(&response.body)
            .replace("\r\n", "\n")
            .into_bytes(),
    )
}

fn response_header<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn run(profile: &str) -> (Vec<NormalizedResponse>, BTreeMap<String, String>) {
    let steps = parse_profile(profile).unwrap();
    let mut server = DavServer::try_new(MemoryRepository::new()).unwrap();
    let mut trace = Vec::new();
    let mut etag = String::new();
    for step in steps {
        let headers = step
            .headers
            .into_iter()
            .map(|(k, v)| (k, v.replace("$ETAG", &etag)))
            .collect();
        let response = server.handle(Request {
            method: step.method.clone(),
            path: step.path.clone(),
            headers,
            body: step.body,
        });
        if let Some((_, value)) = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
        {
            etag = value.clone();
        }
        assert_eq!(
            response.status, step.expected_status,
            "{} {}",
            step.method, step.path
        );
        for expectation in step.expectations {
            if let Some(expectation) = expectation.strip_prefix("HEADER ") {
                let (name, expected) = expectation
                    .split_once(':')
                    .ok_or_else(|| format!("invalid header expectation: {expectation}"))
                    .unwrap();
                let actual = response_header(&response, name.trim());
                if expected.trim() == "*" {
                    assert!(actual.is_some(), "missing expected header {}", name.trim());
                } else {
                    assert_eq!(actual, Some(expected.trim()), "header {name}");
                }
            } else {
                let (negative, needle) = expectation
                    .strip_prefix('!')
                    .map_or((false, expectation.as_str()), |value| (true, value));
                let body = String::from_utf8_lossy(&response.body);
                if negative {
                    assert!(
                        !body.contains(needle),
                        "body unexpectedly contains {needle}"
                    );
                } else {
                    assert!(body.contains(needle), "body lacks {needle}");
                }
            }
        }
        trace.push(normalize(&response));
    }
    let mut snapshot = BTreeMap::new();
    for resource in server
        .repository
        .list_resources(&server.contacts, true)
        .unwrap()
        .into_iter()
        .chain(
            server
                .repository
                .list_resources(&server.tasks, true)
                .unwrap(),
        )
    {
        snapshot.insert(
            resource.envelope.resource_id.to_string(),
            format!(
                "{}:{}:{}",
                resource.etag.as_str(),
                resource.archived,
                resource.envelope.revision
            ),
        );
    }
    (trace, snapshot)
}

#[test]
fn fixture_profiles_execute_twice_with_identical_trace_and_snapshot() {
    for profile in [
        include_str!("../../../fixtures/protocol/carddav-lifecycle.http"),
        include_str!("../../../fixtures/protocol/tasksorg-vtodo.http"),
    ] {
        assert_eq!(run(profile), run(profile));
    }
}

#[test]
fn malformed_profile_is_rejected() {
    assert!(parse_profile("not a request").is_err());
    assert!(parse_profile("PUT /x\nBEGIN:VCARD").is_ok());
}

#[test]
fn https_proxy_profile_headers_are_preserved_in_requests() {
    let steps = parse_profile(include_str!(
        "../../../fixtures/protocol/carddav-lifecycle.http"
    ))
    .unwrap();
    assert!(steps[0]
        .headers
        .iter()
        .any(|(key, value)| key == "Host" && value == "dav.example.test"));
    assert!(steps[0]
        .headers
        .iter()
        .any(|(key, value)| key == "X-Forwarded-Proto" && value == "https"));
}
