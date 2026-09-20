use any_cal_core::MemoryRepository;
use any_cal_dav_server::{serve_one, DavServer};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn request(raw: &str) -> Option<String> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let address = listener.local_addr().unwrap();
    let mut server = DavServer::new(MemoryRepository::new());
    let task = thread::spawn(move || serve_one(&mut server, &listener).unwrap());
    let mut client = TcpStream::connect(address).unwrap();
    client.write_all(raw.as_bytes()).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut output = String::new();
    client.read_to_string(&mut output).unwrap();
    task.join().unwrap();
    Some(output)
}

#[test]
fn localhost_options_and_discovery_are_http_responses() {
    let Some(options) = request("OPTIONS /carddav/contacts HTTP/1.1\r\nHost: localhost\r\n\r\n")
    else {
        return;
    };
    assert!(options.starts_with("HTTP/1.1 200 OK"));
    assert!(options.contains("DAV: 1, 3, addressbook"));
    assert!(!options.contains("calendar-access"));
    assert!(!options.contains("sync-collection") && !options.contains("VEVENT"));
    let Some(task_options) = request("OPTIONS /caldav/tasks HTTP/1.1\r\nHost: localhost\r\n\r\n")
    else {
        return;
    };
    assert!(task_options.starts_with("HTTP/1.1 200 OK"));
    assert!(task_options.contains("DAV: 1, 3, calendar-access"));
    assert!(!task_options.contains("addressbook"));
    assert!(!task_options.contains("sync-collection") && !task_options.contains("VEVENT"));
    let Some(propfind) =
        request("PROPFIND /carddav/contacts HTTP/1.1\r\nHost: localhost\r\nDepth: 0\r\n\r\n")
    else {
        return;
    };
    assert!(propfind.starts_with("HTTP/1.1 207 Multi-Status"));
    assert!(propfind.contains("<collection/>") && propfind.contains("/carddav/contacts"));
}

#[test]
fn well_known_redirect_has_found_reason_and_proxy_absolute_location() {
    let Some(response) = request(
        "GET /.well-known/caldav HTTP/1.1\r\nHost: dav.example.test\r\nX-Forwarded-Proto: https\r\nConnection: close\r\n\r\n",
    ) else {
        return;
    };
    assert!(response.starts_with("HTTP/1.1 302 Found\r\n"));
    assert!(response.contains("Location: https://dav.example.test/caldav/\r\n"));
    assert!(response.contains("Content-Length: 0\r\n"));
    assert!(response.contains("Connection: close\r\n"));
    assert!(!response.contains("Internal Server Error"));
}

#[test]
fn propfind_well_known_redirects_for_caldav_and_carddav() {
    for path in ["caldav", "carddav"] {
        let raw = format!(
            "PROPFIND /.well-known/{path} HTTP/1.1\r\nHost: dav.example.test\r\nX-Forwarded-Proto: https\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
        );
        let Some(response) = request(&raw) else {
            return;
        };
        assert!(response.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(response.contains(&format!("Location: https://dav.example.test/{path}/\r\n")));
        assert!(response.contains("Content-Length: 0\r\n"));
        assert!(response.contains("Connection: close\r\n"));
    }
}

#[test]
fn malformed_http_is_rejected_by_listener() {
    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
        return;
    };
    let address = listener.local_addr().unwrap();
    let mut server = DavServer::new(MemoryRepository::new());
    let task = thread::spawn(move || serve_one(&mut server, &listener));
    let mut client = TcpStream::connect(address).unwrap();
    client.write_all(b"bad").unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    assert!(task.join().unwrap().is_err());
}
