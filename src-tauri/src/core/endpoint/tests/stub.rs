//! The stub upstream, taken literally from `docs/TASKS.md` T-042:
//! *"Against the T-041 stub upstream"*.
//!
//! A hand-rolled HTTP/1.1 origin server on an ephemeral loopback port that
//! records every request it was asked for and answers with a scripted reply.
//! It is deliberately raw sockets rather than a framework: the acceptance
//! criteria are about bytes on the wire — status lines, header sets, chunk
//! boundaries and their timing — and a second HTTP stack on the test side
//! would normalise away exactly the differences the suite exists to catch.
//!
//! Two framing modes are understood on the request side (`Content-Length` and
//! `Transfer-Encoding: chunked`), because the listener re-derives the framing
//! of the request it forwards and either is legitimate.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A request as the stub received it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Recorded {
    pub(super) method: String,
    /// Path **and query**, verbatim from the request line.
    pub(super) target: String,
    /// Header names lowercased, in arrival order; values verbatim.
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl Recorded {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What the stub answers with.
#[derive(Clone, Debug)]
pub(super) enum Reply {
    /// A fixed answer, byte for byte what the client must receive.
    Fixed {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: Vec<u8>,
    },
    /// Status 200 and an answer that *reflects the request*: the method, the
    /// target, the value of `x-tag` and the request's own body bytes. The
    /// transparency test needs a reply that depends on the request — against a
    /// fixed one, a listener that dropped the body and the query string would
    /// still compare equal.
    Echo,
    /// A chunked body written one chunk at a time, `first_delay` before the
    /// first and `gap` between the rest. This is SSE, and it is also how the
    /// slow-response cases are scripted without a sleep in the test itself.
    Chunked {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        chunks: Vec<Vec<u8>>,
        /// Slept before the status line is written: the response *begins* late.
        head_delay: Duration,
        /// Slept between chunks: SSE at its real pace.
        gap: Duration,
    },
}

/// The SSE fixture: a chunked body with the headers a real event stream
/// carries, written chunk by chunk with `gap` between them.
pub(super) fn sse_reply(chunks: Vec<Vec<u8>>, head_delay: Duration, gap: Duration) -> Reply {
    Reply::Chunked {
        status: 200,
        headers: vec![
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
            ("x-upstream", "stub"),
        ],
        chunks,
        head_delay,
        gap,
    }
}

/// An SSE fixture: `count` server-sent events, one per chunk.
pub(super) fn sse_chunks(count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| format!("data: chunk-{index}\n\n").into_bytes())
        .collect()
}

pub(super) struct Stub {
    address: SocketAddr,
    state: Arc<Mutex<State>>,
}

struct State {
    default: Reply,
    routes: HashMap<String, Reply>,
    requests: Vec<Recorded>,
    /// A `Date` header added to every answer, emulating what a real origin
    /// sends (llama.cpp's server includes one). `None` — the default — writes
    /// exactly what the reply declares, which is how the "the upstream sent no
    /// Date" branch is scripted.
    date: Option<String>,
}

impl Stub {
    /// Bind an ephemeral loopback port and start answering. The stub lives for
    /// the process lifetime: tests own their own ports and never need to stop
    /// it (the same pattern the T-041 suite uses).
    pub(super) fn start() -> Stub {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(err) => panic!("could not bind the stub's port: {err}"),
        };
        let address = match listener.local_addr() {
            Ok(address) => address,
            Err(err) => panic!("could not read the stub's address: {err}"),
        };
        let state = Arc::new(Mutex::new(State {
            default: Reply::Echo,
            routes: HashMap::new(),
            requests: Vec::new(),
            date: None,
        }));
        let worker = Arc::clone(&state);
        thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else {
                    continue;
                };
                let worker = Arc::clone(&worker);
                thread::spawn(move || handle(socket, worker));
            }
        });
        Stub { address, state }
    }

    pub(super) fn address(&self) -> SocketAddr {
        self.address
    }

    /// The URL a client (or the listener) uses to reach `target`.
    pub(super) fn url(&self, target: &str) -> String {
        format!("http://{}{target}", self.address)
    }

    /// Add a `Date` header to every answer, as a real origin does.
    pub(super) fn set_date(&self, date: Option<&str>) {
        self.lock().date = date.map(str::to_string);
    }

    pub(super) fn set(&self, method: &str, target: &str, reply: Reply) {
        self.lock().routes.insert(route_key(method, target), reply);
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.lock().requests.clone()
    }

    /// How many requests the stub has been asked for, without copying them.
    pub(super) fn request_count(&self) -> usize {
        self.lock().requests.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn route_key(method: &str, target: &str) -> String {
    format!("{method} {target}")
}

/// Serve one connection until the client closes it or sends something
/// unparseable. Keeping connections alive is what makes the latency
/// measurement a measurement of the listener rather than of 1 000 TCP
/// handshakes.
fn handle(mut socket: TcpStream, state: Arc<Mutex<State>>) {
    let _ = socket.set_nodelay(true);
    loop {
        let Some(request) = read_request(&mut socket) else {
            return;
        };
        let (reply, date) = {
            let mut state = match state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            state.requests.push(request.clone());
            let reply = state
                .routes
                .get(&route_key(&request.method, &request.target))
                .cloned()
                .unwrap_or_else(|| state.default.clone());
            (reply, state.date.clone())
        };
        if write_reply(&mut socket, &reply, date.as_deref(), &request).is_err() {
            return;
        }
    }
}

fn read_request(socket: &mut TcpStream) -> Option<Recorded> {
    let mut buffer: Vec<u8> = Vec::new();
    let head_end = loop {
        if let Some(at) = find(&buffer, b"\r\n\r\n") {
            break at;
        }
        let mut chunk = [0u8; 4096];
        match socket.read(&mut chunk) {
            Ok(0) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => return None,
        }
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }

    let mut body: Vec<u8> = buffer[head_end + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"));
    if chunked {
        body = read_chunked_body(socket, body)?;
    } else {
        let wanted = header_value(&headers, "content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while body.len() < wanted {
            let mut chunk = [0u8; 4096];
            match socket.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => body.extend_from_slice(&chunk[..read]),
                Err(_) => return None,
            }
        }
        body.truncate(wanted);
    }

    Some(Recorded {
        method,
        target,
        headers,
        body,
    })
}

fn read_chunked_body(socket: &mut TcpStream, mut buffer: Vec<u8>) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = loop {
            if let Some(at) = find(&buffer, b"\r\n") {
                break at;
            }
            let mut chunk = [0u8; 4096];
            match socket.read(&mut chunk) {
                Ok(0) => return Some(body),
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                Err(_) => return None,
            }
        };
        let size_line = String::from_utf8_lossy(&buffer[..line_end]).to_string();
        let size = usize::from_str_radix(size_line.trim().split(';').next()?, 16).ok()?;
        buffer.drain(..line_end + 2);
        if size == 0 {
            // The trailer section, terminated by the blank line.
            return Some(body);
        }
        while buffer.len() < size + 2 {
            let mut chunk = [0u8; 4096];
            match socket.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                Err(_) => return None,
            }
        }
        body.extend_from_slice(&buffer[..size]);
        buffer.drain(..size + 2);
    }
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.clone())
}

fn write_reply(
    socket: &mut TcpStream,
    reply: &Reply,
    date: Option<&str>,
    request: &Recorded,
) -> std::io::Result<()> {
    match reply {
        Reply::Fixed {
            status,
            headers,
            body,
        } => {
            let mut head = format!(
                "HTTP/1.1 {status} {}\r\ncontent-length: {}\r\n",
                reason(*status),
                body.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            write_date(&mut head, headers, date);
            head.push_str("\r\n");
            socket.write_all(head.as_bytes())?;
            socket.write_all(body)?;
        }
        Reply::Echo => {
            let tag = header_value(&request.headers, "x-tag").unwrap_or_default();
            let body = format!(
                "{} {}\ntag={tag}\n{}",
                request.method,
                request.target,
                String::from_utf8_lossy(&request.body)
            );
            let mut head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\nx-echo-method: {}\r\nx-echo-target: {}\r\ncontent-length: {}\r\n",
                request.method,
                request.target,
                body.len()
            );
            write_date(&mut head, &[], date);
            head.push_str("\r\n");
            socket.write_all(head.as_bytes())?;
            socket.write_all(body.as_bytes())?;
        }
        Reply::Chunked {
            status,
            headers,
            chunks,
            head_delay,
            gap,
        } => {
            // The delay is *before* the status line: the response begins late,
            // which is the thing a long-held request is about. A delay after
            // the head would have the status already on the wire, and no proxy
            // can retract a status it has sent.
            if !head_delay.is_zero() {
                thread::sleep(*head_delay);
            }
            let mut head = format!(
                "HTTP/1.1 {status} {}\r\ntransfer-encoding: chunked\r\n",
                reason(*status)
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            write_date(&mut head, headers, date);
            head.push_str("\r\n");
            socket.write_all(head.as_bytes())?;
            socket.flush()?;
            for (index, chunk) in chunks.iter().enumerate() {
                if index > 0 && !gap.is_zero() {
                    thread::sleep(*gap);
                }
                socket.write_all(format!("{:x}\r\n", chunk.len()).as_bytes())?;
                socket.write_all(chunk)?;
                socket.write_all(b"\r\n")?;
                // Flushed per chunk, which is the whole point: a stub that
                // buffered would make the boundary assertions vacuous.
                socket.flush()?;
            }
            socket.write_all(b"0\r\n\r\n")?;
        }
    }
    socket.flush()
}

/// Append the emulated `Date`, unless the reply already carries one.
fn write_date(head: &mut String, headers: &[(&'static str, &'static str)], date: Option<&str>) {
    let Some(date) = date else {
        return;
    };
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("date"))
    {
        return;
    }
    head.push_str(&format!("date: {date}\r\n"));
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        418 => "I'm a teapot",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
