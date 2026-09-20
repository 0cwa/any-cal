use any_cal_core::{FailureMode, MemoryRepository, Repository};
use any_cal_dav_server::{DavServer, Request, Response};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Step {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    status: u16,
    checks: Vec<String>,
    failure: Option<FailureMode>,
}
type Trace = Vec<(u16, Vec<(String, String)>, Vec<u8>)>;
type Snapshot = BTreeMap<String, String>;

fn parse(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut current = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        let first = line.split_whitespace().next().unwrap_or("");
        if ["OPTIONS", "PROPFIND", "PUT", "GET", "REPORT", "DELETE"].contains(&first) {
            if let Some(step) = current.take() {
                steps.push(step);
            }
            let (left, status) = line.split_once("=>").ok_or("missing status")?;
            let left = left.trim_end();
            let status = status.trim().parse().map_err(|_| "invalid status")?;
            let mut parts = left.split_whitespace();
            let method = parts.next().unwrap().to_owned();
            let path = parts.next().ok_or("missing path")?.to_owned();
            let headers = left
                .split_once('(')
                .and_then(|(_, x)| x.strip_suffix(')'))
                .map(|x| {
                    x.split(';')
                        .filter_map(|p| {
                            p.trim()
                                .split_once(':')
                                .map(|(k, v)| (k.trim().into(), v.trim().into()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            current = Some(Step {
                method,
                path,
                headers,
                body: Vec::new(),
                status,
                checks: Vec::new(),
                failure: None,
            });
        } else if let Some(check) = line.strip_prefix("EXPECT ") {
            current
                .as_mut()
                .ok_or("expectation without request")?
                .checks
                .push(check.into());
        } else if let Some(check) = line.strip_prefix("EXPECT-HEADER ") {
            current
                .as_mut()
                .ok_or("header expectation without request")?
                .checks
                .push(format!("HEADER {check}"));
        } else if let Some(step) = current.as_mut() {
            let header_line = line.split_once(':').filter(|(key, _)| {
                ["Content-Type", "Depth", "If-Match", "If-None-Match"].contains(key)
                    && step.body.is_empty()
            });
            if let Some((key, value)) = header_line {
                step.headers.push((key.into(), value.trim().into()));
            } else if let Some(value) = line.strip_prefix("FAIL ") {
                step.failure = Some(match value {
                    "timeout" => FailureMode::Timeout,
                    "malformed" => FailureMode::MalformedState,
                    "delay" => FailureMode::ReadAfterWriteDelay,
                    "archive" => FailureMode::ArchiveFailure,
                    "duplicate" => FailureMode::DuplicateWrite,
                    _ => return Err("unknown failure".into()),
                });
            } else if !line.starts_with('#') && !line.is_empty() {
                step.body.extend_from_slice(line.as_bytes());
                step.body.push(b'\n');
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            return Err("content without request".into());
        }
    }
    if let Some(step) = current {
        steps.push(step);
    }
    if steps.is_empty() {
        Err("empty profile".into())
    } else {
        Ok(steps)
    }
}

fn response_header(response: &Response, name: &str) -> Option<String> {
    response
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn run(profile: &str) -> Result<(Trace, Snapshot), String> {
    let steps = parse(profile)?;
    let mut server = DavServer::try_new(MemoryRepository::new()).map_err(|e| e.to_string())?;
    let mut etag = String::new();
    let mut queued: Option<Step> = None;
    let mut trace = Vec::new();
    for step in steps {
        if let Some(failure) = &step.failure {
            server.repository.inject_failure(failure.clone());
        }
        let headers = step
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.replace("$ETAG", &etag)))
            .collect();
        let response = server.handle(Request {
            method: step.method.clone(),
            path: step.path.replace("$ETAG", &etag),
            headers,
            body: step.body.clone(),
        });
        if response.status == 409
            && step.method == "PUT"
            && step
                .headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("If-None-Match") && v == "*")
        {
            queued = Some(step.clone());
        }
        if response.status < 500 && step.method == "PUT" {
            if let Some(pending) = queued.take() {
                if pending.path != step.path || pending.body != step.body {
                    return Err("replayed write did not preserve queued intent".into());
                }
            }
        }
        if response.status != step.status {
            return Err(format!(
                "{} {} returned {} expected {}",
                step.method, step.path, response.status, step.status
            ));
        }
        let text = String::from_utf8_lossy(&response.body);
        for check in step.checks {
            if let Some(check) = check.strip_prefix("HEADER ") {
                let (key, expected) = check.split_once(':').ok_or("invalid header expectation")?;
                if response_header(&response, key.trim()).as_deref() != Some(expected.trim()) {
                    return Err(format!("header check failed: {check}"));
                }
                continue;
            }
            let (negative, needle) = check
                .strip_prefix('!')
                .map_or((false, check.as_str()), |x| (true, x));
            if negative == text.contains(needle) {
                return Err(format!("body check failed: {check}"));
            }
        }
        if let Some(value) = response_header(&response, "ETag") {
            etag = value;
        }
        if step.method == "REPORT"
            && response.status == 207
            && response_header(&response, "Content-Type").as_deref() != Some("application/xml")
        {
            return Err("REPORT content type mismatch".into());
        }
        if (step.method == "PUT" && (response.status == 201 || response.status == 204))
            && response_header(&response, "ETag").is_none()
        {
            return Err("successful PUT missing ETag".into());
        }
        if step.method == "GET" && response.status == 200 {
            let expected = if step.path.ends_with(".vcf") {
                "text/vcard"
            } else {
                "text/calendar"
            };
            if response_header(&response, "Content-Type").as_deref() != Some(expected)
                || response_header(&response, "ETag").is_none()
            {
                return Err("GET representation headers mismatch".into());
            }
        }
        let mut headers = response
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_ascii_lowercase(),
                    if k.eq_ignore_ascii_case("last-modified") {
                        "<volatile-clock>".into()
                    } else {
                        v.clone()
                    },
                )
            })
            .collect::<Vec<_>>();
        headers.sort();
        trace.push((response.status, headers, response.body));
    }
    let mut snapshot = BTreeMap::new();
    for collection in [server.contacts.clone(), server.tasks.clone()] {
        for row in server
            .repository
            .list_resources(&collection, true)
            .map_err(|e| e.to_string())?
        {
            snapshot.insert(
                row.envelope.resource_id.to_string(),
                format!(
                    "{}:{}:{}",
                    row.etag.as_str(),
                    row.archived,
                    row.envelope.revision
                ),
            );
        }
    }
    Ok((trace, snapshot))
}

#[test]
fn sanitized_android_profiles_consume_fixtures_twice() {
    for profile in [
        include_str!("../../../fixtures/protocol/android/davx5-discovery.http"),
        include_str!("../../../fixtures/protocol/android/davx5-contact.http"),
        include_str!("../../../fixtures/protocol/android/tasksorg-vtodo.http"),
    ] {
        assert_eq!(run(profile).unwrap(), run(profile).unwrap());
    }
}

#[test]
fn malformed_android_fixture_is_rejected() {
    assert!(parse("PUT /x\nEXPECT bad\n").is_err());
    assert!(parse("nonsense").is_err());
}
