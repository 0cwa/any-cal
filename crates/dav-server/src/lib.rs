//! Small protocol-facing adapter. Transport integration is intentionally left
//! to callers; this crate accepts a parsed request and returns a response.
use any_cal_core::{
    ical::{Calendar, Entry},
    vcard, vtodo, AnytypeObjectId, CanonicalDocument, CollectionId, DavKind, DavUid, Repository,
    ResourceEnvelope, ResourceId, StoredResource, StructuredDocument, WriteCondition,
};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

mod http_date;

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub struct DavServer<R> {
    pub repository: R,
    pub contacts: CollectionId,
    pub tasks: CollectionId,
}

/// Local-only HTTP serving helper. This crate handles HTTP/DAV protocol
/// behavior but does not terminate TLS or authenticate requests; a production
/// deployment must put it behind an HTTPS/authenticated transport wrapper or
/// reverse proxy and must not expose this listener directly.
pub fn serve_one<R: Repository>(
    server: &mut DavServer<R>,
    listener: &TcpListener,
) -> std::io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    let request = read_http_request(&mut stream)?;
    write_http(&mut stream, server.handle(request), false)
}

/// Serve framed HTTP/1.1 requests, allowing multiple requests on a connection
/// unless the client explicitly asks to close it. Every request is bounded by
/// the header and Content-Length limits in `read_http_request`.
pub fn serve<R: Repository + Send>(
    server: &mut DavServer<R>,
    listener: &TcpListener,
) -> std::io::Result<()> {
    let state = std::sync::Mutex::new(server);
    std::thread::scope(|scope| {
        for stream in listener.incoming() {
            let mut stream = stream?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            let state = &state;
            scope.spawn(move || {
                let _ = serve_connection(&mut stream, state);
            });
        }
        Ok(())
    })
}

fn serve_connection<R: Repository + Send>(
    stream: &mut TcpStream,
    state: &std::sync::Mutex<&mut DavServer<R>>,
) -> std::io::Result<()> {
    loop {
        let request = match read_http_request(stream) {
            Ok(request) => request,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                write_http(stream, protocol_error(408, "request timeout"), false)?;
                break;
            }
            Err(_) => {
                write_http(stream, protocol_error(400, "bad request"), false)?;
                break;
            }
        };
        let close = header(&request, "connection").is_some_and(connection_requests_close);
        let response = {
            let mut server = state
                .lock()
                .map_err(|_| std::io::Error::other("DAV state lock poisoned"))?;
            server.handle(request)
        };
        write_http(stream, response, !close)?;
        if close {
            break;
        }
    }
    Ok(())
}

fn protocol_error(status: u16, message: &str) -> Response {
    response(
        status,
        vec![("Content-Type", "text/plain".into())],
        message.as_bytes().to_vec(),
    )
}

impl<R: Repository> DavServer<R> {
    pub fn try_new(mut repository: R) -> Result<Self, any_cal_core::RepositoryError> {
        let contacts = CollectionId::try_from("contacts").unwrap();
        let tasks = CollectionId::try_from("tasks").unwrap();
        repository
            .create_collection(any_cal_core::Collection {
                id: contacts.clone(),
                name: "Contacts".into(),
            })
            .or_else(|e| match e {
                any_cal_core::RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                other => Err(other),
            })?;
        repository
            .create_collection(any_cal_core::Collection {
                id: tasks.clone(),
                name: "Tasks".into(),
            })
            .or_else(|e| match e {
                any_cal_core::RepositoryError::CollectionAlreadyExists(_) => Ok(()),
                other => Err(other),
            })?;
        Ok(Self {
            repository,
            contacts,
            tasks,
        })
    }

    pub fn new(repository: R) -> Self {
        Self::try_new(repository).expect("default DAV collections must be creatable")
    }
    /// Handle one parsed request and apply the protocol-level response
    /// contract shared by all DAV branches.  Individual handlers intentionally
    /// return only the status/body they know about; this outer seam supplies
    /// bounded diagnostics, cache policy, correlation, and Allow metadata so
    /// early-return error paths cannot silently diverge.
    pub fn handle(&mut self, request: Request) -> Response {
        let suppress_body = request.method.eq_ignore_ascii_case("HEAD");
        let request_path = request.path.clone();
        let request_id = request
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("x-request-id"))
            .and_then(|(_, value)| safe_request_id(value));
        let mut response = self.handle_inner(request);
        finalize_response(&mut response, request_id.as_deref(), Some(&request_path));
        // HEAD has the same status and representation metadata as GET but
        // never carries a response body, including on an error path.
        if suppress_body {
            response.body.clear();
        }
        response
    }

    fn handle_inner(&mut self, request: Request) -> Response {
        let origin = request_origin(&request);
        if matches!(request.method.as_str(), "GET" | "HEAD" | "PROPFIND") {
            if request.path == "/.well-known/caldav" {
                return response(
                    302,
                    vec![("Location", href(origin.as_deref(), "/caldav/"))],
                    vec![],
                );
            }
            if request.path == "/.well-known/carddav" {
                return response(
                    302,
                    vec![("Location", href(origin.as_deref(), "/carddav/"))],
                    vec![],
                );
            }
        }
        if request.method == "PROPFIND"
            && (request.path == "/" || request.path == "/principals/users/default")
        {
            let depth = header(&request, "depth").unwrap_or_else(|| "0".into());
            // Depth 1 is a compatibility no-op for the synthetic root and
            // principal: only that requested resource is returned. Clients
            // continue discovery through the well-known and home-set links.
            if depth != "0" && depth != "1" {
                return response(400, vec![], vec![]);
            }
            let Ok(props) = PropfindProps::parse(&request.body) else {
                return response(400, vec![], vec![]);
            };
            let path = if request.path == "/" {
                "/"
            } else {
                "/principals/users/default"
            };
            let response_href = href(origin.as_deref(), path);
            let mut body = String::new();
            if props.wants("current-user-principal") {
                body.push_str(&format!(
                    "<current-user-principal><href>{}</href></current-user-principal>",
                    xml_escape(&href(origin.as_deref(), "/principals/users/default"))
                ));
            }
            if props.wants("addressbook-home-set") {
                body.push_str(&format!("<addressbook-home-set xmlns=\"urn:ietf:params:xml:ns:carddav\"><d:href xmlns:d=\"DAV:\">{}</d:href></addressbook-home-set>", xml_escape(&href(origin.as_deref(), "/carddav/"))));
            }
            if props.wants("calendar-home-set") {
                body.push_str(&format!("<calendar-home-set xmlns=\"urn:ietf:params:xml:ns:caldav\"><d:href xmlns:d=\"DAV:\">{}</d:href></calendar-home-set>", xml_escape(&href(origin.as_deref(), "/caldav/"))));
            }
            let xml = format!("<multistatus xmlns=\"DAV:\"><response><href>{}</href><propstat><prop>{body}</prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>", xml_escape(&response_href));
            return response(
                207,
                vec![("Content-Type", "application/xml".into())],
                xml.into_bytes(),
            );
        }
        if request.method == "PROPFIND" && request.path == "/caldav/" {
            return self.caldav_home_propfind(
                &request.body,
                header(&request, "depth"),
                origin.as_deref(),
            );
        }
        let is_contact = request.path.starts_with("/carddav/");
        let is_task = request.path.starts_with("/caldav/");
        if request.method == "OPTIONS" {
            // Do not answer OPTIONS for an unknown namespace with the DAV
            // profile of whichever boolean happened to be false.  A client
            // must never infer CardDAV/CalDAV support from an unrelated path.
            if !is_contact && !is_task {
                return response(404, vec![], vec![]);
            }
            let (dav, accept) = if is_contact {
                ("1, 3, addressbook".into(), "text/vcard".into())
            } else if is_task {
                ("1, 3, calendar-access".into(), "text/calendar".into())
            } else {
                unreachable!("unknown OPTIONS path returned above")
            };
            return response(
                200,
                vec![
                    ("DAV", dav),
                    (
                        "Allow",
                        "OPTIONS, PROPFIND, REPORT, GET, HEAD, PUT, DELETE".into(),
                    ),
                    ("Accept", accept),
                    ("Content-Type", "application/xml; charset=utf-8".into()),
                    ("Cache-Control", "no-store".into()),
                ],
                vec![],
            );
        }
        if !(is_contact || is_task) {
            return response(404, vec![], vec![]);
        }
        let collection = if is_contact {
            self.contacts.clone()
        } else {
            self.tasks.clone()
        };
        let base = if is_contact { "/carddav/" } else { "/caldav/" };
        if request.method == "PROPFIND" {
            let depth = header(&request, "depth").unwrap_or_else(|| "0".into());
            let id = request
                .path
                .strip_prefix(base)
                .and_then(|p| p.strip_prefix(collection.as_str()))
                .and_then(|p| p.strip_prefix('/'))
                .filter(|p| !p.is_empty());
            return self.propfind(
                &collection,
                base,
                id,
                &depth,
                &request.body,
                is_contact,
                origin.as_deref(),
            );
        }
        let id = request
            .path
            .strip_prefix(base)
            .and_then(|p| p.strip_prefix(collection.as_str()))
            .and_then(|p| p.strip_prefix('/'))
            .filter(|p| !p.is_empty());
        if request.method == "REPORT" {
            let query = ReportQuery::parse(&request.body, is_contact);
            if query.is_err() {
                return response(400, vec![], vec![]);
            }
            return self.report(&collection, is_contact, query.unwrap(), origin.as_deref());
        }
        let Some(name) = id else {
            return response(405, vec![], vec![]);
        };
        let suffix = if is_contact { ".vcf" } else { ".ics" };
        if !name.ends_with(suffix) || name[..name.len() - suffix.len()].contains('/') {
            return response(400, vec![], vec![]);
        }
        let rid = match ResourceId::try_from(&name[..name.len() - suffix.len()]) {
            Ok(v) => v,
            Err(_) => return response(400, vec![], vec![]),
        };
        // This profile deliberately does not implement WebDAV locking.  An
        // `If` header is therefore never a harmless hint: accepting a
        // resource mutation while ignoring a lock-token condition could let a
        // client overwrite a resource it believes is locked.  Reject the
        // mutation before touching the repository and keep the boundary
        // explicit until durable lock-token support is implemented.
        if matches!(request.method.as_str(), "PUT" | "DELETE") && header(&request, "if").is_some() {
            return response(412, vec![], vec![]);
        }
        match request.method.as_str() {
            "GET" | "HEAD" => self.get(
                &rid,
                is_contact,
                header(&request, "if-match"),
                header(&request, "if-none-match"),
                header(&request, "if-modified-since"),
            ),
            "DELETE" => self.delete(
                &rid,
                &collection,
                is_contact,
                header(&request, "if-match"),
                header(&request, "if-none-match"),
                header(&request, "if-unmodified-since"),
            ),
            "PUT" => self.put(&rid, &collection, is_contact, request),
            _ => response(405, vec![], vec![]),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn propfind(
        &mut self,
        collection: &CollectionId,
        base: &str,
        id: Option<&str>,
        depth: &str,
        body: &[u8],
        contact: bool,
        origin: Option<&str>,
    ) -> Response {
        let Ok(props) = PropfindProps::parse(body) else {
            return response(400, vec![], vec![]);
        };
        let depth = match depth {
            "0" => 0,
            "1" => 1,
            _ => return response(400, vec![], vec![]),
        };
        let path = if let Some(name) = id {
            format!("{}{}/{}", base, collection, name)
        } else {
            format!("{}{}", base, collection)
        };
        let href = href(origin, &path);
        let mut xml = String::from("<multistatus xmlns=\"DAV:\">");
        if let Some(name) = id {
            let suffix = if contact { ".vcf" } else { ".ics" };
            if !name.ends_with(suffix) {
                return response(404, vec![], vec![]);
            }
            let rid = match ResourceId::try_from(&name[..name.len() - suffix.len()]) {
                Ok(v) => v,
                Err(_) => return response(404, vec![], vec![]),
            };
            let expected_kind = if contact {
                DavKind::Contact
            } else {
                DavKind::Task
            };
            let row = match self.repository.get_resource(&rid) {
                Ok(Some(row))
                    if !row.archived
                        && row.envelope.collection_id == *collection
                        && row.envelope.kind == expected_kind =>
                {
                    row
                }
                _ => return response(404, vec![], vec![]),
            };
            let properties = resource_properties(&row, contact, &props);
            let supported = properties
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            xml.push_str(&prop_response_for(
                &href,
                &properties,
                &props.unknown_requested(&supported),
                props.propname,
            ));
        } else {
            let collection_row = match self.repository.get_collection(collection) {
                Ok(Some(row)) => row,
                Ok(None) => return response(404, vec![], vec![]),
                Err(error) => return response(status_for(&error), vec![], vec![]),
            };
            let properties = collection_properties(&collection_row, contact, &props, origin);
            let supported = properties
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            xml.push_str(&prop_response_for(
                &href,
                &properties,
                &props.unknown_requested(&supported),
                props.propname,
            ));
            if depth == 1 {
                let rows = match self.repository.list_resources(collection, false) {
                    Ok(rows) => rows,
                    Err(error) => return response(status_for(&error), vec![], vec![]),
                };
                for row in rows {
                    let expected_kind = if contact {
                        DavKind::Contact
                    } else {
                        DavKind::Task
                    };
                    if row.envelope.kind != expected_kind {
                        continue;
                    }
                    let suffix = if contact { ".vcf" } else { ".ics" };
                    let child_href = format!("{}/{}{}", href, row.envelope.resource_id, suffix);
                    let properties = resource_properties(&row, contact, &props);
                    let supported = properties
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>();
                    xml.push_str(&prop_response_for(
                        &child_href,
                        &properties,
                        &props.unknown_requested(&supported),
                        props.propname,
                    ));
                }
            }
        }
        xml.push_str("</multistatus>");
        response(
            207,
            vec![("Content-Type", "application/xml".into())],
            xml.into_bytes(),
        )
    }
    fn caldav_home_propfind(
        &mut self,
        body: &[u8],
        depth: Option<String>,
        origin: Option<&str>,
    ) -> Response {
        let Ok(props) = PropfindProps::parse(body) else {
            return response(400, vec![], vec![]);
        };
        let depth = depth.unwrap_or_else(|| "0".into());
        if depth != "0" && depth != "1" {
            return response(400, vec![], vec![]);
        }
        let Ok(Some(collection)) = self.repository.get_collection(&self.tasks) else {
            return response(404, vec![], vec![]);
        };
        let mut properties = Vec::new();
        if props.wants("resourcetype") {
            properties.push(("resourcetype".into(), "<collection/>".into()));
        }
        if props.wants("displayname") {
            properties.push(("displayname".into(), "Calendars".into()));
        }
        if props.wants("current-user-principal") {
            properties.push((
                "current-user-principal".into(),
                format!(
                    "<href>{}</href>",
                    xml_escape(&href(origin, "/principals/users/default"))
                ),
            ));
        }
        if props.wants("calendar-home-set") {
            properties.push((
                "calendar-home-set".into(),
                format!(
                    "<d:href xmlns:d=\"DAV:\">{}</d:href>",
                    xml_escape(&href(origin, "/caldav/"))
                ),
            ));
        }
        let mut xml = String::from("<multistatus xmlns=\"DAV:\">");
        xml.push_str(&prop_response(&href(origin, "/caldav/"), &properties));
        if depth == "1" {
            xml.push_str(&prop_response(
                &href(origin, "/caldav/tasks"),
                &collection_properties(&collection, false, &props, origin),
            ));
        }
        xml.push_str("</multistatus>");
        response(
            207,
            vec![("Content-Type", "application/xml".into())],
            xml.into_bytes(),
        )
    }
    fn report(
        &mut self,
        collection: &CollectionId,
        contact: bool,
        query: ReportQuery,
        origin: Option<&str>,
    ) -> Response {
        let rows = match self.repository.list_resources(collection, false) {
            Ok(rows) => rows,
            Err(error) => return response(status_for(&error), vec![], vec![]),
        };
        let mut xml = String::from("<multistatus xmlns=\"DAV:\">");
        let mut matched_hrefs = std::collections::BTreeSet::new();
        for row in rows {
            let suffix = if contact { ".vcf" } else { ".ics" };
            let matching_hrefs: Vec<_> = query
                .hrefs
                .iter()
                .filter(|h| h.ends_with(&format!("{}{}", row.envelope.resource_id, suffix)))
                .collect();
            if !query.hrefs.is_empty() && matching_hrefs.is_empty() {
                continue;
            }
            let expected_kind = if contact {
                DavKind::Contact
            } else {
                DavKind::Task
            };
            if row.archived || row.envelope.kind != expected_kind || !query.matches(&row, contact) {
                continue;
            }
            for href in matching_hrefs {
                matched_hrefs.insert(href.clone());
            }
            let body = resource_body(&row, contact);
            let data_name = if contact {
                "address-data"
            } else {
                "calendar-data"
            };
            let want_data =
                query.properties.is_empty() || query.properties.iter().any(|p| p == data_name);
            let want_etag =
                query.properties.is_empty() || query.properties.iter().any(|p| p == "getetag");
            let want_last_modified = query.properties.is_empty()
                || query.properties.iter().any(|p| p == "getlastmodified");
            let want_content_type = query.properties.is_empty()
                || query.properties.iter().any(|p| p == "getcontenttype");
            let want_content_length = query.properties.is_empty()
                || query.properties.iter().any(|p| p == "getcontentlength");
            let mut properties = String::new();
            if want_etag {
                properties.push_str(&format!("<getetag>{}</getetag>", row.etag.as_str()));
            }
            if want_last_modified {
                properties.push_str(&format!(
                    "<getlastmodified>{}</getlastmodified>",
                    last_modified(&row)
                ));
            }
            if want_content_type {
                properties.push_str(&format!(
                    "<getcontenttype>{}</getcontenttype>",
                    if contact {
                        "text/vcard"
                    } else {
                        "text/calendar"
                    }
                ));
            }
            if want_content_length {
                properties.push_str(&format!(
                    "<getcontentlength>{}</getcontentlength>",
                    body.len()
                ));
            }
            if want_data {
                properties.push_str(&format!(
                    "<{} content-type=\"{}\">{}</{}>",
                    data_name,
                    if contact {
                        "text/vcard"
                    } else {
                        "text/calendar"
                    },
                    xml_escape(&body),
                    data_name
                ));
            }
            let known = [
                "getetag",
                "getlastmodified",
                "getcontenttype",
                "getcontentlength",
                data_name,
            ];
            let unknown = query
                .properties
                .iter()
                .filter(|name| !known.iter().any(|candidate| candidate == name))
                .map(|name| format!("<{} />", xml_escape(name)))
                .collect::<String>();
            let path = format!(
                "{}/{}{}",
                if contact {
                    "/carddav/contacts"
                } else {
                    "/caldav/tasks"
                },
                row.envelope.resource_id,
                suffix
            );
            let mut response_xml = format!(
                "<response><href>{}</href>",
                xml_escape(&href(origin, &path))
            );
            if !properties.is_empty() {
                response_xml.push_str(&format!(
                    "<propstat><prop>{properties}</prop><status>HTTP/1.1 200 OK</status></propstat>"
                ));
            }
            if !unknown.is_empty() {
                response_xml.push_str(&format!(
                    "<propstat><prop>{unknown}</prop><status>HTTP/1.1 404 Not Found</status></propstat>"
                ));
            }
            response_xml.push_str("</response>");
            xml.push_str(&response_xml);
        }
        for href in &query.hrefs {
            if !matched_hrefs.contains(href) {
                xml.push_str(&format!("<d:response xmlns:d=\"DAV:\"><d:href>{}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>", xml_escape(href)));
            }
        }
        xml.push_str("</multistatus>");
        response(
            207,
            vec![("Content-Type", "application/xml".into())],
            xml.into_bytes(),
        )
    }
    fn get(
        &mut self,
        id: &ResourceId,
        contact: bool,
        if_match: Option<String>,
        if_none_match: Option<String>,
        if_modified_since: Option<String>,
    ) -> Response {
        let row = match self.repository.get_resource(id) {
            Ok(Some(row)) => row,
            Ok(None) => return response(404, vec![], vec![]),
            Err(error) => return response(status_for(&error), vec![], vec![]),
        };
        if row.archived
            || row.envelope.collection_id
                != if contact {
                    self.contacts.clone()
                } else {
                    self.tasks.clone()
                }
            || row.envelope.kind
                != if contact {
                    DavKind::Contact
                } else {
                    DavKind::Task
                }
        {
            return response(404, vec![], vec![]);
        }
        // RFC 9110 evaluates If-Match before If-None-Match.  A matching
        // If-None-Match on a safe retrieval produces 304; a failed If-Match
        // is a 412 and must not expose the representation.
        if let Some(value) = if_match {
            let matches = value
                .split(',')
                .any(|candidate| candidate.trim() == "*" || candidate.trim() == row.etag.as_str());
            if !matches {
                return response(412, vec![], vec![]);
            }
        }
        if let Some(value) = if_none_match {
            let matches = value
                .split(',')
                .any(|candidate| candidate.trim() == "*" || candidate.trim() == row.etag.as_str());
            if matches {
                return response(
                    304,
                    vec![
                        ("ETag", row.etag.as_str().into()),
                        ("Last-Modified", last_modified(&row)),
                    ],
                    vec![],
                );
            }
        } else if if_modified_since
            .as_deref()
            .and_then(http_date::parse)
            .is_some_and(|date| row.modified_at.unix_seconds() <= date)
        {
            return response(
                304,
                vec![
                    ("ETag", row.etag.as_str().into()),
                    ("Last-Modified", last_modified(&row)),
                ],
                vec![],
            );
        }
        let body = resource_body(&row, contact);
        response(
            200,
            vec![
                (
                    "Content-Type",
                    if contact {
                        "text/vcard"
                    } else {
                        "text/calendar"
                    }
                    .into(),
                ),
                ("ETag", row.etag.as_str().into()),
                ("Last-Modified", last_modified(&row)),
            ],
            body.into_bytes(),
        )
    }
    fn delete(
        &mut self,
        id: &ResourceId,
        collection: &CollectionId,
        contact: bool,
        if_match: Option<String>,
        if_none_match: Option<String>,
        if_unmodified_since: Option<String>,
    ) -> Response {
        let row = match self.repository.get_resource(id) {
            Err(error) => return response(status_for(&error), vec![], vec![]),
            Ok(Some(row)) => {
                if row.archived
                    || row.envelope.collection_id != *collection
                    || row.envelope.kind
                        != if contact {
                            DavKind::Contact
                        } else {
                            DavKind::Task
                        }
                {
                    return response(404, vec![], vec![]);
                }
                row
            }
            Ok(None) => return response(404, vec![], vec![]),
        };
        if if_none_match.as_deref().is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == "*" || candidate.trim() == row.etag.as_str())
        }) {
            return response(412, vec![], vec![]);
        }
        // If-Match takes precedence over If-Unmodified-Since.  Invalid dates
        // are ignored by HTTP semantics; valid stale dates fail before any
        // repository mutation is attempted.
        if if_match.is_none()
            && if_unmodified_since
                .as_deref()
                .and_then(http_date::parse)
                .is_some_and(|date| row.modified_at.unix_seconds() > date)
        {
            return response(412, vec![], vec![]);
        }
        let condition = match if_match {
            // The resource existence check above establishes that `*` in
            // If-Match matches the current representation.  Mapping it to
            // IfNoneMatch would reject every existing resource (and is the
            // opposite of HTTP conditional semantics).
            Some(x) if x.trim() == "*" => Some(WriteCondition::Unconditional),
            Some(x) => match self.condition_for(id, &x) {
                Ok(value) => Some(value),
                Err(error) => return response(status_for(&error), vec![], vec![]),
            },
            None => None,
        };
        match self
            .repository
            .archive_resource(id, condition.unwrap_or(WriteCondition::Unconditional))
        {
            Ok(_) => response(204, vec![], vec![]),
            Err(error) => response(status_for(&error), vec![], vec![]),
        }
    }

    fn condition_for(
        &mut self,
        id: &ResourceId,
        supplied: &str,
    ) -> Result<WriteCondition, any_cal_core::RepositoryError> {
        match self.repository.get_resource(id)? {
            Some(row)
                if supplied
                    .split(',')
                    .any(|candidate| candidate.trim() == row.etag.as_str()) =>
            {
                Ok(WriteCondition::IfMatch(row.etag))
            }
            _ => Ok(WriteCondition::IfMatch(any_cal_core::typed_etag_for_bytes(
                b"mismatch",
            ))),
        }
    }
    fn put(
        &mut self,
        id: &ResourceId,
        collection: &CollectionId,
        contact: bool,
        request: Request,
    ) -> Response {
        let expected = if contact {
            "text/vcard"
        } else {
            "text/calendar"
        };
        let valid_type = header(&request, "content-type").is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case(expected))
        });
        if !valid_type {
            return response(415, vec![], vec![]);
        }
        let text = String::from_utf8(request.body.clone()).ok();
        let Some(text) = text else {
            return response(400, vec![], vec![]);
        };
        let (doc, opaque_calendar) = if contact {
            match vcard::parse(&text) {
                Ok(value) => (value.fields, None),
                Err(_) => return response(400, vec![], vec![]),
            }
        } else {
            let calendar = match Calendar::parse(&text) {
                Ok(value) => value,
                Err(_) => return response(400, vec![], vec![]),
            };
            let todo = match vtodo_from_calendar(&calendar) {
                Some(value) => value,
                None => return response(400, vec![], vec![]),
            };
            (todo.fields, Some(calendar))
        };
        if doc.fields.is_empty() {
            return response(400, vec![], vec![]);
        }
        let uid = doc
            .fields
            .get("UID")
            .and_then(|v| v.first())
            .map(|x| x.value.clone())
            .unwrap_or_else(|| id.to_string());
        let Ok(dav_uid) = DavUid::try_from(uid) else {
            return response(400, vec![], vec![]);
        };
        let existing = match self.repository.get_resource(id) {
            Ok(row) => row,
            Err(error) => return response(status_for(&error), vec![], vec![]),
        };
        if let Some(row) = &existing {
            // Anytype assigns the remote object ID on create.  A DAV client
            // continues to address the resource by its stable UID, so an
            // update must not require those two independent identities to be
            // equal.  Only a UID change is an identity conflict.
            if row.envelope.dav_uid != dav_uid {
                return response(409, vec![], vec![]);
            }
        }
        // New DAV resources have no server-generated Anytype ID yet; using
        // the UID as a provisional value lets the repository validate and
        // deduplicate the write.  Existing resources retain the ID returned
        // by Anytype so the adapter can issue PATCH/archive/delete against
        // the correct remote object.
        let anytype_object_id = existing
            .as_ref()
            .map(|row| row.envelope.anytype_object_id.clone())
            .or_else(|| AnytypeObjectId::try_from(dav_uid.to_string()).ok());
        let Some(anytype_object_id) = anytype_object_id else {
            return response(400, vec![], vec![]);
        };
        let env = ResourceEnvelope {
            collection_id: collection.clone(),
            resource_id: id.clone(),
            kind: if contact {
                DavKind::Contact
            } else {
                DavKind::Task
            },
            anytype_object_id,
            dav_uid,
            document: CanonicalDocument {
                version: 1,
                content: doc,
                opaque_calendar,
            },
            revision: 0,
        };
        let updating = existing.is_some();
        if header(&request, "if-none-match").is_some_and(|value| value.trim() == "*") && updating {
            return response(412, vec![], vec![]);
        }
        // If-Match is the stronger validator.  Otherwise a valid
        // If-Unmodified-Since date guards an overwrite; malformed dates are
        // ignored.  Evaluate this before parsing/applying the repository
        // write condition so a rejected request cannot mutate state.
        if header(&request, "if-match").is_none()
            && existing.as_ref().is_some_and(|row| {
                header(&request, "if-unmodified-since")
                    .as_deref()
                    .and_then(http_date::parse)
                    .is_some_and(|date| row.modified_at.unix_seconds() > date)
            })
        {
            return response(412, vec![], vec![]);
        }
        let result = if let Some(value) = header(&request, "if-match") {
            let condition = if value.trim() == "*" {
                // `If-Match: *` succeeds when the resource exists.  The
                // existing-resource lookup above establishes that fact.
                Ok(WriteCondition::IfMatch(
                    existing
                        .as_ref()
                        .expect("updating implies existing")
                        .etag
                        .clone(),
                ))
            } else {
                self.condition_for(id, &value)
            };
            match condition {
                Ok(condition) => self.repository.update_resource(env, condition),
                Err(error) => return response(status_for(&error), vec![], vec![]),
            }
        } else if updating {
            self.repository
                .update_resource(env, WriteCondition::Unconditional)
        } else {
            self.repository
                .create_resource(env, WriteCondition::IfNoneMatch)
        };
        match result {
            Ok(row) => response(
                if updating { 204 } else { 201 },
                {
                    let mut headers = vec![
                        ("ETag", row.etag.as_str().into()),
                        ("Last-Modified", last_modified(&row)),
                        (
                            "Content-Type",
                            if contact {
                                "text/vcard"
                            } else {
                                "text/calendar"
                            }
                            .into(),
                        ),
                    ];
                    if !updating {
                        headers.push((
                            "Location",
                            format!(
                                "/{}/{}/{}{}",
                                if contact { "carddav" } else { "caldav" },
                                collection,
                                id,
                                if contact { ".vcf" } else { ".ics" }
                            ),
                        ));
                    }
                    headers
                },
                if updating { vec![] } else { request.body },
            ),
            Err(error) => response(status_for(&error), vec![], vec![]),
        }
    }
}

fn resource_body(row: &StoredResource, contact: bool) -> String {
    if !contact {
        if let Some(calendar) = &row.envelope.document.opaque_calendar {
            return calendar.serialize();
        }
    }
    if contact {
        vcard::serialize(&vcard::Contact {
            fields: row.envelope.document.content.clone(),
            metadata: StructuredDocument::default(),
        })
    } else {
        vtodo::serialize(&vtodo::VTodo {
            fields: row.envelope.document.content.clone(),
            calendar_metadata: StructuredDocument::default(),
        })
    }
}

fn last_modified(row: &StoredResource) -> String {
    // Repository writes always use a representable UTC second.  The fallback
    // is only for legacy rows decoded with the serde default.
    http_date::format(row.modified_at.unix_seconds())
        .unwrap_or_else(|| "Thu, 01 Jan 1970 00:00:00 GMT".into())
}

/// Build the editable VTODO property projection while retaining the complete
/// parsed calendar separately in the repository envelope.  Direct
/// VCALENDAR properties become metadata and only the first VTODO contributes
/// editable fields; sibling VEVENT/VTIMEZONE/unknown components are opaque.
fn vtodo_from_calendar(calendar: &Calendar) -> Option<vtodo::VTodo> {
    let mut metadata: BTreeMap<String, Vec<any_cal_core::Occurrence>> = BTreeMap::new();
    let mut fields: BTreeMap<String, Vec<any_cal_core::Occurrence>> = BTreeMap::new();
    let mut todo_found = false;
    for entry in &calendar.root().entries {
        match entry {
            Entry::Property(property) => {
                metadata
                    .entry(property.name.clone())
                    .or_default()
                    .push(any_cal_core::Occurrence {
                        value: property.value.clone(),
                        params: property.params.clone(),
                    });
            }
            Entry::Component(component) if component.name == "VTODO" && !todo_found => {
                todo_found = true;
                for child in &component.entries {
                    let Entry::Property(property) = child else {
                        continue;
                    };
                    fields.entry(property.name.clone()).or_default().push(
                        any_cal_core::Occurrence {
                            value: property.value.clone(),
                            params: property.params.clone(),
                        },
                    );
                }
            }
            Entry::Component(_) => {}
        }
    }
    todo_found.then_some(vtodo::VTodo {
        fields: StructuredDocument { fields },
        calendar_metadata: StructuredDocument { fields: metadata },
    })
}

fn header(r: &Request, key: &str) -> Option<String> {
    r.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.clone())
}

/// Build an absolute origin only from a simple, validated Host and proxy
/// scheme. Hostless requests retain relative hrefs for local protocol tests.
fn request_origin(request: &Request) -> Option<String> {
    let host = header(request, "host")?;
    let host = host.trim();
    if host.is_empty()
        || host.chars().any(|c| c.is_control() || c.is_whitespace())
        || host.contains(['/', '?', '#', '@', '\\'])
        || !valid_host(host)
    {
        return None;
    }
    let scheme = header(request, "x-forwarded-proto")
        .and_then(|value| {
            value
                .split(',')
                .next()
                .map(str::trim)
                .map(str::to_ascii_lowercase)
        })
        .filter(|value| value == "http" || value == "https")
        .unwrap_or_else(|| "http".into());
    Some(format!("{scheme}://{host}"))
}

fn valid_host(host: &str) -> bool {
    let (name, port) = if let Some(rest) = host.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return false;
        };
        let (address, suffix) = rest.split_at(end);
        if address.is_empty()
            || !address
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        {
            return false;
        }
        let port = suffix
            .strip_prefix(']')
            .map(|value| value.strip_prefix(':'));
        (address, port.flatten())
    } else if let Some((name, port)) = host.rsplit_once(':') {
        if name.is_empty() || name.contains(':') {
            return false;
        }
        (name, Some(port))
    } else {
        (host, None)
    };
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return false;
    }
    port.is_none_or(|value| !value.is_empty() && value.parse::<u16>().is_ok())
}

fn href(origin: Option<&str>, path: &str) -> String {
    origin.map_or_else(|| path.into(), |origin| format!("{origin}{path}"))
}

fn connection_requests_close(value: String) -> bool {
    value
        .split(',')
        .any(|token| token.trim().eq_ignore_ascii_case("close"))
}

struct ReportQuery {
    hrefs: Vec<String>,
    properties: Vec<String>,
    filters: Vec<PropertyFilter>,
    components: Vec<String>,
}
#[derive(Clone, Debug)]
struct PropertyFilter {
    name: String,
    text_match: Option<String>,
    time_start: Option<String>,
    time_end: Option<String>,
}
impl ReportQuery {
    fn parse(body: &[u8], contact: bool) -> Result<Self, ()> {
        // DAV clients conventionally qualify elements (for example
        // `c:calendar-query` and `d:prop`).  The protocol is namespace based,
        // so the prefix is not semantically significant.  Normalize element
        // prefixes before applying the deliberately small bounded parser;
        // attribute values, including namespace declarations, are untouched.
        let text = normalize_xml_names(&String::from_utf8(body.to_vec()).map_err(|_| ())?);
        if text.is_empty() {
            return Ok(Self {
                hrefs: vec![],
                properties: vec![],
                filters: vec![],
                components: vec![],
            });
        }
        let names = if contact {
            ["addressbook-query", "addressbook-multiget"]
        } else {
            ["calendar-query", "calendar-multiget"]
        };
        if !names.iter().any(|n| text.contains(&format!("<{n}"))) {
            return Err(());
        }
        let tokens = tokenize_xml(&text)?;
        if tokens
            .iter()
            .filter(|t| t.name == names[0] || t.name == names[1])
            .count()
            == 0
        {
            return Err(());
        }
        let mut hrefs = Vec::new();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("<href>") {
            let tail = &rest[start + 6..];
            let end = tail.find("</href>").ok_or(())?;
            hrefs.push(xml_unescape(&tail[..end]));
            rest = &tail[end + 7..];
        }
        let properties = extract_tag_texts(&text, "prop")
            .into_iter()
            .flat_map(|body| extract_tag_names(&body))
            .collect();
        let filters = extract_filters(&text)?;
        if contact
            && filters
                .iter()
                .any(|filter| filter.time_start.is_some() || filter.time_end.is_some())
        {
            // CardDAV property-filter has no time-range form. Reject it
            // rather than silently broadening or narrowing the addressbook.
            return Err(());
        }
        // An addressbook-query may omit its filter entirely, but an explicit
        // empty filter is not a meaningful CardDAV addressbook filter.  Keep
        // this bounded parser fail-closed instead of silently broadening the
        // query to every contact.
        if contact && text.contains("<filter") && !text.contains("<prop-filter") {
            return Err(());
        }
        let components = extract_component_filters(&text)?;
        Ok(Self {
            hrefs,
            properties,
            filters,
            components,
        })
    }

    fn matches(&self, row: &any_cal_core::StoredResource, contact: bool) -> bool {
        let component_ok = self.components.iter().all(|component| {
            if contact {
                return false;
            }
            let Some(calendar) = row.envelope.document.opaque_calendar.as_ref() else {
                return component == "VTODO";
            };
            fn contains(component: &any_cal_core::ical::Component, wanted: &str) -> bool {
                component.name == wanted
                    || component.entries.iter().any(|entry| match entry {
                        any_cal_core::ical::Entry::Component(child) => contains(child, wanted),
                        any_cal_core::ical::Entry::Property(_) => false,
                    })
            }
            contains(calendar.root(), component)
        });
        if !component_ok {
            return false;
        }
        self.filters.iter().all(|filter| {
            if contact && filter.name == "UID" || !contact && filter.name == "UID" {
                return row
                    .envelope
                    .dav_uid
                    .as_str()
                    .eq_ignore_ascii_case(&filter.text_match.clone().unwrap_or_default())
                    || filter.text_match.is_none();
            }
            let values = row
                .envelope
                .document
                .content
                .fields
                .get(&filter.name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let text_ok = filter.text_match.as_ref().is_none_or(|needle| {
                values.iter().any(|v| {
                    v.value
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
                })
            });
            let start_ok = filter.time_start.as_ref().is_none_or(|start| {
                values
                    .iter()
                    .any(|v| normalize_time_value(&v.value).is_some_and(|value| value >= *start))
            });
            let end_ok = filter
                .time_end
                .as_ref()
                // RFC 4791 time-range end bounds are exclusive.
                .is_none_or(|end| {
                    values
                        .iter()
                        .any(|v| normalize_time_value(&v.value).is_some_and(|value| value < *end))
                });
            text_ok && start_ok && end_ok
        })
    }
}

/// Normalize basic or extended iCalendar date and date-time values to a
/// sortable wall-clock key. UTC and floating/TZID values are deliberately
/// compared without timezone conversion to keep this adapter lossless.
fn normalize_time_value(value: &str) -> Option<String> {
    let value = value.trim();
    let compact = if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{value}000000")
    } else if value.len() == 15
        && value.as_bytes()[8] == b'T'
        && value[..8].bytes().all(|byte| byte.is_ascii_digit())
        && value[9..].bytes().all(|byte| byte.is_ascii_digit())
    {
        value.replace('T', "")
    } else if value.len() == 16
        && value.ends_with('Z')
        && value.as_bytes()[8] == b'T'
        && value[..8].bytes().all(|byte| byte.is_ascii_digit())
        && value[9..15].bytes().all(|byte| byte.is_ascii_digit())
    {
        value[..15].replace('T', "")
    } else if value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value[..4].bytes().all(|byte| byte.is_ascii_digit())
        && value[5..7].bytes().all(|byte| byte.is_ascii_digit())
        && value[8..].bytes().all(|byte| byte.is_ascii_digit())
    {
        format!("{}{}{}000000", &value[..4], &value[5..7], &value[8..])
    } else if value.len() == 19
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value.as_bytes()[10] == b'T'
        && value.as_bytes()[13] == b':'
        && value.as_bytes()[16] == b':'
        && value[..4].bytes().all(|byte| byte.is_ascii_digit())
        && value[5..7].bytes().all(|byte| byte.is_ascii_digit())
        && value[8..10].bytes().all(|byte| byte.is_ascii_digit())
        && value[11..13].bytes().all(|byte| byte.is_ascii_digit())
        && value[14..16].bytes().all(|byte| byte.is_ascii_digit())
        && value[17..].bytes().all(|byte| byte.is_ascii_digit())
    {
        format!(
            "{}{}{}{}{}{}",
            &value[..4],
            &value[5..7],
            &value[8..10],
            &value[11..13],
            &value[14..16],
            &value[17..]
        )
    } else if value.len() == 20 && value.ends_with('Z') {
        normalize_time_value(&value[..19])?
    } else {
        return None;
    };
    let year = compact[..4].parse::<u16>().ok()?;
    let month = compact[4..6].parse::<u8>().ok()?;
    let day = compact[6..8].parse::<u8>().ok()?;
    let hour = compact[8..10].parse::<u8>().ok()?;
    let minute = compact[10..12].parse::<u8>().ok()?;
    let second = compact[12..14].parse::<u8>().ok()?;
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return None,
    };
    (day > 0 && day <= month_days && hour < 24 && minute < 60 && second < 60).then_some(compact)
}
fn normalize_xml_names(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let end = start + end;
        let tag = &rest[start..=end];
        if tag.starts_with("<!--") || tag.starts_with("<?") || tag.starts_with("<!") {
            output.push_str(tag);
        } else {
            let body = &tag[1..tag.len() - 1];
            let (prefix, trimmed) = if let Some(stripped) = body.strip_prefix('/') {
                ("/", stripped.trim_start())
            } else {
                ("", body.trim_start())
            };
            let token_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
            let token = &trimmed[..token_end];
            let local = token.rsplit_once(':').map_or(token, |(_, local)| local);
            output.push('<');
            output.push_str(prefix);
            output.push_str(local);
            output.push_str(&trimmed[token_end..]);
            output.push('>');
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

fn extract_component_filters(input: &str) -> Result<Vec<String>, ()> {
    let mut out = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("<comp-filter") {
        let tail = &rest[start..];
        let end = tail.find('>').ok_or(())?;
        let head = &tail[..end];
        let name = head
            .split("name=\"")
            .nth(1)
            .and_then(|value| value.split('"').next())
            .ok_or(())?
            .to_ascii_uppercase();
        out.push(name);
        if head.trim_end().ends_with('/') {
            rest = &tail[end + 1..];
        } else if let Some(close) = tail[end + 1..].find("</comp-filter>") {
            rest = &tail[end + 1 + close + "</comp-filter>".len()..];
        } else {
            return Err(());
        }
    }
    Ok(out)
}
#[derive(Clone, Debug)]
struct XmlToken {
    name: String,
}
fn tokenize_xml(input: &str) -> Result<Vec<XmlToken>, ()> {
    let mut out = Vec::new();
    let mut rest = input;
    let mut stack = Vec::new();
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let end = after.find('>').ok_or(())?;
        let raw = after[..end].trim();
        rest = &after[end + 1..];
        if raw.starts_with('?') || raw.starts_with('!') {
            continue;
        }
        let closing = raw.starts_with('/');
        let mut body = raw.trim_start_matches('/').trim();
        let self_closing = body.ends_with('/');
        body = body.trim_end_matches('/').trim();
        let mut parts = body.split_whitespace();
        let name = parts
            .next()
            .ok_or(())?
            .split(':')
            .next_back()
            .unwrap()
            .to_ascii_lowercase();
        if closing {
            if stack.pop() != Some(name.clone()) {
                return Err(());
            }
        } else if !self_closing {
            stack.push(name.clone());
        }
        out.push(XmlToken { name });
    }
    if !stack.is_empty() {
        return Err(());
    }
    Ok(out)
}
fn extract_tag_texts(input: &str, wanted: &str) -> Vec<String> {
    let mut out = Vec::new();
    let open = format!("<{wanted}");
    let close = format!("</{wanted}>");
    let mut rest = input;
    while let Some(s) = rest.find(&open) {
        let a = &rest[s..];
        let Some(gt) = a.find('>') else { break };
        let Some(e) = a[gt + 1..].find(&close) else {
            break;
        };
        out.push(a[gt + 1..gt + 1 + e].to_string());
        rest = &a[gt + 1 + e + close.len()..];
    }
    out
}
fn extract_tag_names(input: &str) -> Vec<String> {
    input
        .split('<')
        .skip(1)
        .filter_map(|x| x.split(['>', ' ', '/']).next())
        .map(|x| x.split(':').next_back().unwrap().to_ascii_lowercase())
        .filter(|x| !x.is_empty())
        .collect()
}
fn extract_filters(input: &str) -> Result<Vec<PropertyFilter>, ()> {
    let mut out = Vec::new();
    let mut rest = input;
    while let Some(s) = rest.find("<prop-filter") {
        let a = &rest[s..];
        let gt = a.find('>').ok_or(())?;
        let head = &a[..gt];
        let name = head
            .split("name=\"")
            .nth(1)
            .and_then(|x| x.split('"').next())
            .ok_or(())?
            .to_ascii_uppercase();
        let (body, consumed) = if head.trim_end().ends_with('/') {
            ("", gt + 1)
        } else {
            let end = a[gt + 1..]
                .find("</prop-filter>")
                .map(|offset| gt + 1 + offset)
                .ok_or(())?;
            (&a[gt + 1..end], end + "</prop-filter>".len())
        };
        let text_match = extract_tag_texts(body, "text-match")
            .first()
            .map(|value| xml_unescape(value));
        let time_start = body
            .split("start=\"")
            .nth(1)
            .and_then(|x| x.split('"').next())
            .map(|raw| normalize_time_value(raw).ok_or(()))
            .transpose()?;
        let time_end = body
            .split("end=\"")
            .nth(1)
            .and_then(|x| x.split('"').next())
            .map(|raw| normalize_time_value(raw).ok_or(()))
            .transpose()?;
        if let (Some(start), Some(end)) = (&time_start, &time_end) {
            if start >= end {
                return Err(());
            }
        }
        out.push(PropertyFilter {
            name,
            text_match,
            time_start,
            time_end,
        });
        rest = &a[consumed..];
    }
    Ok(out)
}
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

struct PropfindProps {
    names: Option<Vec<String>>,
    propname: bool,
}
impl PropfindProps {
    fn parse(body: &[u8]) -> Result<Self, ()> {
        if body.is_empty() {
            return Ok(Self {
                names: None,
                propname: false,
            });
        }
        // DAV property names are identified by namespace, not by the prefix
        // chosen by the client.  Normalize prefixes before the small bounded
        // parser inspects the request.
        let text = normalize_xml_names(&String::from_utf8(body.to_vec()).map_err(|_| ())?);
        let tokens = tokenize_xml(&text)?;
        if tokens.is_empty() {
            return Err(());
        }
        if text.to_ascii_lowercase().contains("<allprop") {
            return Ok(Self {
                names: None,
                propname: false,
            });
        }
        if text.to_ascii_lowercase().contains("<propname") {
            return Ok(Self {
                names: None,
                propname: true,
            });
        }
        let names = extract_tag_texts(&text, "prop")
            .first()
            .map(|b| extract_tag_names(b))
            .ok_or(())?;
        Ok(Self {
            names: Some(names),
            propname: false,
        })
    }
    fn wants(&self, name: &str) -> bool {
        if self.propname {
            return true;
        }
        self.names.as_ref().is_none_or(|names| {
            names
                .iter()
                .any(|n| local_xml_name(n) == local_xml_name(name))
        })
    }
    fn unknown_requested(&self, supported: &[String]) -> Vec<String> {
        let Some(names) = &self.names else {
            return Vec::new();
        };
        names
            .iter()
            .filter(|name| {
                !supported
                    .iter()
                    .any(|candidate| local_xml_name(candidate) == local_xml_name(name))
            })
            .cloned()
            .collect()
    }
}
fn local_xml_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}
fn prop_response_for(
    href: &str,
    properties: &[(String, String)],
    unknown: &[String],
    propname: bool,
) -> String {
    let body = properties
        .iter()
        .map(|(name, value)| {
            if propname {
                format!("<{name}/>")
            } else {
                format!("<{name}>{value}</{name}>")
            }
        })
        .collect::<String>();
    let mut response = format!("<response><href>{}</href>", xml_escape(href));
    if !body.is_empty() {
        response.push_str(&format!(
            "<propstat><prop>{body}</prop><status>HTTP/1.1 200 OK</status></propstat>"
        ));
    }
    if !unknown.is_empty() {
        let unknown_body = unknown
            .iter()
            .map(|name| format!("<{name}/>"))
            .collect::<String>();
        response.push_str(&format!(
            "<propstat><prop>{unknown_body}</prop><status>HTTP/1.1 404 Not Found</status></propstat>"
        ));
    }
    response.push_str("</response>");
    response
}
fn prop_response(href: &str, properties: &[(String, String)]) -> String {
    prop_response_for(href, properties, &[], false)
}
fn collection_properties(
    collection: &any_cal_core::Collection,
    contact: bool,
    props: &PropfindProps,
    origin: Option<&str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let add =
        |out: &mut Vec<(String, String)>, props: &PropfindProps, name: &str, value: String| {
            if props.wants(name) {
                out.push((name.into(), value));
            }
        };
    add(
        &mut out,
        props,
        "resourcetype",
        if contact {
            "<collection/><addressbook xmlns=\"urn:ietf:params:xml:ns:carddav\"/>"
        } else {
            "<collection/><calendar xmlns=\"urn:ietf:params:xml:ns:caldav\"/>"
        }
        .into(),
    );
    add(&mut out, props, "displayname", xml_escape(&collection.name));
    add(
        &mut out,
        props,
        "current-user-principal",
        format!(
            "<d:href xmlns:d=\"DAV:\">{}</d:href>",
            xml_escape(&href(origin, "/principals/users/default"))
        ),
    );
    add(
        &mut out,
        props,
        if contact {
            "addressbook-home-set"
        } else {
            "calendar-home-set"
        },
        format!(
            "<d:href xmlns:d=\"DAV:\">{}</d:href>",
            xml_escape(&href(
                origin,
                if contact { "/carddav/" } else { "/caldav/" }
            ))
        ),
    );
    if contact {
        add(&mut out, props, "d:supported-report-set", "<d:supported-report xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:report><c:addressbook-query/></d:report></d:supported-report><d:supported-report xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:report><c:addressbook-multiget/></d:report></d:supported-report>".into());
    } else {
        add(
            &mut out,
            props,
            "supported-calendar-component-set",
            "<comp name=\"VTODO\"/>".into(),
        );
        add(&mut out, props, "d:supported-report-set", "<d:supported-report xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:report><c:calendar-query/></d:report></d:supported-report><d:supported-report xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:report><c:calendar-multiget/></d:report></d:supported-report>".into());
    }
    out
}
fn resource_properties(
    row: &any_cal_core::StoredResource,
    contact: bool,
    props: &PropfindProps,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let add =
        |out: &mut Vec<(String, String)>, props: &PropfindProps, name: &str, value: String| {
            if props.wants(name) {
                out.push((name.into(), value));
            }
        };
    add(&mut out, props, "getetag", row.etag.as_str().into());
    add(
        &mut out,
        props,
        "getcontenttype",
        if contact {
            "text/vcard"
        } else {
            "text/calendar"
        }
        .into(),
    );
    let body = if contact {
        vcard::serialize(&vcard::Contact {
            fields: row.envelope.document.content.clone(),
            metadata: StructuredDocument::default(),
        })
    } else {
        vtodo::serialize(&vtodo::VTodo {
            fields: row.envelope.document.content.clone(),
            calendar_metadata: StructuredDocument::default(),
        })
    };
    add(&mut out, props, "getcontentlength", body.len().to_string());
    out
}
fn status_for(error: &any_cal_core::RepositoryError) -> u16 {
    use any_cal_core::RepositoryError::*;
    match error {
        ResourceNotFound(_) | CollectionNotFound(_) => 404,
        PreconditionFailed { .. } => 412,
        ResourceAlreadyExists(_) | IdentityAlreadyExists(_) => 409,
        InvalidEnvelope(_) => 400,
        Auth => 401,
        Forbidden => 403,
        RateLimited => 429,
        // Preserve the retryable timeout category at the HTTP boundary so
        // the service health/recovery layer can distinguish it from an
        // unavailable backend.
        Timeout => 408,
        Unavailable => 503,
        _ => 500,
    }
}
fn response(status: u16, headers: Vec<(&str, String)>, body: Vec<u8>) -> Response {
    Response {
        status,
        headers: headers.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        body,
    }
}

fn safe_request_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 128 {
        return None;
    }
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
        .then(|| value.to_owned())
}

fn header_present(headers: &[(String, String)], key: &str) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(key))
}

fn generic_error_body(status: u16) -> &'static [u8] {
    match status {
        400 => b"bad request",
        401 => b"unauthorized",
        403 => b"forbidden",
        404 => b"not found",
        405 => b"method not allowed",
        408 => b"request timeout",
        409 => b"conflict",
        412 => b"precondition failed",
        415 => b"unsupported media type",
        429 => b"rate limited",
        503 => b"service unavailable",
        _ => b"internal server error",
    }
}

fn allow_for_path(path: Option<&str>) -> &'static str {
    match path {
        Some(path) if path.starts_with("/carddav/") || path.starts_with("/caldav/") => {
            "OPTIONS, PROPFIND, REPORT, GET, HEAD, PUT, DELETE"
        }
        Some("/") | Some("/principals/users/default") => "PROPFIND",
        _ => "OPTIONS, PROPFIND",
    }
}

fn finalize_response(response: &mut Response, request_id: Option<&str>, path: Option<&str>) {
    if let Some(request_id) = request_id {
        if !header_present(&response.headers, "X-Request-ID") {
            response
                .headers
                .push(("X-Request-ID".into(), request_id.into()));
        }
    }
    if response.status < 400 {
        return;
    }
    if !header_present(&response.headers, "Content-Type") {
        response
            .headers
            .push(("Content-Type".into(), "text/plain; charset=utf-8".into()));
    }
    if !header_present(&response.headers, "Cache-Control") {
        response
            .headers
            .push(("Cache-Control".into(), "no-store".into()));
    }
    if response.status == 405 && !header_present(&response.headers, "Allow") {
        response
            .headers
            .push(("Allow".into(), allow_for_path(path).into()));
    }
    if response.body.is_empty() {
        response.body = generic_error_body(response.status).to_vec();
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn parse_http(bytes: &[u8]) -> Option<Request> {
    let (header_end, separator_len) =
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            (index, 4)
        } else {
            (bytes.windows(2).position(|window| window == b"\n\n")?, 2)
        };
    let head = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let mut lines = head.lines();
    let first = lines.next()?.split_whitespace().collect::<Vec<_>>();
    if first.len() != 3 {
        return None;
    }
    if !valid_http_token(first[0])
        || first[1].is_empty()
        || first[1]
            .chars()
            .any(|c| c.is_ascii_control() || c.is_whitespace())
        || !matches!(first[2], "HTTP/1.0" | "HTTP/1.1")
    {
        return None;
    }
    let mut headers = Vec::new();
    let mut content_length = None;
    for line in lines {
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        let value = value.trim();
        if !valid_http_token(key) || value.chars().any(|c| c.is_ascii_control()) {
            return None;
        }
        if key.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return None;
            }
            content_length = Some(value.parse::<usize>().ok()?);
        }
        headers.push((key.into(), value.into()));
    }
    let body = &bytes[header_end + separator_len..];
    if body.len() != content_length.unwrap_or(0) {
        return None;
    }
    Some(Request {
        method: first[0].into(),
        path: first[1].into(),
        headers,
        body: body.to_vec(),
    })
}
fn read_http_request(stream: &mut impl Read) -> std::io::Result<Request> {
    let mut bytes = Vec::new();
    let mut one = [0_u8; 1];
    let header_end = loop {
        if stream.read(&mut one)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated HTTP headers",
            ));
        }
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") || bytes.ends_with(b"\n\n") {
            break bytes.len();
        }
        if bytes.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP headers too large",
            ));
        }
    };
    let header_text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid HTTP headers")
    })?;
    let mut content_length = None;
    for line in header_text.lines().skip(1) {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed HTTP header")
        })?;
        if !valid_http_token(name.trim()) || value.chars().any(|c| c.is_ascii_control()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid HTTP header token",
            ));
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported transfer encoding",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "duplicate content length",
                ));
            }
            content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid content length")
            })?);
        }
    }
    let content_length = content_length.unwrap_or(0);
    if content_length > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP body too large",
        ));
    }
    bytes.resize(header_end + content_length, 0);
    stream.read_exact(&mut bytes[header_end..])?;
    parse_http(&bytes)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed HTTP"))
}

fn valid_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            matches!(
                byte,
                b'!' | b'#'..=b'\''
                    | b'*'
                    | b'+'..=b'-'
                    | b'.'
                    | b'0'..=b'9'
                    | b'A'..=b'Z'
                    | b'^'..=b'`'
                    | b'a'..=b'z'
                    | b'|' | b'~'
            )
        })
}

fn write_http(
    stream: &mut impl Write,
    response: Response,
    keep_alive: bool,
) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        302 => "Found",
        204 => "No Content",
        207 => "Multi-Status",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        412 => "Precondition Failed",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: {}\r\n",
        response.status,
        reason,
        response.body.len(),
        if keep_alive { "keep-alive" } else { "close" }
    )?;
    for (key, value) in response.headers {
        write!(stream, "{}: {}\r\n", key, value)?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&response.body)
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn framed_reader_preserves_boundaries_for_sequential_requests() {
        let bytes = b"PUT /one HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /two HTTP/1.1\r\nConnection: close\r\n\r\n";
        let mut stream = Cursor::new(bytes.as_slice());
        let first = read_http_request(&mut stream).unwrap();
        assert_eq!(first.path, "/one");
        assert_eq!(first.body, b"abc");
        let second = read_http_request(&mut stream).unwrap();
        assert_eq!(second.path, "/two");
        assert!(second.body.is_empty());
        assert_eq!(header(&second, "connection").as_deref(), Some("close"));
    }

    #[test]
    fn framed_reader_rejects_duplicate_or_truncated_lengths() {
        for bytes in [
            b"GET / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"PUT / HTTP/1.1\r\nContent-Length: 4\r\n\r\nabc".as_slice(),
            b"GET / HTTP/1.1\r\nBroken\r\n\r\n".as_slice(),
        ] {
            assert!(read_http_request(&mut Cursor::new(bytes)).is_err());
        }
    }

    #[test]
    fn framed_reader_rejects_unsupported_chunked_encoding_and_bad_request_line() {
        for bytes in [
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".as_slice(),
            b"GET / HTTP/9.9\r\n\r\n".as_slice(),
            b"GET / HTTP/1.1\r\nBad Header: value\r\n\r\n".as_slice(),
        ] {
            assert!(read_http_request(&mut Cursor::new(bytes)).is_err());
        }
    }

    #[test]
    fn parse_http_rejects_control_characters_in_target_or_header_values() {
        for bytes in [
            b"GET /bad\x01path HTTP/1.1\r\n\r\n".as_slice(),
            b"GET / HTTP/1.1\r\nX-Test: bad\x01value\r\n\r\n".as_slice(),
        ] {
            assert!(parse_http(bytes).is_none());
        }
    }

    #[test]
    fn response_connection_semantics_are_explicit() {
        let mut output = Vec::new();
        write_http(
            &mut output,
            Response {
                status: 200,
                headers: vec![],
                body: b"ok".to_vec(),
            },
            true,
        )
        .unwrap();
        assert!(String::from_utf8(output)
            .unwrap()
            .contains("Connection: keep-alive\r\n"));
    }

    #[test]
    fn connection_tokens_honor_close_in_a_comma_list() {
        assert!(connection_requests_close("keep-alive, close".into()));
    }

    #[test]
    fn response_reason_phrases_cover_auth_rate_limit_and_unavailable() {
        for (status, reason) in [
            (401, "Unauthorized"),
            (403, "Forbidden"),
            (408, "Request Timeout"),
            (429, "Too Many Requests"),
            (503, "Service Unavailable"),
        ] {
            let mut output = Vec::new();
            write_http(
                &mut output,
                Response {
                    status,
                    headers: vec![],
                    body: vec![],
                },
                false,
            )
            .unwrap();
            assert!(String::from_utf8(output)
                .unwrap()
                .starts_with(&format!("HTTP/1.1 {status} {reason}\r\n")));
        }
    }
}
