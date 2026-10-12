//! A yard for the tests: a unix socket in a temp dir that answers calls
//! with the JSON a test gives it, as docs/api.md of the yardr repository
//! shows it, and serves the stream. Or the same yard at its listener, a
//! TCP port of this machine: the calls under `/user/` for a device's
//! token, `/user/me`, and a pairing that gives a token for a code.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::api::{Address, Client};

/// The token of the device a yard at an address starts with, and who
/// `/user/me` says it is.
pub const TOKEN: &str = "yardr-dev-fake";
const ME: &str = r#"{"user": "jo", "said_by": "laptop", "yard": "south"}"#;

/// A connection to the yard, by its socket or its listener.
enum Wire {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Wire {
    fn try_clone(&self) -> io::Result<Wire> {
        Ok(match self {
            Wire::Unix(conn) => Wire::Unix(conn.try_clone()?),
            Wire::Tcp(conn) => Wire::Tcp(conn.try_clone()?),
        })
    }

    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match self {
            Wire::Unix(conn) => conn.set_nonblocking(nonblocking),
            Wire::Tcp(conn) => conn.set_nonblocking(nonblocking),
        }
    }
}

impl Read for Wire {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Wire::Unix(conn) => conn.read(buf),
            Wire::Tcp(conn) => conn.read(buf),
        }
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Wire::Unix(conn) => conn.write(buf),
            Wire::Tcp(conn) => conn.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Wire::Unix(conn) => conn.flush(),
            Wire::Tcp(conn) => conn.flush(),
        }
    }
}

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
    /// Sent in chunks of this many bytes, as a long reply of the server is.
    chunk: Option<usize>,
}

#[derive(Default)]
struct State {
    answers: HashMap<String, Reply>,
    /// Replies to the calls of a method with one argument a given string,
    /// by (method, argument, its value), before the standing one.
    answers_for: HashMap<(String, String, String), Reply>,
    /// Replies given once, before the standing one of `answers`.
    once: HashMap<String, VecDeque<Reply>>,
    /// The methods a call of which is read and never answered.
    silent: Vec<String>,
    asked: Vec<(String, Value)>,
    last_request: String,
    /// Every request that came, let in or not.
    requests: usize,
    events: Vec<String>,
    streams: Vec<u64>,
    /// The images the yard holds, by hash, and every hash asked for.
    blobs: HashMap<String, Vec<u8>>,
    blobs_asked: Vec<String>,
    /// The conversations the yard holds, by whom they are with: a call of
    /// `Conversation` with that party is answered from its letters, by the
    /// call's `after`, `limit` and `before`.
    conversations: HashMap<String, Vec<Value>>,
    /// A yard built before `Conversation` had pages: it refuses `limit`
    /// and `before` as arguments the call does not have.
    old: bool,
    /// At an address: the tokens the yard lets in.
    devices: Vec<String>,
    /// The pairing that is open: its code, and the wrong codes so far.
    pairing: Option<(String, u32)>,
    /// How many devices were paired: each has a token of its own.
    paired: u32,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

pub struct Yard {
    /// The socket's directory, when the yard made it: it goes with the yard.
    #[allow(dead_code)]
    dir: Option<tempfile::TempDir>,
    socket: PathBuf,
    /// Of a yard at its listener: `http://127.0.0.1:<port>`.
    address: Option<String>,
    shared: Arc<Shared>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

impl Yard {
    /// A yard at a socket in a temp dir of its own.
    pub fn start() -> Yard {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yard.sock");
        let mut yard = Yard::start_at(&socket);
        yard.dir = Some(dir);
        yard
    }

    /// A yard at a socket the test names.
    pub fn start_at(socket: &Path) -> Yard {
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        // The listener goes with the thread; the socket's file stays, as
        // that of a server that was killed does.
        Yard::run(socket.to_path_buf(), None, move || {
            listener.accept().map(|(conn, _)| Wire::Unix(conn))
        })
    }

    /// A yard at its listener, on a free port of this machine, with one
    /// device: the one whose token is [`TOKEN`].
    pub fn start_tcp() -> Yard {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let yard = Yard::run(PathBuf::new(), Some(address), move || {
            listener.accept().map(|(conn, _)| Wire::Tcp(conn))
        });
        yard.shared.state.lock().unwrap().devices = vec![TOKEN.to_string()];
        yard
    }

    fn run(
        socket: PathBuf,
        address: Option<String>,
        accept: impl Fn() -> io::Result<Wire> + Send + 'static,
    ) -> Yard {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let listener = address.is_some();
        let accept = thread::spawn({
            let shared = shared.clone();
            move || {
                while !shared.state.lock().unwrap().stopped {
                    match accept() {
                        Ok(conn) => {
                            let shared = shared.clone();
                            thread::spawn(move || serve(conn, &shared, listener));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            }
        });
        Yard {
            dir: None,
            socket,
            address,
            shared,
            accept: Mutex::new(Some(accept)),
        }
    }

    /// A client of the yard: by its socket, or at its address as the
    /// device it started with.
    pub fn client(&self) -> Client {
        match &self.address {
            Some(_) => self.client_with(TOKEN),
            None => Client::new(&self.socket),
        }
    }

    /// A client at the yard's address with a token of the test's.
    pub fn client_with(&self, token: &str) -> Client {
        Client::remote(Address::parse(self.address()).unwrap(), token)
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The address of a yard at its listener.
    pub fn address(&self) -> &str {
        self.address.as_deref().expect("a yard at its listener")
    }

    /// The operator opens a pairing: the next right code is answered with
    /// a token, once.
    pub fn pair(&self, code: &str) {
        self.shared.state.lock().unwrap().pairing = Some((code.to_string(), 0));
    }

    pub fn pairing_open(&self) -> bool {
        self.shared.state.lock().unwrap().pairing.is_some()
    }

    /// The tokens the yard lets in.
    pub fn devices(&self) -> Vec<String> {
        self.shared.state.lock().unwrap().devices.clone()
    }

    /// The operator adds a device by hand, with this token.
    pub fn add_device(&self, token: &str) {
        self.shared
            .state
            .lock()
            .unwrap()
            .devices
            .push(token.to_string());
    }

    /// The operator removes a device: its token is refused from now on,
    /// and the streams it holds are closed.
    pub fn remove_device(&self, token: &str) {
        self.shared
            .state
            .lock()
            .unwrap()
            .devices
            .retain(|device| device != token);
        self.shared.changed.notify_all();
    }

    /// What the yard answers to a call of the method from now on.
    pub fn answer(&self, method: &str, body: &str) {
        self.answer_status(method, 200, body);
    }

    pub fn answer_status(&self, method: &str, status: u16, body: &str) {
        self.set(
            method,
            Reply {
                status,
                body: body.into(),
                chunk: None,
            },
        );
    }

    pub fn answer_chunked(&self, method: &str, body: &str, chunk: usize) {
        self.set(
            method,
            Reply {
                status: 200,
                body: body.into(),
                chunk: Some(chunk),
            },
        );
    }

    /// What it answers from now on to a call of the method whose argument
    /// `arg` is `is`: the conversation with one party, mail to one peer.
    pub fn answer_for(&self, method: &str, arg: &str, is: &str, body: &str) {
        let reply = Reply {
            status: 200,
            body: body.into(),
            chunk: None,
        };
        self.shared
            .state
            .lock()
            .unwrap()
            .answers_for
            .insert((method.into(), arg.into(), is.into()), reply);
    }

    /// What it answers to the next call of the method only.
    pub fn answer_once_status(&self, method: &str, status: u16, body: &str) {
        let reply = Reply {
            status,
            body: body.into(),
            chunk: None,
        };
        self.shared
            .state
            .lock()
            .unwrap()
            .once
            .entry(method.into())
            .or_default()
            .push_back(reply);
    }

    /// A call of the method is taken and the connection closed with no
    /// reply, as by a server that went away over it, until the method is
    /// given an answer again.
    pub fn answer_nothing(&self, method: &str) {
        self.shared.state.lock().unwrap().silent.push(method.into());
    }

    fn set(&self, method: &str, reply: Reply) {
        let mut state = self.shared.state.lock().unwrap();
        state.silent.retain(|silent| silent != method);
        state.answers.insert(method.into(), reply);
    }

    /// The arguments of every call of the method so far.
    pub fn asked(&self, method: &str) -> Vec<Value> {
        let state = self.shared.state.lock().unwrap();
        state
            .asked
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, args)| args.clone())
            .collect()
    }

    /// The method of every call so far, in the order they came.
    pub fn calls(&self) -> Vec<String> {
        let state = self.shared.state.lock().unwrap();
        state.asked.iter().map(|(m, _)| m.clone()).collect()
    }

    /// How many requests came so far, those that were not let in too.
    pub fn requests(&self) -> usize {
        self.shared.state.lock().unwrap().requests
    }

    /// The head of the last request, as it came.
    pub fn last_request(&self) -> String {
        self.shared.state.lock().unwrap().last_request.clone()
    }

    /// An event of the yard, one line of JSON with its seq: kept for the
    /// streams opened later, and sent to those that are open.
    pub fn emit(&self, event: &str) {
        self.shared.state.lock().unwrap().events.push(event.into());
        self.shared.changed.notify_all();
    }

    /// An image the yard holds: `GET /blobs/<hash>` answers the bytes.
    pub fn blob(&self, hash: &str, bytes: &[u8]) {
        self.shared
            .state
            .lock()
            .unwrap()
            .blobs
            .insert(hash.into(), bytes.to_vec());
    }

    /// The yard holds the conversation with `with`, these letters oldest
    /// first, and answers `Conversation` from it as docs/api.md says: what
    /// `after` and `before` leave, the newest `limit` of that, and `more`
    /// when the limit left older ones out.
    pub fn converse(&self, with: &str, letters: Vec<Value>) {
        self.shared
            .state
            .lock()
            .unwrap()
            .conversations
            .insert(with.into(), letters);
    }

    /// The yard is one built before `Conversation` had pages: a call with
    /// `limit` or `before` is refused, as any argument it does not have.
    pub fn old(&self) {
        self.shared.state.lock().unwrap().old = true;
    }

    /// The hash of every image asked for so far.
    pub fn blobs_asked(&self) -> Vec<String> {
        self.shared.state.lock().unwrap().blobs_asked.clone()
    }

    /// The `after` of every stream opened so far.
    pub fn streams_asked(&self) -> Vec<u64> {
        self.shared.state.lock().unwrap().streams.clone()
    }

    /// The server goes away: its streams end and nothing listens any more.
    pub fn stop(&self) {
        self.shared.state.lock().unwrap().stopped = true;
        self.shared.changed.notify_all();
        if let Some(accept) = self.accept.lock().unwrap().take() {
            accept.join().unwrap();
        }
    }

    /// As `stop`, named for the test that is about the socket left behind.
    pub fn stop_listening(&self) {
        self.stop();
    }
}

impl Drop for Yard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(conn: Wire, shared: &Shared, listener: bool) {
    // A client that hangs up mid-reply is no failure of the test.
    let _ = serve_one(conn, shared, listener);
}

/// An answer that is no call's: its status, and a body.
fn plain(conn: &mut Wire, status: &str, kind: &str, body: &str) -> io::Result<()> {
    write!(
        conn,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    conn.flush()
}

/// A code as the yard reads it: without its dash and spaces, in capitals.
fn code_of(typed: &str) -> String {
    typed
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// `POST /user/pair`: a token for the right code, once. A wrong code is
/// counted, and the fifth ends the pairing.
fn pair(conn: &mut Wire, shared: &Shared, body: &[u8]) -> io::Result<()> {
    const JSON: &str = "application/json";
    let typed = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| body["code"].as_str().map(code_of))
        .unwrap_or_default();
    if typed.len() != 8 {
        return plain(conn, "400 Bad Request", "text/plain", "no code\n");
    }
    let mut state = shared.state.lock().unwrap();
    let Some((code, wrong)) = state.pairing.as_mut() else {
        unreachable!("the path is there while a pairing is open")
    };
    if code_of(code) != typed {
        *wrong += 1;
        if *wrong == 5 {
            state.pairing = None;
        }
        drop(state);
        return plain(conn, "403 Forbidden", JSON, r#"{"refused": "wrong_code"}"#);
    }
    state.pairing = None;
    state.paired += 1;
    let token = format!("yardr-dev-paired-{}", state.paired);
    state.devices.push(token.clone());
    drop(state);
    let answer =
        format!(r#"{{"token": "{token}", "user": "jo", "device": "laptop", "yard": "south"}}"#);
    plain(conn, "200 OK", JSON, &answer)
}

fn serve_one(conn: Wire, shared: &Shared, listener: bool) -> io::Result<()> {
    conn.set_nonblocking(false)?;
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut conn = conn;
    let mut head = String::new();
    let mut length = 0usize;
    let mut token = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("authorization") {
                token = value.trim().strip_prefix("Bearer ").map(str::to_string);
            }
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let mut target = head.split_whitespace().nth(1).unwrap_or("").to_string();
    {
        let mut state = shared.state.lock().unwrap();
        state.last_request = head.clone();
        state.requests += 1;
    }

    // At the listener everything is under /user/, for a device's token:
    // 404 in a yard with no device, 401 for whatever else is not let in,
    // whatever was asked for. The pairing alone takes a code instead.
    if listener {
        let (pairing, devices, known) = {
            let state = shared.state.lock().unwrap();
            let known = token.as_ref().is_some_and(|t| state.devices.contains(t));
            (state.pairing.is_some(), state.devices.len(), known)
        };
        if target == "/user/pair" && pairing {
            return pair(&mut conn, shared, &body);
        }
        if devices == 0 {
            return plain(
                &mut conn,
                "404 Not Found",
                "text/plain",
                "404 page not found\n",
            );
        }
        let under = target.strip_prefix("/user").map(str::to_string);
        match under {
            Some(path) if known => target = path,
            _ => {
                return plain(
                    &mut conn,
                    "401 Unauthorized",
                    "text/plain",
                    "Unauthorized\n",
                );
            }
        }
        if target == "/me" {
            return plain(&mut conn, "200 OK", "application/json", ME);
        }
    }
    // The stream of a device ends when the device is removed.
    let token = token.filter(|_| listener);

    if let Some(query) = target.strip_prefix("/events") {
        let after: u64 = query
            .strip_prefix("?after=")
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        return stream(conn, shared, after, token);
    }
    if let Some(hash) = target.strip_prefix("/blobs/") {
        let blob = {
            let mut state = shared.state.lock().unwrap();
            state.blobs_asked.push(hash.to_string());
            state.blobs.get(hash).cloned()
        };
        match blob {
            Some(bytes) => {
                write!(
                    conn,
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
                    bytes.len()
                )?;
                conn.write_all(&bytes)?;
            }
            // The one answer that is no JSON.
            None => {
                let line = "no such image\n";
                write!(
                    conn,
                    "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{line}",
                    line.len()
                )?;
            }
        }
        return conn.flush();
    }

    let method = target.strip_prefix("/call/").unwrap_or("").to_string();
    let reply = {
        let mut state = shared.state.lock().unwrap();
        let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let by_arg = state
            .answers_for
            .iter()
            .find(|((m, arg, is), _)| *m == method && args[arg.as_str()] == is.as_str())
            .map(|(_, reply)| reply.clone());
        state.asked.push((method.clone(), args.clone()));
        if state.silent.contains(&method) {
            return Ok(());
        }
        let once = state.once.get_mut(&method).and_then(VecDeque::pop_front);
        let held = (method == "Conversation")
            .then(|| conversation(&state, &args))
            .flatten();
        once.or(by_arg)
            .or(held)
            .or_else(|| state.answers.get(&method).cloned())
            .unwrap_or_else(|| Reply {
                status: 200,
                body: format!(
                    r#"{{"error":{{"code":"error","message":"no such method {method}"}}}}"#
                ),
                chunk: None,
            })
    };
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nX-Yardr-Revision: fake\r\nX-Yardr-Pid: 1\r\n",
        reply.status
    );
    match reply.chunk {
        None => write!(
            conn,
            "{head}Content-Length: {}\r\n\r\n{}",
            reply.body.len(),
            reply.body
        )?,
        Some(size) => {
            write!(conn, "{head}Transfer-Encoding: chunked\r\n\r\n")?;
            for chunk in reply.body.as_bytes().chunks(size) {
                write!(conn, "{:x}\r\n", chunk.len())?;
                conn.write_all(chunk)?;
                conn.write_all(b"\r\n")?;
            }
            conn.write_all(b"0\r\n\r\n")?;
        }
    }
    conn.flush()
}

/// The answer to `Conversation` from a conversation the yard holds, none
/// when it holds none with that party.
fn conversation(state: &State, args: &Value) -> Option<Reply> {
    let reply = |body: String| Reply {
        status: 200,
        body,
        chunk: None,
    };
    let paged = ["limit", "before"]
        .into_iter()
        .find(|name| !args[*name].is_null());
    if let Some(name) = paged.filter(|_| state.old) {
        return Some(reply(
            serde_json::json!({"error": {"code": "error", "message": format!(
                "argument \"{name}\": Conversation has none of that name (it takes a, b, after)"
            )}})
            .to_string(),
        ));
    }
    let letters = state.conversations.get(args["b"].as_str()?)?;
    let number = |name: &str| args[name].as_u64().unwrap_or(0);
    let (after, limit, before) = (number("after"), number("limit"), number("before"));
    let left: Vec<&Value> = letters
        .iter()
        .filter(|l| {
            let seq = l["seq"].as_u64().unwrap_or(0);
            seq > after && (before == 0 || seq < before)
        })
        .collect();
    let cut = match limit as usize {
        0 => 0,
        limit => left.len().saturating_sub(limit),
    };
    let mut body = serde_json::json!({ "result": left[cut..] });
    if cut > 0 {
        body["more"] = Value::Bool(true);
    }
    Some(reply(body.to_string()))
}

/// The stream: the events with a seq after `after`, then every one as the
/// test emits it, until the yard stops.
fn stream(
    mut conn: Wire,
    shared: &Shared,
    after: u64,
    device: Option<String>,
) -> std::io::Result<()> {
    write!(
        conn,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\n\r\n"
    )?;
    conn.flush()?;
    let mut state = shared.state.lock().unwrap();
    state.streams.push(after);
    let mut sent = 0;
    loop {
        while sent < state.events.len() {
            let event = &state.events[sent];
            sent += 1;
            let seq = serde_json::from_str::<Value>(event)
                .ok()
                .and_then(|e| e["seq"].as_u64())
                .unwrap_or(0);
            if seq > after {
                let line = format!("{event}\n");
                write!(conn, "{:x}\r\n{line}\r\n", line.len())?;
                conn.flush()?;
            }
        }
        // Cut off, as a server that is killed cuts it: no last chunk. So is
        // the stream of a device that was removed.
        let removed = device.as_ref().is_some_and(|d| !state.devices.contains(d));
        if state.stopped || removed {
            return Ok(());
        }
        state = shared.changed.wait(state).unwrap();
    }
}
