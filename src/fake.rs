//! A yard for the tests: a unix socket in a temp dir that answers calls
//! with the JSON a test gives it, as docs/api.md of the yardr repository
//! shows it, and serves the stream.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::api::Client;

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
    /// Replies given once, before the standing one of `answers`.
    once: HashMap<String, VecDeque<Reply>>,
    asked: Vec<(String, Value)>,
    last_request: String,
    events: Vec<String>,
    streams: Vec<u64>,
    /// The images the yard holds, by hash, and every hash asked for.
    blobs: HashMap<String, Vec<u8>>,
    blobs_asked: Vec<String>,
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
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let accept = thread::spawn({
            let shared = shared.clone();
            move || {
                while !shared.state.lock().unwrap().stopped {
                    match listener.accept() {
                        Ok((conn, _)) => {
                            let shared = shared.clone();
                            thread::spawn(move || serve(conn, &shared));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
                // The listener goes here; the socket's file stays, as that
                // of a server that was killed does.
            }
        });
        Yard {
            dir: None,
            socket: socket.to_path_buf(),
            shared,
            accept: Mutex::new(Some(accept)),
        }
    }

    pub fn client(&self) -> Client {
        Client::new(&self.socket)
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

    fn set(&self, method: &str, reply: Reply) {
        self.shared
            .state
            .lock()
            .unwrap()
            .answers
            .insert(method.into(), reply);
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

fn serve(conn: UnixStream, shared: &Shared) {
    // A client that hangs up mid-reply is no failure of the test.
    let _ = serve_one(conn, shared);
}

fn serve_one(conn: UnixStream, shared: &Shared) -> std::io::Result<()> {
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
    let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
    shared.state.lock().unwrap().last_request = head.clone();

    if let Some(query) = target.strip_prefix("/events") {
        let after: u64 = query
            .strip_prefix("?after=")
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        return stream(conn, shared, after);
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
        let args = serde_json::from_slice(&body).unwrap_or(Value::Null);
        state.asked.push((method.clone(), args));
        let once = state.once.get_mut(&method).and_then(VecDeque::pop_front);
        once.or_else(|| state.answers.get(&method).cloned())
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

/// The stream: the events with a seq after `after`, then every one as the
/// test emits it, until the yard stops.
fn stream(mut conn: UnixStream, shared: &Shared, after: u64) -> std::io::Result<()> {
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
        if state.stopped {
            // Cut off, as a server that is killed cuts it: no last chunk.
            return Ok(());
        }
        state = shared.changed.wait(state).unwrap();
    }
}
