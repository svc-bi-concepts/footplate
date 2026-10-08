//! The yard's socket API, as docs/api.md of the yardr repository promises it
//! to a program that is not yardr: HTTP/1.1 over a unix socket, a call a
//! `POST /call/<Method>` with its arguments as a JSON object, the stream a
//! `GET /events?after=<seq>` of one JSON event a line.
//!
//! Only the calls and fields that page lists are read here. The few lines of
//! HTTP are written out, since a call is one request on one connection and
//! neither needs a runtime nor a client library with a unix connector.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fmt, thread};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

/// What footplate says it is to the yard. Nothing is decided by it today.
const CLIENT: &str = concat!("footplate/", env!("CARGO_PKG_VERSION"));

/// How long a call may take to answer. The stream has no such limit: it is
/// silent while nothing happens.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// A server that replaces itself with a newer build refuses calls meanwhile;
/// yardr makes the call again after this long, for up to that long.
const REPLACING_PAUSE: Duration = Duration::from_millis(100);
const REPLACING_PATIENCE: Duration = Duration::from_secs(30);

/// The error code of a server that is replacing itself, and of a name the
/// yard does not have.
const REPLACING: &str = "replacing";
pub const NOT_FOUND: &str = "not-found";

/// The most bytes `PutBlob` takes in a yard that has set no cap of its own.
pub const BLOB_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Why a call or the stream did not answer.
#[derive(Debug)]
pub enum Error {
    /// Nothing listens: the socket is missing, refused, or went away.
    Down(io::Error),
    /// The call reached the yard, and the yard said no. `message` is a
    /// sentence for a person: shown, never read.
    Refused { code: String, message: String },
    /// What came back is not what docs/api.md says.
    Protocol(String),
}

impl Error {
    /// Whether the yard answered that it has no such user, crew member or
    /// peer.
    pub fn not_found(&self) -> bool {
        matches!(self, Error::Refused { code, .. } if code == NOT_FOUND)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Down(err) => write!(f, "the yard does not answer ({err})"),
            Error::Refused { message, .. } => f.write_str(message),
            Error::Protocol(what) => {
                write!(f, "the yard answered what footplate cannot read: {what}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Down(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// `null` where a list (or an object) is promised is the empty one.
fn or_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The yard's settings, of which a client reads the operator and the cap on
/// an image.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub operator: String,
    /// A whole number in a string; absent when the yard has set none.
    #[serde(default)]
    pub blob_max_bytes: String,
}

impl Settings {
    /// The most bytes `PutBlob` takes.
    pub fn blob_cap(&self) -> u64 {
        self.blob_max_bytes.parse().unwrap_or(BLOB_MAX_BYTES)
    }
}

/// An image mail carries, as a letter names it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BlobRef {
    /// The SHA-256 of its bytes, in hex: `GET /blobs/<hash>` answers them.
    #[serde(default)]
    pub hash: String,
    /// `image/png` or `image/jpeg`.
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub bytes: u64,
    /// On mail from a linked yard only: why this yard did not take the
    /// image, when it did not. There is none to ask for then.
    #[serde(default)]
    pub not_taken: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct User {
    pub name: String,
    /// The user's mail is read through this seq.
    #[serde(default)]
    pub cursor: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CrewMember {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    /// `stopped`, `running` or `failed`.
    #[serde(default)]
    pub state: String,
    /// What a running session does: `working`, `idle`, `blocked`, `unknown`,
    /// or nothing when none runs or the daemon has not looked yet.
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Peer {
    pub name: String,
    #[serde(default, deserialize_with = "or_default")]
    pub send: Vec<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub receive: Vec<String>,
    /// What the peer last said of itself; a yard that has not said exposes
    /// nobody.
    #[serde(default)]
    pub said: Option<Said>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Said {
    #[serde(default, deserialize_with = "or_default")]
    pub roster: Vec<RosterEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RosterEntry {
    pub name: String,
    /// `crew` or `user`.
    #[serde(default)]
    pub kind: String,
    /// One line of what they do, in the peer's words.
    #[serde(default)]
    pub role: String,
    /// Whether the peer's side of the link takes mail for them.
    #[serde(default)]
    pub mail: bool,
}

/// One mail, of an inbox or a conversation.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Letter {
    pub seq: u64,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub read: bool,
    /// The images it carries, in the order it was sent with them.
    #[serde(default, deserialize_with = "or_default")]
    pub files: Vec<BlobRef>,
    /// On mail to a linked yard only: `queued`, `sent` or `failed`.
    #[serde(default)]
    pub delivery: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Inbox {
    #[serde(default, deserialize_with = "or_default")]
    pub mail: Vec<Letter>,
    /// All the unread, shown or not.
    #[serde(default)]
    pub unread: u64,
    /// The seq of the newest unread, 0 when none is.
    #[serde(default)]
    pub last: u64,
    /// Who the unread are from, in the order they first wrote.
    #[serde(default, deserialize_with = "or_default")]
    pub from: Vec<String>,
}

/// Mail as it waits in the outbox for a peer: what `MailPeer` answers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Queued {
    #[serde(default)]
    pub peer: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub at: String,
}

/// One event of the stream. `data` depends on the kind, and a client passes
/// over a kind or a field it does not know.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Event {
    pub seq: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub at: String,
    #[serde(default, deserialize_with = "or_default")]
    pub data: Map<String, Value>,
}

impl Event {
    /// A text of the event's data, empty when it has none of that name.
    pub fn text(&self, field: &str) -> &str {
        self.data.get(field).and_then(Value::as_str).unwrap_or("")
    }

    /// A seq of the event's data, 0 when it has none of that name.
    pub fn number(&self, field: &str) -> u64 {
        self.data.get(field).and_then(Value::as_u64).unwrap_or(0)
    }

    /// The images of a `mail` event, none when it carries none.
    pub fn files(&self) -> Vec<BlobRef> {
        self.data
            .get("files")
            .and_then(|files| serde_json::from_value(files.clone()).ok())
            .unwrap_or_default()
    }

    pub fn flag(&self, field: &str) -> bool {
        self.data
            .get(field)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// The reply to a call: a result, nothing, or an error.
#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Refusal>,
}

#[derive(Deserialize)]
struct Refusal {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// The yard's socket: `$YARDR_HOME/yard.sock`, else `~/.yardr/yard.sock`.
pub fn socket_path() -> PathBuf {
    socket_under(
        env::var_os("YARDR_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
}

fn socket_under(yardr_home: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    match yardr_home.filter(|h| !h.as_os_str().is_empty()) {
        Some(h) => h.join("yard.sock"),
        None => home.unwrap_or_default().join(".yardr").join("yard.sock"),
    }
}

/// A client of one yard. It holds no connection: every call opens its own.
#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client {
            socket: socket.into(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The seq of the newest event.
    pub fn head(&self) -> Result<u64> {
        self.answer("Head", json!({}))
    }

    pub fn settings(&self) -> Result<Settings> {
        self.answer("Settings", json!({}))
    }

    /// The users of the yard. The window asks `GetUser` for the one it is
    /// for and does not call this; it is here as part of what the page
    /// promises a chat client, held by the same tests.
    #[allow(dead_code)]
    pub fn users(&self) -> Result<Vec<User>> {
        self.list("Users", json!({}))
    }

    /// One user; `not-found` for a name that is no user of the yard.
    pub fn get_user(&self, name: &str) -> Result<User> {
        self.answer("GetUser", json!({ "name": name }))
    }

    /// The crew, running or not.
    pub fn crew(&self) -> Result<Vec<CrewMember>> {
        self.list("Crew", json!({}))
    }

    /// The linked yards, each with the roster it last said.
    pub fn peers(&self) -> Result<Vec<Peer>> {
        self.list("Peers", json!({}))
    }

    /// The mail to a user, the unread first; `limit` 0 is all.
    pub fn user_inbox(&self, name: &str, limit: u64) -> Result<Inbox> {
        self.answer("UserInbox", json!({ "name": name, "limit": limit }))
    }

    /// Marks the user's mail read through a seq.
    pub fn ack_user_inbox(&self, name: &str, through: u64) -> Result<()> {
        self.call("AckUserInbox", json!({ "name": name, "through": through }))
            .map(drop)
    }

    /// Sends mail to a crew member or a user of this yard, with the images
    /// `files` names by the hash `PutBlob` answered. With one, the text may
    /// be empty.
    pub fn mail(&self, to: &str, text: &str, actor: &str, files: &[String]) -> Result<()> {
        let mut args = json!({ "to": to, "text": text, "actor": actor });
        if !files.is_empty() {
            args["files"] = json!(files);
        }
        self.call("Mail", args).map(drop)
    }

    /// Gives the yard an image, a PNG or a JPEG, for mail to carry.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<BlobRef> {
        self.answer("PutBlob", json!({ "data": BASE64.encode(bytes) }))
    }

    /// The bytes of an image, `None` for a hash the yard does not hold: it
    /// removes an image once no mail it still has names it.
    pub fn blob(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        // The hash is what a letter said, and goes into the request's first
        // line: one that is no hex is no image of the yard's.
        if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(None);
        }
        let mut conn = UnixStream::connect(&self.socket)?;
        conn.set_read_timeout(Some(CALL_TIMEOUT))?;
        conn.set_write_timeout(Some(CALL_TIMEOUT))?;
        write!(
            conn,
            "GET /blobs/{hash} HTTP/1.1\r\nHost: yard\r\nX-Yardr-Client: {CLIENT}\r\nConnection: close\r\n\r\n"
        )?;
        let (status, mut body) = respond(conn)?;
        match status {
            200 => {
                let mut bytes = Vec::new();
                body.read_to_end(&mut bytes)?;
                Ok(Some(bytes))
            }
            404 => Ok(None),
            _ => Err(Error::Protocol(format!(
                "the image {hash}: status {status}"
            ))),
        }
    }

    /// Queues mail to one of a linked yard, `<to>@<peer>`, with the images
    /// `files` names as for [`Client::mail`]. The yard refuses mail with one
    /// when the link does not carry files, and says why.
    pub fn mail_peer(
        &self,
        to: &str,
        peer: &str,
        text: &str,
        actor: &str,
        files: &[String],
    ) -> Result<Queued> {
        let mut args = json!({ "to": to, "peer": peer, "text": text, "actor": actor });
        if !files.is_empty() {
            args["files"] = json!(files);
        }
        self.answer("MailPeer", args)
    }

    /// The mail between the user `a` and `b`, oldest first, newer than
    /// `after`.
    pub fn conversation(&self, a: &str, b: &str, after: u64) -> Result<Vec<Letter>> {
        self.list("Conversation", json!({ "a": a, "b": b, "after": after }))
    }

    /// The stream of events after a seq: those the yard still has, then
    /// every one as it happens. It ends when the server goes away.
    pub fn events(&self, after: u64) -> Result<Events> {
        let mut conn = UnixStream::connect(&self.socket)?;
        write!(
            conn,
            "GET /events?after={after} HTTP/1.1\r\nHost: yard\r\nX-Yardr-Client: {CLIENT}\r\nAccept: application/x-ndjson\r\n\r\n"
        )?;
        let (status, body) = respond(conn)?;
        if status != 200 {
            return Err(Error::Protocol(format!(
                "the stream answered status {status}"
            )));
        }
        Ok(Events {
            lines: BufReader::new(body),
        })
    }

    fn answer<T: DeserializeOwned>(&self, method: &str, args: Value) -> Result<T> {
        let result = self.call(method, args)?.unwrap_or(Value::Null);
        serde_json::from_value(result).map_err(|err| Error::Protocol(format!("{method}: {err}")))
    }

    fn list<T: DeserializeOwned>(&self, method: &str, args: Value) -> Result<Vec<T>> {
        Ok(self
            .answer::<Option<Vec<T>>>(method, args)?
            .unwrap_or_default())
    }

    /// Makes a call and gives its result, if it answered one. A server that
    /// is replacing itself did not run the call: it is made again, also
    /// through the moment in which nothing listens between the two builds.
    fn call(&self, method: &str, args: Value) -> Result<Option<Value>> {
        let mut replacing: Option<Instant> = None;
        loop {
            let reply = self.call_once(method, &args);
            let again = match &reply {
                Err(Error::Refused { code, .. }) => code == REPLACING,
                Err(Error::Down(_)) => replacing.is_some(),
                _ => false,
            };
            if !again || replacing.get_or_insert_with(Instant::now).elapsed() >= REPLACING_PATIENCE
            {
                return reply;
            }
            thread::sleep(REPLACING_PAUSE);
        }
    }

    fn call_once(&self, method: &str, args: &Value) -> Result<Option<Value>> {
        let mut conn = UnixStream::connect(&self.socket)?;
        conn.set_read_timeout(Some(CALL_TIMEOUT))?;
        conn.set_write_timeout(Some(CALL_TIMEOUT))?;
        let body = args.to_string();
        write!(
            conn,
            "POST /call/{method} HTTP/1.1\r\nHost: yard\r\nX-Yardr-Client: {CLIENT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        let (status, mut reply) = respond(conn)?;
        let mut bytes = Vec::new();
        reply.read_to_end(&mut bytes)?;
        // An error has its status with it (200, or 503 while replacing), so
        // the body is read first, and the status only when it says nothing.
        let reply: Reply = serde_json::from_slice(&bytes).map_err(|err| {
            Error::Protocol(format!(
                "{method}: status {status}, and no JSON object ({err})"
            ))
        })?;
        if let Some(Refusal { code, message }) = reply.error {
            return Err(Error::Refused { code, message });
        }
        if status != 200 {
            return Err(Error::Protocol(format!("{method}: status {status}")));
        }
        Ok(reply.result)
    }
}

/// The events of a stream, one a line, until it ends.
pub struct Events {
    lines: BufReader<Body>,
}

impl Iterator for Events {
    type Item = Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        loop {
            line.clear();
            match self.lines.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) if line.trim().is_empty() => continue,
                Ok(_) => {
                    return Some(
                        serde_json::from_str(&line)
                            .map_err(|err| Error::Protocol(format!("an event: {err}"))),
                    );
                }
                Err(err) => return Some(Err(Error::Down(err))),
            }
        }
    }
}

/// Reads the status and the headers of a response, and gives its body.
fn respond(conn: UnixStream) -> Result<(u16, Body)> {
    let mut conn = BufReader::new(conn);
    let mut line = String::new();
    conn.read_line(&mut line)?;
    // "HTTP/1.1 200 OK"
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| Error::Protocol(format!("no HTTP status in {:?}", line.trim_end())))?;
    let mut length = None;
    let mut chunked = false;
    loop {
        line.clear();
        if conn.read_line(&mut line)? == 0 {
            return Err(Error::Down(io::ErrorKind::UnexpectedEof.into()));
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<u64>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.to_ascii_lowercase().contains("chunked");
        }
    }
    let framing = match (chunked, length) {
        (true, _) => Framing::Chunked {
            left: 0,
            done: false,
        },
        (false, Some(n)) => Framing::Length(n),
        (false, None) => Framing::UntilClosed,
    };
    Ok((status, Body { conn, framing }))
}

/// The body of a response, as the server frames it. Go's server sends a
/// short reply with its length and a long or a streamed one in chunks.
struct Body {
    conn: BufReader<UnixStream>,
    framing: Framing,
}

enum Framing {
    Length(u64),
    /// `left` bytes remain of the chunk being read.
    Chunked {
        left: u64,
        done: bool,
    },
    UntilClosed,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        match &mut self.framing {
            Framing::UntilClosed => self.conn.read(buf),
            Framing::Length(left) => {
                if *left == 0 {
                    return Ok(0);
                }
                let want = buf.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
                let n = self.conn.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                *left -= n as u64;
                Ok(n)
            }
            Framing::Chunked { left, done } => {
                if *done {
                    return Ok(0);
                }
                if *left == 0 {
                    // "<size in hex>[;extension]\r\n"
                    let mut size = String::new();
                    if self.conn.read_line(&mut size)? == 0 {
                        // The server went away between two chunks: for the
                        // stream that is its end.
                        *done = true;
                        return Ok(0);
                    }
                    let hex = size.trim().split(';').next().unwrap_or("").trim();
                    *left = u64::from_str_radix(hex, 16).map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("no chunk size in {size:?}"),
                        )
                    })?;
                    if *left == 0 {
                        *done = true;
                        return Ok(0);
                    }
                }
                let want = buf.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
                let n = self.conn.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                *left -= n as u64;
                if *left == 0 {
                    // The line break that ends the chunk.
                    let mut end = [0u8; 2];
                    self.conn.read_exact(&mut end)?;
                }
                Ok(n)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Yard;

    #[test]
    fn socket_is_under_yardr_home_else_the_home() {
        assert_eq!(
            socket_under(Some("/y".into()), Some("/h".into())),
            PathBuf::from("/y/yard.sock")
        );
        assert_eq!(
            socket_under(None, Some("/h".into())),
            PathBuf::from("/h/.yardr/yard.sock")
        );
        assert_eq!(
            socket_under(Some("".into()), Some("/h".into())),
            PathBuf::from("/h/.yardr/yard.sock")
        );
    }

    #[test]
    fn head() {
        let yard = Yard::start();
        yard.answer("Head", r#"{"result": 12}"#);
        assert_eq!(yard.client().head().unwrap(), 12);
        assert_eq!(yard.asked("Head"), vec![json!({})]);
    }

    #[test]
    fn settings_with_and_without_an_operator() {
        let yard = Yard::start();
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        assert_eq!(yard.client().settings().unwrap().operator, "jo");
        yard.answer("Settings", r#"{"result": {}}"#);
        assert_eq!(yard.client().settings().unwrap().operator, "");
    }

    #[test]
    fn users() {
        let yard = Yard::start();
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 7, "notification_cursor": 0}]}"#,
        );
        assert_eq!(
            yard.client().users().unwrap(),
            vec![User {
                name: "jo".into(),
                cursor: 7
            }]
        );
    }

    #[test]
    fn get_user_and_the_one_the_yard_does_not_have() {
        let yard = Yard::start();
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "jo", "cursor": 3, "notification_cursor": 0}}"#,
        );
        assert_eq!(yard.client().get_user("jo").unwrap().cursor, 3);
        assert_eq!(yard.asked("GetUser"), vec![json!({"name": "jo"})]);

        yard.answer(
            "GetUser",
            r#"{"error": {"code": "not-found", "message": "user nobody: not found"}}"#,
        );
        let err = yard.client().get_user("nobody").unwrap_err();
        assert!(err.not_found(), "{err:?}");
        assert_eq!(err.to_string(), "user nobody: not found");
    }

    #[test]
    fn crew_with_the_fields_that_may_be_absent() {
        let yard = Yard::start();
        yard.answer(
            "Crew",
            r#"{"result": [
                {"name": "yardmaster", "enabled": true, "state": "running", "status": "working", "started_at": "2026-10-08T09:30:00Z", "config": {"x": 1}},
                {"name": "assembly", "enabled": false, "state": "stopped"}
            ]}"#,
        );
        let crew = yard.client().crew().unwrap();
        assert_eq!(crew.len(), 2);
        assert_eq!(
            (
                crew[0].name.as_str(),
                crew[0].state.as_str(),
                crew[0].status.as_str()
            ),
            ("yardmaster", "running", "working")
        );
        assert_eq!(
            (
                crew[1].enabled,
                crew[1].state.as_str(),
                crew[1].status.as_str()
            ),
            (false, "stopped", "")
        );
    }

    #[test]
    fn null_where_a_list_is_promised_is_the_empty_list() {
        let yard = Yard::start();
        yard.answer("Crew", r#"{"result": null}"#);
        assert_eq!(yard.client().crew().unwrap(), vec![]);
        yard.answer("Conversation", r#"{}"#);
        assert_eq!(
            yard.client().conversation("jo", "yardmaster", 0).unwrap(),
            vec![]
        );
        yard.answer("Peers", r#"{"result": [{"name": "north", "send": null, "receive": null, "said": {"roster": null, "at": "2026-10-08T09:30:00Z"}}]}"#);
        let peers = yard.client().peers().unwrap();
        assert_eq!(peers[0].send, Vec::<String>::new());
        assert_eq!(peers[0].said.as_ref().unwrap().roster, vec![]);
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": null, "unread": 0, "last": 0, "from": null}}"#,
        );
        assert_eq!(yard.client().user_inbox("jo", 1).unwrap(), Inbox::default());
    }

    #[test]
    fn peers_with_the_roster() {
        let yard = Yard::start();
        yard.answer(
            "Peers",
            r#"{"result": [
                {"name": "north", "send": ["mail"], "receive": ["mail"], "key": "k",
                 "said": {"roster": [{"name": "clerk", "kind": "crew", "role": "Keeps the books", "mail": true}], "at": "2026-10-08T09:30:00Z"}},
                {"name": "south", "send": [], "receive": []}
            ]}"#,
        );
        let peers = yard.client().peers().unwrap();
        assert_eq!(peers[0].name, "north");
        assert_eq!(peers[0].send, vec!["mail"]);
        assert_eq!(
            peers[0].said.as_ref().unwrap().roster,
            vec![RosterEntry {
                name: "clerk".into(),
                kind: "crew".into(),
                role: "Keeps the books".into(),
                mail: true
            }]
        );
        assert_eq!(peers[1].said, None);
    }

    #[test]
    fn user_inbox() {
        let yard = Yard::start();
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}],
                "unread": 1, "last": 12, "from": ["yardmaster"]}}"#,
        );
        let inbox = yard.client().user_inbox("jo", 10).unwrap();
        assert_eq!(
            yard.asked("UserInbox"),
            vec![json!({"name": "jo", "limit": 10})]
        );
        assert_eq!(
            (inbox.unread, inbox.last, inbox.from),
            (1, 12, vec!["yardmaster".to_string()])
        );
        assert_eq!(inbox.mail[0].seq, 12);
        assert_eq!(inbox.mail[0].from, "yardmaster");
        assert!(!inbox.mail[0].read);
    }

    #[test]
    fn ack_user_inbox_answers_nothing() {
        let yard = Yard::start();
        yard.answer("AckUserInbox", "{}");
        yard.client().ack_user_inbox("jo", 12).unwrap();
        assert_eq!(
            yard.asked("AckUserInbox"),
            vec![json!({"name": "jo", "through": 12})]
        );
    }

    #[test]
    fn mail_and_what_the_yard_refuses() {
        let yard = Yard::start();
        yard.answer("Mail", "{}");
        yard.client()
            .mail("yardmaster", "Thanks. Land it.", "jo", &[])
            .unwrap();
        assert_eq!(
            yard.asked("Mail"),
            vec![json!({"to": "yardmaster", "text": "Thanks. Land it.", "actor": "jo"})]
        );

        yard.answer(
            "Mail",
            r#"{"error": {"code": "error", "message": "mail is 1 to 4096 bytes"}}"#,
        );
        let err = yard.client().mail("yardmaster", "", "jo", &[]).unwrap_err();
        assert!(
            matches!(&err, Error::Refused { code, .. } if code == "error"),
            "{err:?}"
        );
        assert!(!err.not_found());
    }

    /// The PNG of docs/api.md: one pixel, 70 bytes.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    const HASH: &str = "497790947d4666760ce38f3c00e852c71fdb66cae849bae8e9ede352719e1581";

    #[test]
    fn the_cap_on_an_image_is_the_yard_s_or_8_mib() {
        let yard = Yard::start();
        yard.answer(
            "Settings",
            r#"{"result": {"blob_max_bytes": "1024", "operator": "jo"}}"#,
        );
        assert_eq!(yard.client().settings().unwrap().blob_cap(), 1024);
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        assert_eq!(yard.client().settings().unwrap().blob_cap(), 8388608);
        yard.answer("Settings", r#"{"result": {"blob_max_bytes": "many"}}"#);
        assert_eq!(yard.client().settings().unwrap().blob_cap(), 8388608);
    }

    #[test]
    fn put_blob_then_mail_with_the_hash() {
        let yard = Yard::start();
        let bytes = BASE64.decode(PNG).unwrap();
        yard.answer(
            "PutBlob",
            &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
        );
        yard.answer("Mail", "{}");
        let blob = yard.client().put_blob(&bytes).unwrap();
        assert_eq!(yard.asked("PutBlob"), vec![json!({"data": PNG})]);
        assert_eq!(
            blob,
            BlobRef {
                hash: HASH.into(),
                kind: "image/png".into(),
                bytes: 70,
                not_taken: String::new()
            }
        );
        yard.client()
            .mail("yardmaster", "", "jo", &[blob.hash])
            .unwrap();
        assert_eq!(
            yard.asked("Mail"),
            vec![json!({"to": "yardmaster", "text": "", "actor": "jo", "files": [HASH]})]
        );

        yard.answer(
            "PutBlob",
            r#"{"error": {"code": "error", "message": "an image is a PNG or a JPEG"}}"#,
        );
        let err = yard.client().put_blob(b"GIF89a").unwrap_err();
        assert_eq!(err.to_string(), "an image is a PNG or a JPEG");
    }

    #[test]
    fn an_image_by_its_hash_and_the_one_the_yard_does_not_hold() {
        let yard = Yard::start();
        // Bytes that are no text, and more than one read gives.
        let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        yard.blob(HASH, &bytes);
        assert_eq!(yard.client().blob(HASH).unwrap(), Some(bytes));
        let request = yard.last_request();
        assert!(
            request.starts_with(&format!("GET /blobs/{HASH} HTTP/1.1\r\n")),
            "{request}"
        );

        let gone = "00".repeat(32);
        assert_eq!(yard.client().blob(&gone).unwrap(), None);
        assert_eq!(yard.blobs_asked(), vec![HASH.to_string(), gone]);

        // What is no hash is not asked for: it would be a request of its own.
        assert_eq!(yard.client().blob("").unwrap(), None);
        assert_eq!(
            yard.client()
                .blob("ab HTTP/1.1\r\nX: y\r\n\r\nGET /blobs/cd")
                .unwrap(),
            None
        );
        assert_eq!(yard.client().blob("../yard.db").unwrap(), None);
        assert_eq!(yard.blobs_asked().len(), 2);

        yard.stop();
        assert!(matches!(yard.client().blob(HASH), Err(Error::Down(_))));
    }

    #[test]
    fn a_letter_and_a_mail_event_name_their_images() {
        let yard = Yard::start();
        yard.answer(
            "Conversation",
            &json!({"result": [
                {"seq": 12, "from": "jo", "to": "yardmaster", "text": "Paint the fence like this.", "read": false,
                 "files": [{"hash": HASH, "type": "image/png", "bytes": 70}]},
                {"seq": 13, "from": "yardmaster", "to": "jo", "text": "Will do.", "files": null},
                {"seq": 14, "from": "clerk@north", "to": "jo", "text": "The ledger.",
                 "files": [{"hash": HASH, "type": "image/png", "bytes": 70, "not_taken": "the link does not take files"}]}
            ]})
            .to_string(),
        );
        let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
        let png = BlobRef {
            hash: HASH.into(),
            kind: "image/png".into(),
            bytes: 70,
            not_taken: String::new(),
        };
        assert_eq!(letters[0].files, vec![png.clone()]);
        // One this yard did not take from a linked yard says why.
        assert_eq!(
            letters[2].files[0].not_taken,
            "the link does not take files"
        );
        assert_eq!(letters[1].files, vec![]);

        let event: Event = serde_json::from_value(json!({"seq": 12, "kind": "mail", "actor": "jo",
            "data": {"to": "yardmaster", "text": "Paint the fence like this.",
                     "files": [{"hash": HASH, "type": "image/png", "bytes": 70}]}}))
        .unwrap();
        assert_eq!(event.files(), vec![png]);
        let plain: Event =
            serde_json::from_value(json!({"seq": 13, "kind": "mail", "data": {"files": "none"}}))
                .unwrap();
        assert_eq!(plain.files(), vec![]);
    }

    #[test]
    fn mail_peer() {
        let yard = Yard::start();
        yard.answer(
            "MailPeer",
            r#"{"result": {"peer": "north", "kind": "mail", "at": "2026-10-08T09:30:00Z"}}"#,
        );
        let queued = yard
            .client()
            .mail_peer("clerk", "north", "Is the ledger closed?", "jo", &[])
            .unwrap();
        assert_eq!(
            yard.asked("MailPeer"),
            vec![
                json!({"to": "clerk", "peer": "north", "text": "Is the ledger closed?", "actor": "jo"})
            ]
        );
        assert_eq!(
            queued,
            Queued {
                peer: "north".into(),
                kind: "mail".into(),
                at: "2026-10-08T09:30:00Z".into()
            }
        );

        // With images the mail names them, and needs no text.
        yard.client()
            .mail_peer("clerk", "north", "", "jo", &[HASH.to_string()])
            .unwrap();
        assert_eq!(
            yard.asked("MailPeer")[1],
            json!({"to": "clerk", "peer": "north", "text": "", "actor": "jo", "files": [HASH]})
        );
    }

    #[test]
    fn conversation_with_a_crew_member_and_with_one_of_a_linked_yard() {
        let yard = Yard::start();
        yard.answer(
            "Conversation",
            r#"{"result": [
                {"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": true},
                {"seq": 14, "at": "2026-10-08T09:31:00Z", "from": "jo", "to": "clerk@north", "text": "Is the ledger closed?", "read": false, "delivery": "queued"}
            ]}"#,
        );
        let letters = yard.client().conversation("jo", "yardmaster", 5).unwrap();
        assert_eq!(
            yard.asked("Conversation"),
            vec![json!({"a": "jo", "b": "yardmaster", "after": 5})]
        );
        assert_eq!(
            (
                letters[0].seq,
                letters[0].read,
                letters[0].delivery.as_str()
            ),
            (12, true, "")
        );
        assert_eq!(
            (letters[1].to.as_str(), letters[1].delivery.as_str()),
            ("clerk@north", "queued")
        );
    }

    #[test]
    fn a_long_reply_comes_in_chunks() {
        let yard = Yard::start();
        let text = "x".repeat(4000);
        let letters: Vec<Value> = (1..=20)
            .map(|seq| json!({"seq": seq, "from": "yardmaster", "to": "jo", "text": text}))
            .collect();
        yard.answer_chunked(
            "Conversation",
            &json!({ "result": letters }).to_string(),
            1000,
        );
        let got = yard.client().conversation("jo", "yardmaster", 0).unwrap();
        assert_eq!(got.len(), 20);
        assert_eq!(got[19].seq, 20);
        assert_eq!(got[19].text, text);
    }

    #[test]
    fn no_yard_is_an_error_and_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        // The socket is missing.
        let missing = Client::new(dir.path().join("yard.sock"));
        assert!(matches!(missing.head(), Err(Error::Down(_))));
        assert!(matches!(missing.events(0), Err(Error::Down(_))));
        // The file is there and nothing listens: the server is down.
        let yard = Yard::start();
        let client = yard.client();
        let socket = client.socket().to_path_buf();
        yard.stop_listening();
        assert!(socket.exists());
        assert!(matches!(client.head(), Err(Error::Down(_))));
        assert!(matches!(
            client.mail("yardmaster", "hello", "jo", &[]),
            Err(Error::Down(_))
        ));
    }

    #[test]
    fn what_is_no_reply_is_an_error_and_no_panic() {
        let yard = Yard::start();
        yard.answer("Head", "not JSON");
        assert!(matches!(yard.client().head(), Err(Error::Protocol(_))));
        yard.answer("Head", r#"{"result": "twelve"}"#);
        assert!(matches!(yard.client().head(), Err(Error::Protocol(_))));
        yard.answer_status("Head", 500, "{}");
        assert!(matches!(yard.client().head(), Err(Error::Protocol(_))));
    }

    #[test]
    fn a_call_refused_by_a_server_that_replaces_itself_is_made_again() {
        let yard = Yard::start();
        yard.answer("Head", r#"{"result": 4}"#);
        yard.answer_once_status(
            "Head",
            503,
            r#"{"error": {"code": "replacing", "message": "the server is replacing itself"}}"#,
        );
        assert_eq!(yard.client().head().unwrap(), 4);
        assert_eq!(yard.asked("Head").len(), 2);
    }

    #[test]
    fn a_call_names_the_client_and_no_revision() {
        let yard = Yard::start();
        yard.answer("Head", r#"{"result": 0}"#);
        yard.client().head().unwrap();
        let request = yard.last_request();
        assert!(
            request.starts_with("POST /call/Head HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request.contains(&format!(
                "X-Yardr-Client: footplate/{}\r\n",
                env!("CARGO_PKG_VERSION")
            )),
            "{request}"
        );
        assert!(
            !request.to_ascii_lowercase().contains("x-yardr-revision"),
            "{request}"
        );
    }

    #[test]
    fn the_stream_gives_the_events_after_a_seq_until_it_ends() {
        let yard = Yard::start();
        yard.emit(r#"{"seq": 12, "kind": "mail", "actor": "yardmaster", "data": {"text": "Shall I land it?", "to": "jo", "to_user": true}, "at": "2026-10-08T09:30:00Z"}"#);
        yard.emit(r#"{"seq": 15, "kind": "mail_read", "data": {"reader": "jo", "through": 12}}"#);
        let mut events = yard.client().events(11).unwrap();

        let mail = events.next().unwrap().unwrap();
        assert_eq!(
            (
                mail.seq,
                mail.kind.as_str(),
                mail.actor.as_str(),
                mail.at.as_str()
            ),
            (12, "mail", "yardmaster", "2026-10-08T09:30:00Z")
        );
        assert_eq!(
            (mail.text("to"), mail.text("text"), mail.flag("to_user")),
            ("jo", "Shall I land it?", true)
        );
        assert_eq!(mail.text("to_peer"), "");

        let read = events.next().unwrap().unwrap();
        assert_eq!(
            (
                read.kind.as_str(),
                read.text("reader"),
                read.number("through")
            ),
            ("mail_read", "jo", 12)
        );

        // What happens while the stream is open comes as it happens: an
        // event of a kind and with fields footplate does not know, and one
        // with no data at all.
        yard.emit(r#"{"seq": 16, "kind": "bead_sung", "actor": "yard", "id": "e-16", "data": {"verse": 2}}"#);
        yard.emit(r#"{"seq": 17, "kind": "crew_started"}"#);
        let other = events.next().unwrap().unwrap();
        assert_eq!((other.seq, other.kind.as_str()), (16, "bead_sung"));
        let bare = events.next().unwrap().unwrap();
        assert_eq!((bare.seq, bare.data.len()), (17, 0));

        assert_eq!(yard.streams_asked(), vec![11]);
        // The server goes away: the stream ends, and nothing panics.
        yard.stop();
        assert!(events.next().is_none());
    }

    #[test]
    fn the_stream_gives_only_what_is_after_the_seq() {
        let yard = Yard::start();
        yard.emit(r#"{"seq": 3, "kind": "mail"}"#);
        yard.emit(r#"{"seq": 4, "kind": "mail"}"#);
        let mut events = yard.client().events(3).unwrap();
        assert_eq!(events.next().unwrap().unwrap().seq, 4);
    }
}
