//! A yard for the tests, and in a debug build for the screenshots: a unix
//! socket in a temp dir that answers calls with the JSON a test gives it,
//! as docs/api.md of the yardr repository shows it, and serves the stream.
//! Or the same yard at its listener, a TCP port of this machine: the calls
//! under `/user/` for a device's token, `/user/me`, and a pairing that
//! gives a token for a code. Windows has no socket, so the yard there is at
//! its listener alone, and the yard a test starts with, [`Yard::start`], is
//! that one there: a test that never names its socket runs on both. Beside
//! the yard, [`Router`] stands for the edge in front of it, [`Posthog`]
//! for the capture that feedback is sent to, and [`Live`] for OpenAI's Live
//! socket that a voice session talks to (the provider's, not the yard's).

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
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

/// A TCP connection that sends what is written to it at once, as a Go
/// server's does by default, and so a real yard's. The fake writes a reply
/// in several small pieces, and Nagle's algorithm would hold each piece
/// after the first until the client has acknowledged the one before.
fn at_once(conn: TcpStream) -> TcpStream {
    let _ = conn.set_nodelay(true);
    conn
}

/// A connection to the yard, by its socket or its listener.
enum Wire {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Wire {
    fn try_clone(&self) -> io::Result<Wire> {
        Ok(match self {
            #[cfg(unix)]
            Wire::Unix(conn) => Wire::Unix(conn.try_clone()?),
            Wire::Tcp(conn) => Wire::Tcp(conn.try_clone()?),
        })
    }

    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Wire::Unix(conn) => conn.set_nonblocking(nonblocking),
            Wire::Tcp(conn) => conn.set_nonblocking(nonblocking),
        }
    }

    /// Whether the other end has hung up: a read that would block says it
    /// has not, one that gives nothing or fails says it has. What a client
    /// sends on a connection it only listens to is read and let go.
    fn hung_up(&mut self) -> bool {
        if self.set_nonblocking(true).is_err() {
            return true;
        }
        let mut sink = [0u8; 64];
        let gone = loop {
            match self.read(&mut sink) {
                Ok(0) => break true,
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break false,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break true,
            }
        };
        let _ = self.set_nonblocking(false);
        gone
    }
}

impl Read for Wire {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Wire::Unix(conn) => conn.read(buf),
            Wire::Tcp(conn) => conn.read(buf),
        }
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Wire::Unix(conn) => conn.write(buf),
            Wire::Tcp(conn) => conn.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
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
    /// The requests that are read and never answered: the connection is
    /// held open until the yard stops, so the client's read gives up for
    /// its timeout. By target prefix: "/call/Head" holds those calls,
    /// "/events" the streams before their head.
    stall: Vec<String>,
    /// The requests held once: the first whose target starts with one of
    /// these is held, and the rule is gone.
    stall_once: Vec<String>,
    /// The methods whose calls are held before they are answered
    /// ([`Yard::hold_calls`]), and the calls held now, one entry each.
    hold_calls: Vec<String>,
    calls_held: Vec<String>,
    asked: Vec<(String, Value)>,
    last_request: String,
    /// Every request that came, let in or not.
    requests: usize,
    events: Vec<String>,
    streams: Vec<u64>,
    /// How many event lines the streams have written, those of every
    /// stream counted ([`Yard::events_sent`]).
    events_sent: usize,
    /// The streams opened before this many stay open and bring nothing,
    /// as one a proxy holds open after what was behind it went away: what
    /// is emitted meanwhile is the yard's all the same, its calls answer
    /// as they did, and a stream opened since brings what it asks for.
    muted: usize,
    /// The streams opened before this many have ended, the server staying
    /// ([`Yard::end_streams`]).
    ended: usize,
    /// The streams stay open when their client hangs up ([`Yard::hold_streams`]).
    held: bool,
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
    /// The user and the device name the open pairing makes, where a user
    /// of the yard opened it for their own ([`open_for`]); none where the
    /// operator opened it, for the yard's one user and its one device.
    pairing_for: Option<(String, String)>,
    /// Who a paired device's token is, by token: the user and the device
    /// name `/user/me` says from then on. The tokens of no entry are jo's
    /// laptop.
    who: HashMap<String, (String, String)>,
    /// How many pairings a user of the yard opened: each has a code of its
    /// own.
    opened: u32,
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
    /// The screenshots name their socket instead, so they need no temp dir
    /// of `tempfile`'s, which the built footplate does not have.
    #[cfg(test)]
    #[allow(dead_code)]
    dir: Option<tempfile::TempDir>,
    socket: PathBuf,
    /// Of a yard at its listener: `http://127.0.0.1:<port>`.
    address: Option<String>,
    shared: Arc<Shared>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

impl Yard {
    /// The yard a test starts with: at a socket in a temp dir of its own,
    /// and where there is no socket (Windows) at its listener, as
    /// [`Yard::start_tcp`]. [`Yard::client`] is a client of either; a test
    /// that reads [`Yard::socket`] is about the socket and is unix's alone.
    #[cfg(test)]
    pub fn start() -> Yard {
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("yard.sock");
            let mut yard = Yard::start_at(&socket);
            yard.dir = Some(dir);
            yard
        }
        #[cfg(not(unix))]
        Yard::start_tcp()
    }

    /// A yard at a socket the test names.
    #[cfg(unix)]
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
            listener.accept().map(|(conn, _)| Wire::Tcp(at_once(conn)))
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
            #[cfg(test)]
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
        #[cfg(unix)]
        if self.address.is_none() {
            return Client::new(&self.socket);
        }
        self.client_with(TOKEN)
    }

    /// A client at the yard's address with a token of the test's.
    pub fn client_with(&self, token: &str) -> Client {
        Client::remote(Address::parse(self.address()).unwrap(), token)
    }

    /// The socket of a yard that has one: empty at its listener, so a test
    /// that reads it is about the socket, and unix's alone.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The address of a yard at its listener.
    pub fn address(&self) -> &str {
        self.address.as_deref().expect("a yard at its listener")
    }

    /// Whether the yard is at its listener, with no socket: the kind that
    /// [`Yard::client`] tells apart, for a test that sets up or expects
    /// something of the one kind and not the other.
    pub fn at_listener(&self) -> bool {
        self.address.is_some()
    }

    /// The operator opens a pairing: the next right code is answered with
    /// a token, once.
    pub fn pair(&self, code: &str) {
        let mut state = self.shared.state.lock().unwrap();
        state.pairing = Some((code.to_string(), 0));
        state.pairing_for = None;
    }

    pub fn pairing_open(&self) -> bool {
        self.shared.state.lock().unwrap().pairing.is_some()
    }

    /// The operator ends the pairing that is open, whoever opened it.
    pub fn end_pairing(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.pairing = None;
        state.pairing_for = None;
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

    /// A request for `target` is taken and never answered: the connection
    /// is held open until the yard stops, as by a yard too busy to answer,
    /// so the client's read gives up for its timeout. By prefix of the
    /// request's target: "/call/Head" holds those calls, "/events" the
    /// streams before their head.
    pub fn stall(&self, target: &str) {
        self.shared.state.lock().unwrap().stall.push(target.into());
    }

    /// The next request for `target` alone is held as [`Yard::stall`]
    /// holds every one: the one after it is answered as ever.
    pub fn stall_once(&self, target: &str) {
        self.shared
            .state
            .lock()
            .unwrap()
            .stall_once
            .push(target.into());
    }

    /// From now on a call of the method is asked and held before it is
    /// answered, until [`Yard::release_calls`]: its caller waits as one of a
    /// yard that is slow, and does nothing else meanwhile. It is answered
    /// when it is let through, as the yard answers then. A test with
    /// something to do while a caller is in its call (a follower in a call
    /// reads no stream, so no quiet of its stream runs) holds the call and
    /// waits for [`Yard::calls_held`]. The client's own timeout still ends
    /// the wait, so a call is held for seconds, not for ever.
    pub fn hold_calls(&self, method: &str) {
        self.shared
            .state
            .lock()
            .unwrap()
            .hold_calls
            .push(method.into());
    }

    /// The calls of the method that are held are answered, and so are those
    /// that come after.
    pub fn release_calls(&self, method: &str) {
        self.shared
            .state
            .lock()
            .unwrap()
            .hold_calls
            .retain(|held| held != method);
        self.shared.changed.notify_all();
    }

    /// How many calls of the method are held now.
    pub fn calls_held(&self, method: &str) -> usize {
        let state = self.shared.state.lock().unwrap();
        state.calls_held.iter().filter(|m| *m == method).count()
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

    /// The streams that are open bring nothing from here on, and stay
    /// open.
    pub fn mute(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.muted = state.streams.len();
    }

    /// The streams that are open end and the server stays: an edge or a
    /// proxy that cuts them, or a link that drops and comes back. A stream
    /// opened since brings what it asks for.
    pub fn end_streams(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.ended = state.streams.len();
        drop(state);
        self.shared.changed.notify_all();
    }

    /// The streams stay open when their client hangs up, from here on: the
    /// yard of a proxy or a tunnel that takes a client's FIN for a half-close
    /// and goes on serving it. A real yard ends its stream then, and the
    /// fake does by default; a test of what a client does to stop its own
    /// stream asks for this one, or the yard's closing does the stopping.
    pub fn hold_streams(&self) {
        self.shared.state.lock().unwrap().held = true;
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

    /// How many event lines the streams have written so far, each stream's
    /// counted. [`Yard::emit`] only hands an event to the streams: when this
    /// has gone up for it, the event is on the wire, and a client that reads
    /// has it at once.
    pub fn events_sent(&self) -> usize {
        self.shared.state.lock().unwrap().events_sent
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
    let (user, device) = state
        .pairing_for
        .take()
        .unwrap_or(("jo".to_string(), "laptop".to_string()));
    state
        .who
        .insert(token.clone(), (user.clone(), device.clone()));
    drop(state);
    let answer = format!(
        r#"{{"token": "{token}", "user": "{user}", "device": "{device}", "yard": "south"}}"#
    );
    plain(conn, "200 OK", JSON, &answer)
}

/// A user of the yard opens a pairing for a device of their own, as the
/// yard's web page does for the user the edge's token maps: a code for the
/// user and the device name. A pairing that is open already is refused with
/// the user who holds it, the operator's included.
fn open_for(shared: &Shared, user: &str, device: &str) -> Result<String, String> {
    let mut state = shared.state.lock().unwrap();
    if state.pairing.is_some() {
        return Err(state
            .pairing_for
            .as_ref()
            .map_or("jo".to_string(), |(holder, _)| holder.clone()));
    }
    state.opened += 1;
    let code = format!("OPEN{:04}", state.opened);
    state.pairing = Some((code.clone(), 0));
    state.pairing_for = Some((user.to_string(), device.to_string()));
    Ok(format!("{}-{}", &code[..4], &code[4..]))
}

/// Whether a request for `target` is held without answer: one whose
/// prefix is stalled, or the first for a prefix stalled once.
fn stalled(shared: &Shared, target: &str) -> bool {
    let mut state = shared.state.lock().unwrap();
    if state.stall.iter().any(|prefix| target.starts_with(prefix)) {
        return true;
    }
    if let Some(held) = state
        .stall_once
        .iter()
        .position(|prefix| target.starts_with(prefix))
    {
        state.stall_once.remove(held);
        return true;
    }
    false
}

/// Holds a stalled request's connection open until the yard stops, as a
/// yard that never answers holds it.
fn hold_until_stopped(shared: &Shared) {
    let mut state = shared.state.lock().unwrap();
    while !state.stopped {
        state = shared.changed.wait_timeout(state, LOOK).unwrap().0;
    }
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
        // Who the device is: the user and the name it was paired as, jo's
        // laptop for one the yard started with.
        let me = token
            .as_ref()
            .and_then(|token| shared.state.lock().unwrap().who.get(token).cloned())
            .map_or(ME.to_string(), |(user, device)| {
                format!(r#"{{"user": "{user}", "said_by": "{device}", "yard": "south"}}"#)
            });
        if target == "/user/pair" && pairing {
            if stalled(shared, &target) {
                hold_until_stopped(shared);
                return Ok(());
            }
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
            // As the yard refuses a token it does not know: plain
            // `Unauthorized` with `WWW-Authenticate: Bearer`, which the
            // edge passes through byte for byte, adding nothing.
            _ => {
                let body = "Unauthorized\n";
                write!(
                    conn,
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )?;
                return conn.flush();
            }
        }
        if target == "/me" {
            return plain(&mut conn, "200 OK", "application/json", &me);
        }
    }
    // The stream of a device ends when the device is removed.
    let token = token.filter(|_| listener);

    // A stalled request is read and never answered: the connection is held
    // open until the yard stops, so the client's read gives up for its
    // timeout. A stalled call was still asked.
    if stalled(shared, &target) {
        if let Some(method) = target.strip_prefix("/call/") {
            let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            shared
                .state
                .lock()
                .unwrap()
                .asked
                .push((method.to_string(), args));
        }
        hold_until_stopped(shared);
        return Ok(());
    }

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
        state.asked.push((method.clone(), args.clone()));
        // A held call is asked and not answered yet: it waits here, with the
        // lock let go, for the test to let it through.
        if state.hold_calls.contains(&method) {
            state.calls_held.push(method.clone());
            while !state.stopped && state.hold_calls.contains(&method) {
                state = shared.changed.wait_timeout(state, LOOK).unwrap().0;
            }
            if let Some(at) = state.calls_held.iter().position(|m| *m == method) {
                state.calls_held.remove(at);
            }
        }
        let by_arg = state
            .answers_for
            .iter()
            .find(|((m, arg, is), _)| *m == method && args[arg.as_str()] == is.as_str())
            .map(|(_, reply)| reply.clone());
        if state.silent.contains(&method) {
            return Ok(());
        }
        // As the yard takes the arguments of `ApproveAsk`, where it has the
        // call: at its listener by name, as every call's there, and on its
        // socket in their order, since by name the socket takes only the
        // calls its API page names.
        let misshapen = method == "ApproveAsk"
            && state.answers.contains_key(&method)
            && args.is_object() != listener;
        let refused = misshapen.then(|| {
            let why = if listener {
                "arguments: over the network ApproveAsk takes them as a JSON object, by name"
            } else {
                "arguments: ApproveAsk takes them as a JSON array: only the calls docs/api.md names take them by name"
            };
            Reply {
                status: 200,
                body: serde_json::json!({"error": {"code": "error", "message": why}}).to_string(),
                chunk: None,
            }
        });
        let once = state.once.get_mut(&method).and_then(VecDeque::pop_front);
        let held = (method == "Conversation")
            .then(|| conversation(&state, &args))
            .flatten();
        refused
            .or(once)
            .or(by_arg)
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

/// How often a stream that has nothing to send looks whether its client is
/// still there.
const LOOK: Duration = Duration::from_millis(50);

/// The stream: the events with a seq after `after`, then every one as the
/// test emits it, until the yard stops or the client hangs up. A real yard
/// sees the hang-up and ends the stream, and a client that stops its own
/// stream by shutting its end down could rely on that: where shutting down
/// wakes no reader blocked on the same socket (Windows) it is the yard's
/// closing that does. A client may not rely on it, so a test of stopping
/// has [`Yard::hold_streams`] keep the stream open.
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
    let opened = state.streams.len();
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
            if seq > after && opened >= state.muted {
                let line = format!("{event}\n");
                write!(conn, "{:x}\r\n{line}\r\n", line.len())?;
                conn.flush()?;
                state.events_sent += 1;
            }
        }
        // Cut off, as a server that is killed cuts it: no last chunk. So is
        // the stream of a device that was removed.
        let removed = device.as_ref().is_some_and(|d| !state.devices.contains(d));
        if state.stopped || removed || opened < state.ended {
            return Ok(());
        }
        state = shared.changed.wait_timeout(state, LOOK).unwrap().0;
        if !state.held && conn.hung_up() {
            return Ok(());
        }
    }
}

/// The session token a stand-in edge lets in, in the tests and the
/// screenshots.
#[cfg(any(test, debug_assertions))]
pub const SESSION: &str = "kratos-dev-session";

/// The Kratos account the stand-in edge signs in, and its password: what
/// the hosted tests sign in with, and the signed-out shot's account.
#[cfg(any(test, debug_assertions))]
pub const KRATOS_EMAIL: &str = "jo@example.net";
#[cfg(any(test, debug_assertions))]
pub const KRATOS_PASSWORD: &str = "sign-me-in-1";
/// The handle Kratos answers the account has.
#[cfg(any(test, debug_assertions))]
pub const KRATOS_HANDLE: &str = "jo";
/// The login flow the stand-in answers: any other is expired.
#[cfg(any(test, debug_assertions))]
const KRATOS_FLOW: &str = "test-flow-id";

/// What the edge answers a session it does not know: its own 401, naming
/// the edge, carrying `X-Edge-Refusal: session`. The yard's own 401 it
/// passes through byte for byte, adding nothing.
#[cfg(any(test, debug_assertions))]
const NO_SESSION: &str = "{\"error\":\"edge: no Kratos session; sign in first\"}\n";

#[cfg(any(test, debug_assertions))]
#[derive(Clone)]
struct EdgeRefusal {
    status: u16,
    /// The `X-Edge-Refusal` value the edge answers with: `away` for 503,
    /// `busy` for 429, none for 403 and 504.
    refusal: Option<String>,
    /// The router's sentence after its "edge: " tag.
    why: String,
}

#[cfg(any(test, debug_assertions))]
struct EdgeShared {
    sessions: Mutex<Vec<String>>,
    last_request: Mutex<String>,
    /// How many requests were proxied to the yard.
    proxied: Mutex<usize>,
    /// The open relays, each with the session it was proxied for.
    relays: Mutex<Vec<(String, TcpStream)>>,
    /// How many logins to answer expired before one succeeds: a flow the
    /// edge forgot is opened again.
    expire: Mutex<u32>,
    /// How many sessions a sign-in minted: each is known from then on.
    logins: Mutex<u32>,
    /// A pause before login answers, so the window can be checked while
    /// its request is still in flight.
    login_delay: Mutex<Duration>,
    /// The router's refusal past the session check, where one stands in
    /// for every request: no ready yard (403), no live link (503), no
    /// answer in time (504), or past what the link holds (429). None
    /// where every request past the session check is proxied.
    edge_refusal: Mutex<Option<EdgeRefusal>>,
    /// The paths a request for which is turned away with the 401 of the
    /// yard's web gateway, reaching no yard.
    turned_away: Mutex<Vec<String>>,
    /// The yard behind the edge, for the pairings the edge opens as a
    /// member.
    yard: Arc<Shared>,
    /// The sessions that are active members of the yard, each with the
    /// yard user the yard maps it to ([`Router::member`]).
    members: Mutex<HashMap<String, String>>,
    /// The yard user a sign-in's session is a member as, where the account
    /// that signs in is a member ([`Router::member_signs_in`]).
    login_member: Mutex<Option<String>>,
    /// The head of every request that came, in order.
    heads: Mutex<Vec<String>>,
    /// The pairing the edge opened and the session it opened it for: the
    /// code stands for that session alone.
    opener: Mutex<Option<(String, String)>>,
    stopped: Mutex<bool>,
}

/// What the yard's web view answers a request it is sent with no minted
/// token: `http.Error` of internal/web's JWT gateway, 401 with the plain
/// sentence, and none of the listener's `WWW-Authenticate: Bearer`. The
/// edge's router sends a hosted yard's stream down that socket by mistake
/// (yardr-hosted-c0sb).
#[cfg(any(test, debug_assertions))]
const GATEWAY_REFUSAL: &str =
    "the gateway's token is missing: the X-Edge-JWT header is not on the request\n";

/// A stand-in for the hosted edge in front of a yard at its listener: it
/// checks `X-Edge-Token` as the router checks the Kratos session, proxies
/// a known session to the yard with the credential stripped, and refuses
/// any other with the edge's own 401, reaching no yard. Where the edge
/// refuses every request it answers its own 403, 503, 504 or 429 past the
/// session check, and a path
/// it is told to turn away it answers with the web gateway's 401 (a hosted
/// vps yard's stream, sent down the wrong socket). Under
/// `/kratos` it answers the login API flow, minting a session a sign-in
/// for the account it knows. The yard answers the device token as ever; a test
/// stops the yard before the edge, so no relayed stream outlives what it
/// relays.
#[cfg(any(test, debug_assertions))]
pub struct Router {
    /// `http://127.0.0.1:<port>`.
    address: String,
    shared: Arc<EdgeShared>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

#[cfg(any(test, debug_assertions))]
impl Router {
    /// An edge in front of `yard`, which lets `session` in.
    pub fn start(yard: &Yard, session: &str) -> Router {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let target = yard
            .address()
            .strip_prefix("http://")
            .unwrap_or_else(|| yard.address())
            .to_string();
        let shared = Arc::new(EdgeShared {
            sessions: Mutex::new(vec![session.to_string()]),
            last_request: Mutex::new(String::new()),
            proxied: Mutex::new(0),
            relays: Mutex::new(Vec::new()),
            expire: Mutex::new(0),
            logins: Mutex::new(0),
            login_delay: Mutex::new(Duration::ZERO),
            edge_refusal: Mutex::new(None),
            turned_away: Mutex::new(Vec::new()),
            yard: yard.shared.clone(),
            members: Mutex::new(HashMap::new()),
            login_member: Mutex::new(None),
            heads: Mutex::new(Vec::new()),
            opener: Mutex::new(None),
            stopped: Mutex::new(false),
        });
        let accept = thread::spawn({
            let shared = shared.clone();
            let origin = address.clone();
            move || {
                while !*shared.stopped.lock().unwrap() {
                    match listener.accept() {
                        Ok((conn, _)) => {
                            let conn = at_once(conn);
                            let shared = shared.clone();
                            let target = target.clone();
                            let origin = origin.clone();
                            thread::spawn(move || {
                                let _ = serve_edge(conn, &target, &origin, &shared);
                            });
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            }
        });
        Router {
            address,
            shared,
            accept: Mutex::new(Some(accept)),
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// A client at the edge's address with this device token and session,
    /// for the account's own yard: no yard is named.
    pub fn client_with(&self, token: &str, session: &str) -> Client {
        self.client_with_yard(token, session, "")
    }

    /// A client at the edge's address with this device token and session,
    /// naming which of the account's yards with each call.
    pub fn client_with_yard(&self, token: &str, session: &str, yard: &str) -> Client {
        Client::remote_hosted(
            Address::parse(self.address()).unwrap(),
            token,
            session,
            yard,
        )
    }

    /// The next `n` logins answer expired, as a flow the edge forgot: the
    /// client opens another.
    pub fn expire_logins(&self, n: u32) {
        *self.shared.expire.lock().unwrap() = n;
    }

    /// Holds login answers for `delay`, so an in-flight request can be
    /// observed in a window test.
    pub fn delay_logins(&self, delay: Duration) {
        *self.shared.login_delay.lock().unwrap() = delay;
    }

    /// The edge knows this session from now on.
    pub fn allow(&self, session: &str) {
        self.shared
            .sessions
            .lock()
            .unwrap()
            .push(session.to_string());
    }

    /// This session is an active member of the yard behind the edge, acting
    /// there as `user`: the edge opens a pairing for a device of its own at
    /// that user (`POST /user/pairing`, which the edge answers itself, as
    /// the router does), and the code it gives redeems in this session
    /// alone. A session that is no member is refused, and reaches no yard.
    pub fn member(&self, session: &str, user: &str) {
        self.shared
            .members
            .lock()
            .unwrap()
            .insert(session.to_string(), user.to_string());
    }

    /// The account the stand-in signs in is an active member of the yard,
    /// acting there as `user`: every session a sign-in mints from now on is
    /// a member's, as [`Router::member`] makes one.
    pub fn member_signs_in(&self, user: &str) {
        *self.shared.login_member.lock().unwrap() = Some(user.to_string());
    }

    /// The edge knows this session no more, as Kratos after a revocation:
    /// the next request with it is refused, and what it relayed is cut,
    /// as a removed device's streams are.
    pub fn revoke(&self, session: &str) {
        self.shared
            .sessions
            .lock()
            .unwrap()
            .retain(|known| known != session);
        let mut relays = self.shared.relays.lock().unwrap();
        for (_, live) in relays.iter().filter(|(known, _)| known == session) {
            let _ = live.shutdown(std::net::Shutdown::Both);
        }
        relays.retain(|(known, _)| known != session);
    }

    /// No yard is ready behind the edge: every request past the session
    /// check is refused with the edge's own 403, as the router's noYard
    /// for a yard still building, stopped or failed, or a name of no
    /// yard of the account, reaching no yard. `why` is the router's
    /// sentence after its "edge: " tag.
    pub fn no_yard(&self, why: &str) {
        *self.shared.edge_refusal.lock().unwrap() = Some(EdgeRefusal {
            status: 403,
            refusal: None,
            why: why.to_string(),
        });
    }

    /// No link is up behind the edge: every request past the session
    /// check is refused with the edge's own 503, as the router's
    /// refuseCall for a yard that is away, reaching no yard. `why` is
    /// the router's sentence after its "edge: " tag.
    pub fn away(&self, why: &str) {
        *self.shared.edge_refusal.lock().unwrap() = Some(EdgeRefusal {
            status: 503,
            refusal: Some("away".to_string()),
            why: why.to_string(),
        });
    }

    /// The yard gave no answer in time: every request past the session
    /// check is refused with the edge's own 504, as the router's
    /// refuseCall, reaching no yard further. `why` is the router's
    /// sentence after its "edge: " tag.
    pub fn no_answer(&self, why: &str) {
        *self.shared.edge_refusal.lock().unwrap() = Some(EdgeRefusal {
            status: 504,
            refusal: None,
            why: why.to_string(),
        });
    }

    /// Past what the link holds: every request past the session check
    /// is refused with the edge's own 429, as the router's refuseCall
    /// for a link holding as many open calls and streams as the edge
    /// carries, reaching no yard. `why` is the router's sentence after
    /// its "edge: " tag.
    pub fn busy(&self, why: &str) {
        *self.shared.edge_refusal.lock().unwrap() = Some(EdgeRefusal {
            status: 429,
            refusal: Some("busy".to_string()),
            why: why.to_string(),
        });
    }

    /// A request for `path` (as the edge has it, `/user/events`, the query
    /// aside) is answered as the yard's web view answers a request with no
    /// minted token: 401, plain words, no `WWW-Authenticate`, and the
    /// listener hears nothing of it. Every other request passes, as the
    /// calls of a hosted vps yard do while its stream is turned away.
    pub fn turn_away(&self, path: &str) {
        self.shared
            .turned_away
            .lock()
            .unwrap()
            .push(path.to_string());
    }

    /// Every path that was turned away passes again.
    pub fn let_through(&self) {
        self.shared.turned_away.lock().unwrap().clear();
    }

    /// The head of the last request, as it came to the edge.
    pub fn last_request(&self) -> String {
        self.shared.last_request.lock().unwrap().clone()
    }

    /// The heads of the requests that came to the edge, oldest first, whose
    /// first line starts with `line`: `POST /user/pairing `, say.
    pub fn seen(&self, line: &str) -> Vec<String> {
        let heads = self.shared.heads.lock().unwrap();
        heads
            .iter()
            .filter(|head| head.starts_with(line))
            .cloned()
            .collect()
    }

    /// How many requests reached the yard through the edge.
    pub fn proxied(&self) -> usize {
        *self.shared.proxied.lock().unwrap()
    }

    /// The edge goes away: nothing listens any more.
    pub fn stop(&self) {
        *self.shared.stopped.lock().unwrap() = true;
        if let Some(accept) = self.accept.lock().unwrap().take() {
            accept.join().unwrap();
        }
    }
}

#[cfg(any(test, debug_assertions))]
impl Drop for Router {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One request at the edge: under `/kratos` the login API flow, which
/// takes no session, else refused with the edge's own 401 where the
/// session is none it knows, else proxied to the yard with the credential
/// stripped and the yard's answer relayed byte for byte.
#[cfg(any(test, debug_assertions))]
fn serve_edge(conn: TcpStream, target: &str, origin: &str, shared: &EdgeShared) -> io::Result<()> {
    // The listener is nonblocking so it can be stopped, and what it
    // accepts inherits that: serve blocking, as the yard does, or a
    // request read before its bytes arrive hangs up instead of waiting
    // for them.
    conn.set_nonblocking(false)?;
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut conn = conn;
    let mut head = String::new();
    let mut length = 0usize;
    let mut session = None;
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
            } else if name.eq_ignore_ascii_case("x-edge-token") {
                session = Some(value.trim().to_string());
            }
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    *shared.last_request.lock().unwrap() = head.clone();
    shared.heads.lock().unwrap().push(head.clone());
    let asked = head.lines().next().unwrap_or_default().to_string();
    if asked.starts_with("GET /kratos/self-service/login/api ")
        || asked.starts_with("POST /kratos/self-service/login")
    {
        return serve_kratos(conn, origin, &asked, &body, shared);
    }
    let known = session
        .as_ref()
        .is_some_and(|token| shared.sessions.lock().unwrap().contains(token));
    if !known {
        return refuse_session(&mut conn);
    }
    // Past the session check, the edge's own refusal where one stands
    // in: 403 with no header, 503 with away, 504 with none, 429 with
    // busy. The yard hears nothing of the request.
    if let Some(refusal) = shared.edge_refusal.lock().unwrap().clone() {
        let body = format!("{{\"error\":\"edge: {}\"}}\n", refusal.why);
        let reason = match refusal.status {
            403 => "Forbidden",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            429 => "Too Many Requests",
            _ => "Error",
        };
        let header = refusal
            .refusal
            .map(|r| format!("X-Edge-Refusal: {r}\r\n"))
            .unwrap_or_default();
        write!(
            conn,
            "HTTP/1.1 {} {reason}\r\n{header}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            refusal.status,
            body.len()
        )?;
        return conn.flush();
    }
    let asker = session.clone().unwrap_or_default();
    // A member's request for a pairing is the edge's own to answer, as the
    // router answers it: no part of it is the yard's path.
    if asked.starts_with("POST /user/pairing ") {
        return serve_pairing(conn, &head, &body, &asker, shared);
    }
    // A pairing's code presented in a session other than the one it was
    // opened for is refused and spent, and the yard hears nothing of it.
    if asked.starts_with("POST /user/pair ") {
        let typed = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|body| body["code"].as_str().map(code_of))
            .unwrap_or_default();
        let mut opener = shared.opener.lock().unwrap();
        if let Some((code, opened_for)) = opener.clone()
            && code_of(&code) == typed
        {
            *opener = None;
            if opened_for != asker {
                drop(opener);
                let mut yard = shared.yard.state.lock().unwrap();
                yard.pairing = None;
                yard.pairing_for = None;
                drop(yard);
                let body = "{\"error\":\"edge: that pairing code was opened for another account; it is spent — open a new pairing\"}\n";
                write!(
                    conn,
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )?;
                return conn.flush();
            }
        }
    }
    let path = asked.split_whitespace().nth(1).unwrap_or("");
    let path = path.split('?').next().unwrap_or(path);
    if shared.turned_away.lock().unwrap().iter().any(|p| p == path) {
        write!(
            conn,
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain; charset=utf-8\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\n\r\n{GATEWAY_REFUSAL}",
            GATEWAY_REFUSAL.len()
        )?;
        return conn.flush();
    }
    *shared.proxied.lock().unwrap() += 1;
    // The Kratos credential never rides on: the yard verifies devices,
    // not the account's session.
    let mut head: String = head
        .lines()
        .filter(|line| !line.is_empty())
        .filter(|line| {
            line.split_once(':')
                .is_none_or(|(name, _)| !name.eq_ignore_ascii_case("x-edge-token"))
        })
        .map(|line| format!("{line}\r\n"))
        .collect();
    head.push_str("\r\n");
    relay(
        conn,
        &head,
        &body,
        target,
        &session.unwrap_or_default(),
        shared,
    )
}

/// One request of the login API flow: a flow is opened, and the
/// password method answers it. A sign-in for the account the edge knows
/// mints a session, which is known from then on; any other is refused as
/// Kratos refuses it, in the flow with its messages. The flow's own action
/// names another flow than the one opened: the client answers the flow it
/// opened, never the action's.
#[cfg(any(test, debug_assertions))]
fn serve_kratos(
    mut conn: TcpStream,
    origin: &str,
    asked: &str,
    body: &[u8],
    shared: &EdgeShared,
) -> io::Result<()> {
    let reply = |conn: &mut TcpStream, status: &str, body: String| -> io::Result<()> {
        write!(
            conn,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        conn.flush()
    };
    let action = format!("{origin}/kratos/self-service/login?flow=another-flow");
    if asked.starts_with("GET ") {
        let flow = serde_json::json!({
            "id": KRATOS_FLOW,
            "type": "api",
            "ui": {"action": action, "method": "POST", "nodes": []},
        });
        return reply(&mut conn, "200 OK", flow.to_string());
    }
    let expired = {
        let mut expire = shared.expire.lock().unwrap();
        match *expire {
            0 => false,
            n => {
                *expire = n - 1;
                true
            }
        }
    };
    let flow = asked
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .split("flow=")
        .nth(1)
        .unwrap_or_default()
        .split('&')
        .next()
        .unwrap_or_default();
    if expired || flow != KRATOS_FLOW {
        let gone = serde_json::json!({"error": {"code": 410, "status": "Gone", "message": "The login flow expired."}});
        return reply(&mut conn, "410 Gone", gone.to_string());
    }
    let answered: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let good = answered["method"].as_str() == Some("password")
        && answered["identifier"].as_str() == Some(KRATOS_EMAIL)
        && answered["password"].as_str() == Some(KRATOS_PASSWORD);
    let delay = *shared.login_delay.lock().unwrap();
    thread::sleep(delay);
    if !good {
        let refused = serde_json::json!({
            "id": KRATOS_FLOW,
            "type": "api",
            "ui": {
                "action": action,
                "method": "POST",
                "messages": [{"id": 4000006, "text": "The provided credentials are invalid, check for spelling mistakes in your password or username, email address, or phone number.", "type": "error"}],
                "nodes": [],
            },
        });
        return reply(&mut conn, "400 Bad Request", refused.to_string());
    }
    let mut logins = shared.logins.lock().unwrap();
    *logins += 1;
    let session = format!("kratos-test-session-{}", *logins);
    shared.sessions.lock().unwrap().push(session.clone());
    if let Some(user) = shared.login_member.lock().unwrap().clone() {
        shared.members.lock().unwrap().insert(session.clone(), user);
    }
    let signed = serde_json::json!({
        "session_token": session,
        "session": {
            "id": "test-session-id",
            "identity": {
                "id": "test-identity-id",
                "traits": {"email": KRATOS_EMAIL, "handle": KRATOS_HANDLE},
            },
        },
    });
    reply(&mut conn, "200 OK", signed.to_string())
}

/// The edge's own 401 for a session it does not know, naming the edge and
/// carrying `X-Edge-Refusal: session`.
#[cfg(any(test, debug_assertions))]
fn refuse_session(conn: &mut TcpStream) -> io::Result<()> {
    write!(
        conn,
        "HTTP/1.1 401 Unauthorized\r\nX-Edge-Refusal: session\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{NO_SESSION}",
        NO_SESSION.len()
    )?;
    conn.flush()
}

/// The value of the request's header `name`, from the head as it came.
#[cfg(any(test, debug_assertions))]
fn header_of(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (head_name, value) = line.split_once(':')?;
        head_name
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

/// `POST /user/pairing`, which the edge answers itself: a session that is an
/// active member of the yard has a pairing opened at the yard for a device
/// of its own, for the yard user the yard maps it to, and the code stands
/// for that session alone. Any other session is refused as holding no such
/// yard, and a request while the yard has a pairing open is told whose it is.
/// A device's token never opens one: a request that carries it, in
/// `Authorization` or in place of the session, is refused as one with no
/// session, and the yard hears nothing.
#[cfg(any(test, debug_assertions))]
fn serve_pairing(
    mut conn: TcpStream,
    head: &str,
    body: &[u8],
    session: &str,
    shared: &EdgeShared,
) -> io::Result<()> {
    let reply = |conn: &mut TcpStream, status: &str, body: Value| -> io::Result<()> {
        let body = format!("{body}\n");
        write!(
            conn,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
        conn.flush()
    };
    if header_of(head, "authorization").is_some() {
        return refuse_session(&mut conn);
    }
    let member = shared.members.lock().unwrap().get(session).cloned();
    let Some(user) = member else {
        let why = format!("edge: {}", crate::api::HOLDS_NO_YARD);
        return reply(
            &mut conn,
            "403 Forbidden",
            serde_json::json!({ "error": why }),
        );
    };
    let device = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| body["device"].as_str().map(str::to_string))
        .unwrap_or_default();
    let yard = header_of(head, "x-edge-yard").unwrap_or_default();
    match open_for(&shared.yard, &user, &device) {
        Ok(code) => {
            *shared.opener.lock().unwrap() = Some((code.clone(), session.to_string()));
            let opened = serde_json::json!({
                "code": code, "user": user, "device": device, "yard": yard,
            });
            reply(&mut conn, "200 OK", opened)
        }
        Err(holder) => {
            let why =
                format!("edge: a pairing is already open for {holder}, try again in a few minutes");
            reply(
                &mut conn,
                "409 Conflict",
                serde_json::json!({ "error": why }),
            )
        }
    }
}

/// One request proxied to the yard, its answer relayed byte for byte. The
/// relay is cut when its session is revoked, and forgotten when it ends.
#[cfg(any(test, debug_assertions))]
fn relay(
    mut conn: TcpStream,
    head: &str,
    body: &[u8],
    target: &str,
    session: &str,
    shared: &EdgeShared,
) -> io::Result<()> {
    let peer = conn.peer_addr().ok();
    shared
        .relays
        .lock()
        .unwrap()
        .push((session.to_string(), conn.try_clone()?));
    let result = relay_inner(&mut conn, head, body, target);
    if let Some(peer) = peer {
        shared
            .relays
            .lock()
            .unwrap()
            .retain(|(_, live)| live.peer_addr().ok() != Some(peer));
    }
    result
}

#[cfg(any(test, debug_assertions))]
fn relay_inner(conn: &mut TcpStream, head: &str, body: &[u8], target: &str) -> io::Result<()> {
    let mut yard = at_once(TcpStream::connect(target)?);
    yard.write_all(head.as_bytes())?;
    yard.write_all(body)?;
    yard.flush()?;
    let mut yard = BufReader::new(yard);
    loop {
        let mut line = String::new();
        if yard.read_line(&mut line)? == 0 {
            return Ok(());
        }
        conn.write_all(line.as_bytes())?;
        if line == "\r\n" {
            break;
        }
    }
    conn.flush()?;
    io::copy(&mut yard, conn)?;
    Ok(())
}

/// What a [`Posthog`] keeps of the requests and answers them with.
#[cfg(any(test, debug_assertions))]
struct PosthogShared {
    /// The head and the body of each request, in the order they came.
    sent: Mutex<Vec<(String, String)>>,
    /// What every request is answered with.
    status: Mutex<u16>,
    stopped: Mutex<bool>,
}

/// PostHog's capture for the tests and the shots, which is no yard: a port of
/// this machine that reads each request whole, keeps it, and answers every
/// one with the status it was told to, 200 until then. It checks nothing of
/// a request: the test reads what came ([`Posthog::sent`]), so that the
/// real host and the real key are never needed.
#[cfg(any(test, debug_assertions))]
pub struct Posthog {
    /// `http://127.0.0.1:<port>`.
    address: String,
    shared: Arc<PosthogShared>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

#[cfg(any(test, debug_assertions))]
impl Posthog {
    pub fn start() -> Posthog {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let shared = Arc::new(PosthogShared {
            sent: Mutex::new(Vec::new()),
            status: Mutex::new(200),
            stopped: Mutex::new(false),
        });
        let accept = thread::spawn({
            let shared = shared.clone();
            move || {
                while !*shared.stopped.lock().unwrap() {
                    match listener.accept() {
                        Ok((conn, _)) => {
                            let conn = at_once(conn);
                            let shared = shared.clone();
                            thread::spawn(move || {
                                let _ = serve_posthog(conn, &shared);
                            });
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            }
        });
        Posthog {
            address,
            shared,
            accept: Mutex::new(Some(accept)),
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// Every request from now on is answered with `status`.
    pub fn answer(&self, status: u16) {
        *self.shared.status.lock().unwrap() = status;
    }

    /// The head and the body of each request that came, in order.
    pub fn sent(&self) -> Vec<(String, String)> {
        self.shared.sent.lock().unwrap().clone()
    }

    /// Nothing listens any more.
    pub fn stop(&self) {
        *self.shared.stopped.lock().unwrap() = true;
        if let Some(accept) = self.accept.lock().unwrap().take() {
            accept.join().unwrap();
        }
    }
}

#[cfg(any(test, debug_assertions))]
impl Drop for Posthog {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One request at the capture: kept whole, and answered.
#[cfg(any(test, debug_assertions))]
fn serve_posthog(conn: TcpStream, shared: &PosthogShared) -> io::Result<()> {
    // The listener is nonblocking so it can be stopped, and what it
    // accepts inherits that: served blocking, as the yard is.
    conn.set_nonblocking(false)?;
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut conn = conn;
    let mut head = String::new();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    shared
        .sent
        .lock()
        .unwrap()
        .push((head, String::from_utf8_lossy(&body).into_owned()));
    let status = *shared.status.lock().unwrap();
    let answer = r#"{"status":1}"#;
    write!(
        conn,
        "HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
        answer.len()
    )?;
    conn.flush()
}

/// How many bytes of `content` the stand-in takes in one append. The
/// reference limits an append to 500 tokens and a token is at least a byte,
/// so a client under this is under that, and one over it is told so.
#[cfg(any(test, debug_assertions))]
const LIVE_APPEND_BYTES: usize = 500;

/// What the model says by itself when a delegation is not answered in its
/// acknowledgement delay (the 'Lemme check.' of runs 3 and 4).
#[cfg(any(test, debug_assertions))]
const LIVE_FILLER: &str = "Lemme check.";

/// The built-in voices a session may start in, and the one it speaks in when
/// it names none: the Live WebSocket reference's, under `session.start`
/// (https://developers.openai.com/api/reference/resources/live/primary-websocket,
/// read 3 Nov 2026), where `audio.output.voice` is `"alloy"`, `"ash"`,
/// `"ballad"` "or 28 more", or a custom voice's id of 1 to 128 characters,
/// and "defaults to `marin`". Written out here from the reference and not
/// taken from the module that starts the session, so that a name that module
/// offers and the provider does not know is a refusal in a test.
#[cfg(any(test, debug_assertions))]
const LIVE_VOICES: [&str; 31] = [
    "alloy", "ash", "ballad", "beacon", "bossa", "brise", "cedar", "cinder", "coral", "delta",
    "echo", "flitz", "gleam", "harema", "juni", "marin", "meridian", "nira", "noeul", "nuri",
    "quartz", "ripple", "sage", "shida", "shimmer", "sillage", "stone", "tempo", "verse", "vesper",
    "willow",
];
#[cfg(any(test, debug_assertions))]
const LIVE_DEFAULT_VOICE: &str = "marin";

/// An append a [`Live`] took.
#[cfg(any(test, debug_assertions))]
#[derive(Clone, Debug, PartialEq)]
pub struct LiveAppend {
    /// `commentary` or `thinking`.
    pub kind: String,
    /// The id the client gave it, which the stand-in's acknowledgement names.
    pub event_id: String,
    /// The delegation it came under: none for general context.
    pub delegation_id: Option<String>,
    pub content: String,
}

#[cfg(any(test, debug_assertions))]
#[derive(Default)]
struct LiveState {
    /// Every event the client sent, as JSON, in order.
    sent: Vec<Value>,
    /// What the server sends next, as text, in order.
    said: VecDeque<String>,
    appends: Vec<LiveAppend>,
    /// The delegations raised, by the ids the stand-in gave them.
    delegations: Vec<String>,
    /// The delegations whose automatic acknowledgement is still to come.
    unanswered: Vec<String>,
    expires_at: i64,
    started: bool,
    closed: bool,
    events: u64,
    /// The speech the server has queued and not sent.
    audio: VecDeque<Vec<i16>>,
    dropped: usize,
}

#[cfg(any(test, debug_assertions))]
impl LiveState {
    /// Queues an event of the server's, with an `event_id` of its own.
    fn say(&mut self, mut event: Value) {
        self.events += 1;
        event["event_id"] = serde_json::json!(format!("evt_{}", self.events));
        self.said.push_back(event.to_string());
    }

    /// An `error` that names the client's event, where it had an id.
    fn refuse(&mut self, client_event_id: Option<String>, message: &str) {
        let mut error = serde_json::json!({
            "type": "invalid_request_error",
            "code": "invalid_event",
            "message": message,
        });
        if let Some(id) = client_event_id {
            error["client_event_id"] = id.into();
        }
        self.say(serde_json::json!({ "type": "error", "error": error }));
    }

    fn start(&mut self, event: &Value, id: Option<String>) {
        let session = &event["session"];
        let format = &session["audio"]["format"];
        let voice = &session["audio"]["output"]["voice"];
        let rate = format["rate"].as_u64().unwrap_or(24_000);
        let delay = session["delegation"]["acknowledgement_delay_ms"]
            .as_u64()
            .unwrap_or(0);
        let problem = if self.started {
            Some("The session has started already.")
        } else if !(format.is_null()
            || (format["type"] == "audio/pcm" && [16_000, 24_000].contains(&rate)))
        {
            Some("Audio is PCM at 16000 or 24000 Hz.")
        } else if session["delegation"]["type"] != "client" {
            Some("This stand-in delegates to the client alone.")
        } else if delay > 800 {
            Some("acknowledgement_delay_ms is 0 to 800.")
        } else {
            None
        };
        if let Some(problem) = problem {
            return self.refuse(id, problem);
        }
        // A voice is one of the reference's names or a custom voice's id, or
        // none at all, which is the provider's own; the refusal says which
        // value it would not take.
        let known = voice.is_null()
            || voice
                .as_str()
                .is_some_and(|name| LIVE_VOICES.contains(&name))
            || voice["id"]
                .as_str()
                .is_some_and(|id| (1..=128).contains(&id.len()));
        if !known {
            let shown = voice
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| voice.to_string());
            return self.refuse(
                id,
                &format!(
                    "Invalid value: '{shown}'. 'session.audio.output.voice' is a built-in voice or a custom voice's id."
                ),
            );
        }
        self.started = true;
        let started = serde_json::json!({
            "type": "session.started",
            "client_event_id": id,
            "session": {
                "id": "live_fake",
                "model": "gpt-live-1",
                "status": "active",
                "expires_at": self.expires_at,
                "instructions": session["instructions"],
                "input": [],
                "audio": {
                    "format": { "type": "audio/pcm", "rate": rate },
                    "output": {
                        "voice": if voice.is_null() { serde_json::json!(LIVE_DEFAULT_VOICE) } else { voice.clone() },
                    },
                },
                "delegation": { "type": "client", "acknowledgement_delay_ms": delay },
            },
        });
        self.say(started);
    }

    /// A `session.commentary.append` or `session.thinking.append`, taken as
    /// the reference has it: `content` (not `text`), a `delegation_id` that is
    /// there, and null or an id a delegation of this session gave.
    fn append(&mut self, kind: &str, event: &Value, id: Option<String>) {
        let content = event["content"].as_str();
        let delegation = &event["delegation_id"];
        let problem = if !self.started {
            Some("The session has not started.")
        } else if event.get("delegation_id").is_none() {
            Some("delegation_id is required: null for general context.")
        } else if !(delegation.is_null() || delegation.is_string()) {
            Some("delegation_id is a string or null.")
        } else if delegation
            .as_str()
            .is_some_and(|id| !self.delegations.iter().any(|known| known == id))
        {
            Some("delegation_id names no delegation of this session.")
        } else if content.is_none() {
            Some("content is required: the field is content, not text.")
        } else if content.is_some_and(|content| content.len() > LIVE_APPEND_BYTES) {
            Some("content is over 500 tokens.")
        } else {
            None
        };
        if let Some(problem) = problem {
            return self.refuse(id, problem);
        }
        self.appends.push(LiveAppend {
            kind: kind.to_string(),
            event_id: id.clone().unwrap_or_default(),
            delegation_id: delegation.as_str().map(str::to_string),
            content: content.unwrap_or_default().to_string(),
        });
        // Any thinking or commentary append cancels every acknowledgement
        // still to come, those of other delegations too.
        self.unanswered.clear();
        self.say(serde_json::json!({
            "type": format!("session.{kind}.appended"),
            "client_event_id": id,
            "start_ms": 0,
            "end_ms": 0,
        }));
    }

    fn close(&mut self, reason: &str, id: Option<String>) {
        self.closed = true;
        self.say(serde_json::json!({
            "type": "session.closed",
            "client_event_id": id,
            "reason": reason,
            "usage": { "seconds": 1.5 },
            "session": {},
        }));
    }
}

/// OpenAI's Live socket for the tests and the voice screenshot: the server's
/// side of one session as the events that cross it, text for text, with no
/// socket in between (the WebSocket client is the session's, and has its own
/// tests; `voice::stand_in` serves this over one). The test hands it what the
/// client sends ([`Live::receive`]), scripts what the server says
/// ([`Live::person_says`], [`Live::voice_says`], [`Live::delegate`],
/// [`Live::queue_audio`]) and reads what it says back ([`Live::take`]) and
/// what it kept ([`Live::sent`], [`Live::appends`]).
///
/// It holds a client to the reference and not to the harness: a start in a
/// rate the socket does not take, or in a voice that is neither a built-in
/// one nor a custom voice's id, an append with `text` for `content`, one
/// with no `delegation_id`, one under an id no delegation of the session
/// gave, one over 500 bytes, each is answered with an `error` that names it
/// by `client_event_id`. A session says in `session.started` which voice it
/// speaks in: the one asked for, or the provider's own where none was. A
/// valid append is acknowledged by that id, and
/// cancels every acknowledgement still to come ([`Live::delay_passes`]). The
/// speech the server has queued can be dropped, as the server drops it when
/// the person speaks over it ([`Live::interrupt`]). No yard is asked
/// anything: this is the provider's, and the yard's fake is never involved.
#[cfg(any(test, debug_assertions))]
pub struct Live {
    state: Mutex<LiveState>,
}

#[cfg(any(test, debug_assertions))]
impl Live {
    pub fn start() -> Live {
        Live {
            state: Mutex::new(LiveState {
                expires_at: 1_788_555_600,
                ..LiveState::default()
            }),
        }
    }

    /// The moment, in Unix seconds, `session.started` will say the session
    /// ends.
    pub fn expires_at(&self, unix: i64) {
        self.state.lock().unwrap().expires_at = unix;
    }

    /// An event of the client's, as it sends it. Kept in [`Live::sent`], and
    /// answered as the server answers it: `session.start` with
    /// `session.started`, an append with its acknowledgement, `session.close`
    /// with `session.closed`; what is wrong with it with an `error`. Once the
    /// session is closed nothing is heard.
    pub fn receive(&self, event: &str) {
        let mut state = self.state.lock().unwrap();
        let Ok(event) = serde_json::from_str::<Value>(event) else {
            return state.refuse(None, "The event is not JSON.");
        };
        state.sent.push(event.clone());
        if state.closed {
            return;
        }
        let id = event["event_id"].as_str().map(str::to_string);
        match event["type"].as_str() {
            Some("session.start") => state.start(&event, id),
            Some(kind @ ("session.commentary.append" | "session.thinking.append")) => {
                let kind = kind
                    .strip_prefix("session.")
                    .and_then(|kind| kind.strip_suffix(".append"))
                    .unwrap_or(kind);
                state.append(kind, &event, id)
            }
            // The microphone's audio: heard, and nothing is made of it.
            Some("session.input_audio.append") => {}
            Some("session.close") => state.close("close_requested", id),
            other => state.refuse(id, &format!("Not an event of this stand-in: {other:?}.")),
        }
    }

    /// The person's words, a piece of them, from `start_ms` to `end_ms` on
    /// the session's timeline.
    pub fn person_says(&self, text: &str, start_ms: u64, end_ms: u64) {
        self.state.lock().unwrap().say(serde_json::json!({
            "type": "session.input_transcript.delta",
            "delta": text,
            "start_ms": start_ms,
            "end_ms": end_ms,
        }));
    }

    /// The voice's own words, a piece of them.
    pub fn voice_says(&self, text: &str, start_ms: u64, end_ms: u64) {
        self.state.lock().unwrap().say(serde_json::json!({
            "type": "session.output_transcript.delta",
            "delta": text,
            "start_ms": start_ms,
            "end_ms": end_ms,
        }));
    }

    /// The model delegates: `session.delegation.created` at `offset_ms` on the
    /// timeline, with an id of the stand-in's. It carries no words. The
    /// delegation's acknowledgement is due until an append comes.
    pub fn delegate(&self, offset_ms: u64) -> String {
        let mut state = self.state.lock().unwrap();
        let id = format!("item_fake{}", state.delegations.len() + 1);
        state.delegations.push(id.clone());
        state.unanswered.push(id.clone());
        state.say(serde_json::json!({
            "type": "session.delegation.created",
            "offset_ms": offset_ms,
            "delegation": { "id": id, "type": "delegation", "target": "client" },
        }));
        id
    }

    /// The acknowledgement delay passes. Each delegation no append has come
    /// for is acknowledged by the model itself, in a word of its own
    /// ('Lemme check.'), as in runs 3, 4 and 6: how many were.
    pub fn delay_passes(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        let due = std::mem::take(&mut state.unanswered);
        for _ in &due {
            state.say(serde_json::json!({
                "type": "session.output_transcript.delta",
                "delta": LIVE_FILLER,
                "start_ms": 0,
                "end_ms": 0,
            }));
        }
        due.len()
    }

    /// Speech the server has to say: a piece of PCM, mono 16-bit, queued and
    /// not sent until [`Live::play`].
    pub fn queue_audio(&self, pcm: Vec<i16>) {
        self.state.lock().unwrap().audio.push_back(pcm);
    }

    /// Sends up to `chunks` of the queued speech, as
    /// `session.output_audio.delta`, and says how many it sent.
    pub fn play(&self, chunks: usize) -> usize {
        let mut state = self.state.lock().unwrap();
        let mut sent = 0;
        while sent < chunks
            && let Some(pcm) = state.audio.pop_front()
        {
            let bytes: Vec<u8> = pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect();
            let delta = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
            state.say(serde_json::json!({
                "type": "session.output_audio.delta",
                "delta": delta,
            }));
            sent += 1;
        }
        sent
    }

    /// The person speaks over the voice, and the server stops: the speech it
    /// has queued and not sent is dropped, and how many chunks were is said.
    /// What it had sent stands sent.
    pub fn interrupt(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        let dropped = state.audio.len();
        state.audio.clear();
        state.dropped += dropped;
        dropped
    }

    /// The session's moment comes: the server closes it, as it says it will
    /// in `session.started`.
    pub fn expire(&self) {
        self.state.lock().unwrap().close("expired", None);
    }

    /// What the server has said since the last call, in order.
    pub fn take(&self) -> Vec<String> {
        self.state.lock().unwrap().said.drain(..).collect()
    }

    /// Every event the client sent, in order, as JSON.
    pub fn sent(&self) -> Vec<Value> {
        self.state.lock().unwrap().sent.clone()
    }

    /// The appends it took, in order.
    pub fn appends(&self) -> Vec<LiveAppend> {
        self.state.lock().unwrap().appends.clone()
    }

    /// How many chunks of queued speech were dropped by interruptions.
    pub fn dropped(&self) -> usize {
        self.state.lock().unwrap().dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;

    /// Opens the stream of `yard` as a client does, reads its head, hangs
    /// up on the client's side only (it still reads) and gives what the
    /// yard sends from then on, for as long as `wait`.
    fn hang_up_on(yard: &Yard, wait: Duration) -> io::Result<Vec<u8>> {
        let host = yard.address().strip_prefix("http://").unwrap();
        let mut conn = TcpStream::connect(host).unwrap();
        // The head is waited for as long as a slow machine needs.
        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // In one write, as the client sends it.
        let request = format!(
            "GET /user/events?after=0 HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n"
        );
        conn.write_all(request.as_bytes()).unwrap();
        let mut reader = BufReader::new(conn.try_clone().unwrap());
        // The head of the answer: the stream is open.
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0, "the stream opened");
            if line == "\r\n" {
                break;
            }
        }
        conn.shutdown(Shutdown::Write).unwrap();
        // Not waited for for ever where the yard does not end it.
        conn.set_read_timeout(Some(wait)).unwrap();
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest)?;
        Ok(rest)
    }

    /// A client that hangs up its end of a stream has the stream ended by
    /// the yard, as a real yard ends it: that wakes a follower stopped
    /// while it waits for an event on a system where shutting down one's
    /// own end wakes nothing that reads (Windows). A test that must not be
    /// helped so holds the streams, and then the yard leaves them open.
    #[test]
    fn a_stream_whose_client_hung_up_is_ended_by_the_yard_unless_it_is_held() {
        let yard = Yard::start_tcp();
        let rest = hang_up_on(&yard, Duration::from_secs(5)).expect("the yard ended the stream");
        assert!(rest.is_empty());

        yard.hold_streams();
        // Several of the yard's looks at its connection go by.
        let err = hang_up_on(&yard, 5 * LOOK).expect_err("the yard held the stream open");
        assert!(
            matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{err}"
        );
    }

    /// A call that is held is asked and not answered: its caller waits in it
    /// until the call is let through, and is answered then as the yard
    /// answers then. Calls that come after are not held.
    #[test]
    fn a_held_call_is_answered_when_it_is_let_through() {
        let yard = Yard::start();
        yard.answer("Head", r#"{"result": 20}"#);
        yard.hold_calls("Head");
        let (answered, answer) = std::sync::mpsc::channel();
        let caller = thread::spawn({
            let client = yard.client();
            move || answered.send(client.head()).unwrap()
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.calls_held("Head") == 0 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(yard.calls_held("Head"), 1);
        assert_eq!(yard.asked("Head").len(), 1);
        assert!(answer.try_recv().is_err(), "a held call is not answered");

        yard.answer("Head", r#"{"result": 21}"#);
        yard.release_calls("Head");
        let head = answer.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(head.unwrap(), 21);
        assert_eq!(yard.calls_held("Head"), 0);
        assert_eq!(yard.client().head().unwrap(), 21);
        caller.join().unwrap();
    }

    /// The events a [`Live`] has said, as JSON, in order.
    fn said(live: &Live) -> Vec<Value> {
        live.take()
            .iter()
            .map(|event| serde_json::from_str(event).unwrap())
            .collect()
    }

    const LIVE_START: &str = r#"{"type":"session.start","event_id":"s1","session":{"model":"gpt-live-1","instructions":"Help.","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client","acknowledgement_delay_ms":800},"store":false}}"#;

    /// [`LIVE_START`] asking for `voice` as `audio.output.voice`, whatever it
    /// is.
    fn live_start_in(voice: Value) -> String {
        let mut event: Value = serde_json::from_str(LIVE_START).unwrap();
        event["session"]["audio"]["output"] = serde_json::json!({ "voice": voice });
        event.to_string()
    }

    /// A scripted session, as the reference has its events: started with the
    /// moment it ends, deltas of both voices, a delegation, appends recorded
    /// and acknowledged under their ids, speech sent and then dropped, a
    /// close. The client's side is raw JSON here, so that the stand-in is
    /// held to the reference and not to the module that uses it.
    #[test]
    fn the_live_stand_in_answers_a_scripted_session_end_to_end() {
        let live = Live::start();
        live.expires_at(1_788_555_600);
        live.receive(LIVE_START);
        let started = said(&live);
        assert_eq!(started.len(), 1);
        assert_eq!(started[0]["type"], "session.started");
        assert_eq!(started[0]["client_event_id"], "s1");
        assert_eq!(started[0]["session"]["expires_at"], 1_788_555_600);
        assert_eq!(started[0]["session"]["instructions"], "Help.");
        assert_eq!(started[0]["session"]["audio"]["format"]["rate"], 24000);
        // No voice was asked for: the session speaks in the provider's own.
        assert_eq!(started[0]["session"]["audio"]["output"]["voice"], "marin");
        assert_eq!(
            started[0]["session"]["delegation"]["acknowledgement_delay_ms"],
            800
        );

        // Deltas, both ways, with their places on the timeline.
        live.person_says("Hello", 2000, 2200);
        live.voice_says("Hi!", 2200, 2600);
        let id = live.delegate(3600);
        let events = said(&live);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["type"], "session.input_transcript.delta");
        assert_eq!(events[0]["delta"], "Hello");
        assert_eq!(
            (events[0]["start_ms"].as_u64(), events[0]["end_ms"].as_u64()),
            (Some(2000), Some(2200))
        );
        assert_eq!(events[1]["type"], "session.output_transcript.delta");
        assert_eq!(events[1]["delta"], "Hi!");
        assert_eq!(events[2]["type"], "session.delegation.created");
        assert_eq!(events[2]["offset_ms"], 3600);
        assert_eq!(events[2]["delegation"]["id"], id.as_str());
        // Every event of the server's has an id of its own.
        assert!(events.iter().all(|event| event["event_id"].is_string()));
        assert_ne!(events[0]["event_id"], events[1]["event_id"]);

        // Appends are recorded and acknowledged by the id the client gave.
        live.receive(&format!(
            r#"{{"type":"session.thinking.append","event_id":"e1","delegation_id":"{id}","content":""}}"#
        ));
        live.receive(&format!(
            r#"{{"type":"session.commentary.append","event_id":"e2","delegation_id":"{id}","content":"It shipped."}}"#
        ));
        live.receive(
            r#"{"type":"session.thinking.append","event_id":"e3","delegation_id":null,"content":"A decision waits."}"#,
        );
        let acknowledged = said(&live);
        assert_eq!(
            acknowledged
                .iter()
                .map(|event| (
                    event["type"].as_str().unwrap(),
                    event["client_event_id"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            [
                ("session.thinking.appended", "e1"),
                ("session.commentary.appended", "e2"),
                ("session.thinking.appended", "e3"),
            ]
        );
        assert_eq!(
            live.appends(),
            [
                LiveAppend {
                    kind: "thinking".into(),
                    event_id: "e1".into(),
                    delegation_id: Some(id.clone()),
                    content: String::new()
                },
                LiveAppend {
                    kind: "commentary".into(),
                    event_id: "e2".into(),
                    delegation_id: Some(id.clone()),
                    content: "It shipped.".into()
                },
                LiveAppend {
                    kind: "thinking".into(),
                    event_id: "e3".into(),
                    delegation_id: None,
                    content: "A decision waits.".into()
                },
            ]
        );
        // The microphone's audio is heard and nothing is made of it.
        live.receive(r#"{"type":"session.input_audio.append","audio":"AAA="}"#);
        assert!(live.take().is_empty());

        // Speech queued, some of it sent, the rest dropped when the person
        // speaks over it.
        for n in 1..=4 {
            live.queue_audio(vec![n, -n]);
        }
        assert_eq!(live.play(2), 2);
        let audio = said(&live);
        assert_eq!(audio[0]["type"], "session.output_audio.delta");
        assert_eq!(audio[0]["delta"], "AQD//w==");
        assert_eq!(audio[1]["delta"], "AgD+/w==");
        assert_eq!(live.interrupt(), 2);
        assert_eq!(live.play(2), 0);
        assert!(live.take().is_empty());
        assert_eq!(live.dropped(), 2);

        // A close is answered, and after it nothing is heard.
        live.receive(r#"{"type":"session.close","event_id":"c1"}"#);
        let closed = said(&live);
        assert_eq!(closed[0]["type"], "session.closed");
        assert_eq!(closed[0]["reason"], "close_requested");
        live.receive(&format!(
            r#"{{"type":"session.commentary.append","event_id":"e4","delegation_id":"{id}","content":"late"}}"#
        ));
        assert!(live.take().is_empty());
        assert_eq!(live.appends().len(), 3);
        // All of it, kept as it was sent.
        assert_eq!(
            live.sent()
                .iter()
                .map(|event| event["type"].as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            [
                "session.start",
                "session.thinking.append",
                "session.commentary.append",
                "session.thinking.append",
                "session.input_audio.append",
                "session.close",
                "session.commentary.append",
            ]
        );
    }

    #[test]
    fn the_live_stand_in_refuses_what_the_reference_refuses() {
        let refused = |live: &Live, event: &str| -> Option<String> {
            live.receive(event);
            let said = said(live);
            assert_eq!(said.len(), 1, "{event}: {said:?}");
            assert_eq!(said[0]["type"], "error", "{event}");
            said[0]["error"]["client_event_id"]
                .as_str()
                .map(str::to_string)
        };
        // Before the session starts there is nothing to append to.
        let live = Live::start();
        let early = r#"{"type":"session.commentary.append","event_id":"a0","delegation_id":null,"content":"x"}"#;
        assert_eq!(refused(&live, early).as_deref(), Some("a0"));
        // Audio the socket does not take, another way to delegate, a delay
        // past the greatest, a voice that is no built-in name and no custom
        // voice's id (one with none, or with one past 128 characters), a
        // second start.
        for start in [
            LIVE_START.replace("24000", "44100"),
            LIVE_START.replace("audio/pcm", "audio/pcmu"),
            LIVE_START.replace(r#""type":"client""#, r#""type":"responses""#),
            LIVE_START.replace("800", "900"),
            live_start_in(serde_json::json!("nobody")),
            live_start_in(serde_json::json!(7)),
            live_start_in(serde_json::json!({ "id": "" })),
            live_start_in(serde_json::json!({ "id": "v".repeat(129) })),
        ] {
            assert_eq!(refused(&live, &start).as_deref(), Some("s1"), "{start}");
        }
        live.receive(LIVE_START);
        assert_eq!(said(&live).len(), 1);
        assert_eq!(refused(&live, LIVE_START).as_deref(), Some("s1"));
        let id = live.delegate(0);
        assert_eq!(said(&live).len(), 1);
        // Each fault in an append is an error that names the append.
        let with = |fields: &str| {
            format!(r#"{{"type":"session.commentary.append","event_id":"a1",{fields}}}"#)
        };
        for fields in [
            // `text`, not `content`.
            format!(r#""delegation_id":"{id}","text":"x""#),
            // No delegation_id at all: it is required, and may be null.
            r#""content":"x""#.to_string(),
            // An id no delegation of the session gave.
            r#""delegation_id":"item_other","content":"x""#.to_string(),
            r#""delegation_id":7,"content":"x""#.to_string(),
            // Over the limit of 500 tokens, as 500 bytes bound it.
            format!(r#""delegation_id":"{id}","content":"{}""#, "x".repeat(501)),
        ] {
            assert_eq!(
                refused(&live, &with(&fields)).as_deref(),
                Some("a1"),
                "{fields}"
            );
        }
        assert!(live.appends().is_empty());
        // The edge of the limit, and a null id, are fine.
        live.receive(&with(&format!(
            r#""delegation_id":"{id}","content":"{}""#,
            "x".repeat(500)
        )));
        live.receive(&with(r#""delegation_id":null,"content":"""#));
        assert_eq!(said(&live).len(), 2);
        assert_eq!(live.appends().len(), 2);
        // What is not JSON, and what the stand-in does not model, is refused
        // too; an event with no id is refused without one to name.
        assert_eq!(refused(&live, "not json"), None);
        assert_eq!(
            refused(&live, r#"{"type":"session.update","event_id":"u1"}"#).as_deref(),
            Some("u1")
        );
        assert_eq!(
            refused(
                &live,
                r#"{"type":"session.commentary.append","delegation_id":null}"#
            ),
            None
        );
    }

    /// A start that names a voice is answered in it: each built-in name of the
    /// reference, and a custom voice's id, come back as they were asked for.
    #[test]
    fn the_live_stand_in_starts_in_the_voice_it_is_asked_for() {
        let started_in = |voice: Value| -> Value {
            let live = Live::start();
            live.receive(&live_start_in(voice));
            let said = said(&live);
            assert_eq!(said.len(), 1, "{said:?}");
            assert_eq!(said[0]["type"], "session.started", "{said:?}");
            said[0]["session"]["audio"]["output"]["voice"].clone()
        };
        for name in LIVE_VOICES {
            assert_eq!(started_in(serde_json::json!(name)), name);
        }
        let custom = serde_json::json!({ "id": "voice_abc123" });
        assert_eq!(started_in(custom.clone()), custom);
        // The names are the reference's thirty-one, none twice.
        let mut names = LIVE_VOICES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 31);
        assert!(LIVE_VOICES.contains(&LIVE_DEFAULT_VOICE));
    }

    #[test]
    fn the_live_stand_in_speaks_its_filler_unless_an_append_came_first() {
        let live = Live::start();
        live.receive(LIVE_START);
        live.take();
        // Two delegations, no append: the delay passes and each is
        // acknowledged by the model itself.
        live.delegate(1000);
        live.delegate(2000);
        live.take();
        assert_eq!(live.delay_passes(), 2);
        let spoken = said(&live);
        assert!(
            spoken
                .iter()
                .all(|event| event["type"] == "session.output_transcript.delta"
                    && event["delta"] == "Lemme check.")
        );
        // Once they were, none is due again.
        assert_eq!(live.delay_passes(), 0);
        // An append under one delegation cancels the acknowledgement of that
        // one and of every other still to come: here two were due.
        let third = live.delegate(3000);
        live.delegate(4000);
        live.take();
        live.receive(&format!(
            r#"{{"type":"session.thinking.append","event_id":"q1","delegation_id":"{third}","content":""}}"#
        ));
        assert_eq!(live.delay_passes(), 0);
        assert!(
            said(&live)
                .iter()
                .all(|event| event["type"] == "session.thinking.appended")
        );
        // The session's moment comes, and the server closes it.
        live.expire();
        let closed = said(&live);
        assert_eq!(closed.last().unwrap()["type"], "session.closed");
        assert_eq!(closed.last().unwrap()["reason"], "expired");
    }
}

/// ElevenAgents' documented event shapes, scripted independently of the
/// client. It never invokes a model, opens audio or uses an account key.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Agent {
    pub(crate) incoming: Mutex<Vec<Value>>,
    pub(crate) outgoing: Mutex<VecDeque<Value>>,
    pub(crate) heads: Mutex<Vec<String>>,
    pub(crate) status: Mutex<Option<u16>>,
    pub(crate) signed_status: Mutex<Option<u16>>,
    pub(crate) signed_body: Mutex<Option<Value>>,
    pub(crate) tool_status: Mutex<Option<u16>>,
    pub(crate) upgrade_status: Mutex<Option<u16>>,
    pub(crate) pause: Mutex<Option<String>>,
    pub(crate) config: Mutex<Value>,
    pub(crate) metadata: Mutex<Value>,
    pub(crate) errors: Mutex<Vec<String>>,
}

#[cfg(test)]
impl Agent {
    pub(crate) fn new() -> Self {
        use serde_json::json;
        Self {
            config: Mutex::new(json!({
                "agent_id": "agent_test",
                "platform_settings": {"auth": {"enable_auth": true}, "overrides": {
                    "conversation_config_override": {"agent": {"prompt": {"prompt": true}}, "tts": {"voice_id": true}}
                }},
                "conversation_config": {"agent": {"language": "de", "prompt": {
                    "tools": [{"type": "client", "name": "send_to_yard", "expects_response": true,
                        "response_timeout_secs": 30,
                        "parameters": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}}]
                }}}
            })),
            metadata: Mutex::new(
                json!({"type": "conversation_initiation_metadata", "conversation_initiation_metadata_event": {
                    "conversation_id": "conv_test", "agent_output_audio_format": "pcm_16000", "user_input_audio_format": "pcm_16000"
                }}),
            ),
            ..Self::default()
        }
    }

    pub(crate) fn emit(&self, event: Value) {
        self.outgoing.lock().unwrap().push_back(event);
    }

    pub(crate) fn tool(&self, id: &str, text: &str) {
        self.emit(
            serde_json::json!({"type": "client_tool_call", "client_tool_call": {
                "tool_call_id": id, "tool_name": "send_to_yard", "parameters": {"text": text}
            }}),
        );
    }

    pub(crate) fn receive(&self, text: &str) {
        let event: Value = serde_json::from_str(text).unwrap();
        match event["type"].as_str() {
            Some("conversation_initiation_client_data") => {
                self.emit(self.metadata.lock().unwrap().clone())
            }
            Some("client_tool_result") => {
                if !event["result"].is_string()
                    || !event["tool_call_id"].is_string()
                    || !event["is_error"].is_boolean()
                {
                    self.errors
                        .lock()
                        .unwrap()
                        .push("invalid client tool result".into());
                }
            }
            Some("contextual_update" | "pong") => {}
            None if event["user_audio_chunk"].is_string() => {}
            _ => self
                .errors
                .lock()
                .unwrap()
                .push("unexpected Agent client event".into()),
        }
        self.incoming.lock().unwrap().push(event);
    }

    pub(crate) fn sent(&self, kind: &str) -> Vec<Value> {
        self.incoming
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == kind)
            .cloned()
            .collect()
    }
}
