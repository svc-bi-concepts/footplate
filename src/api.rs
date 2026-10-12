//! The yard's socket API, as docs/api.md of the yardr repository promises it
//! to a program that is not yardr: HTTP/1.1 over a unix socket, a call a
//! `POST /call/<Method>` with its arguments as a JSON object, the stream a
//! `GET /events?after=<seq>` of one JSON event a line.
//!
//! A yard that is not on this machine is reached at its listener instead,
//! as that page's "Over the network" has it: the same requests under
//! `/user/`, over TCP with TLS, each with the device's token.
//!
//! Only the calls and fields that page lists are read here. The few lines of
//! HTTP are written out, since a call is one request on one connection and
//! neither needs a runtime nor a client library with a unix connector.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
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

/// How long a yard's address may take to be connected to.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A stream over the network is silent while nothing happens, and a link
/// that went away says nothing either: the system asks the other end after
/// this long a silence, so that a dead stream ends and is opened again.
const KEEPALIVE: Duration = Duration::from_secs(30);

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
    /// The yard's listener answered 401: the token is not a device's, or
    /// the device was removed. It is no fault that passes, so nothing is
    /// asked again until the device has a token again.
    NotLetIn,
    /// At the yard's address nothing is under `/user/`: 404 to a call. A
    /// yard answers so while it has no device at all, with a token or
    /// without, and so does an address that is no yard's listener.
    NoDevice,
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
            Error::NotLetIn => f.write_str("this device is no longer let in"),
            Error::NoDevice => {
                f.write_str("the yard has no device, or the address is not a yard's")
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
    /// The same for the user's notifications.
    #[serde(default)]
    pub notification_cursor: u64,
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
    /// The names of the peer's yard the link trusts as a user of this one,
    /// each with that user: mail from `<name>@<peer>` is the user speaking.
    #[serde(default, deserialize_with = "or_default")]
    pub trust: HashMap<String, String>,
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

/// One notification of a user: what the yard reports apart from every
/// conversation.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Notification {
    pub seq: u64,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub bead: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub read: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Notifications {
    #[serde(default, deserialize_with = "or_default")]
    pub notifications: Vec<Notification>,
    /// All the unread, shown or not.
    #[serde(default)]
    pub unread: u64,
    /// The seq of the newest unread, 0 when none is.
    #[serde(default)]
    pub last: u64,
}

/// What a secret ask asks for: the answer is a value for each name, typed
/// into a field that does not echo and given with `GiveSecret`. Such an ask
/// has no options and takes no words.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Secret {
    /// The declared source the secret is the credential of.
    #[serde(default)]
    pub source: String,
    /// The names to ask the person a value for.
    #[serde(default, deserialize_with = "or_default")]
    pub env: Vec<String>,
    /// What the card says is asked for, in the yard's words and never the
    /// asker's: shown as it is.
    #[serde(default)]
    pub line: String,
}

/// What `GiveSecret` answers: the names that were written and where, never
/// a value.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Given {
    #[serde(default, deserialize_with = "or_default")]
    pub given: Vec<String>,
    /// The file they are in, on the yard's machine.
    #[serde(default)]
    pub file: String,
    /// Where every variable of the source's credential is set now, in
    /// words; absent while one is still missing.
    #[serde(default)]
    pub credential: String,
    /// What the credential still wants, in words; absent when it is whole.
    #[serde(default)]
    pub missing: String,
}

/// One open question addressed to the user, as the queue lists it: the
/// question in plain words, without its options or context, which the card
/// reads with `Bead`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct OpenAsk {
    pub bead: String,
    #[serde(default)]
    pub bead_title: String,
    /// The bead's first `topic:` label, with its prefix, `""` when none.
    #[serde(default)]
    pub topic: String,
    /// The ask note's seq: what an answer names.
    #[serde(default)]
    pub ask: u64,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub asker: String,
    /// When the ask was written: how long it waits.
    #[serde(default)]
    pub since: String,
    /// What a secret ask asks for; absent on any other ask.
    #[serde(default)]
    pub secret: Option<Secret>,
}

/// One the user dealt with: answered, put aside or declined, newest first.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DecidedAsk {
    pub bead: String,
    #[serde(default)]
    pub bead_title: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub ask: u64,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub asker: String,
    #[serde(default)]
    pub since: String,
    /// `answered`, `put aside` or `declined`.
    #[serde(default)]
    pub state: String,
    /// The picked option's number; absent on a words answer, an aside ask
    /// and a declined one.
    #[serde(default)]
    pub option: u64,
    #[serde(default)]
    pub option_label: String,
    /// What went with the pick, the words answer, or why declined.
    #[serde(default)]
    pub words: String,
    /// When a put-aside ask comes back; absent otherwise.
    #[serde(default)]
    pub return_at: String,
    /// What a secret ask asks for; absent on any other ask.
    #[serde(default)]
    pub secret: Option<Secret>,
    /// The names a secret ask was given values for.
    #[serde(default, deserialize_with = "or_default")]
    pub given: Vec<String>,
}

/// The person's decisions: the open asks, oldest first, with what they
/// dealt with beside them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Asks {
    #[serde(default, deserialize_with = "or_default")]
    pub open: Vec<OpenAsk>,
    #[serde(default, deserialize_with = "or_default")]
    pub decided: Vec<DecidedAsk>,
}

/// One option of an ask, in order, `you decide` last.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AskOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub consequence: String,
}

/// One bead an ask is about, a step away from the card.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AboutBead {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub stage: String,
}

/// One of the bead's asks with its options, recommendation and state.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BeadAsk {
    #[serde(default)]
    pub ask: u64,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub asker: String,
    #[serde(default)]
    pub since: String,
    #[serde(default)]
    pub question: String,
    #[serde(default, deserialize_with = "or_default")]
    pub context: Vec<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub options: Vec<AskOption>,
    /// The recommended option's number; 0 when nothing is recommended.
    #[serde(default)]
    pub recommend: u64,
    #[serde(default)]
    pub why: String,
    #[serde(default, deserialize_with = "or_default")]
    pub about: Vec<AboutBead>,
    /// `open`, `answered`, `put aside` or `declined`.
    #[serde(default)]
    pub state: String,
    /// The picked option's number; 0 until one is picked.
    #[serde(default)]
    pub option: u64,
    #[serde(default)]
    pub words: String,
    #[serde(default)]
    pub return_at: String,
    /// What a secret ask asks for; absent on any other ask.
    #[serde(default)]
    pub secret: Option<Secret>,
    /// The names a secret ask was given values for; none until it is given.
    #[serde(default, deserialize_with = "or_default")]
    pub given: Vec<String>,
}

/// One of the bead's notes as a client shows them: every note but the
/// asks and answers, which [`BeadAsk`] carries folded into their ask.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BeadNote {
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub at: String,
}

/// One bead whole: body, stage, labels, notes and its asks.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Bead {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default, deserialize_with = "or_default")]
    pub labels: Vec<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub notes: Vec<BeadNote>,
    #[serde(default, deserialize_with = "or_default")]
    pub asks: Vec<BeadAsk>,
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
    /// The bead the event is about, when it is about one.
    #[serde(default)]
    pub bead: String,
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

    /// Whether the event became a notification for `user`: its data names
    /// the notified users with the reason each.
    pub fn notified(&self, user: &str) -> bool {
        self.data
            .get("notified")
            .and_then(Value::as_object)
            .is_some_and(|n| n.contains_key(user))
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

/// The address of a yard that is not reached by its socket: its listener,
/// or what stands in front of it with TLS. `https://`, or `http://` for a
/// host that is this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Address {
    tls: bool,
    /// The host that is connected to, and with the port, when one was
    /// given, as the request's `Host` names it.
    host: String,
    authority: String,
    port: u16,
    /// What stands before `/user/`, with no slash at its end.
    path: String,
}

impl Address {
    /// Reads an address as `yardr user device pair` prints it. Plain HTTP
    /// is refused off this machine: the yard does no TLS and cannot tell
    /// whether anything in front of it does, so the rule is the client's.
    pub fn parse(text: &str) -> std::result::Result<Address, String> {
        let text = text.trim();
        let (tls, rest) = if let Some(rest) = text.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(format!(
                "{text:?} is no address of a yard: one starts with https://, or with http:// for a yard on this machine"
            ));
        };
        // The address goes into the head of every request as it is.
        if rest
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '@'))
        {
            return Err(format!(
                "{text:?} is no address of a yard: it has no space, no query, no # and no name before an @"
            ));
        }
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let no_host = || format!("{text:?} is no address of a yard: it names no host");
        let (host, port) = match authority.strip_prefix('[') {
            // "[::1]:8790"
            Some(v6) => {
                let (host, port) = v6.split_once(']').ok_or_else(no_host)?;
                (host, port.strip_prefix(':'))
            }
            None => match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            },
        };
        if host.is_empty() || host.contains(':') && !authority.starts_with('[') {
            return Err(no_host());
        }
        let port = match port {
            Some(port) => port
                .parse()
                .map_err(|_| format!("{text:?} is no address of a yard: {port:?} is no port"))?,
            None if tls => 443,
            None => 80,
        };
        let here = host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if !tls && !here {
            return Err(format!(
                "{text} is refused: plain http is for a yard on this machine alone (127.0.0.1, ::1, localhost). The token would cross the network for anyone to read. Give the https:// address of what stands in front of the yard's listener."
            ));
        }
        Ok(Address {
            tls,
            host: host.to_string(),
            authority: authority.to_string(),
            port,
            path: path.trim_end_matches('/').to_string(),
        })
    }

    fn connect(&self, timeout: Option<Duration>) -> io::Result<Conn> {
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} names no address", self.host),
        );
        let mut tcp = None;
        for addr in (self.host.as_str(), self.port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(conn) => {
                    tcp = Some(conn);
                    break;
                }
                Err(err) => last = err,
            }
        }
        let Some(tcp) = tcp else {
            return Err(last);
        };
        tcp.set_read_timeout(timeout)?;
        tcp.set_write_timeout(Some(CALL_TIMEOUT))?;
        socket2::SockRef::from(&tcp)
            .set_tcp_keepalive(&socket2::TcpKeepalive::new().with_time(KEEPALIVE))?;
        if !self.tls {
            return Ok(Conn::Tcp(tcp));
        }
        let name = rustls::pki_types::ServerName::try_from(self.host.clone())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
        let session = rustls::ClientConnection::new(tls()?, name).map_err(io::Error::other)?;
        Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(session, tcp))))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.tls { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority, self.path)
    }
}

/// How a server is trusted: by the roots this system trusts, read once.
/// yardr does no TLS itself, so what answers is the operator's proxy or
/// tunnel, with a certificate as any site has one.
fn tls() -> io::Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<std::result::Result<Arc<rustls::ClientConfig>, String>> =
        OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            let found = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(found.certs);
            if roots.is_empty() {
                let why: Vec<String> = found.errors.iter().map(ToString::to_string).collect();
                return Err(format!(
                    "this system's root certificates could not be read: {}",
                    why.join("; ")
                ));
            }
            // The provider is named, not the process's default: gpui's tree
            // may bring another, and rustls then has no default to take.
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|err| err.to_string())?
                .with_root_certificates(roots)
                .with_no_client_auth();
            // The requests are written out as HTTP/1.1.
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Ok(Arc::new(config))
        })
        .clone()
        .map_err(io::Error::other)
}

/// One connection to a yard, whichever way it is reached.
enum Conn {
    Unix(UnixStream),
    Tcp(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Unix(conn) => conn.read(buf),
            Conn::Tcp(conn) => conn.read(buf),
            Conn::Tls(conn) => conn.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Unix(conn) => conn.write(buf),
            Conn::Tcp(conn) => conn.write(buf),
            Conn::Tls(conn) => conn.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Unix(conn) => conn.flush(),
            Conn::Tcp(conn) => conn.flush(),
            Conn::Tls(conn) => conn.flush(),
        }
    }
}

/// Who a device's token is, as `GET /user/me` answers: in place of the
/// name a client on the yard's machine takes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Me {
    /// The user every call is made as.
    pub user: String,
    /// The device's name.
    #[serde(default)]
    pub said_by: String,
    /// The yard's name.
    #[serde(default)]
    pub yard: String,
}

/// What a yard answers a pairing code with: the device's token, the one
/// time it is anywhere, and what `/user/me` says of it from then on.
#[derive(Clone, Deserialize)]
pub struct Paired {
    pub token: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub yard: String,
}

/// Whether `token` can stand in a request's head: one line of visible
/// characters, as a device's token is.
pub fn is_token(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic())
}

/// Redeems a pairing code at a yard's address: one request with no token,
/// answered with the device's. The code is sent as it was typed; the yard
/// reads it without its dash and in either case.
pub fn pair(address: &Address, code: &str) -> Result<Paired> {
    let body = json!({ "code": code }).to_string();
    let mut conn = address.connect(Some(CALL_TIMEOUT))?;
    conn.write_all(
        format!(
            "POST {}/user/pair HTTP/1.1\r\nHost: {}\r\nX-Yardr-Client: {CLIENT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            address.path,
            address.authority,
            body.len()
        )
        .as_bytes(),
    )?;
    conn.flush()?;
    let (status, mut reply) = respond(conn)?;
    let refused = |code: &str, message: &str| Error::Refused {
        code: code.to_string(),
        message: message.to_string(),
    };
    match status {
        200 => {
            let mut bytes = Vec::new();
            reply.read_to_end(&mut bytes)?;
            let paired: Paired = serde_json::from_slice(&bytes)
                .map_err(|err| Error::Protocol(format!("the pairing: {err}")))?;
            if !is_token(&paired.token) {
                return Err(Error::Protocol(
                    "the pairing: no token in the answer".into(),
                ));
            }
            Ok(paired)
        }
        403 => Err(refused(
            "wrong_code",
            "That is not the code. The yard counts a wrong one, and the fifth ends the pairing.",
        )),
        409 => Err(refused(
            "not_added",
            "The code was right and is used up, but the yard could not add the device. Its operator opens another pairing.",
        )),
        400 => Err(refused(
            "no_code",
            "That is no code: a code is eight letters and digits, as the yard printed them.",
        )),
        401 | 404 => Err(refused(
            "no_pairing",
            "No pairing is open at this yard: none was opened, or the code was used, ran out or was ended. Its operator opens one: yardr user device pair <user> <device>",
        )),
        _ => Err(Error::Protocol(format!("the pairing: status {status}"))),
    }
}

/// How a yard is reached: by the socket of this machine's, or by the
/// address of another's listener with a device's token.
#[derive(Clone)]
enum Way {
    Socket(PathBuf),
    Listener { address: Address, token: String },
}

/// A client of one yard. It holds no connection: every call opens its own.
#[derive(Clone)]
pub struct Client {
    way: Way,
}

/// Without the token: a client is printed where a test fails.
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Client({})", self.place())
    }
}

impl Client {
    /// The yard of this machine, by its socket.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client {
            way: Way::Socket(socket.into()),
        }
    }

    /// A yard at an address, as the device `token` is of.
    pub fn remote(address: Address, token: impl Into<String>) -> Self {
        Client {
            way: Way::Listener {
                address,
                token: token.into(),
            },
        }
    }

    /// Whether the yard is reached at an address, and not by its socket.
    pub fn is_remote(&self) -> bool {
        matches!(self.way, Way::Listener { .. })
    }

    /// Where the yard is looked for, in words: the socket's path or the
    /// address.
    pub fn place(&self) -> String {
        match &self.way {
            Way::Socket(socket) => format!("The socket: {}", socket.display()),
            Way::Listener { address, .. } => format!("The yard: {address}"),
        }
    }

    /// Who the token is: the user every call is made as, the device and
    /// the yard. Of a yard at an address only; the socket has no such path.
    pub fn me(&self) -> Result<Me> {
        let (status, mut body) = self.send("GET", "/me", "", "", Some(CALL_TIMEOUT))?;
        match status {
            200 => {}
            404 => return Err(Error::NoDevice),
            _ => {
                return Err(Error::Protocol(format!(
                    "who this device is: status {status}"
                )));
            }
        }
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)
            .map_err(|err| Error::Protocol(format!("who this device is: {err}")))
    }

    /// Sends one request on a connection of its own and reads the answer's
    /// head. `path` is as docs/api.md has it on the socket; at an address
    /// it stands under `/user/`, with the token beside it. `headers` are
    /// whole lines. A call's answer is waited for `timeout` long.
    fn send(
        &self,
        verb: &str,
        path: &str,
        headers: &str,
        body: &str,
        timeout: Option<Duration>,
    ) -> Result<(u16, Body)> {
        let (mut conn, head) = match &self.way {
            Way::Socket(socket) => {
                let conn = UnixStream::connect(socket)?;
                conn.set_read_timeout(timeout)?;
                conn.set_write_timeout(Some(CALL_TIMEOUT))?;
                (
                    Conn::Unix(conn),
                    format!("{verb} {path} HTTP/1.1\r\nHost: yard\r\n"),
                )
            }
            Way::Listener { address, token } => (
                address.connect(timeout)?,
                format!(
                    "{verb} {}/user{path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n",
                    address.path, address.authority
                ),
            ),
        };
        // In one write: over TLS each write is a record of its own.
        conn.write_all(
            format!("{head}X-Yardr-Client: {CLIENT}\r\n{headers}\r\n{body}").as_bytes(),
        )?;
        conn.flush()?;
        let (status, body) = respond(conn)?;
        if status == 401 {
            return Err(Error::NotLetIn);
        }
        Ok((status, body))
    }

    /// The seq of the newest event.
    pub fn head(&self) -> Result<u64> {
        self.answer("Head", json!({}))
    }

    pub fn settings(&self) -> Result<Settings> {
        self.answer("Settings", json!({}))
    }

    /// The users of the yard: each has a row to be written to in, the
    /// window's own among them.
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

    /// What the yard reports to a user, apart from every conversation,
    /// oldest first; `limit` 0 is all the inbox keeps.
    pub fn user_notifications(&self, name: &str, limit: u64) -> Result<Notifications> {
        self.answer("UserNotifications", json!({ "name": name, "limit": limit }))
    }

    /// Marks the user's notifications read through a seq: a position of its
    /// own, which the mail's does not move.
    pub fn ack_user_notifications(&self, name: &str, through: u64) -> Result<()> {
        self.call(
            "AckUserNotifications",
            json!({ "name": name, "through": through }),
        )
        .map(drop)
    }

    /// The person's decisions: the open asks, oldest first, with what they
    /// dealt with beside them.
    pub fn user_asks(&self, name: &str) -> Result<Asks> {
        self.answer("UserAsks", json!({ "name": name }))
    }

    /// One bead whole: body, stage, labels, notes, its asks with their
    /// options, recommendation and state, and the about-beads.
    pub fn bead(&self, bead: &str) -> Result<Bead> {
        self.answer("Bead", json!({ "bead": bead }))
    }

    /// Picks an option with words alongside, or answers an ask with no
    /// options with words (`option` 0). Words alone on an ask with options
    /// are refused, and go as a plain note instead.
    pub fn answer_ask(
        &self,
        bead: &str,
        ask: u64,
        option: u64,
        words: &str,
        actor: &str,
    ) -> Result<()> {
        let mut args = json!({ "bead": bead, "ask": ask, "option": option, "actor": actor });
        if !words.is_empty() {
            args["words"] = json!(words);
        }
        self.call("AnswerAsk", args).map(drop)
    }

    /// Answers a secret ask: each name of the ask with the value the person
    /// typed. The yard writes the values to the file the source's
    /// credential is read from and answers the names and the file. A value
    /// is the person's secret: it is in this call's body and in nothing
    /// else of the client, no error and no log.
    pub fn give_secret(
        &self,
        bead: &str,
        ask: u64,
        values: &[(String, String)],
        actor: &str,
    ) -> Result<Given> {
        let values: serde_json::Map<String, Value> = values
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        self.answer(
            "GiveSecret",
            json!({ "bead": bead, "ask": ask, "values": values, "actor": actor }),
        )
    }

    /// Puts the ask aside: it leaves the queue and comes back as the yard
    /// says.
    pub fn defer_ask(&self, bead: &str, ask: u64, actor: &str) -> Result<()> {
        self.call(
            "DeferAsk",
            json!({ "bead": bead, "ask": ask, "actor": actor }),
        )
        .map(drop)
    }

    /// Ends the ask with no option picked: the person will not decide it.
    pub fn decline_ask(&self, bead: &str, ask: u64, words: &str, actor: &str) -> Result<()> {
        self.call(
            "DeclineAsk",
            json!({ "bead": bead, "ask": ask, "words": words, "actor": actor }),
        )
        .map(drop)
    }

    /// Puts a plain note on the bead. Words about a question with no
    /// option chosen go here, and leave the ask open.
    pub fn add_note(&self, bead: &str, text: &str, actor: &str) -> Result<()> {
        self.call(
            "AddNote",
            json!({ "bead": bead, "text": text, "actor": actor }),
        )
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
        let (status, mut body) = self.send(
            "GET",
            &format!("/blobs/{hash}"),
            "Connection: close\r\n",
            "",
            Some(CALL_TIMEOUT),
        )?;
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
        let (status, body) = self.send(
            "GET",
            &format!("/events?after={after}"),
            "Accept: application/x-ndjson\r\n",
            "",
            None,
        )?;
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
        let body = args.to_string();
        let (status, mut reply) = self.send(
            "POST",
            &format!("/call/{method}"),
            &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            ),
            &body,
            Some(CALL_TIMEOUT),
        )?;
        // No call is answered 404 by a yard: at an address it is the answer
        // of one with nothing under /user/.
        if status == 404 && self.is_remote() {
            return Err(Error::NoDevice);
        }
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
fn respond(conn: Conn) -> Result<(u16, Body)> {
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
    conn: BufReader<Conn>,
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
    use crate::fake::{TOKEN, Yard};

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
    fn an_address_is_https_or_this_machine_s() {
        let address = Address::parse(" https://mac.example.net/ ").unwrap();
        assert_eq!(address.to_string(), "https://mac.example.net");
        assert_eq!(
            (address.tls, address.host.as_str(), address.port),
            (true, "mac.example.net", 443)
        );
        // A port and a path of its own, as a proxy may have them.
        let address = Address::parse("https://mac.example.net:8443/yard/").unwrap();
        assert_eq!(address.to_string(), "https://mac.example.net:8443/yard");
        assert_eq!(
            (
                address.port,
                address.authority.as_str(),
                address.path.as_str()
            ),
            (8443, "mac.example.net:8443", "/yard")
        );
        for (here, host, port) in [
            ("http://127.0.0.1:8790", "127.0.0.1", 8790),
            ("http://localhost", "localhost", 80),
            ("http://LOCALHOST:1", "LOCALHOST", 1),
            ("http://[::1]:8790", "::1", 8790),
            ("http://127.8.9.10", "127.8.9.10", 80),
        ] {
            let address = Address::parse(here).unwrap();
            assert_eq!(
                (address.tls, address.host.as_str(), address.port),
                (false, host, port),
                "{here}"
            );
            assert_eq!(address.to_string(), here);
        }
        // Plain HTTP off this machine: the token would be read on the way.
        for far in [
            "http://mac.example.net",
            "http://192.168.1.20:8790",
            "http://100.64.0.7",
            "http://[fd7a::1]:8790",
            "http://localhost.example.net",
            "http://0.0.0.0:8790",
        ] {
            let why = Address::parse(far).unwrap_err();
            assert!(
                why.starts_with(&format!("{far} is refused: plain http")),
                "{why}"
            );
        }
        // What is no address, and what would be more lines of a request.
        for none in [
            "",
            "mac.example.net",
            "ftp://mac.example.net",
            "https://",
            "https:///user",
            "https://mac.example.net:port",
            "https://mac.example.net:99999",
            "https://::1",
            "https://[::1",
            "https://jo@mac.example.net",
            "https://mac.example.net/?a=b",
            "https://mac.example.net#K7QM-2XPD",
            "https://mac.example.net/a b",
            "https://mac.example.net\r\nX-User: jo",
        ] {
            let why = Address::parse(none).unwrap_err();
            assert!(why.contains("is no address of a yard"), "{none:?}: {why}");
        }
    }

    /// A secret is given in its own call, by the socket and at an address
    /// alike: the value is in the call's body, and in nothing that comes
    /// back of the call, whatever the yard does with it.
    #[test]
    fn a_secret_is_given_by_the_socket_and_at_an_address() {
        const VALUE: &str = "bk-live-3f9a";
        for yard in [Yard::start(), Yard::start_tcp()] {
            let client = yard.client();
            let way = client.place();
            let values = [("BOOKS_TOKEN".to_string(), VALUE.to_string())];
            let give = || client.give_secret("yd-crm", 18, &values, "jo");
            let without = |err: &Error| {
                assert!(!err.to_string().contains(VALUE), "{way}: {err}");
                assert!(!format!("{err:?}").contains(VALUE), "{way}: {err:?}");
            };
            // A yard that does not have the call says so, as of any method.
            let err = give().unwrap_err();
            assert_eq!(err.to_string(), "no such method GiveSecret", "{way}");
            without(&err);
            // One that has it answers the names and the file.
            yard.answer(
                "GiveSecret",
                r#"{"result": {"given": ["BOOKS_TOKEN"], "file": "/home/jo/.yardr/sources/books.env",
                    "credential": "BOOKS_TOKEN is set in /home/jo/.yardr/sources/books.env"}}"#,
            );
            let given = give().unwrap();
            assert_eq!(given.given, ["BOOKS_TOKEN"], "{way}");
            assert_eq!(given.file, "/home/jo/.yardr/sources/books.env", "{way}");
            assert!(given.credential.starts_with("BOOKS_TOKEN is set in"));
            assert_eq!(given.missing, "", "{way}");
            assert_eq!(
                yard.asked("GiveSecret").last(),
                Some(&json!({"bead": "yd-crm", "ask": 18,
                             "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo"})),
                "{way}"
            );
            // The value is in the body alone: no line of the request's head
            // has it.
            let head = yard.last_request();
            assert!(head.contains("/call/GiveSecret"), "{way}: {head}");
            assert!(!head.contains(VALUE), "{way}: {head}");
            // The yard says no, in its words.
            yard.answer(
                "GiveSecret",
                r#"{"error": {"code": "error", "message": "ask 18 on yd-crm is declined: it takes no secret"}}"#,
            );
            let err = give().unwrap_err();
            assert!(matches!(err, Error::Refused { .. }), "{way}: {err:?}");
            without(&err);
            // The yard went away over the call: no reply, and no word on
            // whether it wrote.
            yard.answer_nothing("GiveSecret");
            let err = give().unwrap_err();
            assert!(
                matches!(err, Error::Down(_) | Error::Protocol(_)),
                "{way}: {err:?}"
            );
            without(&err);
            // And what is no reply.
            yard.answer("GiveSecret", "not json");
            let err = give().unwrap_err();
            assert!(matches!(err, Error::Protocol(_)), "{way}: {err:?}");
            without(&err);
        }
    }

    #[test]
    fn a_secret_ask_in_the_queue_and_on_the_bead() {
        let yard = Yard::start();
        let secret = json!({"source": "books", "env": ["BOOKS_TOKEN"],
            "line": "a secret: BOOKS_TOKEN, for the source books (https://books.example.com)"});
        yard.answer(
            "UserAsks",
            &json!({"result": {
                "open": [{"bead": "yd-crm", "ask": 18, "question": "The token for the books",
                          "asker": "yardmaster", "secret": secret}],
                "decided": [{"bead": "yd-crm", "ask": 17, "question": "The token for the books",
                             "asker": "yardmaster", "state": "answered",
                             "secret": secret, "given": ["BOOKS_TOKEN"]},
                            {"bead": "yd-crm", "ask": 16, "question": "Move them now?",
                             "asker": "yardmaster", "state": "answered", "option": 1}]
            }})
            .to_string(),
        );
        let asks = yard.client().user_asks("jo").unwrap();
        let asked = asks.open[0].secret.as_ref().unwrap();
        assert_eq!(asked.source, "books");
        assert_eq!(asked.env, ["BOOKS_TOKEN"]);
        assert!(
            asked
                .line
                .starts_with("a secret: BOOKS_TOKEN, for the source books")
        );
        assert_eq!(asks.decided[0].given, ["BOOKS_TOKEN"]);
        assert_eq!(asks.decided[0].secret.as_ref(), Some(asked));
        // Any other ask has neither.
        assert_eq!(asks.decided[1].secret, None);
        assert!(asks.decided[1].given.is_empty());

        yard.answer(
            "Bead",
            &json!({"result": {"id": "yd-crm", "asks": [
                {"ask": 18, "question": "The token for the books", "options": [],
                 "state": "open", "secret": secret},
                {"ask": 17, "question": "The token for the books", "options": [],
                 "state": "answered", "secret": secret, "given": ["BOOKS_TOKEN"]},
                {"ask": 16, "question": "Move them now?", "state": "answered", "option": 1}
            ]}})
            .to_string(),
        );
        let bead = yard.client().bead("yd-crm").unwrap();
        assert_eq!(bead.asks[0].secret.as_ref(), Some(asked));
        assert!(bead.asks[0].given.is_empty());
        assert_eq!(bead.asks[1].given, ["BOOKS_TOKEN"]);
        assert_eq!(bead.asks[2].secret, None);
    }

    /// The same client, the same requests, the same answers, whichever way
    /// the yard is reached.
    #[test]
    fn a_call_an_image_and_the_stream_by_the_socket_and_at_an_address() {
        for yard in [Yard::start(), Yard::start_tcp()] {
            let client = yard.client();
            let way = client.place();
            // A call, its result and what it was asked with.
            yard.answer(
                "GetUser",
                r#"{"result": {"name": "jo", "cursor": 3, "notification_cursor": 0}}"#,
            );
            assert_eq!(client.get_user("jo").unwrap().cursor, 3, "{way}");
            assert_eq!(yard.asked("GetUser"), vec![json!({"name": "jo"})], "{way}");
            // One that answers nothing, a list, and null for a list.
            yard.answer("AckUserInbox", "{}");
            client.ack_user_inbox("jo", 12).unwrap();
            yard.answer("Crew", r#"{"result": null}"#);
            assert_eq!(client.crew().unwrap(), vec![], "{way}");
            // The yard says no, in its words.
            yard.answer(
                "Mail",
                r#"{"error": {"code": "error", "message": "mail is 1 to 4096 bytes"}}"#,
            );
            let err = client.mail("yardmaster", "", "jo", &[]).unwrap_err();
            assert_eq!(err.to_string(), "mail is 1 to 4096 bytes", "{way}");
            // What is no reply.
            yard.answer_status("Head", 500, "{}");
            assert!(matches!(client.head(), Err(Error::Protocol(_))), "{way}");
            // A server that replaces itself is asked again.
            yard.answer("Head", r#"{"result": 4}"#);
            yard.answer_once_status(
                "Head",
                503,
                r#"{"error": {"code": "replacing", "message": "the server is replacing itself"}}"#,
            );
            assert_eq!(client.head().unwrap(), 4, "{way}");
            assert_eq!(yard.asked("Head").len(), 3, "{way}");
            // A long reply in chunks.
            let text = "x".repeat(4000);
            let letters: Vec<Value> = (1..=20)
                .map(|seq| json!({"seq": seq, "from": "yardmaster", "to": "jo", "text": text}))
                .collect();
            yard.answer_chunked(
                "Conversation",
                &json!({ "result": letters }).to_string(),
                1000,
            );
            let got = client.conversation("jo", "yardmaster", 0).unwrap();
            assert_eq!((got.len(), got[19].text.as_str()), (20, text.as_str()));
            // An image: bytes that are no text, and the one that is gone.
            let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
            yard.blob(HASH, &bytes);
            assert_eq!(client.blob(HASH).unwrap(), Some(bytes), "{way}");
            assert_eq!(client.blob(&"00".repeat(32)).unwrap(), None, "{way}");
            // An image to the yard, larger than one write.
            yard.answer(
                "PutBlob",
                &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
            );
            let large = vec![7u8; 300_000];
            assert_eq!(client.put_blob(&large).unwrap().hash, HASH, "{way}");
            assert_eq!(
                yard.asked("PutBlob"),
                vec![json!({"data": BASE64.encode(&large)})],
                "{way}"
            );
            // The stream: what is after the seq, then what happens, until
            // the server goes away.
            yard.emit(r#"{"seq": 3, "kind": "mail"}"#);
            yard.emit(r#"{"seq": 4, "kind": "mail", "data": {"text": "four"}}"#);
            let mut events = client.events(3).unwrap();
            assert_eq!(events.next().unwrap().unwrap().text("text"), "four");
            yard.emit(r#"{"seq": 5, "kind": "crew_started"}"#);
            assert_eq!(events.next().unwrap().unwrap().seq, 5, "{way}");
            assert_eq!(yard.streams_asked(), vec![3], "{way}");
            yard.stop();
            assert!(events.next().is_none(), "{way}");
            // And nothing listens.
            assert!(matches!(client.head(), Err(Error::Down(_))), "{way}");
            assert!(matches!(client.events(0), Err(Error::Down(_))), "{way}");
            assert!(matches!(client.blob(HASH), Err(Error::Down(_))), "{way}");
        }
    }

    #[test]
    fn at_an_address_a_request_stands_under_user_with_the_token() {
        let yard = Yard::start_tcp();
        let client = yard.client();
        let host = yard.address().strip_prefix("http://").unwrap().to_string();
        assert!(client.is_remote());
        assert_eq!(client.place(), format!("The yard: {}", yard.address()));
        // The token is in no word a client says of itself.
        assert!(!format!("{client:?}").contains(TOKEN));

        yard.answer("Head", r#"{"result": 12}"#);
        assert_eq!(client.head().unwrap(), 12);
        let request = yard.last_request();
        assert!(
            request.starts_with("POST /user/call/Head HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains(&format!("Host: {host}\r\n")), "{request}");
        assert!(
            request.contains(&format!("Authorization: Bearer {TOKEN}\r\n")),
            "{request}"
        );
        assert!(
            request.contains(&format!(
                "X-Yardr-Client: footplate/{}\r\n",
                env!("CARGO_PKG_VERSION")
            )),
            "{request}"
        );

        yard.blob(HASH, b"png");
        client.blob(HASH).unwrap();
        assert!(
            yard.last_request()
                .starts_with(&format!("GET /user/blobs/{HASH} HTTP/1.1\r\n"))
        );
        let _stream = client.events(7).unwrap();
        assert!(
            yard.last_request()
                .starts_with("GET /user/events?after=7 HTTP/1.1\r\n")
        );

        // Who the token is.
        assert_eq!(
            client.me().unwrap(),
            Me {
                user: "jo".into(),
                said_by: "laptop".into(),
                yard: "south".into()
            }
        );
        assert!(yard.last_request().starts_with("GET /user/me HTTP/1.1\r\n"));

        // By the socket no request has a token to carry.
        let near = Yard::start();
        near.answer("Head", r#"{"result": 0}"#);
        near.client().head().unwrap();
        assert!(
            !near
                .last_request()
                .to_ascii_lowercase()
                .contains("authorization"),
            "{}",
            near.last_request()
        );
    }

    #[test]
    fn a_token_the_yard_does_not_know_is_not_let_in_and_nothing_is_asked_twice() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let stranger = yard.client_with("yardr-dev-unknown");
        let before = yard.requests();
        let err = stranger.head().unwrap_err();
        assert!(matches!(err, Error::NotLetIn), "{err:?}");
        assert_eq!(err.to_string(), "this device is no longer let in");
        // One request: a 401 is no fault that passes.
        assert_eq!(yard.requests(), before + 1);
        assert!(matches!(stranger.me(), Err(Error::NotLetIn)));
        assert!(matches!(stranger.blob(HASH), Err(Error::NotLetIn)));
        assert!(matches!(stranger.events(0), Err(Error::NotLetIn)));
        assert_eq!(yard.requests(), before + 4);
        // The call never reached the yard's store.
        assert_eq!(yard.asked("Head"), Vec::<Value>::new());

        // A device that is removed: its stream is closed, and its token is
        // refused from then on.
        let client = yard.client();
        assert_eq!(client.head().unwrap(), 12);
        let mut events = client.events(0).unwrap();
        yard.add_device("yardr-dev-phone");
        yard.remove_device(TOKEN);
        assert!(events.next().is_none());
        assert!(matches!(client.head(), Err(Error::NotLetIn)));

        // A yard with no device at all has nothing under /user/, with a
        // token or without: 404, which is said as what it is.
        yard.remove_device("yardr-dev-phone");
        for err in [client.me().unwrap_err(), client.head().unwrap_err()] {
            assert!(matches!(err, Error::NoDevice), "{err:?}");
        }
        // An image that is gone is 404 too, and is no such thing.
        yard.add_device(TOKEN);
        assert_eq!(client.blob(HASH).unwrap(), None);
    }

    #[test]
    fn a_code_is_answered_with_the_device_s_token_once() {
        let yard = Yard::start_tcp();
        let address = Address::parse(yard.address()).unwrap();
        let refused = |code: &str| match pair(&address, code) {
            Err(Error::Refused { code, message }) => (code, message),
            Err(err) => panic!("{err:?}"),
            Ok(_) => panic!("{code} was taken"),
        };
        // No pairing is open: the path is as any other without a token.
        assert_eq!(refused("K7QM-2XPD").0, "no_pairing");

        // The operator opens one. The code as it is typed: small letters,
        // no dash.
        yard.pair("K7QM-2XPD");
        let request_before = yard.requests();
        let paired = pair(&address, "k7qm2xpd").unwrap();
        assert_eq!(yard.requests(), request_before + 1);
        let request = yard.last_request();
        assert!(
            request.starts_with("POST /user/pair HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            !request.to_ascii_lowercase().contains("authorization"),
            "{request}"
        );
        assert_eq!(
            (
                paired.user.as_str(),
                paired.device.as_str(),
                paired.yard.as_str()
            ),
            ("jo", "laptop", "south")
        );
        assert!(yard.devices().contains(&paired.token));
        // The token is the device's from then on.
        assert_eq!(yard.client_with(&paired.token).me().unwrap().user, "jo");
        // The code is good once.
        assert_eq!(refused("K7QM-2XPD").0, "no_pairing");

        // A wrong code is counted, and the fifth ends the pairing: the
        // right one comes too late then.
        yard.pair("ABCD-EFGH");
        // What is no code is no try.
        assert_eq!(refused("ABCD").0, "no_code");
        for _ in 0..5 {
            let (code, message) = refused("WXYZ-2345");
            assert_eq!(code, "wrong_code");
            assert!(message.starts_with("That is not the code."), "{message}");
        }
        assert!(!yard.pairing_open());
        assert_eq!(refused("ABCD-EFGH").0, "no_pairing");
        assert_eq!(yard.devices().len(), 2);

        // A yard with no device and no pairing has no such path at all.
        for token in yard.devices() {
            yard.remove_device(&token);
        }
        assert_eq!(refused("ABCD-EFGH").0, "no_pairing");
        // And one with a pairing open for its first device has.
        yard.pair("ABCD-EFGH");
        assert!(pair(&address, "ABCD-EFGH").is_ok());
        yard.stop();
        assert!(matches!(pair(&address, "ABCD-EFGH"), Err(Error::Down(_))));
    }

    /// The handshake, the system's roots and a call's answer through them:
    /// against a site of the internet, since the fake yard speaks no TLS.
    /// Run by hand: `cargo test tls -- --ignored`.
    #[test]
    #[ignore = "asks a site of the internet"]
    fn tls_reaches_a_site_and_reads_its_answer() {
        let client = Client::remote(Address::parse("https://example.com").unwrap(), TOKEN);
        // No yard: its 404 is an HTTP answer read over TLS.
        let err = client.me().unwrap_err();
        assert!(matches!(err, Error::NoDevice), "{err:?}");
        // A name the certificate is not for is no yard to talk to.
        let wrong = Client::remote(
            Address::parse("https://wrong.host.badssl.com").unwrap(),
            TOKEN,
        );
        assert!(matches!(wrong.me(), Err(Error::Down(_))));
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
                cursor: 7,
                notification_cursor: 0
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
                 "trust": {"jo": "jo"},
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
        // Whom the link trusts as a user of this yard; nobody when absent.
        assert_eq!(
            peers[0].trust,
            [("jo".to_string(), "jo".to_string())].into()
        );
        assert!(peers[1].trust.is_empty());
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
    fn user_notifications_and_ack() {
        let yard = Yard::start();
        yard.answer(
            "UserNotifications",
            r#"{"result": {"notifications": [
                {"seq": 10, "at": "2026-10-08T09:30:00Z", "kind": "created", "bead": "yd-shed",
                 "text": "yd-shed created (all) by yardmaster", "read": false},
                {"seq": 11, "kind": "noted", "text": "yd-shed noted", "read": true}
            ], "unread": 1, "last": 10}}"#,
        );
        let notifications = yard.client().user_notifications("jo", 0).unwrap();
        assert_eq!(
            yard.asked("UserNotifications"),
            vec![json!({"name": "jo", "limit": 0})]
        );
        assert_eq!((notifications.unread, notifications.last), (1, 10));
        assert_eq!(notifications.notifications.len(), 2);
        assert_eq!(
            notifications.notifications[0],
            Notification {
                seq: 10,
                at: "2026-10-08T09:30:00Z".into(),
                kind: "created".into(),
                bead: "yd-shed".into(),
                text: "yd-shed created (all) by yardmaster".into(),
                read: false,
            }
        );
        // The bead is absent when the event is about none.
        assert_eq!(notifications.notifications[1].bead, "");
        assert!(notifications.notifications[1].read);

        yard.answer("AckUserNotifications", "{}");
        yard.client().ack_user_notifications("jo", 10).unwrap();
        assert_eq!(
            yard.asked("AckUserNotifications"),
            vec![json!({"name": "jo", "through": 10})]
        );
    }

    #[test]
    fn user_asks_open_oldest_first_with_what_was_decided() {
        let yard = Yard::start();
        yard.answer(
            "UserAsks",
            r#"{"result": {"open": [
                {"bead": "yd-links", "bead_title": "Move the links", "topic": "topic:linear-out",
                 "ask": 3, "question": "Move the links now?", "asker": "yardmaster",
                 "since": "2026-10-08T09:30:00Z"},
                {"bead": "yd-greet", "bead_title": "The greeting", "topic": "",
                 "ask": 5, "question": "What tone?", "asker": "yardmaster",
                 "since": "2026-10-09T09:30:00Z"}
            ], "decided": [
                {"bead": "yd-links", "bead_title": "Move the links", "topic": "topic:linear-out",
                 "ask": 6, "question": "Send them to build?", "asker": "yardmaster",
                 "since": "2026-10-07T09:30:00Z", "state": "answered",
                 "option": 1, "option_label": "Send them", "words": "tonight"},
                {"bead": "yd-greet", "bead_title": "The greeting", "topic": "",
                 "ask": 12, "question": "Build the cleanup?", "asker": "assembly",
                 "since": "2026-10-06T09:30:00Z", "state": "put aside",
                 "return_at": "2026-10-10T09:30:00Z"}
            ]}}"#,
        );
        let asks = yard.client().user_asks("jo").unwrap();
        assert_eq!(yard.asked("UserAsks"), vec![json!({"name": "jo"})]);
        assert_eq!(asks.open.len(), 2);
        assert_eq!(
            (asks.open[0].bead.as_str(), asks.open[0].ask),
            ("yd-links", 3)
        );
        assert_eq!(asks.open[0].topic, "topic:linear-out");
        assert_eq!(asks.open[1].topic, "");
        assert_eq!(asks.decided.len(), 2);
        assert_eq!(
            (
                asks.decided[0].state.as_str(),
                asks.decided[0].option,
                asks.decided[0].option_label.as_str()
            ),
            ("answered", 1, "Send them")
        );
        assert_eq!(asks.decided[1].return_at, "2026-10-10T09:30:00Z");
    }

    #[test]
    fn bead_whole_with_options_recommendation_and_state() {
        let yard = Yard::start();
        yard.answer(
            "Bead",
            r#"{"result": {
                "id": "yd-links", "title": "Move the links",
                "body": "The links file declares the new source.",
                "stage": "new", "labels": ["topic:linear-out"],
                "notes": [
                    {"seq": 2, "kind": "note", "author": "yardmaster",
                     "text": "the test passed on a copy", "at": "2026-10-08T09:30:00Z"}
                ],
                "asks": [
                    {"ask": 3, "to": "jo", "asker": "yardmaster",
                     "since": "2026-10-08T09:30:00Z",
                     "question": "Move this yard's links to the declared source now?",
                     "context": ["The links file declares the new source."],
                     "options": [
                        {"label": "Move them now", "consequence": "all 64 links point at the new source tonight"},
                        {"label": "You decide", "consequence": "leave it to yardmaster"}
                     ],
                     "recommend": 1, "why": "the test passed",
                     "about": [{"id": "yd-train", "title": "Release train", "stage": "new"}],
                     "state": "open"},
                    {"ask": 5, "to": "jo", "asker": "yardmaster",
                     "since": "2026-10-08T09:30:00Z",
                     "question": "What tone?", "context": ["the greeting is new."],
                     "options": [], "state": "open"}
                ]}}"#,
        );
        let bead = yard.client().bead("yd-links").unwrap();
        assert_eq!(yard.asked("Bead"), vec![json!({"bead": "yd-links"})]);
        assert_eq!(
            (bead.id.as_str(), bead.title.as_str()),
            ("yd-links", "Move the links")
        );
        assert_eq!(bead.labels, ["topic:linear-out"]);
        assert_eq!(bead.notes.len(), 1);
        assert_eq!(bead.notes[0].author, "yardmaster");
        assert_eq!(bead.asks.len(), 2);
        let ask = &bead.asks[0];
        assert_eq!(ask.options.len(), 2);
        assert_eq!(ask.options[1].label, "You decide");
        assert_eq!((ask.recommend, ask.why.as_str()), (1, "the test passed"));
        assert_eq!(ask.about[0].id, "yd-train");
        assert_eq!(ask.state, "open");
        // An ask with no options wants words.
        assert!(bead.asks[1].options.is_empty());
        assert_eq!(bead.asks[1].recommend, 0);
    }

    #[test]
    fn null_where_a_bead_lists_are_promised_is_no_lists() {
        let yard = Yard::start();
        yard.answer(
            "Bead",
            r#"{"result": {"id": "yd-links", "title": "t", "body": "", "stage": "new",
                "labels": null, "notes": null, "asks": null}}"#,
        );
        let bead = yard.client().bead("yd-links").unwrap();
        assert_eq!(bead.labels, Vec::<String>::new());
        assert_eq!(bead.notes, vec![]);
        assert_eq!(bead.asks, vec![]);
    }

    #[test]
    fn answer_defer_decline_and_note_write_what_they_are_given() {
        let yard = Yard::start();
        yard.answer("AnswerAsk", "{}");
        yard.answer("DeferAsk", "{}");
        yard.answer("DeclineAsk", "{}");
        yard.answer("AddNote", "{}");
        let client = yard.client();
        // A pick with words in the one answer.
        client
            .answer_ask("yd-links", 3, 1, "move them tonight", "jo")
            .unwrap();
        // A pick alone sends no words.
        client.answer_ask("yd-links", 3, 2, "", "jo").unwrap();
        // Words answer an ask with no options.
        client.answer_ask("yd-greet", 5, 0, "warmly", "jo").unwrap();
        assert_eq!(
            yard.asked("AnswerAsk"),
            vec![
                json!({"bead": "yd-links", "ask": 3, "option": 1, "words": "move them tonight", "actor": "jo"}),
                json!({"bead": "yd-links", "ask": 3, "option": 2, "actor": "jo"}),
                json!({"bead": "yd-greet", "ask": 5, "option": 0, "words": "warmly", "actor": "jo"}),
            ]
        );
        client.defer_ask("yd-links", 4, "jo").unwrap();
        assert_eq!(
            yard.asked("DeferAsk"),
            vec![json!({"bead": "yd-links", "ask": 4, "actor": "jo"})]
        );
        client
            .decline_ask("yd-greet", 5, "the greeting can wait", "jo")
            .unwrap();
        assert_eq!(
            yard.asked("DeclineAsk"),
            vec![
                json!({"bead": "yd-greet", "ask": 5, "words": "the greeting can wait", "actor": "jo"})
            ]
        );
        client
            .add_note("yd-greet", "the newcomer arrives Monday", "jo")
            .unwrap();
        assert_eq!(
            yard.asked("AddNote"),
            vec![json!({"bead": "yd-greet", "text": "the newcomer arrives Monday", "actor": "jo"})]
        );
    }

    #[test]
    fn an_event_names_its_bead_and_whom_it_notified() {
        let event: Event = serde_json::from_value(json!({
            "seq": 11, "kind": "noted", "actor": "yardmaster", "bead": "yd-shed",
            "data": {"note": 1, "notified": {"jo": "all"}}
        }))
        .unwrap();
        assert_eq!(event.bead, "yd-shed");
        assert!(event.notified("jo"));
        assert!(!event.notified("ada"));
        let bare: Event = serde_json::from_value(json!({"seq": 12, "kind": "mail"})).unwrap();
        assert_eq!(bare.bead, "");
        assert!(!bare.notified("jo"));
    }

    #[test]
    fn null_where_asks_are_promised_is_no_asks() {
        let yard = Yard::start();
        yard.answer("UserAsks", r#"{"result": {"open": null, "decided": null}}"#);
        assert_eq!(yard.client().user_asks("jo").unwrap(), Asks::default());
        yard.answer(
            "UserNotifications",
            r#"{"result": {"notifications": null, "unread": 0, "last": 0}}"#,
        );
        assert_eq!(
            yard.client().user_notifications("jo", 0).unwrap(),
            Notifications::default()
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
        let socket = yard.socket().to_path_buf();
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
