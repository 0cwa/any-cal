use std::io::{self, Read, Write};

pub(crate) fn connection_requests_close(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"))
    })
}

pub(crate) fn parse_http(raw: &[u8]) -> io::Result<any_cal_dav_server::Request> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed HTTP headers"))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP"))?;
    let mut lines = head.lines();
    let first = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut parts = first.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?;
    let path = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?;
    let mut headers = Vec::new();
    let mut length = None;
    for line in lines {
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed header"))?;
        if key.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate content length",
                ));
            }
            length = Some(value.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
            })?);
        }
        headers.push((key.into(), value.trim().into()));
    }
    let body = &raw[split + 4..];
    let length = length.unwrap_or(0);
    if body.len() != length {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated body",
        ));
    }
    Ok(any_cal_dav_server::Request {
        method: method.into(),
        path: path.into(),
        headers,
        body: body[..length].to_vec(),
    })
}

pub(crate) fn read_http_request(stream: &mut impl Read) -> io::Result<any_cal_dav_server::Request> {
    let mut bytes = Vec::new();
    let mut one = [0_u8; 1];
    let header_end = loop {
        if stream.read(&mut one)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated HTTP headers",
            ));
        }
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP headers too large",
            ));
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP headers"))?;
    let mut length = 0usize;
    let mut saw_length = false;
    for line in head.lines().skip(1) {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed header"))?;
        if key.eq_ignore_ascii_case("content-length") {
            if saw_length {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate content length",
                ));
            }
            saw_length = true;
            length = value.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
            })?;
        }
    }
    if length > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP body too large",
        ));
    }
    bytes.resize(header_end + length, 0);
    stream.read_exact(&mut bytes[header_end..])?;
    parse_http(&bytes)
}

pub(crate) fn write_http(
    stream: &mut impl Write,
    response: any_cal_dav_server::Response,
    keep_alive: bool,
) -> io::Result<()> {
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
