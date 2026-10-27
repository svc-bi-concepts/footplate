//! The yard's socket API, as docs/api.md of the yardr repository promises it
//! to a program that is not yardr: HTTP/1.1 over a unix socket, a call a
//! `POST /call/<Method>` with its arguments as a JSON object, the stream a
//! `GET /events?after=<seq>` of one JSON event a line.
//!
//! A yard that is not on this machine is reached at its listener instead,
//! as that page's "Over the network" has it: the same requests under
//! `/user/`, over TCP with TLS, each with the device's token.
//!
//! Only the calls and fields that page lists are read here. The few
//! lines of HTTP are written out, since a call is one request on one
//! connection and neither needs a runtime nor a client library with a unix
//! connector.
//!
//! The socket is a unix one: on Windows there is no yard of this machine to
//! reach by it, and a yard is reached at its address alone.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use std::{env, fmt, thread};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};

use crate::log;

/// What footplate says it is to the yard. Nothing is decided by it today.
const CLIENT: &str = concat!("footplate/", env!("CARGO_PKG_VERSION"));

/// How long a call may take to answer. The stream is silent while nothing
/// happens: it is read as long as its follower says (`Client::events`).
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a yard's address may take to be connected to.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The sandbox origin's port: a note's pages run there, on the web
/// view's host, unless the run base says otherwise.
const RUN_PORT: u16 = 8789;

/// A stream over the network is silent while nothing happens, and a link
/// that went away says nothing either: the system asks the other end after
/// this long a silence, then again every `KEEPALIVE_INTERVAL`, and after
/// `KEEPALIVE_RETRIES` asked with no answer the connection is dead. So a
/// dead stream ends within about a minute and is opened again, where the
/// system's own interval and count (75 s, 8 on macOS) take ten.
const KEEPALIVE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
const KEEPALIVE_RETRIES: u32 = 3;

/// A server that replaces itself with a newer build refuses calls meanwhile;
/// yardr makes the call again after this long, for up to that long.
const REPLACING_PAUSE: Duration = Duration::from_millis(100);
const REPLACING_PATIENCE: Duration = Duration::from_secs(30);

/// A GET the yard is slow to answer is asked again after this long, this
/// many times: GETs change nothing, so asking again is safe. Anything else
/// is asked once: a write may have run, and a read called by POST is no GET.
const SLOW_PAUSE: Duration = Duration::from_millis(100);
const SLOW_RETRIES: u32 = 1;

/// The error code of a server that is replacing itself, and of a name the
/// yard does not have.
const REPLACING: &str = "replacing";
pub const NOT_FOUND: &str = "not-found";

/// The most bytes `PutBlob` takes in a yard that has set no cap of its own.
pub const BLOB_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The most files a mail or a note carries in a yard that answers no
/// count of its own.
pub const FILES_MAX: u64 = 64;

/// Why a call or the stream did not answer.
#[derive(Debug)]
pub enum Error {
    /// Nothing listens: the socket is missing, refused, or went away.
    Down(io::Error),
    /// The yard is slow to answer: a connect, read or write gave up for
    /// its timeout (on macOS a slow unix socket reads as EAGAIN, "os error
    /// 35"). `idempotent` says the request changes nothing, so its timeout
    /// says nothing of the yard having done anything: a GET was asked
    /// again first, and a read is read again by the window. Anything else
    /// was asked once, and may have run. A call is a POST whether it reads
    /// or writes, so it is told by the call (`reads`) and not by the verb.
    /// `sent` says the request went out: a connect that gave up sent
    /// nothing, so its write is said to be unsent and safe to send again.
    Slow { idempotent: bool, sent: bool },
    /// The call reached the yard, and the yard said no. `message` is a
    /// sentence for a person: shown, never read.
    Refused { code: String, message: String },
    /// What came back is not what docs/api.md says.
    Protocol(String),
    /// The yard's listener answered 401 with `WWW-Authenticate: Bearer`:
    /// the token is not a device's, or the device was removed. It is no
    /// fault that passes, so nothing is asked again until the device has a
    /// token again.
    NotLetIn,
    /// The edge answered 401 carrying `X-Edge-Refusal: session`: the
    /// session token is none Kratos knows, or none was sent. It is no
    /// fault that passes either, so nothing is asked again until signed
    /// in again. The device pairing is untouched.
    NotSignedIn,
    /// A 401 that is neither of those: it carries neither the edge's
    /// `X-Edge-Refusal: session` nor the listener's `WWW-Authenticate:
    /// Bearer`, so no check of this device refused it. Something else
    /// answered, as the yard's web view does a request the edge sent it by
    /// mistake. The device is not out, so this is no reason to pair again;
    /// what asked is asked again.
    Gateway,
    /// At the yard's address nothing is under `/user/`: 404 to a call. A
    /// yard answers so while it has no device at all, with a token or
    /// without, and so does an address that is no yard's listener.
    NoDevice,
    /// The stream said nothing for as long as its reader waits, and stands
    /// between two events: it may be read on. Its reader asks the yard
    /// whether anything happened meanwhile.
    Silent,
}

impl Error {
    /// Whether the yard answered that it has no such user, crew member or
    /// peer.
    pub fn not_found(&self) -> bool {
        matches!(self, Error::Refused { code, .. } if code == NOT_FOUND)
    }

    /// Whether the edge answered for its link and not the yard: the yard
    /// is away, gave no answer in time, or the link is busy. None of them
    /// is the yard's word on the call, so what asked is asked again.
    pub fn edge_link(&self) -> bool {
        matches!(self, Error::Refused { code, .. } if matches!(code.as_str(), "away" | "no_answer" | "busy"))
    }

    /// Whether the call may have run all the same: the edge sent it down
    /// the link and heard nothing back in time. (Away and busy are
    /// answered before it is sent.)
    pub fn may_have_run(&self) -> bool {
        matches!(self, Error::Refused { code, .. } if code == "no_answer")
    }

    /// How a request that failed so is said in the line it leaves
    /// ([`Sent`]).
    fn outcome(&self) -> &'static str {
        match self {
            Error::Slow { .. } | Error::Silent => "timeout",
            Error::Down(err) if timed_out(err) => "timeout",
            Error::Down(_) => "down",
            Error::Refused { .. }
            | Error::NotLetIn
            | Error::NotSignedIn
            | Error::Gateway
            | Error::NoDevice => "refused",
            Error::Protocol(_) => "unreadable",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Down(err) => write!(f, "the yard does not answer ({err})"),
            Error::Slow {
                idempotent: true, ..
            } => f.write_str("the yard is slow to answer; trying again"),
            Error::Slow {
                idempotent: false,
                sent: false,
            } => f.write_str(
                "the yard did not answer in time, and nothing was sent; it is safe to send again",
            ),
            Error::Slow {
                idempotent: false,
                sent: true,
            } => f.write_str(
                "the yard did not answer in time, and the action may or may not have happened",
            ),
            Error::Refused { message, .. } => f.write_str(message),
            Error::Protocol(what) => {
                write!(f, "the yard answered what footplate cannot read: {what}")
            }
            Error::NotLetIn => f.write_str("this device is no longer let in"),
            Error::NotSignedIn => f.write_str("this session is no longer signed in"),
            Error::Gateway => f.write_str(
                "a gateway turned the request away (401), which does not mean this device is out",
            ),
            Error::NoDevice => {
                f.write_str("the yard has no device, or the address is not a yard's")
            }
            Error::Silent => f.write_str("the yard's stream said nothing for a while"),
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

/// An ask the yard says nothing of is concealed: a yard from before the
/// ask said draws every secret hidden.
fn concealed() -> bool {
    true
}

/// The yard's settings, of which a client reads the operator, the cap on
/// a file, and how many of them a mail or a note carries.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub operator: String,
    /// A whole number in a string; absent when the yard has set none.
    #[serde(default)]
    pub blob_max_bytes: String,
    /// A whole number in a string; absent when the yard answers none.
    #[serde(default)]
    pub files_max: String,
}

impl Settings {
    /// The most bytes `PutBlob` takes.
    pub fn blob_cap(&self) -> u64 {
        self.blob_max_bytes.parse().unwrap_or(BLOB_MAX_BYTES)
    }

    /// The most files a mail or a note carries.
    pub fn files_cap(&self) -> u64 {
        self.files_max.parse().unwrap_or(FILES_MAX)
    }
}

/// A file mail carries, or a note of a bead, or a hook delivery: an image,
/// a page, or any other bytes under the cap, with its base name. As a
/// letter, a note or `DeliveryFiles` names it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BlobRef {
    /// The SHA-256 of its bytes, in hex: `GET /blobs/<hash>` answers them.
    #[serde(default)]
    pub hash: String,
    /// What the bytes are, as the yard read them: `image/png`,
    /// `image/jpeg`, `text/html; charset=utf-8` for a page, anything else
    /// for a file.
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub bytes: u64,
    /// The file's base name, as the sender gave it; empty for what was
    /// pasted, and for a letter from before files carried names.
    #[serde(default)]
    pub name: String,
    /// On mail from a linked yard and on a hook delivery's file only: why
    /// this yard did not take the file, when it did not. There is none to
    /// ask for then, and no hash.
    #[serde(default)]
    pub not_taken: String,
}

/// A file `Mail`, `MailPeer` and `AddNote` carry, by the hash `PutBlob`
/// answered and the base name the person gave it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BlobFile {
    pub hash: String,
    pub name: String,
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
    /// Whether the running session sits at its harness's login screen,
    /// which its runtime often reads as idle, and whether the yard leads
    /// that login through asks of the operator. The yard serves each only
    /// while true, so an absent field reads false, as does a yard from
    /// before it served them; the handle is not read. Both are listed on
    /// the `Crew` table of yardr's docs/api.md.
    #[serde(default)]
    pub login_screen: bool,
    #[serde(default)]
    pub login_led: bool,
}

/// The arrivals board as the yard serves it: the rows in arrival order.
/// footplate draws them flat in that order; the platforms list the yard
/// serves beside them is not read.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Arrivals {
    #[serde(default, deserialize_with = "or_default")]
    pub rows: Vec<Arrival>,
}

/// One row of the arrivals board: a bead still travelling, its title,
/// its platform (stage) and line (depot), the expected arrival as the
/// yard prints it, and the remark with its tone. Served ordered and
/// remarked; footplate draws the rows as they come.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Arrival {
    pub bead: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub platform: String,
    /// The depot the bead is worked in, `""` when it has none.
    #[serde(default)]
    pub line: String,
    /// The expected arrival as the yard prints it: the usual range, one
    /// span when both ends agree, `—` where the yard serves none.
    #[serde(default)]
    pub expected: String,
    #[serde(default)]
    pub remark: String,
    /// `red`, `amber` or `none`.
    #[serde(default)]
    pub tone: String,
    /// Who holds the bead, as the yard names them. Empty where no hold
    /// is on, and where the yard kept no holder: absent is unknown.
    #[serde(default)]
    pub held_by: String,
}

/// The flows view as the yard serves it: the depots as lines, each with
/// its travelling total, its flows, each flow its stages in travel order
/// with the open beads on each and its lamp, and its Elsewhere rows; the
/// beads travelling with no line; and the yard-wide travelling total.
/// footplate draws the lines, their flows, stages, beads and lamps as
/// they come, in the served order.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Flows {
    /// The yard's open beads outside the backlog: the depots' totals plus
    /// the unassigned count. Complete, not capped.
    #[serde(default)]
    pub travelling: usize,
    #[serde(default, deserialize_with = "or_default")]
    pub depots: Vec<FlowDepot>,
    /// The open beads outside the backlog with no depot, the newest
    /// change first, each row shaped as a flow bead.
    #[serde(default, deserialize_with = "or_default")]
    pub unassigned: Vec<FlowBead>,
}

/// One depot as the flows view shows it: its flows, and the open beads
/// in no stage of them. Every depot stands here, travelling or not.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FlowDepot {
    pub name: String,
    /// `git` or `dir`, as the depot is.
    #[serde(default)]
    pub kind: String,
    /// The depot's open beads outside the backlog.
    #[serde(default)]
    pub travelling: usize,
    #[serde(default, deserialize_with = "or_default")]
    pub flows: Vec<Flow>,
    /// The depot's open beads in no stage of the flows above, the newest
    /// change first, as on /flows.
    #[serde(default, deserialize_with = "or_default")]
    pub elsewhere: Vec<FlowBead>,
    /// Why the depot's flows could not be read; its beads are then all
    /// in Elsewhere. Absent when they read.
    #[serde(default)]
    pub error: String,
}

/// One flow of a depot: the depot's own (an empty type) or a bead
/// type's, with its stages in travel order.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Flow {
    pub name: String,
    /// The bead type it governs, `""` for the depot's own.
    #[serde(default, rename = "type")]
    pub flow_type: String,
    /// Where the flow comes from, as `yardr flow show` says it: `store`,
    /// `repo@<sha>`, `repo (directory)`.
    #[serde(default)]
    pub source: String,
    #[serde(default, deserialize_with = "or_default")]
    pub stages: Vec<FlowStage>,
}

/// One stage of a flow and the open beads in it, with the stage's wait
/// light, whether entering it closes the bead, and its routed group's
/// seats.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FlowStage {
    pub stage: String,
    /// The group a bead of the flow's type would be routed to in this
    /// stage, `""` when no route matches.
    #[serde(default)]
    pub group: String,
    /// The group is a manual one.
    #[serde(default)]
    pub human: bool,
    /// `fault` where a held, blocked or stranded bead stands here,
    /// `turn` where only a person's turn does, `none` otherwise.
    #[serde(default)]
    pub lamp: String,
    /// The stage closes the bead entering it.
    #[serde(default)]
    pub terminal: bool,
    /// The routed group's seat limit, `0` pausing the group. Absent
    /// where no route matches, so a reader can tell unrouted from
    /// paused.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many of the routed group's seats live sessions hold across
    /// the whole yard, the same number on every stage routed to the
    /// group. Absent where no route matches.
    #[serde(default)]
    pub taken: Option<usize>,
    #[serde(default, deserialize_with = "or_default")]
    pub beads: Vec<FlowBead>,
}

/// One bead of the flows view: the bead with its Arrivals row, so the
/// board and the flows can never disagree about it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FlowBead {
    pub bead: String,
    #[serde(default)]
    pub title: String,
    /// The expected arrival as the yard prints it: the usual range, one
    /// span when both ends agree, `—` where the yard serves none.
    #[serde(default)]
    pub expected: String,
    #[serde(default)]
    pub remark: String,
    /// `red` for a hold and a stall, `amber` for a block, a wait and a
    /// late mark, `none` with no remark.
    #[serde(default)]
    pub tone: String,
    /// Who holds the bead, as the yard names them. Empty where no hold
    /// is on, and where the yard kept no holder: absent is unknown.
    #[serde(default)]
    pub held_by: String,
    /// An agent's or a command's session claimed it; a manual group's
    /// claim is a person's turn and no work.
    #[serde(default)]
    pub working: bool,
    /// Who works it, as `yardr session list` shows the claiming
    /// session. Absent where no session claimed it, and on a manual
    /// group's claim.
    #[serde(default)]
    pub worker: String,
    /// The group whose session works the bead, as `yardr session list`
    /// shows it. Absent where `worker` is.
    #[serde(default)]
    pub group: String,
    /// The harness kind the session started with (`claude`, `muse`,
    /// ...). Absent where the session has none: a group whose runner
    /// is not herdr, or a session from before the yard recorded it.
    #[serde(default)]
    pub kind: String,
    /// The model the session started with, as served. Absent where
    /// its group's config named none, and the harness's own was used.
    #[serde(default)]
    pub model: String,
    /// When the bead entered its stage, as an RFC 3339 time. Absent
    /// where the stay has no clock, whether or not a session claimed it.
    #[serde(default)]
    pub entered_stage_at: String,
}

/// One source's grant to one holder: what of it they may call, and as
/// whom, with when, by whom and on which card each call was given. What
/// `yardr source grants` prints, and footplate's grants view lists. A
/// holder's plain grant and its grant as a user are two records.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SourceGrant {
    pub source: String,
    /// The group that holds it, each session it starts; absent for a crew
    /// member's.
    #[serde(default)]
    pub group: String,
    /// The crew member that holds it; absent for a group's.
    #[serde(default)]
    pub crew: String,
    /// The user whose credential a call under it is made with; absent for
    /// a plain grant.
    #[serde(default, rename = "as")]
    pub as_user: String,
    /// `read` and the names of calls granted, sorted.
    #[serde(default, deserialize_with = "or_default")]
    pub calls: Vec<String>,
    /// When the record last gained a call: the record's, for no call in
    /// particular.
    #[serde(default)]
    pub at: String,
    /// Who gave the last call added to the record; the record's, for no
    /// call in particular.
    #[serde(default)]
    pub by: String,
    /// Each entry of `calls` to its own grant, keyed by the entry; a call
    /// with no key was granted before its history was kept, and a record
    /// with none has no `given`.
    #[serde(default, deserialize_with = "or_default")]
    pub given: HashMap<String, GrantGiven>,
    /// Each entry of `calls` to its kind, `read` or `update`, as the yard
    /// reads it from the source's declaration, keyed by the entry as `given`
    /// is. A call with no key has no kind served: its source is gone or
    /// unreadable, or no longer declares it. So has every call of a yard
    /// from before it served kinds. Absent is unknown, and the window takes
    /// it for an update (`model::grant_is_update`).
    #[serde(default, deserialize_with = "or_default")]
    pub kinds: HashMap<String, String>,
}

/// When one call of a grant was given and by whom, and the card it was
/// approved on where it was: the bead, the ask and who asked it. Asker is
/// empty where the ask's note is gone; it is never guessed.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GrantGiven {
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub by: String,
    /// The bead of the card it was approved on; absent for a grant given
    /// at a terminal.
    #[serde(default)]
    pub bead: String,
    /// The ask it was approved on; absent for a grant at a terminal.
    #[serde(default)]
    pub ask: u64,
    /// Who asked it; absent for a grant at a terminal, and where the ask's
    /// note is gone.
    #[serde(default)]
    pub asker: String,
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

/// A part of a conversation, oldest first, and whether the yard has older
/// mail of it than this.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub letters: Vec<Letter>,
    pub more: bool,
}

/// One party's emoji on a mail, as [`Client::react`] put it there.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Reaction {
    /// Who put it: the mail's sender or its receiver.
    #[serde(default)]
    pub from: String,
    /// Which of the yard's fixed set it is.
    #[serde(default)]
    pub emoji: String,
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
    /// The parties' emoji on the mail, by who put them. A yard from
    /// before reactions sends no such field, and the mail reads as it
    /// always has.
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    /// The files it carries, in the order it was sent with them.
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

/// What `SessionScreen` answers: a running session's visible screen,
/// with the terminal's ANSI styling.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SessionScreen {
    #[serde(default)]
    pub screen: String,
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
    /// What is left to do with what was given, in one line of the yard's:
    /// the command that tells a person through the source their own
    /// address was given for. Absent when nothing is; shown as it is.
    #[serde(default)]
    pub next: String,
}

/// The command a run ask proposes: one yardr command of the yard's fixed
/// list, which the yard runs as its operator when the ask is approved with
/// `ApproveAsk`. All of it is the yard's, written from its own records when
/// the ask was, and none of it the asker's. Such an ask has no options and
/// takes no words.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Run {
    /// The command as the yard parsed it, without the leading `yardr`.
    #[serde(default, deserialize_with = "or_default")]
    pub argv: Vec<String>,
    /// The command whole, as it is typed in a terminal: shown as it is,
    /// never cut.
    #[serde(default)]
    pub command: String,
    /// The command's one line from its help, shown under the command.
    /// Absent on a yard from before the card said it.
    #[serde(default)]
    pub description: String,
    /// The yard's sentences for what the command changes and what is there
    /// now. For a source or a pack they are the whole card: every call,
    /// command and object, each on its own line, never a summary.
    #[serde(default, deserialize_with = "or_default")]
    pub does: Vec<String>,
    /// The command that takes the change back.
    #[serde(default)]
    pub back: String,
    /// Who may run `back`, in words that stand before "can take it back".
    /// Absent where that is anyone.
    #[serde(default)]
    pub back_by: String,
    /// What adding a source takes, for an ask that adds one: each source's
    /// declaration with its command files' texts, the yard's own bytes for
    /// checking behind the card. Absent on any other command.
    #[serde(default, deserialize_with = "or_default")]
    pub carried: Vec<RunSource>,
    /// The pack a pack apply ask names, with a depot kind's plan. Absent
    /// on any other command.
    #[serde(default)]
    pub pack: Option<RunPack>,
}

/// What adding one source takes, carried on a run ask: the source's name,
/// the declaration's whole text, and each command file's bytes. A pack
/// that would replace a source also carries each command file's change
/// and each entry the copy would remove. The yard carries the whole
/// destination inventory beside these for its own stale holding, which no
/// card shows and this struct does not keep.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunSource {
    #[serde(default)]
    pub name: String,
    /// The declaration's whole text, the ask's own.
    #[serde(default)]
    pub text: String,
    #[serde(default, deserialize_with = "or_default")]
    pub files: Vec<RunFile>,
    /// Each command file the pack's file would change, with its unified
    /// diff old against new. The old bytes are not carried: the diff is
    /// the showing, and the two hashes are what the approval is held to.
    #[serde(default, deserialize_with = "or_default")]
    pub old: Vec<RunFileDiff>,
    /// Each entry neither declaration names that the copy would remove.
    #[serde(default, deserialize_with = "or_default")]
    pub removed: Vec<RunRemoved>,
}

/// One command file a run ask carries: the program as the declaration
/// names it, its whole text, and the sha256 of that text.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunFile {
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub sha256: String,
}

/// One command file a pack ask would change: its program, the sha256 of
/// the copy that stands and of the pack's file, whole, and the unified
/// diff old against new.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunFileDiff {
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub old_sha256: String,
    #[serde(default)]
    pub new_sha256: String,
    #[serde(default)]
    pub diff: String,
}

/// One entry under a source's copies that neither declaration names: its
/// path, its hash, and its text where it is text, for checking behind
/// the card. A non-text entry has no text: removing it installs nothing.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunRemoved {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub text: String,
}

/// A pack apply ask's pack as the yard's server read it from its own
/// binary: its name, kind, the depot a depot kind would be re-applied to,
/// and the build label the pack is embedded in. A depot kind carries the
/// plan the server built; the plan's objects and governs stand in the
/// yard's sentences, and its file texts and diffs behind the card.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunPack {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub depot: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub plan: Option<RunPlan>,
}

/// A depot pack's plan as a card holds it: every object with its action
/// and changes. The changes stand in the yard's sentences; what is kept
/// here is what the tap shows: each changed file's new text and its
/// unified diff old against new.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunPlan {
    #[serde(default, deserialize_with = "or_default")]
    pub objects: Vec<RunPlanObject>,
}

/// One object of a depot plan: of a file, its hashes whole, old and new,
/// and for a create or change its new text and the unified diff old
/// against new, whole, for the tap.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunPlanObject {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub file_old_sha256: String,
    #[serde(default)]
    pub file_new_sha256: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub diff: String,
}

impl Run {
    /// The command whole. A yard that sent the words alone has them put
    /// together as it does itself.
    pub fn line(&self) -> String {
        if self.command.is_empty() {
            command_line(&self.argv)
        } else {
            self.command.clone()
        }
    }
}

/// What an approved run ask ran: the command, and what it prints at a
/// terminal. A pack apply, and a source add that names commands, are
/// carried out by the yard's server after the approval is recorded: while
/// the answer holds "approved, to be carried out" the ask reads as being
/// carried out, and once the carrying out completes it, as ran or, where
/// `failed` is set, as failed with the failure in words.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Ran {
    #[serde(default, deserialize_with = "or_default")]
    pub argv: Vec<String>,
    #[serde(default)]
    pub result: String,
    /// The carrying out failed: `result` holds the failure in words, and
    /// the card reads as failed, not as ran. Absent on every approval
    /// that ran in its own write.
    #[serde(default)]
    pub failed: bool,
}

impl Ran {
    /// The command that ran, whole.
    pub fn line(&self) -> String {
        command_line(&self.argv)
    }
}

/// A command of the yard's as it is typed in a terminal, from its words.
fn command_line(argv: &[String]) -> String {
    format!("yardr {}", argv.join(" "))
}

/// What `ApproveAsk` answers: the command as it ran, and what it printed.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Approved {
    #[serde(default, deserialize_with = "or_default")]
    pub argv: Vec<String>,
    #[serde(default)]
    pub result: String,
}

/// The setup a setup ask proposes: the pack with the yard's sentences for
/// what approving it does, the account to type beside the preset pick or
/// the candidates to take instead, and the presets the ask's options
/// count. All of it is the yard's, written from its own records when the
/// ask was, and none of it the asker's. Such an ask is approved with
/// `ApproveSetup`, with the preset pick and the account together.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Setup {
    /// The embedded pack to set up, of kind source.
    #[serde(default)]
    pub pack: String,
    /// The build the pack is embedded in, as the card shows it. Absent on
    /// a yard from before the ask said.
    #[serde(default)]
    pub label: String,
    /// The names to type unseen beside the preset pick: the mint's inputs
    /// where the pack's credential is minted, else every credential name.
    #[serde(default, deserialize_with = "or_default")]
    pub account: Vec<String>,
    /// The file a typed value goes to, on the yard's machine.
    #[serde(default)]
    pub account_file: String,
    /// A drift card whose standing file holds every account name: kept
    /// with a bare preset pick, typed another to change it, or a
    /// candidate's file taken. Absent on a first setup, which has no
    /// standing file.
    #[serde(default)]
    pub given: bool,
    /// The account names the standing file does not set where the
    /// update newly needs them: the answer types them, or takes a
    /// candidate. Absent on a first setup.
    #[serde(default, deserialize_with = "or_default")]
    pub missing: Vec<String>,
    /// The yard sources whose account the card offers to reuse; none
    /// where none matches.
    #[serde(default, deserialize_with = "or_default")]
    pub candidates: Vec<SetupCandidate>,
    /// The yard's sentences for what approving changes and what is there
    /// now: the pack, each source with where its credential goes and each
    /// call with its method and path, the mint with its digest, the
    /// account, and the grants with their grantee. Each on its own line,
    /// never a summary.
    #[serde(default, deserialize_with = "or_default")]
    pub does: Vec<String>,
    /// The commands that take the setup back.
    #[serde(default)]
    pub back: String,
    /// Who may run `back`, in words that stand before "can take it back".
    /// Absent where that is anyone.
    #[serde(default)]
    pub back_by: String,
    /// What the pack brings: each source's declaration with its command
    /// files' texts, the yard's own bytes for checking behind the card.
    #[serde(default, deserialize_with = "or_default")]
    pub carried: Vec<RunSource>,
    /// The pack's presets with their grants, in the order the ask's
    /// options count them.
    #[serde(default, deserialize_with = "or_default")]
    pub presets: Vec<SetupPreset>,
    /// Who the grants are for.
    #[serde(default, deserialize_with = "or_default")]
    pub to: Vec<SetupGrantee>,
}

/// One source already in the yard whose account the setup card offers to
/// reuse: the source by name, the file holding the shared account inputs,
/// and the shared input names and values the file holds. Only yard-level
/// source files are ever candidates, never a user's own.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SetupCandidate {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub file: String,
    /// The shared account input names and values the file holds, in the
    /// order of their names.
    #[serde(default, deserialize_with = "or_default")]
    pub values: BTreeMap<String, String>,
}

/// One preset of a setup ask's pack: the pack author's named grant
/// suggestion, with what of each source it grants. The author's about
/// line is no part of it: the card renders each preset's calls from the
/// declarations, in the ask's options.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SetupPreset {
    /// The preset's name, as the option's label shows it.
    #[serde(default)]
    pub name: String,
    /// What of each source the preset grants.
    #[serde(default, deserialize_with = "or_default")]
    pub grants: Vec<SetupGrant>,
}

/// What of one source a preset grants: `read` for every read call, calls
/// by name.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SetupGrant {
    /// The source the pack declares.
    #[serde(default)]
    pub source: String,
    #[serde(default, deserialize_with = "or_default")]
    pub calls: Vec<String>,
}

/// One grantee of a setup ask's grants: the group, and so each session it
/// starts, or the crew member. Exactly one is named.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SetupGrantee {
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub crew: String,
}

/// What `ApproveSetup` answers: the pack with the preset picked, the
/// names given or the candidate taken, and that the server carries the
/// setup out after the call.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SetupApproved {
    #[serde(default)]
    pub bead: String,
    #[serde(default)]
    pub ask: u64,
    #[serde(default)]
    pub asker: String,
    #[serde(default)]
    pub pack: String,
    #[serde(default)]
    pub preset: String,
    /// The account names that were given; none where a candidate was
    /// taken instead.
    #[serde(default, deserialize_with = "or_default")]
    pub given: Vec<String>,
    /// The candidate taken instead of typing; empty where the values
    /// were typed.
    #[serde(default)]
    pub account: String,
    /// The standing account kept with a bare preset pick, on a drift
    /// card whose file holds every name. Exactly one of `given`,
    /// `account` and `keep` is set.
    #[serde(default)]
    pub keep: bool,
    /// `approved, to be carried out`: the server carries the setup out
    /// after the call.
    #[serde(default)]
    pub result: String,
}

/// What a mapping ask maps: the source hearing by identity, the
/// sender's id in it as the delivery carried it, and the channel it
/// arrived in ("" where the envelope named none). Such an ask's picks
/// write a user's identity, so each goes through the weighty gate with
/// `weighed`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Identity {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub channel: String,
}

/// One class of a card's weight, as the yard weighed it when it wrote
/// the card: the class, and the yard's line for it with what this card
/// does. A routine card has none. A client draws a class it does not know
/// as weighty, with its line.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AskWeight {
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub line: String,
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
    /// Whether a secret ask's value is drawn hidden. False where the
    /// source marks the asked names as no secret: the value is asked in
    /// the clear. Absent on a yard from before the ask said, and on any
    /// other ask: concealed.
    #[serde(default = "concealed")]
    pub concealed: bool,
    /// The command a run ask proposes; absent on any other ask.
    #[serde(default)]
    pub run: Option<Run>,
    /// The setup a setup ask proposes; absent on any other ask.
    #[serde(default)]
    pub setup: Option<Setup>,
    /// What a mapping ask maps; absent on any other ask.
    #[serde(default)]
    pub identity: Option<Identity>,
    /// The card's weight, one entry a class; absent on a routine card, and
    /// on a yard from before cards were weighed.
    #[serde(default, deserialize_with = "or_default")]
    pub weight: Vec<AskWeight>,
}

/// One that is open no more: answered, put aside or declined by the user,
/// or taken back by its asker, newest first.
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
    /// `answered`, `put aside`, `declined` or `taken back`.
    #[serde(default)]
    pub state: String,
    /// The picked option's number; absent on a words answer, an aside ask
    /// and a declined one.
    #[serde(default)]
    pub option: u64,
    #[serde(default)]
    pub option_label: String,
    /// What went with the pick, the words answer, why declined, or why
    /// taken back. One another yard took back says where it ended.
    #[serde(default)]
    pub words: String,
    /// Who took it back; absent otherwise, and on one another yard took
    /// back.
    #[serde(default)]
    pub by: String,
    /// When a put-aside ask comes back; absent otherwise.
    #[serde(default)]
    pub return_at: String,
    /// What a secret ask asks for; absent on any other ask.
    #[serde(default)]
    pub secret: Option<Secret>,
    /// Whether a secret ask's value is drawn hidden, as on an open ask.
    #[serde(default = "concealed")]
    pub concealed: bool,
    /// The names a secret ask was given values for.
    #[serde(default, deserialize_with = "or_default")]
    pub given: Vec<String>,
    /// The command a run ask proposes; absent on any other ask.
    #[serde(default)]
    pub run: Option<Run>,
    /// The setup a setup ask proposes; absent on any other ask.
    #[serde(default)]
    pub setup: Option<Setup>,
    /// What a mapping ask maps; absent on any other ask.
    #[serde(default)]
    pub identity: Option<Identity>,
    /// The card's weight, one entry a class; absent on a routine card, and
    /// on a yard from before cards were weighed.
    #[serde(default, deserialize_with = "or_default")]
    pub weight: Vec<AskWeight>,
    /// What the yard ran when a run ask was approved, or what it carries
    /// out for an approved setup ask; none until it is.
    #[serde(default)]
    pub ran: Option<Ran>,
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
    /// `open`, `answered`, `put aside`, `declined` or `taken back`.
    #[serde(default)]
    pub state: String,
    /// The picked option's number; 0 until one is picked.
    #[serde(default)]
    pub option: u64,
    #[serde(default)]
    pub words: String,
    /// Who took it back; absent otherwise, and on one another yard took
    /// back.
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub return_at: String,
    /// What a secret ask asks for; absent on any other ask.
    #[serde(default)]
    pub secret: Option<Secret>,
    /// Whether a secret ask's value is drawn hidden, as on an open ask.
    #[serde(default = "concealed")]
    pub concealed: bool,
    /// The names a secret ask was given values for; none until it is given.
    #[serde(default, deserialize_with = "or_default")]
    pub given: Vec<String>,
    /// The command a run ask proposes; absent on any other ask.
    #[serde(default)]
    pub run: Option<Run>,
    /// The setup a setup ask proposes; absent on any other ask.
    #[serde(default)]
    pub setup: Option<Setup>,
    /// What a mapping ask maps; absent on any other ask.
    #[serde(default)]
    pub identity: Option<Identity>,
    /// The card's weight, one entry a class, as the queue's own: the card
    /// draws the queue's, which is read again with it. Absent on a routine
    /// card, and on a yard from before cards were weighed.
    #[serde(default, deserialize_with = "or_default")]
    pub weight: Vec<AskWeight>,
    /// What the yard ran when a run ask was approved, or what it carries
    /// out for an approved setup ask; none until it is.
    #[serde(default)]
    pub ran: Option<Ran>,
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
    /// The files it carries, in the order they were attached with.
    #[serde(default, deserialize_with = "or_default")]
    pub files: Vec<BlobRef>,
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

    /// The files of a `mail` event, none when it carries none. A `hook`
    /// event has none here: its `files` are the source's own refs, with no
    /// hash to read them by, and the yard's are [`Client::delivery_files`].
    pub fn files(&self) -> Vec<BlobRef> {
        if self.kind == "hook" {
            return Vec::new();
        }
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

    /// Whether the event is one of `me`'s beads added or closed: a
    /// created or closed event about a bead whose originator is `me`.
    /// What lights the timetable's dot, live and in the catch-up after
    /// a restart. A yard from before closed carried its originator
    /// lights nothing for its closes: the field reads empty, as for
    /// another person's bead.
    pub fn timetable_news(&self, me: &str) -> bool {
        matches!(self.kind.as_str(), "created" | "closed")
            && !self.bead.is_empty()
            && !me.is_empty()
            && self.text("originator") == me
    }
}

/// The reply to a call: a result, nothing, or an error.
#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    result: Option<Value>,
    /// Beside a part of a list: there is more of the list.
    #[serde(default)]
    more: bool,
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

/// What an address is read as: a yard's, the web view's, or the run
/// base's. The rules are the same; only the refusals say which.
#[derive(Clone, Copy)]
enum AddressOf {
    Yard,
    View,
    Run,
}

impl AddressOf {
    /// What the refusals call one: `a yard`, `the web view`, `the run
    /// base`.
    fn noun(self) -> &'static str {
        match self {
            AddressOf::Yard => "a yard",
            AddressOf::View => "the web view",
            AddressOf::Run => "the run base",
        }
    }

    /// What stands before it, as the plain-HTTP refusal names it.
    fn front(self) -> &'static str {
        match self {
            AddressOf::Yard => "the yard's listener",
            AddressOf::View => "the view",
            AddressOf::Run => "the sandbox origin",
        }
    }

    /// Why plain HTTP off this machine is refused, past what both share:
    /// a yard's token would cross the network, and the view has none.
    fn secret(self) -> &'static str {
        match self {
            AddressOf::Yard => " The token would cross the network for anyone to read.",
            AddressOf::View | AddressOf::Run => "",
        }
    }
}

impl Address {
    /// Reads an address as `yardr user device pair` prints it. Plain HTTP
    /// is refused off this machine: the yard does no TLS and cannot tell
    /// whether anything in front of it does, so the rule is the client's.
    pub fn parse(text: &str) -> std::result::Result<Address, String> {
        Address::parse_as(text, AddressOf::Yard)
    }

    /// Reads the web view's base address, as `yardr web start` prints it:
    /// by the same rules as a yard's, since a page's bytes are the yard's
    /// to keep. Plain HTTP off this machine is refused with them.
    pub fn parse_view(text: &str) -> std::result::Result<Address, String> {
        Address::parse_as(text, AddressOf::View)
    }

    /// Reads the sandbox origin's base address, set as the run base: by
    /// the same rules as the view's, since a run page is the yard's to
    /// keep. Plain HTTP off this machine is refused with them.
    pub fn parse_run(text: &str) -> std::result::Result<Address, String> {
        Address::parse_as(text, AddressOf::Run)
    }

    fn parse_as(text: &str, of: AddressOf) -> std::result::Result<Address, String> {
        let text = text.trim();
        let noun = of.noun();
        let (tls, rest) = if let Some(rest) = text.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(format!(
                "{text:?} is no address of {noun}: one starts with https://, or with http:// for {noun} on this machine"
            ));
        };
        // The address goes into the head of every request as it is.
        if rest
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '@'))
        {
            return Err(format!(
                "{text:?} is no address of {noun}: it has no space, no query, no # and no name before an @"
            ));
        }
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let no_host = || format!("{text:?} is no address of {noun}: it names no host");
        let (host, port) = match authority.strip_prefix('[') {
            // "[::1]:8790"
            Some(v6) => {
                let (host, suffix) = v6.split_once(']').ok_or_else(no_host)?;
                if host.parse::<std::net::Ipv6Addr>().is_err() {
                    return Err(no_host());
                }
                let port = match suffix {
                    "" => None,
                    suffix if suffix.starts_with(':') => Some(&suffix[1..]),
                    _ => return Err(no_host()),
                };
                (host, port)
            }
            None => match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            },
        };
        if host.is_empty()
            || host.contains(':') && !authority.starts_with('[')
            || !authority.starts_with('[') && (host.contains('[') || host.contains(']'))
        {
            return Err(no_host());
        }
        let port = match port {
            Some(port) => port
                .parse()
                .map_err(|_| format!("{text:?} is no address of {noun}: {port:?} is no port"))?,
            None if tls => 443,
            None => 80,
        };
        let here = host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if !tls && !here {
            return Err(format!(
                "{text} is refused: plain http is for {noun} on this machine alone (127.0.0.1, ::1, localhost).{} Give the https:// address of what stands in front of {}.",
                of.secret(),
                of.front()
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

    /// Whether `other` is the same yard's address: what is connected to
    /// and what is asked for there, however the host's letters are cased
    /// and whether or not the usual port is written out.
    pub fn is(&self, other: &Address) -> bool {
        self.tls == other.tls
            && self.host.eq_ignore_ascii_case(&other.host)
            && self.port == other.port
            && self.path == other.path
    }

    /// Whether the web view answers at this base address: one GET of its
    /// front page. Any HTTP answer says it runs, even a refusal, which is
    /// the browser's to show and not this probe's. The bytes go unread:
    /// the connection closes with them.
    pub fn answers(&self) -> bool {
        let mut conn = match self.connect(Some(CALL_TIMEOUT)) {
            Ok(conn) => conn,
            Err(_) => return false,
        };
        let asked = conn
            .write_all(
                format!(
                    "GET {}/ HTTP/1.1\r\nHost: {}\r\nUser-Agent: {CLIENT}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
                    self.path, self.authority
                )
                .as_bytes(),
            )
            .and(conn.flush());
        if asked.is_err() {
            return false;
        }
        respond(conn).is_ok()
    }

    /// The browser's URL for the page `hash` at this base address: the
    /// `/blobs/` route under it, on the view for Open and on the run base
    /// for run. The hash is what a note said, and goes into the URL's
    /// path: one that is no hex names no page, and opens nothing.
    pub fn page_url(&self, hash: &str) -> Option<String> {
        if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(format!("{}{}/blobs/{hash}", self.origin(), self.path))
    }

    /// The browser's URL for the bead `id` at this base address: the
    /// `/beads/` route under it, for Open. The id is the yard's word, so
    /// every byte outside the unreserved set goes percent-encoded, and it
    /// stays one segment whatever it holds. An empty id opens nothing.
    pub fn bead_url(&self, id: &str) -> Option<String> {
        match id.is_empty() {
            true => None,
            false => Some(format!(
                "{}{}/beads/{}",
                self.origin(),
                self.path,
                encode_name(id)
            )),
        }
    }

    /// The browser's URL for the file `hash` under `name` at this base
    /// address: the `/blobs/<hash>/<name>` route on the view, for Open,
    /// which the browser saves under the name. The name is the sender's
    /// word, so every byte outside the unreserved set goes percent-encoded,
    /// and it stays one segment whatever it holds. An empty name opens the
    /// bare route, as a page does. A hash that is no hex names no file,
    /// and opens nothing.
    pub fn file_url(&self, hash: &str, name: &str) -> Option<String> {
        if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        match name.is_empty() {
            true => Some(format!("{}{}/blobs/{hash}", self.origin(), self.path)),
            false => Some(format!(
                "{}{}/blobs/{hash}/{}",
                self.origin(),
                self.path,
                encode_name(name)
            )),
        }
    }
}

/// A file's name for the `/blobs/<hash>/<name>` route: one segment,
/// whatever the sender gave. The unreserved bytes go as they are, every
/// other byte percent-encoded, so no `/`, `?` or `%` of the name reaches
/// the URL as anything but a byte of the name.
fn encode_name(name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 15) as usize] as char);
            }
        }
    }
    out
}

impl Address {
    /// The sandbox origin's base address for the view at this base
    /// address: the view's host on the sandbox port, with no path. A
    /// proxied second name is set as the run base instead.
    pub fn run_base(&self) -> Address {
        let authority = if self.host.contains(':') {
            format!("[{}]:{RUN_PORT}", self.host)
        } else {
            format!("{}:{RUN_PORT}", self.host)
        };
        Address {
            tls: self.tls,
            host: self.host.clone(),
            authority,
            port: RUN_PORT,
            path: String::new(),
        }
    }

    fn connect(&self, timeout: Option<Duration>) -> io::Result<Conn> {
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} names no address", self.host),
        );
        let mut tcp = None;
        let addrs = (self.host.as_str(), self.port).to_socket_addrs()?;
        let addrs = in_order(&self.host, self.tls, addrs);
        if addrs.is_empty() && !self.tls {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "plain http did not resolve to a loopback address",
            ));
        }
        for addr in addrs {
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
        socket2::SockRef::from(&tcp).set_tcp_keepalive(
            &socket2::TcpKeepalive::new()
                .with_time(KEEPALIVE)
                .with_interval(KEEPALIVE_INTERVAL)
                .with_retries(KEEPALIVE_RETRIES),
        )?;
        if !self.tls {
            return Ok(Conn::Tcp(tcp));
        }
        let name = rustls::pki_types::ServerName::try_from(self.host.clone())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
        let session = rustls::ClientConnection::new(tls()?, name).map_err(io::Error::other)?;
        Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(session, tcp))))
    }
}

impl Address {
    /// The scheme and the host, with nothing after them.
    fn origin(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://{}", self.authority)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.tls { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority, self.path)
    }
}

/// The addresses a host came to, in the order they are tried. Plain HTTP
/// may resolve only to loopback, so a changed hosts file cannot send a local
/// address and its token to another machine. `localhost` is `::1` and
/// `127.0.0.1`, and Windows lists `::1` first: a yard that listens on the
/// IPv4 loopback alone (yardr's listener does, and WSL2 passes its ports to
/// Windows' `localhost`) is refused at `::1` only after the system has sent
/// the SYN again, a second or two for each of the dozen calls a window opens
/// with. The IPv4 ones go first for that name; any other host keeps the
/// order it was given.
fn in_order(host: &str, tls: bool, addrs: impl Iterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let mut addrs: Vec<SocketAddr> = addrs
        .filter(|addr| tls || addr.ip().is_loopback())
        .collect();
    if host.eq_ignore_ascii_case("localhost") {
        addrs.sort_by_key(SocketAddr::is_ipv6);
    }
    addrs
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
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

enum EventSocket {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

/// A handle that shuts down an event stream while its reader is blocked.
/// Calls use their own connections and are not affected. It holds the
/// stream's own socket, so what is set on it is set on the stream.
///
/// The reader is woken whatever the yard does about it: a socket shut down
/// from another thread ends the read that waits on it on macOS and Linux,
/// and on Windows the read is cancelled besides, since the shutdown only
/// sends the FIN there and the read waits for the yard to answer it.
#[derive(Clone)]
pub struct EventStop(Arc<EventSocket>);

impl EventStop {
    pub fn stop(&self) {
        use std::net::Shutdown;

        match self.0.as_ref() {
            #[cfg(unix)]
            EventSocket::Unix(socket) => {
                let _ = socket.shutdown(Shutdown::Both);
            }
            EventSocket::Tcp(socket) => {
                let _ = socket.shutdown(Shutdown::Both);
                #[cfg(windows)]
                cancel_reads(socket);
            }
        }
    }
}

/// Cancels the read that waits on `socket`, whichever thread began it. The
/// stream's reader holds another handle of the socket than this one, since a
/// stop handle is a copy ([`Conn::event_stop`]); the cancel names the socket
/// and not the handle it was begun on. It runs after the shutdown, so that a
/// read begun after the cancel meets a socket that is shut down. A socket
/// with no read waiting is left as it was.
#[cfg(windows)]
fn cancel_reads(socket: &TcpStream) {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::IO::CancelIoEx;

    // SAFETY: the socket is open for as long as the borrow lasts. With no
    // OVERLAPPED named, CancelIoEx marks every I/O still pending on the
    // socket in this process, whichever thread began it, and does nothing
    // when none is.
    unsafe {
        CancelIoEx(socket.as_raw_socket() as HANDLE, std::ptr::null());
    }
}

impl Conn {
    fn event_stop(&self) -> io::Result<EventStop> {
        Ok(EventStop(Arc::new(match self {
            #[cfg(unix)]
            Conn::Unix(socket) => EventSocket::Unix(socket.try_clone()?),
            Conn::Tcp(socket) => EventSocket::Tcp(socket.try_clone()?),
            Conn::Tls(stream) => EventSocket::Tcp(stream.sock.try_clone()?),
        })))
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Conn::Unix(conn) => conn.read(buf),
            Conn::Tcp(conn) => conn.read(buf),
            Conn::Tls(conn) => conn.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Conn::Unix(conn) => conn.write(buf),
            Conn::Tcp(conn) => conn.write(buf),
            Conn::Tls(conn) => conn.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
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

/// The call reached the server, and the server said no: `message` is a
/// sentence for a person, shown, never read.
fn refused(code: &str, message: &str) -> Error {
    Error::Refused {
        code: code.to_string(),
        message: message.to_string(),
    }
}

/// The body's shape where the edge answers for no ready yard: the
/// router's noYard, a 403 of `{"error": "edge: <why>"}`. The yard's own
/// refusals carry `refused`, never this key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeError {
    error: String,
}

/// Where no yard is ready behind the edge, the edge's own 403: `status`
/// with a body of `{"error": "<sentence>"}` is the router's noYard for a
/// yard still building, stopped or failed, or a name of no yard of the
/// account. Anything else is none, and the caller's own arms run. Told by
/// which key the body carries, never by its prefix; one leading "edge: "
/// is the router's tag, not the person's words, and is stripped.
fn no_yard(status: u16, bytes: &[u8]) -> Option<Error> {
    if status != 403 {
        return None;
    }
    let edge: EdgeError = serde_json::from_slice(bytes.trim_ascii()).ok()?;
    let sentence = edge.error.strip_prefix("edge: ").unwrap_or(&edge.error);
    Some(refused("no_yard", sentence))
}

/// Where the edge answers a call for its link, not its yard: the router's
/// refuseCall. A 503 with `X-Edge-Refusal: away` where no link is up, a 504
/// where the yard gave no answer in time (with no refusal header), and a 429
/// with `X-Edge-Refusal: busy` past what the link holds, each with a body of
/// `{"error": "<sentence>"}`. Anything else is none, and the caller's own
/// arms run: a 503 of any other shape is the yard's own, while replacing.
/// Told by the status with its header and by which key the body carries,
/// never by the sentence's prefix; one leading "edge: " is the router's tag,
/// not the person's words, and is stripped.
fn edge_call_refusal(status: u16, refusal: Option<&str>, bytes: &[u8]) -> Option<Error> {
    let code = match status {
        503 if refusal.is_some_and(|r| r.eq_ignore_ascii_case("away")) => "away",
        504 => "no_answer",
        429 if refusal.is_some_and(|r| r.eq_ignore_ascii_case("busy")) => "busy",
        _ => return None,
    };
    let edge: EdgeError = serde_json::from_slice(bytes.trim_ascii()).ok()?;
    let sentence = edge.error.strip_prefix("edge: ").unwrap_or(&edge.error);
    Some(refused(code, sentence))
}

/// Redeems a pairing code at a yard's address: one request with no token,
/// answered with the device's. The code is sent as it was typed; the yard
/// reads it without its dash and in either case.
pub fn pair(address: &Address, code: &str) -> Result<Paired> {
    pair_with(address, code, None, "", CALL_TIMEOUT)
}

/// As [`pair`], at a hosted yard: the session token rides beside the code,
/// which the edge checks before the code reaches the yard, naming which
/// of the account's yards where `edge_yard` names one; empty is the
/// account's own yard, and no header rides then.
pub fn pair_hosted(
    address: &Address,
    code: &str,
    session: &str,
    edge_yard: &str,
) -> Result<Paired> {
    pair_with(address, code, Some(session), edge_yard, CALL_TIMEOUT)
}

fn pair_with(
    address: &Address,
    code: &str,
    session: Option<&str>,
    edge_yard: &str,
    timeout: Duration,
) -> Result<Paired> {
    let body = json!({ "code": code }).to_string();
    // A pairing is no read: a timed out one may already have made the
    // device, unless the connect gave up, which sent nothing.
    let mut conn = address
        .connect(Some(timeout))
        .map_err(|err| connect_error(err.into(), false))?;
    let edge = session
        .filter(|session| is_token(session))
        .map(|session| format!("X-Edge-Token: {session}\r\n"))
        .unwrap_or_default();
    let named = if is_token(edge_yard) {
        format!("X-Edge-Yard: {edge_yard}\r\n")
    } else {
        String::new()
    };
    conn.write_all(
        format!(
            "POST {}/user/pair HTTP/1.1\r\nHost: {}\r\nX-Yardr-Client: {CLIENT}\r\n{edge}{named}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            address.path,
            address.authority,
            body.len()
        )
        .as_bytes(),
    )
    .map_err(|err| slow_or_down(err, false))?;
    conn.flush().map_err(|err| slow_or_down(err, false))?;
    let (status, refuser, mut reply) = respond_call(conn).map_err(|err| slow_error(err, false))?;
    if status == 401 && refuser == Refuser::Edge {
        return Err(refused(
            "no_session",
            "Sign in first: the edge does not know this session, and the code never reached the yard. Then pair this device with the code.",
        ));
    }
    match status {
        200 => {
            let mut bytes = Vec::new();
            reply
                .read_to_end(&mut bytes)
                .map_err(|err| slow_or_down(err, false))?;
            let paired: Paired = serde_json::from_slice(&bytes)
                .map_err(|err| Error::Protocol(format!("the pairing: {err}")))?;
            if !is_token(&paired.token) {
                return Err(Error::Protocol(
                    "the pairing: no token in the answer".into(),
                ));
            }
            Ok(paired)
        }
        403 => {
            // Through the edge, a 403 is the router's noYard where no
            // yard is ready, told from the yard's wrong code by the
            // body's shape; of any other shape today's arm runs.
            if session.is_some() {
                let mut bytes = Vec::new();
                reply
                    .read_to_end(&mut bytes)
                    .map_err(|err| slow_or_down(err, false))?;
                if let Some(edge) = no_yard(status, &bytes) {
                    return Err(edge);
                }
            }
            Err(refused(
                "wrong_code",
                "That is not the code. The yard counts a wrong one, and the fifth ends the pairing.",
            ))
        }
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

/// What signing in at the edge answers: the session token, kept as the
/// device's is, and the account it signed in as.
#[derive(Clone, Debug, PartialEq)]
pub struct SignedIn {
    pub session: String,
    pub account: String,
}

/// Signs the Kratos account in at the edge's `/kratos` path, with the API
/// flow a native client uses: one flow is opened, and the password method
/// answers it. The password goes to the typed address alone: the flow's
/// own action is not followed anywhere else, and no redirect is. A
/// refused sign-in names signing in, never pairing: the code is another
/// road. The password is in the request's body and in nothing else: no
/// error and no log.
pub fn sign_in(address: &Address, email: &str, password: &str) -> Result<SignedIn> {
    let flow = login_flow(address)?;
    match submit_login(address, &flow, email, password) {
        // A flow the edge forgot meanwhile is opened once more; one that
        // is forgotten twice is said, not tried a third time.
        Err(Error::Refused { code, .. }) if code == EXPIRED_FLOW => {
            let flow = login_flow(address)?;
            match submit_login(address, &flow, email, password) {
                Err(Error::Refused { code, .. }) if code == EXPIRED_FLOW => Err(refused(
                    "bad_sign_in",
                    "The sign-in expired before it was sent. Sign in again.",
                )),
                answered => answered,
            }
        }
        answered => answered,
    }
}

/// A submitted flow the edge forgot: answered once more with a new flow,
/// and never shown.
const EXPIRED_FLOW: &str = "expired_flow";

/// Opens a login flow at the edge's `/kratos` path: its id, which the
/// password answers.
fn login_flow(address: &Address) -> Result<String> {
    let mut conn = address
        .connect(Some(CALL_TIMEOUT))
        .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    conn.write_all(
        format!(
            "GET {}/kratos/self-service/login/api HTTP/1.1\r\nHost: {}\r\nX-Yardr-Client: {CLIENT}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            address.path, address.authority
        )
        .as_bytes(),
    )
    .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    conn.flush()
        .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    let (status, mut reply) = respond(conn).map_err(|err| no_sign_in(address, &err.to_string()))?;
    if status != 200 {
        return Err(no_sign_in(address, &format!("status {status}")));
    }
    let mut bytes = Vec::new();
    reply
        .read_to_end(&mut bytes)
        .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    let flow: Value = serde_json::from_slice(&bytes)
        .map_err(|_| no_sign_in(address, "an answer that cannot be read"))?;
    flow["id"]
        .as_str()
        .filter(|id| is_token(id))
        .map(str::to_string)
        .ok_or_else(|| no_sign_in(address, "an answer with no flow"))
}

/// Answers the flow with the password method: the session token and the
/// account, the flow expired meanwhile, or the refusal in Kratos's words.
/// The submit goes to the typed address with the flow's id: the flow's own
/// action names the same URL by the edge's shape, and is not read for
/// where to send the password.
fn submit_login(address: &Address, flow: &str, email: &str, password: &str) -> Result<SignedIn> {
    let body = json!({
        "method": "password",
        "identifier": email,
        "password": password,
    })
    .to_string();
    let mut conn = address
        .connect(Some(CALL_TIMEOUT))
        .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    conn.write_all(
        format!(
            "POST {}/kratos/self-service/login?flow={flow} HTTP/1.1\r\nHost: {}\r\nX-Yardr-Client: {CLIENT}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            address.path,
            address.authority,
            body.len()
        )
        .as_bytes(),
    )
    .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    conn.flush()
        .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    let (status, mut reply) = respond(conn).map_err(|err| no_sign_in(address, &err.to_string()))?;
    let mut bytes = Vec::new();
    reply
        .read_to_end(&mut bytes)
        .map_err(|err| no_sign_in(address, &format!("nothing answers ({err})")))?;
    match status {
        200 => {
            let answered: Value = serde_json::from_slice(&bytes)
                .map_err(|_| no_sign_in(address, "an answer that cannot be read"))?;
            let session = answered["session_token"]
                .as_str()
                .filter(|session| is_token(session))
                .ok_or_else(|| no_sign_in(address, "an answer with no session token"))?;
            let traits = &answered["session"]["identity"]["traits"];
            let account = [traits["email"].as_str(), traits["handle"].as_str()]
                .into_iter()
                .flatten()
                .map(str::trim)
                .find(|held| !held.is_empty())
                .map(|held| held.lines().next().unwrap_or_default().trim().to_string())
                .filter(|held| !held.is_empty())
                .unwrap_or_else(|| email.to_string());
            Ok(SignedIn {
                session: session.to_string(),
                account,
            })
        }
        400 => {
            let why = kratos_errors(&bytes)
                .first()
                .cloned()
                .unwrap_or_else(|| "the edge refused it with no reason".to_string());
            Err(refused(
                "bad_sign_in",
                &format!(
                    "The sign-in failed: {why}. Check the email and password, and sign in again."
                ),
            ))
        }
        410 => Err(refused(EXPIRED_FLOW, "The login flow expired.")),
        _ => Err(no_sign_in(address, &format!("status {status}"))),
    }
}

/// No sign-in answers there at all: nothing listens, it is no edge, or its
/// answer cannot be read. `why` is what was found, in a few words.
fn no_sign_in(address: &Address, why: &str) -> Error {
    refused(
        "no_sign_in",
        &format!("No sign-in answers at {address}: {why}. Check the origin, and sign in again."),
    )
}

/// What a refused login flow says: its messages, then each field's, in
/// Kratos's words.
fn kratos_errors(bytes: &[u8]) -> Vec<String> {
    let Ok(flow) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let mut errors: Vec<String> = flow["ui"]["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message["text"].as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect();
    errors.extend(
        flow["ui"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|node| {
                node["messages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .filter_map(|message| message["text"].as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
    );
    errors
}

/// Reads the file a site has at `path`, which is no yard: one `GET` with
/// no token, as yardr's release is fetched (fetch.rs). A status that is not
/// 200 is given with no bytes, and the site is not followed anywhere else:
/// a redirect is such a status. A file longer than `most` bytes is not
/// read to its end.
pub fn read(address: &Address, path: &str, most: u64) -> Result<(u16, Vec<u8>)> {
    let mut conn = address.connect(Some(CALL_TIMEOUT))?;
    conn.write_all(
        format!(
            "GET {}{path} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {CLIENT}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
            address.path, address.authority
        )
        .as_bytes(),
    )?;
    conn.flush()?;
    let (status, reply) = respond(conn)?;
    if status != 200 {
        return Ok((status, Vec::new()));
    }
    let mut bytes = Vec::new();
    reply.take(most.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > most {
        return Err(Error::Protocol(format!(
            "{path} is longer than {most} bytes"
        )));
    }
    Ok((status, bytes))
}

/// How often a download is sent on to another address before it is given
/// up: a maker's release address answers with one or two.
const HOPS: usize = 5;

/// How long a download may take in all. The longest is a harness's
/// installer, of some hundred megabytes.
const DOWNLOAD_LONGEST: Duration = Duration::from_secs(30 * 60);

/// The most that is read of a response's status and headers. A server that
/// sends more is not one of those spoken with here.
const HEAD_MOST: u64 = 64 * 1024;

/// Reads the file at `url`, which is no yard and not yardr's own site: a
/// program `yardr doctor` names for a Mac, at its maker's address (fetch.rs).
/// Such an address may send on to the maker's storage, so a redirect is
/// followed here, [`HOPS`] times at the most and only to another `https://`
/// address (or one of this machine, as [`Address::parse`] has it). What is
/// read is taken for nothing by where it came from: the caller holds it to
/// the size and the checksum yardr wrote down. A file longer than `most`
/// bytes is not read to its end.
pub fn download(url: &str, most: u64) -> Result<Vec<u8>> {
    let started = Instant::now();
    let mut url = url.to_string();
    let mut secure = false;
    for _ in 0..=HOPS {
        let (address, target) = split_url(&url)?;
        // What began over TLS stays there: no address sends it on to a
        // port of this machine in the clear.
        if secure && !address.tls {
            return Err(Error::Protocol(format!(
                "the file is sent on to {address}, which is not https"
            )));
        }
        secure = address.tls;
        let mut conn = address.connect(Some(CALL_TIMEOUT))?;
        conn.write_all(
            format!(
                "GET {target} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {CLIENT}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
                address.authority
            )
            .as_bytes(),
        )?;
        conn.flush()?;
        let (status, location, _, reply) = respond_with_location(conn)?;
        match status {
            200 => {
                let mut bytes = Vec::new();
                let mut reply = reply.take(most.saturating_add(1));
                let mut some = vec![0u8; 64 * 1024];
                // Each read has its own time to answer in; the whole has
                // one too, so that a host cannot go on forever a byte at
                // a time.
                loop {
                    let n = reply.read(&mut some)?;
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&some[..n]);
                    if started.elapsed() > DOWNLOAD_LONGEST {
                        return Err(Error::Protocol(format!(
                            "the file at {address} took longer than {} minutes",
                            DOWNLOAD_LONGEST.as_secs() / 60
                        )));
                    }
                }
                if bytes.len() as u64 > most {
                    return Err(Error::Protocol(format!(
                        "the file at {address} is longer than {most} bytes"
                    )));
                }
                return Ok(bytes);
            }
            301 | 302 | 303 | 307 | 308 => {
                let to = location.ok_or_else(|| {
                    Error::Protocol(format!("{address} sends on to nowhere (status {status})"))
                })?;
                // "/elsewhere" is of the same host.
                url = match to.starts_with('/') {
                    true => format!("{}{to}", address.origin()),
                    false => to,
                };
            }
            status => {
                return Err(Error::Protocol(format!(
                    "{address} has no such file (status {status})"
                )));
            }
        }
    }
    Err(Error::Protocol(format!(
        "the file is sent on more than {HOPS} times"
    )))
}

/// An address of a file, as its host and what is asked of it there. The
/// second goes into the first line of a request as it is, so it has no
/// space and no control character; it may have a query, which a maker's
/// storage signs its addresses with.
fn split_url(url: &str) -> Result<(Address, String)> {
    let no = || Error::Protocol(format!("{url:?} is no address of a file"));
    let at = url.find("://").ok_or_else(no)? + 3;
    let end = url[at..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |slash| at + slash);
    let address = Address::parse(&url[..end]).map_err(Error::Protocol)?;
    let target = url[end..].split('#').next().unwrap_or("");
    if target.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(no());
    }
    let target = match target {
        "" => "/".to_string(),
        query if query.starts_with('?') => format!("/{query}"),
        path => path.to_string(),
    };
    Ok((address, target))
}

/// How a yard is reached: by the socket of this machine's, or by the
/// address of another's listener with a device's token, and at a hosted
/// yard the session token beside it, naming which of the account's yards
/// where one is set. Windows has the second alone.
#[derive(Clone)]
enum Way {
    #[cfg(unix)]
    Socket(PathBuf),
    Listener {
        address: Address,
        token: String,
        session: Option<String>,
        edge_yard: String,
    },
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
    #[cfg(unix)]
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
                session: None,
                edge_yard: String::new(),
            },
        }
    }

    /// A hosted yard at its edge's address, as the device `token` is of
    /// with the Kratos `session` beside it. Every call carries both; one
    /// that is no token rides on no request, and the edge refuses the
    /// session for it. `edge_yard` names which of the account's yards it
    /// means, and every call names it where one is set; empty is the
    /// account's own yard, and no header rides then.
    pub fn remote_hosted(
        address: Address,
        token: impl Into<String>,
        session: impl Into<String>,
        edge_yard: impl Into<String>,
    ) -> Self {
        Client {
            way: Way::Listener {
                address,
                token: token.into(),
                session: Some(session.into()),
                edge_yard: edge_yard.into(),
            },
        }
    }

    /// Whether the yard is reached at an address, and not by its socket.
    pub fn is_remote(&self) -> bool {
        matches!(self.way, Way::Listener { .. })
    }

    /// Whether calls ride to a hosted yard: at an address with the
    /// session beside the token, where the edge may answer for no yard
    /// of its own. Every call comes to the yard down the link from the
    /// edge, which is why the yard serves some calls to no such client.
    pub fn hosted(&self) -> bool {
        matches!(
            self.way,
            Way::Listener {
                session: Some(_),
                ..
            }
        )
    }

    /// Where the yard is looked for, in words: the socket's path or the
    /// address.
    pub fn place(&self) -> String {
        match &self.way {
            #[cfg(unix)]
            Way::Socket(socket) => format!("The socket: {}", socket.display()),
            Way::Listener { address, .. } => format!("The yard: {address}"),
        }
    }

    /// Who the token is: the user every call is made as, the device and
    /// the yard. Of a yard at an address only; the socket has no such path.
    pub fn me(&self) -> Result<Me> {
        let (status, bytes, request) = self.get_body("/me", "", Some(CALL_TIMEOUT), |status| {
            status == 200 || self.hosted() && status == 403
        })?;
        let sent = Sent {
            verb: "GET",
            path: "/me",
            limit: Some(CALL_TIMEOUT),
            request: request.as_deref(),
            status: Some(status),
        };
        self.me_of(status, &bytes).map_err(|err| sent.failed(err))
    }

    /// Who the token is, from the answer to `/me`: the user, or why it is
    /// none.
    fn me_of(&self, status: u16, bytes: &[u8]) -> Result<Me> {
        // Through the edge, a 403 is the router's noYard where no yard
        // is ready, not this call's own refusal; of any other shape
        // today's arms run, on the status alone.
        if self.hosted()
            && status == 403
            && let Some(edge) = no_yard(status, bytes)
        {
            return Err(edge);
        }
        match status {
            200 => {}
            404 => return Err(Error::NoDevice),
            _ => {
                return Err(Error::Protocol(format!(
                    "who this device is: status {status}"
                )));
            }
        }
        serde_json::from_slice(bytes)
            .map_err(|err| Error::Protocol(format!("who this device is: {err}")))
    }

    /// Sends one request and reads the answer's head. A GET the yard is
    /// slow to answer is asked again, after a short pause, before its
    /// silence reads as an error; anything else is asked once, since it
    /// may have run. `idempotent` is whether the request changes nothing,
    /// which is what its silence is then said to be ([`Error::Slow`]): a
    /// read called by POST is said so, and is asked once all the same.
    fn send(
        &self,
        verb: &str,
        path: &str,
        headers: &str,
        body: &str,
        timeout: Option<Duration>,
        idempotent: bool,
    ) -> Result<(u16, Body)> {
        let mut tries = if verb == "GET" { SLOW_RETRIES + 1 } else { 1 };
        loop {
            match self.send_once(verb, path, headers, body, timeout, idempotent) {
                // A GET whose connect gave up is asked again too: it sent
                // nothing, and only a GET has another try.
                Err(Error::Slow { sent: false, .. }) if tries > 1 => {
                    tries -= 1;
                    thread::sleep(SLOW_PAUSE);
                }
                Err(Error::Down(err)) if timed_out(&err) && tries > 1 => {
                    tries -= 1;
                    thread::sleep(SLOW_PAUSE);
                }
                Err(Error::Down(err)) if timed_out(&err) => {
                    return Err(slow_or_down(err, idempotent));
                }
                reply => return reply,
            }
        }
    }

    /// Sends a finite GET and reads the body where its status needs one,
    /// and gives the status, the body and the edge's id for the request. A
    /// timeout anywhere in the request or body gets the same one retry: the
    /// yard's `/me` and blob answers are safe to ask for again. A body that
    /// cannot be read leaves its line ([`Sent`]), as a try that fails
    /// before its answer does.
    fn get_body(
        &self,
        path: &str,
        headers: &str,
        timeout: Option<Duration>,
        read_body: impl Fn(u16) -> bool,
    ) -> Result<(u16, Vec<u8>, Option<String>)> {
        let mut tries = SLOW_RETRIES + 1;
        loop {
            let result = (|| {
                let (status, mut body) = self.send_once("GET", path, headers, "", timeout, true)?;
                let mut bytes = Vec::new();
                if read_body(status) {
                    let request = body.request.clone();
                    let sent = Sent {
                        verb: "GET",
                        path,
                        limit: timeout,
                        request: request.as_deref(),
                        status: Some(status),
                    };
                    body.read_to_end(&mut bytes)
                        .map_err(|err| sent.failed(err.into()))?;
                }
                Ok((status, bytes, body.request))
            })();
            match result {
                // A GET whose connect gave up is asked again too: it sent
                // nothing.
                Err(Error::Slow { sent: false, .. }) if tries > 1 => {
                    tries -= 1;
                    thread::sleep(SLOW_PAUSE);
                }
                Err(Error::Down(err)) if timed_out(&err) && tries > 1 => {
                    tries -= 1;
                    thread::sleep(SLOW_PAUSE);
                }
                Err(Error::Down(err)) if timed_out(&err) => {
                    return Err(slow_or_down(err, true));
                }
                reply => return reply,
            }
        }
    }

    /// Sends one request on a connection of its own and reads the answer's
    /// head. `path` is as docs/api.md has it on the socket; at an address
    /// it stands under `/user/`, with the token beside it, and at a hosted
    /// yard the session token in `X-Edge-Token` beside that, naming which
    /// of the account's yards in `X-Edge-Yard` where one is set.
    /// `headers` are whole lines. A call's answer is waited for `timeout`
    /// long. `idempotent` is whether the request changes nothing: a
    /// connect that gave up sent nothing, and is said so
    /// ([`connect_error`]). A request that fails before its answer is in
    /// hand, and a 401, leave their line ([`Sent`]) here; what a caller
    /// refuses of an answer it has, it leaves.
    fn send_once(
        &self,
        verb: &str,
        path: &str,
        headers: &str,
        body: &str,
        timeout: Option<Duration>,
        idempotent: bool,
    ) -> Result<(u16, Body)> {
        let sent = Sent {
            verb,
            path,
            limit: timeout,
            request: None,
            status: None,
        };
        // At an address the connect has a wait of its own.
        let connecting = Sent {
            limit: if self.is_remote() {
                Some(CONNECT_TIMEOUT)
            } else {
                timeout
            },
            ..sent
        };
        let (mut conn, head) = self
            .open(verb, path, timeout)
            .map_err(|err| connecting.failed(connect_error(err, idempotent)))?;
        // In one write: over TLS each write is a record of its own.
        conn.write_all(format!("{head}X-Yardr-Client: {CLIENT}\r\n{headers}\r\n{body}").as_bytes())
            .and_then(|()| conn.flush())
            .map_err(|err| sent.failed(err.into()))?;
        let (status, refuser, body) = respond_call(conn).map_err(|err| sent.failed(err))?;
        let sent = Sent {
            request: body.request.as_deref(),
            status: Some(status),
            ..sent
        };
        // A 401 is a state of its own, and of three: the edge refused the
        // session, the yard the device, or neither did, told apart by the
        // response's shape ([`Refuser`]). Only the yard's says the device
        // is out: a 401 of any other shape is not its check.
        if status == 401 {
            return Err(sent.failed(refuser.error()));
        }
        Ok((status, body))
    }

    /// Connects to the yard, and gives the connection with the lines of the
    /// request's head that come first: its request line and, at an address,
    /// whose it is.
    fn open(&self, verb: &str, path: &str, timeout: Option<Duration>) -> Result<(Conn, String)> {
        Ok(match &self.way {
            #[cfg(unix)]
            Way::Socket(socket) => {
                let conn = UnixStream::connect(socket)?;
                conn.set_read_timeout(timeout)?;
                conn.set_write_timeout(Some(CALL_TIMEOUT))?;
                (
                    Conn::Unix(conn),
                    format!("{verb} {path} HTTP/1.1\r\nHost: yard\r\n"),
                )
            }
            Way::Listener {
                address,
                token,
                session,
                edge_yard,
            } => {
                let edge = session
                    .as_deref()
                    .filter(|session| is_token(session))
                    .map(|session| format!("X-Edge-Token: {session}\r\n"))
                    .unwrap_or_default();
                let named = if is_token(edge_yard) {
                    format!("X-Edge-Yard: {edge_yard}\r\n")
                } else {
                    String::new()
                };
                (
                    address.connect(timeout)?,
                    format!(
                        "{verb} {}/user{path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n{edge}{named}",
                        address.path, address.authority
                    ),
                )
            }
        })
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

    /// The files the yard fetched of a hook delivery, by the delivery's seq
    /// (the `seq` of its notification): each as the yard holds it, to be
    /// read with [`Client::blob`], or with the reason it was not taken.
    /// None while the yard has fetched none, which it does after it has
    /// told of the delivery: its `hook_files` event says when to ask again.
    pub fn delivery_files(&self, seq: u64) -> Result<Vec<BlobRef>> {
        self.list("DeliveryFiles", json!({ "seq": seq }))
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

    /// The arrivals board: the open beads still travelling, up to 15
    /// rows in arrival order, each with its expected arrival and remark.
    /// A yard not yet upgraded serves the rows as a bare list, which
    /// reads the same; no answer at all reads as an empty board.
    pub fn arrivals(&self) -> Result<Arrivals> {
        let result = self.call("Arrivals", json!({}))?.unwrap_or(Value::Null);
        match result {
            Value::Null => Ok(Arrivals::default()),
            Value::Array(rows) => {
                let rows: Vec<Arrival> = serde_json::from_value(Value::Array(rows))
                    .map_err(|err| Error::Protocol(format!("Arrivals: {err}")))?;
                Ok(Arrivals { rows })
            }
            board => serde_json::from_value(board)
                .map_err(|err| Error::Protocol(format!("Arrivals: {err}"))),
        }
    }

    /// The flows view: the depots as lines, each with its travelling
    /// total, its flows, each flow its stages in travel order with the
    /// open beads on each, its lamp, its terminal flag and its group's
    /// seats, each bead with who works it and when it entered its stage,
    /// and its Elsewhere rows; the beads travelling with no line; and the
    /// yard-wide travelling total. No answer at all reads as an empty
    /// view.
    pub fn flows(&self) -> Result<Flows> {
        let result = self.call("Flows", json!({}))?.unwrap_or(Value::Null);
        match result {
            Value::Null => Ok(Flows::default()),
            board => serde_json::from_value(board)
                .map_err(|err| Error::Protocol(format!("Flows: {err}"))),
        }
    }

    /// Every grant of the yard's sources: who may make which calls of
    /// each, and as whom, with when, by whom and on which card each call
    /// was given. What `yardr source grants` prints. The operator's to
    /// read: on the listener to a device of theirs alone, on the socket
    /// to a caller on the yard's machine. No answer at all reads as none.
    pub fn source_grants(&self) -> Result<Vec<SourceGrant>> {
        self.list("SourceGrants", json!({}))
    }

    /// What a running session's pane shows: its visible screen with the
    /// terminal's ANSI styling. `session` is a session id or a crew
    /// member's name, the id first; a crew member's is its own session's
    /// screen. The read is passive: it moves nothing in the pane. The
    /// operator's to read: on the socket the operator's alone, and over
    /// the listener a device of the operator's. A screen can show what an
    /// agent printed, a secret among it: it is drawn, and never written
    /// anywhere.
    pub fn session_screen(&self, session: &str) -> Result<SessionScreen> {
        self.answer("SessionScreen", json!({ "session": session }))
    }

    /// One bead whole: body, stage, labels, notes, its asks with their
    /// options, recommendation and state, and the about-beads.
    pub fn bead(&self, bead: &str) -> Result<Bead> {
        self.answer("Bead", json!({ "bead": bead }))
    }

    /// Picks an option with words alongside, or answers an ask with no
    /// options with words (`option` 0). Words alone on an ask with options
    /// are refused, and go as a plain note instead. `weighed` is the
    /// client's word that its person saw the card's weight and passed its
    /// gate: a mapping pick writes a user's identity and is refused
    /// without it. It goes only where it is said: a yard from before the
    /// argument takes the call as it always did.
    pub fn answer_ask(
        &self,
        bead: &str,
        ask: u64,
        option: u64,
        words: &str,
        actor: &str,
        weighed: bool,
    ) -> Result<()> {
        let mut args = json!({ "bead": bead, "ask": ask, "option": option, "actor": actor });
        if !words.is_empty() {
            args["words"] = json!(words);
        }
        if weighed {
            args["weighed"] = json!(true);
        }
        self.call("AnswerAsk", args).map(drop)
    }

    /// Answers a secret ask: each name of the ask with the value the person
    /// typed. The yard writes the values to the file the source's
    /// credential is read from and answers the names and the file. A value
    /// is the person's secret: it is in this call's body and in nothing
    /// else of the client, no error and no log. `weighed` is the client's
    /// word that its person saw the card's weight and passed its gate:
    /// every secret ask is weighty, and the yard refuses one without it.
    /// It goes only where it is said: a yard from before the argument
    /// takes the call as it always did.
    pub fn give_secret(
        &self,
        bead: &str,
        ask: u64,
        values: &[(String, String)],
        actor: &str,
        weighed: bool,
    ) -> Result<Given> {
        let values: serde_json::Map<String, Value> = values
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        let mut args = json!({ "bead": bead, "ask": ask, "values": values, "actor": actor });
        if weighed {
            args["weighed"] = json!(true);
        }
        self.answer("GiveSecret", args)
    }

    /// Approves a run ask: the yard runs the command the ask holds, as its
    /// operator, and answers the command as it ran with what it printed.
    /// The call names the ask and nothing of the command: the yard runs
    /// what it stored, never what a client sends. `weighed` is the
    /// client's word that its person saw the card's weight and passed its
    /// gate: a weighty card is refused without it. It goes only where it
    /// is said: a yard from before the argument takes the call as it
    /// always did.
    pub fn approve_ask(
        &self,
        bead: &str,
        ask: u64,
        actor: &str,
        weighed: bool,
    ) -> Result<Approved> {
        // The one call whose arguments go in two forms. The yard's listener
        // takes every call's by name and no other way. Its socket takes
        // them by name only for the calls its API page names, and this one
        // is on that page only once the listener serves it: until then the
        // socket takes them in their order, which it takes of every call.
        // `weighed` is the trailing word in either form, left out where it
        // is unsaid, as a yard from before the argument takes the call.
        let args = if self.is_remote() {
            let mut args = json!({ "bead": bead, "ask": ask, "actor": actor });
            if weighed {
                args["weighed"] = json!(true);
            }
            args
        } else if weighed {
            json!([bead, ask, actor, true])
        } else {
            json!([bead, ask, actor])
        };
        self.answer("ApproveAsk", args)
    }

    /// Approves a setup ask: the yard then applies the pack, takes the
    /// account, and grants the preset picked, as its operator. The call
    /// names the ask with the preset pick counting the ask's options, and
    /// exactly one of the typed values and the picked candidate settles
    /// the account: `values` is left out where a candidate is taken, and
    /// `account` where the values are typed. Both are left out where a
    /// drift card's standing account is kept with a bare preset pick. A
    /// value is the person's secret: it is in this call's body and in
    /// nothing else of the client, no error and no log. `weighed` is the
    /// client's word that its person saw the card's weight and passed its
    /// gate: every setup ask is weighty, and the yard refuses one without
    /// it. It goes only where it is said: a yard from before the argument
    /// takes the call as it always did.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_setup(
        &self,
        bead: &str,
        ask: u64,
        option: u64,
        values: Option<&[(String, String)]>,
        account: Option<&str>,
        actor: &str,
        weighed: bool,
    ) -> Result<SetupApproved> {
        let mut args = json!({ "bead": bead, "ask": ask, "option": option, "actor": actor });
        if weighed {
            args["weighed"] = json!(true);
        }
        if let Some(values) = values {
            let values: serde_json::Map<String, Value> = values
                .iter()
                .map(|(name, value)| (name.clone(), json!(value)))
                .collect();
            args["values"] = json!(values);
        }
        if let Some(account) = account {
            args["account"] = json!(account);
        }
        self.answer("ApproveSetup", args)
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

    /// Puts a plain note on the bead, with the files `files` names by the
    /// hash `PutBlob` answered, each with its base name. Words about a
    /// question with no option chosen go here, and leave the ask open; a
    /// note with a file needs its words. A yard from before notes carried
    /// files refuses the argument, in its words.
    pub fn add_note(&self, bead: &str, text: &str, actor: &str, files: &[BlobFile]) -> Result<()> {
        let mut args = json!({ "bead": bead, "text": text, "actor": actor });
        if !files.is_empty() {
            args["files"] = json!(files);
        }
        self.call("AddNote", args).map(drop)
    }

    /// Sends mail to a crew member or a user of this yard, with the files
    /// `files` names by the hash `PutBlob` answered, each with its base
    /// name. With one, the text may be empty.
    pub fn mail(&self, to: &str, text: &str, actor: &str, files: &[BlobFile]) -> Result<()> {
        let mut args = json!({ "to": to, "text": text, "actor": actor });
        if !files.is_empty() {
            args["files"] = json!(files);
        }
        self.call("Mail", args).map(drop)
    }

    /// Gives the yard a file: any bytes under the cap, for mail or a note
    /// to carry. What it is the yard reads from the bytes; its name goes
    /// on the mail or the note, not here.
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
        let path = format!("/blobs/{hash}");
        let (status, bytes, request) = self.get_body(
            &path,
            "Connection: close\r\n",
            Some(CALL_TIMEOUT),
            |status| status == 200 || self.hosted() && status == 403,
        )?;
        let sent = Sent {
            verb: "GET",
            path: &path,
            limit: Some(CALL_TIMEOUT),
            request: request.as_deref(),
            status: Some(status),
        };
        self.blob_of(hash, status, bytes)
            .map_err(|err| sent.failed(err))
    }

    /// The image `hash`, from the answer to its request: its bytes, none
    /// for one the yard does not hold, or why it is neither.
    fn blob_of(&self, hash: &str, status: u16, bytes: Vec<u8>) -> Result<Option<Vec<u8>>> {
        // Through the edge, a 403 is the router's noYard where no yard
        // is ready, not this call's own refusal; of any other shape
        // today's arms run, on the status alone.
        if self.hosted()
            && status == 403
            && let Some(edge) = no_yard(status, &bytes)
        {
            return Err(edge);
        }
        match status {
            200 => Ok(Some(bytes)),
            404 => Ok(None),
            _ => Err(Error::Protocol(format!(
                "the image {hash}: status {status}"
            ))),
        }
    }

    /// Queues mail to one of a linked yard, `<to>@<peer>`, with the files
    /// `files` names as for [`Client::mail`]. The yard refuses mail with one
    /// when the link does not carry files, and says why.
    pub fn mail_peer(
        &self,
        to: &str,
        peer: &str,
        text: &str,
        actor: &str,
        files: &[BlobFile],
    ) -> Result<Queued> {
        let mut args = json!({ "to": to, "peer": peer, "text": text, "actor": actor });
        if !files.is_empty() {
            args["files"] = json!(files);
        }
        self.answer("MailPeer", args)
    }

    /// Puts the party `actor`'s reaction on the mail `seq`: one of the
    /// yard's fixed set, or `""` to take it away. One party holds one
    /// reaction: a new one replaces the old one. It answers nothing; the
    /// reaction is then a `reaction` event in the stream, and `reactions`
    /// on the letter.
    pub fn react(&self, seq: u64, emoji: &str, actor: &str) -> Result<()> {
        self.call(
            "React",
            json!({ "seq": seq, "emoji": emoji, "actor": actor }),
        )
        .map(drop)
    }

    /// The mail between the user `a` and `b`, oldest first, newer than
    /// `after`.
    pub fn conversation(&self, a: &str, b: &str, after: u64) -> Result<Vec<Letter>> {
        self.list("Conversation", json!({ "a": a, "b": b, "after": after }))
    }

    /// The newest `limit` mails between the user `a` and `b` that are older
    /// than the seq `before` (0: the newest of all), oldest first, and
    /// whether there are older ones still.
    ///
    /// A yard built before it had pages has no such arguments. One refuses
    /// them, and is asked for the conversation whole instead; one that
    /// answers everything to them says nothing of more. Either way the
    /// page is all there is, so nothing is asked for after it.
    pub fn conversation_page(&self, a: &str, b: &str, limit: u64, before: u64) -> Result<Page> {
        let mut args = json!({ "a": a, "b": b, "limit": limit });
        if before > 0 {
            args["before"] = json!(before);
        }
        match self.reply("Conversation", args) {
            Ok(reply) => {
                let letters: Option<Vec<Letter>> =
                    serde_json::from_value(reply.result.unwrap_or(Value::Null))
                        .map_err(|err| Error::Protocol(format!("Conversation: {err}")))?;
                Ok(Page {
                    letters: letters.unwrap_or_default(),
                    more: reply.more,
                })
            }
            // The code says no more than that the call was refused, and the
            // message is not to be read: whatever it was, the call without
            // the new arguments is refused for it again.
            Err(Error::Refused { code, .. }) if code == "error" => {
                let mut letters = self.conversation(a, b, 0)?;
                letters.retain(|l| before == 0 || l.seq < before);
                Ok(Page {
                    letters,
                    more: false,
                })
            }
            Err(err) => Err(err),
        }
    }

    /// The stream of events after a seq: those the yard still has, then
    /// every one as it happens. It ends when the server goes away. Its
    /// answer is waited for `quiet` long, and so is each event: a stream
    /// that says nothing for that long between two events gives
    /// [`Error::Silent`] and may be read on, and one that stops in the
    /// middle of an event has ended.
    pub fn events(&self, after: u64, quiet: Duration) -> Result<Events> {
        let path = format!("/events?after={after}");
        let (status, body) = self.send(
            "GET",
            &path,
            "Accept: application/x-ndjson\r\n",
            "",
            Some(quiet),
            true,
        )?;
        let request = body.request.clone();
        let sent = Sent {
            verb: "GET",
            path: &path,
            limit: Some(quiet),
            request: request.as_deref(),
            status: Some(status),
        };
        self.events_of(status, body).map_err(|err| sent.failed(err))
    }

    /// The stream, from the answer that opens it: its head read, or why it
    /// is none.
    fn events_of(&self, status: u16, mut body: Body) -> Result<Events> {
        // Through the edge, a 403 is the router's noYard where no yard
        // is ready, not this call's own refusal; of any other shape
        // today's arm runs, on the status alone.
        if self.hosted() && status == 403 {
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes)
                .map_err(|err| slow_or_down(err, true))?;
            if let Some(edge) = no_yard(status, &bytes) {
                return Err(edge);
            }
        }
        if status != 200 {
            return Err(Error::Protocol(format!(
                "the stream answered status {status}"
            )));
        }
        let stop = body.conn.get_ref().event_stop()?;
        Ok(Events {
            lines: BufReader::new(body),
            line: String::new(),
            stop,
        })
    }

    /// Which of `me`'s beads were added or closed after `after`
    /// through `until`, by bead, in the order they changed: the created
    /// and closed events with them as originator. The timetable's
    /// catch-up reads it for what changed while nobody followed: `after`
    /// is when Timetable was last open, `until` the head it was read at.
    /// Nothing is asked where there is no gap: a stream past the head
    /// would wait for what happens next. Past `until` the stream is
    /// left: what happens next comes down the follower's own. What the
    /// yard no longer has is quietly missed. A stream that sends more
    /// events than the gap holds seqs is not describing the gap, and is
    /// left where it outruns it: the gap holds one event of each seq
    /// after `after` through `until`, and no more, so the catch-up
    /// cannot be held open for ever by a yard replaying a seq it has
    /// already sent.
    pub fn timetable_news(&self, me: &str, after: u64, until: u64) -> Result<Vec<String>> {
        if after >= until {
            return Ok(Vec::new());
        }
        // The gap's events are the yard's already, and come at once.
        let events = self.events(after, CALL_TIMEOUT)?;
        let most = until - after;
        let mut read = 0;
        let mut seen = HashSet::new();
        let mut news = Vec::new();
        for event in events {
            let event = event?;
            read += 1;
            if event.seq > until || read > most {
                break;
            }
            if event.timetable_news(me) && seen.insert(event.bead.clone()) {
                news.push(event.bead);
            }
            if event.seq == until {
                break;
            }
        }
        Ok(news)
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
        self.reply(method, args).map(|reply| reply.result)
    }

    /// As [`Client::call`], with what the reply says beside its result.
    fn reply(&self, method: &str, args: Value) -> Result<Reply> {
        let mut replacing: Option<Instant> = None;
        loop {
            let reply = self.call_once(method, &args, CALL_TIMEOUT);
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

    /// One try of a call, its answer waited for `timeout`. A try that fails
    /// leaves its line ([`Sent`]), with the edge's id where the answer's head
    /// carried one.
    fn call_once(&self, method: &str, args: &Value, timeout: Duration) -> Result<Reply> {
        let body = args.to_string();
        let path = format!("/call/{method}");
        let (status, mut reply) = self.send(
            "POST",
            &path,
            &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            ),
            &body,
            Some(timeout),
            reads(method),
        )?;
        let request = reply.request.clone();
        let sent = Sent {
            verb: "POST",
            path: &path,
            limit: Some(timeout),
            request: request.as_deref(),
            status: Some(status),
        };
        self.reply_of(method, status, &mut reply)
            .map_err(|err| sent.failed(err))
    }

    /// What the answer to a call says, its head read: the reply, or why it
    /// is none.
    fn reply_of(&self, method: &str, status: u16, reply: &mut Body) -> Result<Reply> {
        // No call is answered 404 by a yard: at an address it is the answer
        // of one with nothing under /user/.
        if status == 404 && self.is_remote() {
            return Err(Error::NoDevice);
        }
        let mut bytes = Vec::new();
        reply
            .read_to_end(&mut bytes)
            .map_err(|err| slow_or_down(err, reads(method)))?;
        // Through the edge, a 403 is the router's noYard where no yard
        // is ready, and a 503, 504 or 429 its refuseCall for the link,
        // before the Reply parse the edge's string-error body is none of;
        // of any other shape today's parse runs.
        if self.hosted()
            && status == 403
            && let Some(edge) = no_yard(status, &bytes)
        {
            return Err(edge);
        }
        if self.hosted()
            && matches!(status, 503 | 504 | 429)
            && let Some(edge) = edge_call_refusal(status, reply.refusal.as_deref(), &bytes)
        {
            return Err(edge);
        }
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
        Ok(reply)
    }
}

/// The events of a stream, one a line, until it ends.
pub struct Events {
    lines: BufReader<Body>,
    /// What is read of the next event: kept over a silence, so that one
    /// that stopped mid-line is told from one between two lines.
    line: String,
    stop: EventStop,
}

impl Events {
    pub fn stop_handle(&self) -> EventStop {
        self.stop.clone()
    }

    /// Waits `quiet` long for each event from here on, in place of what
    /// the stream was opened with.
    pub fn set_quiet(&self, quiet: Duration) -> io::Result<()> {
        match self.stop.0.as_ref() {
            #[cfg(unix)]
            EventSocket::Unix(socket) => socket.set_read_timeout(Some(quiet)),
            EventSocket::Tcp(socket) => socket.set_read_timeout(Some(quiet)),
        }
    }
}

impl Iterator for Events {
    type Item = Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.lines.read_line(&mut self.line) {
                Ok(0) => return None,
                Ok(_) => {
                    let line = std::mem::take(&mut self.line);
                    if line.trim().is_empty() {
                        continue;
                    }
                    return Some(
                        serde_json::from_str(&line)
                            .map_err(|err| Error::Protocol(format!("an event: {err}"))),
                    );
                }
                Err(err) if timed_out(&err) && self.line.is_empty() => {
                    return Some(Err(Error::Silent));
                }
                Err(err) if timed_out(&err) => {
                    return Some(Err(Error::Slow {
                        idempotent: true,
                        sent: true,
                    }));
                }
                Err(err) => return Some(Err(Error::Down(err))),
            }
        }
    }
}

/// Whether a read gave up for its timeout: the system says so as a read
/// that would block (macOS and Linux) or one that timed out.
fn timed_out(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// The calls this client makes that only read, as README's "It reads" table
/// has them; a test holds the two to each other. A call is one POST whether
/// it reads or writes, so what a timeout on it says is told by its name: a
/// read that timed out changed nothing, and any other call, one a later
/// change adds with no row here too, may have run.
const READS: &[&str] = &[
    "Head",
    "Settings",
    "Users",
    "GetUser",
    "Crew",
    "Peers",
    "UserInbox",
    "UserNotifications",
    "DeliveryFiles",
    "UserAsks",
    "Arrivals",
    "Flows",
    "SourceGrants",
    "SessionScreen",
    "Bead",
    "Conversation",
];

/// Whether the call `method` only reads.
fn reads(method: &str) -> bool {
    READS.contains(&method)
}

/// An I/O failure of a yard connection as the call's error: one that gave
/// up for its timeout says the yard is slow, in plain words, and not the
/// raw OS error; anything else says the yard is down. `idempotent` is
/// whether the request changes nothing ([`Error::Slow`]). The request went
/// out: this is past the connect.
fn slow_or_down(err: io::Error, idempotent: bool) -> Error {
    match timed_out(&err) {
        true => Error::Slow {
            idempotent,
            sent: true,
        },
        false => Error::Down(err),
    }
}

/// A connect that failed as the call's error: one that gave up for its
/// timeout sent nothing, so its write is said to be unsent and safe to
/// send again ([`Error::Slow`]); anything else says the yard is down, as
/// ever. `idempotent` is whether the request changes nothing.
fn connect_error(err: Error, idempotent: bool) -> Error {
    match err {
        Error::Down(io) if timed_out(&io) => Error::Slow {
            idempotent,
            sent: false,
        },
        other => other,
    }
}

/// A response reader that already turned its I/O error into `Error`:
/// classify a timeout the same way as the connection's direct reads.
fn slow_error(err: Error, idempotent: bool) -> Error {
    match err {
        Error::Down(err) => slow_or_down(err, idempotent),
        other => other,
    }
}

/// A request as it went out, with what is known of it when it fails. A
/// failed request leaves one line on stderr, as `notify` and `tray` say what
/// they could not do, and in the log beside it, led by the time ([`log`]),
/// so that the edge's own line for it, which names the same id, can be found:
///
/// `footplate: call failed: request=<id|none> method=<verb> path=<path> outcome=<outcome> status=<status|none> timeout=<limit|none>`
///
/// `request` is the edge's id from the answer's `X-Edge-Request`, none where
/// no answer's head was read or it carried none. `outcome` is `timeout` (a
/// wait ran out), `down` (nothing answered, or the answer was cut off),
/// `refused` (the yard or the edge said no) or `unreadable` (what answered
/// is no reply). `status` is that of the answer's head, where it was read.
/// `timeout` is the wait set on the step that failed: the connect's own at
/// an address, else the request's. No token and no body is in it.
#[derive(Clone, Copy)]
struct Sent<'a> {
    verb: &'a str,
    /// As docs/api.md has it on the socket: at an address the request
    /// stands under `/user` too.
    path: &'a str,
    limit: Option<Duration>,
    request: Option<&'a str>,
    status: Option<u16>,
}

impl Sent<'_> {
    fn line(&self, err: &Error) -> String {
        let status = self.status.map_or("none".to_string(), |s| s.to_string());
        let limit = self.limit.map_or("none".to_string(), |l| format!("{l:?}"));
        format!(
            "call failed: request={} method={} path={} outcome={} status={status} timeout={limit}",
            self.request.unwrap_or("none"),
            self.verb,
            self.path,
            err.outcome(),
        )
    }

    /// Says the line, and gives the error back to be returned.
    fn failed(&self, err: Error) -> Error {
        say(&self.line(&err));
        err
    }
}

/// Says `line` on stderr, led by the program's name as its other lines are,
/// and keeps it in the log with the time ([`log`]): an app started by the
/// Finder or by launchd has its stderr on /dev/null, and would lose it.
fn say(line: &str) {
    #[cfg(test)]
    tests::said(line);
    // A test leaves the person's own log alone, and keeps in the file its
    // thread was given, where it was given one.
    #[cfg(test)]
    let file = tests::log_file();
    #[cfg(not(test))]
    let file = log::file();
    log::say(|text| eprintln!("{text}"), file.as_deref(), line);
}

/// Reads the status and the headers of a response, and gives its body.
fn respond(conn: Conn) -> Result<(u16, Body)> {
    let (status, _, _, body) = respond_with_location(conn)?;
    Ok((status, body))
}

/// As [`respond`], with whose a 401 would be ([`Refuser`]). A call's answer
/// is read so.
fn respond_call(conn: Conn) -> Result<(u16, Refuser, Body)> {
    let (status, _, refuser, body) = respond_with_location(conn)?;
    Ok((status, refuser, body))
}

/// Whose a 401 is, told by the answer's head alone. The edge refuses a
/// session it does not know with `X-Edge-Refusal: session`. The yard's
/// listener refuses a token that is no device's with `WWW-Authenticate:
/// Bearer`, on every such refusal (`refuse` of the yardr repository's
/// internal/rpc/user.go), and the edge passes the yard's answer through
/// with its headers but for its own refusal's, so it arrives so. A 401 with
/// neither was answered by something else, as the yard's web view answers a
/// request the edge sent down its socket by mistake; a message is shown,
/// never parsed, so its words say nothing here.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Refuser {
    Edge,
    Yard,
    Other,
}

impl Refuser {
    /// From what the head carried: `X-Edge-Refusal: session`, and a
    /// `WWW-Authenticate` that offers Bearer. The edge's header wins: the
    /// edge takes its own off whatever the yard sent, so it is the edge's.
    fn of(session_refused: bool, bearer: bool) -> Refuser {
        match (session_refused, bearer) {
            (true, _) => Refuser::Edge,
            (false, true) => Refuser::Yard,
            (false, false) => Refuser::Other,
        }
    }

    /// What a 401 of this refuser is.
    fn error(self) -> Error {
        match self {
            Refuser::Edge => Error::NotSignedIn,
            Refuser::Yard => Error::NotLetIn,
            Refuser::Other => Error::Gateway,
        }
    }
}

/// Whether a `WWW-Authenticate` value offers the Bearer scheme. One value
/// may hold several challenges, each a scheme and then its parameters.
fn offers_bearer(value: &str) -> bool {
    value.split(',').any(|challenge| {
        challenge
            .split_whitespace()
            .next()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
    })
}

/// As [`respond`], with where the response sends on to, when it does,
/// and whose a 401 would be.
fn respond_with_location(conn: Conn) -> Result<(u16, Option<String>, Refuser, Body)> {
    let mut conn = BufReader::new(conn);
    let mut line = String::new();
    // A line of the head, read within what is left of the head's limit.
    let mut left = HEAD_MOST;
    let mut read_line = |conn: &mut BufReader<Conn>, line: &mut String| -> Result<usize> {
        let n = conn.take(left).read_line(line)?;
        left -= n as u64;
        if left == 0 && !line.ends_with('\n') {
            return Err(Error::Protocol(format!(
                "the answer's head is longer than {HEAD_MOST} bytes"
            )));
        }
        Ok(n)
    };
    if read_line(&mut conn, &mut line)? == 0 {
        // The server went away over the call, as it does between two
        // builds or over a connection it never served: nothing listens.
        // Whether that reads as a hang-up or a refusal is the kernel's
        // to say; either way the yard is down.
        return Err(Error::Down(io::ErrorKind::UnexpectedEof.into()));
    }
    // "HTTP/1.1 200 OK"
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| Error::Protocol(format!("no HTTP status in {:?}", line.trim_end())))?;
    let mut length = None;
    let mut chunked = false;
    let mut location = None;
    let mut session_refused = false;
    let mut refusal: Option<String> = None;
    let mut bearer = false;
    let mut request = None;
    loop {
        line.clear();
        if read_line(&mut conn, &mut line)? == 0 {
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
        } else if name.eq_ignore_ascii_case("location") {
            location = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("x-edge-refusal") {
            session_refused = value.eq_ignore_ascii_case("session");
            refusal = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("www-authenticate") {
            bearer |= offers_bearer(value);
        } else if name.eq_ignore_ascii_case("x-edge-request") {
            request = edge_request(value);
        }
    }
    let framing = match (chunked, length) {
        (true, _) => Framing::Chunked {
            left: 0,
            size: String::new(),
            done: false,
        },
        (false, Some(n)) => Framing::Length(n),
        (false, None) => Framing::UntilClosed,
    };
    Ok((
        status,
        location,
        Refuser::of(session_refused, bearer),
        Body {
            conn,
            framing,
            request,
            refusal,
        },
    ))
}

/// The body of a response, as the server frames it. Go's server sends a
/// short reply with its length and a long or a streamed one in chunks.
struct Body {
    conn: BufReader<Conn>,
    framing: Framing,
    /// The edge's id for the request this answers, as it echoes it in
    /// `X-Edge-Request`. None where the answer carried none: a yard reached
    /// without an edge, an edge from before the header, or a value that is
    /// no id.
    request: Option<String>,
    /// What the edge refused the request for, as it names it in
    /// `X-Edge-Refusal`: `away` where no link is up, `busy` past what one
    /// holds, `session` where it knows none. None where the answer
    /// carried none: a yard reached without an edge, or a 504, which the
    /// edge answers with no refusal header.
    refusal: Option<String>,
}

/// The most bytes of an edge's request id that are kept: its own are 26,
/// and a line of the log must stay a line.
const EDGE_REQUEST_MOST: usize = 64;

/// The id the edge gave a request, from its `X-Edge-Request` value: one
/// token of visible characters, so that it stands in a line of the log as
/// one word. A value that is not one is no id.
fn edge_request(value: &str) -> Option<String> {
    (is_token(value) && value.len() <= EDGE_REQUEST_MOST).then(|| value.to_string())
}

enum Framing {
    Length(u64),
    /// `left` bytes remain of the chunk being read. `size` is what is read
    /// of the next chunk's size line: a read that gives up for its timeout
    /// may stop anywhere in it, and goes on from there.
    Chunked {
        left: u64,
        size: String,
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
            Framing::Chunked { left, size, done } => {
                if *done {
                    return Ok(0);
                }
                while *left == 0 {
                    // "<size in hex>[;extension]\r\n", after the line break
                    // that ended the chunk before, which reads as a line of
                    // nothing.
                    if self.conn.read_line(size)? == 0 {
                        // The server went away between two chunks: for the
                        // stream that is its end.
                        *done = true;
                        return Ok(0);
                    }
                    let size = std::mem::take(size);
                    if size.trim().is_empty() {
                        continue;
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
                Ok(n)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// The lines the requests of this test said on stderr.
        static SAID: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
        /// The file the requests of this test keep their lines in, where it
        /// set one ([`keep_in`]).
        static LOG: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    }

    /// Where [`say`] leaves a line in a test, beside stderr, for the test
    /// to read: a call is made on the thread that asks it.
    pub(super) fn said(line: &str) {
        SAID.with(|said| said.borrow_mut().push(line.to_string()));
    }

    /// The lines said since this was last asked, in order.
    fn take_said() -> Vec<String> {
        SAID.with(|said| std::mem::take(&mut *said.borrow_mut()))
    }

    /// The file [`say`] keeps its line in during a test: the one this test
    /// set for its thread, else none, so that no test writes the person's
    /// own log.
    pub(super) fn log_file() -> Option<PathBuf> {
        LOG.with(|file| file.borrow().clone())
    }

    /// Has the requests of this test keep their lines in a log under `dir`,
    /// which is not there yet, and gives its file.
    fn keep_in(dir: &std::path::Path) -> PathBuf {
        let file = dir.join("footplate/footplate.log");
        LOG.with(|kept| *kept.borrow_mut() = Some(file.clone()));
        file
    }

    /// A port of this machine that answers every connection with `reply`
    /// and closes: its address.
    fn answering(reply: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for mut conn in listener.incoming().flatten() {
                let mut asked = [0u8; 1024];
                let _ = conn.read(&mut asked);
                let _ = conn.write_all(&reply);
            }
        });
        address
    }

    /// A download reads the file its address has, a query in the address
    /// and all, and no more of an answer's head than a head is long: a
    /// host that sends headers without end is not read without end.
    #[test]
    fn a_download_reads_a_file_and_not_a_head_without_end() {
        let file = answering(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec());
        assert_eq!(download(&format!("{file}/a?b=c"), 5).unwrap(), b"hello");
        assert!(matches!(
            download(&format!("{file}/a"), 4),
            Err(Error::Protocol(_))
        ));

        let mut endless = b"HTTP/1.1 200 OK\r\nX-Long: ".to_vec();
        endless.extend(vec![b'a'; 2 * HEAD_MOST as usize]);
        let Err(Error::Protocol(why)) = download(&answering(endless), 5) else {
            panic!("a head without end was read");
        };
        assert!(why.starts_with("the answer's head is longer than"), "{why}");

        // No address but one of the web, or of this machine, is asked.
        for no in [
            "ftp://127.0.0.1/x",
            "http://example.com/x",
            "127.0.0.1/x",
            "https://a b/x",
        ] {
            assert!(download(no, 5).is_err(), "{no}");
        }
        let (address, target) = split_url("https://example.com?x=1#top").unwrap();
        assert_eq!(
            (address.to_string().as_str(), target.as_str()),
            ("https://example.com", "/?x=1")
        );
    }
    use crate::fake::{KRATOS_EMAIL, KRATOS_PASSWORD, Router, SESSION, TOKEN, Yard};

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
    fn plain_http_stays_on_loopback_and_localhost_tries_ipv4_first() {
        let v6: SocketAddr = "[::1]:8790".parse().unwrap();
        let v4: SocketAddr = "127.0.0.1:8790".parse().unwrap();
        let other: SocketAddr = "10.0.0.7:8790".parse().unwrap();
        assert_eq!(in_order("localhost", false, [v6, v4].into_iter()), [v4, v6]);
        assert_eq!(in_order("LocalHost", false, [v6, v4].into_iter()), [v4, v6]);
        assert_eq!(in_order("localhost", false, [v4, v6].into_iter()), [v4, v6]);
        assert_eq!(in_order("localhost", false, [v6].into_iter()), [v6]);
        assert_eq!(
            in_order("yard.example.net", true, [v6, other, v4].into_iter()),
            [v6, other, v4]
        );
        assert_eq!(
            in_order("localhost", false, [other, v6, v4].into_iter()),
            [v4, v6]
        );
        assert!(in_order("localhost", false, [other].into_iter()).is_empty());
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
            "https://[::1]junk",
            "https://[localhost]",
            "http://[::1]junk",
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

    #[test]
    fn the_web_view_s_address_reads_as_an_address_in_the_view_s_words() {
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        assert_eq!(
            (address.tls, address.host.as_str(), address.port),
            (false, "127.0.0.1", 8791)
        );
        // What is no address is refused as the view's, not a yard's.
        let why = Address::parse_view("plantime").unwrap_err();
        assert_eq!(
            why,
            "\"plantime\" is no address of the web view: one starts with https://, or with http:// for the web view on this machine"
        );
        let why = Address::parse_view("http://plantime:8791").unwrap_err();
        assert_eq!(
            why,
            "http://plantime:8791 is refused: plain http is for the web view on this machine alone (127.0.0.1, ::1, localhost). Give the https:// address of what stands in front of the view."
        );
        let why = Address::parse_view("https://mac.example.net:port").unwrap_err();
        assert!(why.contains("is no address of the web view"), "{why}");
    }

    #[test]
    fn the_run_base_s_address_reads_as_an_address_in_the_run_base_s_words() {
        let address = Address::parse_run("https://pages.example.net/").unwrap();
        assert_eq!(
            (address.tls, address.host.as_str(), address.port),
            (true, "pages.example.net", 443)
        );
        // What is no address is refused as the run base's, not the view's.
        let why = Address::parse_run("plantime").unwrap_err();
        assert_eq!(
            why,
            "\"plantime\" is no address of the run base: one starts with https://, or with http:// for the run base on this machine"
        );
        let why = Address::parse_run("http://plantime:8789").unwrap_err();
        assert!(
            why.starts_with(
                "http://plantime:8789 is refused: plain http is for the run base on this machine alone"
            ),
            "{why}"
        );
    }

    #[test]
    fn a_page_s_url_is_the_view_s_blobs_route_under_its_base() {
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        assert_eq!(
            address.page_url(&"ab".repeat(32)),
            Some(format!("http://127.0.0.1:8791/blobs/{}", "ab".repeat(32)))
        );
        // With no slash, and under a proxy's path, alike.
        let address = Address::parse_view("http://127.0.0.1:8791").unwrap();
        assert_eq!(
            address.page_url("cd").as_deref(),
            Some("http://127.0.0.1:8791/blobs/cd")
        );
        let address = Address::parse_view("https://mac.example.net/yard/").unwrap();
        assert_eq!(
            address.page_url("cd").as_deref(),
            Some("https://mac.example.net/yard/blobs/cd")
        );
        // And under the run base, which run hands to the browser.
        let address = Address::parse_view("http://127.0.0.1:8791/")
            .unwrap()
            .run_base();
        assert_eq!(
            address.page_url(&"ab".repeat(32)),
            Some(format!("http://127.0.0.1:8789/blobs/{}", "ab".repeat(32)))
        );
    }

    #[test]
    fn a_page_whose_hash_is_no_hex_has_no_url() {
        // The hash is a yard's text: it chooses no other path of the view,
        // no query and no other host.
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        for hash in [
            "",
            "../api/x",
            "ab?x=1",
            "ab#x",
            "ab/../cd",
            "@evil.example",
            "ab cd",
        ] {
            assert_eq!(address.page_url(hash), None, "{hash:?}");
        }
    }

    #[test]
    fn a_file_s_url_names_it_under_its_hash_encoded_as_one_segment() {
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        let hash = "ab".repeat(32);
        assert_eq!(
            address.file_url(&hash, "report.pdf"),
            Some(format!("http://127.0.0.1:8791/blobs/{hash}/report.pdf"))
        );
        // The unreserved bytes go as they are; a space, a quote, a `%` and
        // anything outside ASCII go percent-encoded, and no `/`, `?` or `#`
        // of the name reaches the URL as anything but a byte of the name.
        assert_eq!(
            address.file_url(&hash, "a b/..?c#d%\"é.pdf"),
            Some(format!(
                "http://127.0.0.1:8791/blobs/{hash}/a%20b%2F..%3Fc%23d%25%22%C3%A9.pdf"
            ))
        );
        // Under a proxy's path, alike.
        let address = Address::parse_view("https://mac.example.net/yard/").unwrap();
        assert_eq!(
            address.file_url("cd", "report.pdf").as_deref(),
            Some("https://mac.example.net/yard/blobs/cd/report.pdf")
        );
        // An empty name opens the bare route, as a page does.
        assert_eq!(
            address.file_url("cd", "").as_deref(),
            Some("https://mac.example.net/yard/blobs/cd")
        );
    }

    #[test]
    fn a_file_whose_hash_is_no_hex_has_no_url() {
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        for hash in ["", "ab?x=1", "ab/../cd", "ab cd"] {
            assert_eq!(address.file_url(hash, "report.pdf"), None, "{hash:?}");
        }
    }

    #[test]
    fn a_bead_s_url_is_the_view_s_beads_route_with_its_id_as_one_segment() {
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        assert_eq!(
            address.bead_url("footplate-vnfq").as_deref(),
            Some("http://127.0.0.1:8791/beads/footplate-vnfq")
        );
        // Under a proxy's path, alike.
        let address = Address::parse_view("https://mac.example.net/yard/").unwrap();
        assert_eq!(
            address.bead_url("footplate-vnfq").as_deref(),
            Some("https://mac.example.net/yard/beads/footplate-vnfq")
        );
        // The id is the yard's word: no `/`, `?` or `#` of it reaches
        // the URL as anything but a byte of the id.
        let address = Address::parse_view("http://127.0.0.1:8791/").unwrap();
        assert_eq!(
            address.bead_url("a b/..?c#d%").as_deref(),
            Some("http://127.0.0.1:8791/beads/a%20b%2F..%3Fc%23d%25")
        );
        assert_eq!(address.bead_url(""), None);
    }

    #[test]
    fn the_view_answers_where_it_runs_and_nowhere_else() {
        // Any HTTP answer says it runs, even a refusal.
        for status in [200, 404, 421] {
            let view = answering(
                format!(
                    "HTTP/1.1 {status} Reply\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .into_bytes(),
            );
            assert!(
                Address::parse_view(&view).unwrap().answers(),
                "status {status}"
            );
        }
        // A port nothing listens on: one that was bound, and is not now.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(
            !Address::parse_view(&format!("http://127.0.0.1:{port}"))
                .unwrap()
                .answers()
        );
    }

    #[test]
    fn the_run_base_is_the_view_s_host_on_the_sandbox_port() {
        let run = Address::parse_view("http://127.0.0.1:8791/")
            .unwrap()
            .run_base();
        assert_eq!(run.to_string(), "http://127.0.0.1:8789");
        // A view on another port, and one under a proxy's path, alike:
        // the port is the sandbox's, and the path is none.
        let run = Address::parse_view("http://127.0.0.1:9000/")
            .unwrap()
            .run_base();
        assert_eq!(run.to_string(), "http://127.0.0.1:8789");
        let run = Address::parse_view("https://mac.example.net/yard/")
            .unwrap()
            .run_base();
        assert_eq!(run.to_string(), "https://mac.example.net:8789");
        let run = Address::parse_view("http://[::1]:8791/")
            .unwrap()
            .run_base();
        assert_eq!(run.to_string(), "http://[::1]:8789");
    }

    #[test]
    fn the_run_base_answers_where_it_runs_and_nowhere_else() {
        // Any HTTP answer says it runs, even the origin's 404 for no
        // route: the probe asks for the front page, which is none.
        let base = answering(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        assert!(Address::parse_view(&base).unwrap().answers());
        // A port nothing listens on: one that was bound, and is not now.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(
            !Address::parse_view(&format!("http://127.0.0.1:{port}"))
                .unwrap()
                .answers()
        );
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
            let give = || client.give_secret("yd-crm", 18, &values, "jo", false);
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
            assert_eq!(given.next, "", "{way}");
            assert_eq!(
                yard.asked("GiveSecret").last(),
                Some(&json!({"bead": "yd-crm", "ask": 18,
                             "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo"})),
                "{way}"
            );
            // Said, the client's word goes with it: every secret ask is
            // weighty, and the yard refuses one without it.
            client
                .give_secret("yd-crm", 18, &values, "jo", true)
                .unwrap();
            assert_eq!(
                yard.asked("GiveSecret").last(),
                Some(&json!({"bead": "yd-crm", "ask": 18,
                             "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo",
                             "weighed": true})),
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
            assert!(matches!(err, Error::Down(_)), "{way}: {err:?}");
            without(&err);
            // And what is no reply.
            yard.answer("GiveSecret", "not json");
            let err = give().unwrap_err();
            assert!(matches!(err, Error::Protocol(_)), "{way}: {err:?}");
            without(&err);
        }
    }

    /// A run ask is approved in its own call, which names the ask and
    /// nothing of the command: in their order on the socket, by name at an
    /// address, as the yard takes them on each.
    #[test]
    fn a_run_ask_is_approved_by_the_socket_and_at_an_address() {
        for yard in [Yard::start(), Yard::start_tcp()] {
            let client = yard.client();
            let way = client.place();
            let approve = || client.approve_ask("yd-link", 2710, "jo", false);
            // A yard that does not have the call, or does not serve it to
            // this window, says so as of any method.
            let err = approve().unwrap_err();
            assert_eq!(err.to_string(), "no such method ApproveAsk", "{way}");
            // One that has it answers the command as it ran and its result.
            yard.answer(
                "ApproveAsk",
                r#"{"result": {"bead": "yd-link", "ask": 2710, "asker": "yardmaster",
                    "argv": ["peer", "allow", "airy", "--mail-limit", "300"],
                    "does": ["the link with airy takes 300 mails and asks an hour from it. It takes 30 now."],
                    "result": "peer airy: takes 300 mails and asks an hour, and 2400 files"}}"#,
            );
            let approved = approve().unwrap();
            assert_eq!(
                approved.argv,
                ["peer", "allow", "airy", "--mail-limit", "300"],
                "{way}"
            );
            assert_eq!(
                approved.result, "peer airy: takes 300 mails and asks an hour, and 2400 files",
                "{way}"
            );
            let sent = yard.asked("ApproveAsk").last().cloned();
            if client.is_remote() {
                assert_eq!(
                    sent,
                    Some(json!({"bead": "yd-link", "ask": 2710, "actor": "jo"})),
                    "{way}"
                );
            } else {
                assert_eq!(sent, Some(json!(["yd-link", 2710, "jo"])), "{way}");
            }
            // Said, the client's word goes as the trailing word in either
            // form: a weighty card is refused without it.
            client.approve_ask("yd-link", 2710, "jo", true).unwrap();
            let sent = yard.asked("ApproveAsk").last().cloned();
            if client.is_remote() {
                assert_eq!(
                    sent,
                    Some(json!({"bead": "yd-link", "ask": 2710, "actor": "jo", "weighed": true})),
                    "{way}"
                );
            } else {
                assert_eq!(sent, Some(json!(["yd-link", 2710, "jo", true])), "{way}");
            }
            // The yard says no, in its words, whole.
            let refusal = "ask 2710 says the link with airy takes 30 mails an hour now, and it takes 100: the card is from before that changed. Nothing ran. Decline it; it is asked again if it is still wanted.";
            yard.answer(
                "ApproveAsk",
                &json!({"error": {"code": "error", "message": refusal}}).to_string(),
            );
            assert_eq!(approve().unwrap_err().to_string(), refusal, "{way}");
            // The yard went away over the call: no reply, and no word on
            // whether the command ran.
            yard.answer_nothing("ApproveAsk");
            let err = approve().unwrap_err();
            assert!(matches!(err, Error::Down(_)), "{way}: {err:?}");
        }
    }

    /// A setup ask is approved in its own call, which names the ask with
    /// the preset pick counting the ask's options, and exactly one of the
    /// typed values and the picked candidate: by name on the socket and
    /// at an address alike, as the yard takes them on each.
    #[test]
    fn a_setup_ask_is_approved_with_its_preset_and_its_account() {
        const VALUE: &str = "ops@example.com";
        for yard in [Yard::start(), Yard::start_tcp()] {
            let client = yard.client();
            let way = client.place();
            let values = [("COURIER_ACCOUNT".to_string(), VALUE.to_string())];
            let without = |err: &Error| {
                assert!(!err.to_string().contains(VALUE), "{way}: {err}");
                assert!(!format!("{err:?}").contains(VALUE), "{way}: {err:?}");
            };
            // A yard that does not have the call, or does not serve it to
            // this window, says so as of any method.
            let err = client
                .approve_setup("yd-crm", 41, 1, Some(&values), None, "jo", false)
                .unwrap_err();
            assert_eq!(err.to_string(), "no such method ApproveSetup", "{way}");
            without(&err);
            // One that has it answers the pack with the preset picked and
            // that the server carries the setup out after the call.
            yard.answer(
                "ApproveSetup",
                r#"{"result": {"bead": "yd-crm", "ask": 41, "asker": "yardmaster",
                    "pack": "courier", "preset": "read", "given": ["COURIER_ACCOUNT"],
                    "result": "approved, to be carried out"}}"#,
            );
            let approved = client
                .approve_setup("yd-crm", 41, 1, Some(&values), None, "jo", false)
                .unwrap();
            assert_eq!(approved.pack, "courier", "{way}");
            assert_eq!(approved.preset, "read", "{way}");
            assert_eq!(approved.given, ["COURIER_ACCOUNT"], "{way}");
            assert_eq!(approved.account, "", "{way}");
            assert_eq!(approved.result, "approved, to be carried out", "{way}");
            assert_eq!(
                yard.asked("ApproveSetup").last(),
                Some(&json!({"bead": "yd-crm", "ask": 41, "option": 1,
                             "values": {"COURIER_ACCOUNT": VALUE}, "actor": "jo"})),
                "{way}"
            );
            // Said, the client's word goes with it: every setup ask is
            // weighty, and the yard refuses one without it.
            client
                .approve_setup("yd-crm", 41, 1, Some(&values), None, "jo", true)
                .unwrap();
            assert_eq!(
                yard.asked("ApproveSetup").last(),
                Some(&json!({"bead": "yd-crm", "ask": 41, "option": 1,
                             "values": {"COURIER_ACCOUNT": VALUE}, "actor": "jo",
                             "weighed": true})),
                "{way}"
            );
            // The value is in the body alone: no line of the request's head
            // has it.
            let head = yard.last_request();
            assert!(head.contains("/call/ApproveSetup"), "{way}: {head}");
            assert!(!head.contains(VALUE), "{way}: {head}");
            // A taken candidate goes with no values.
            yard.answer(
                "ApproveSetup",
                r#"{"result": {"bead": "yd-crm", "ask": 41, "asker": "yardmaster",
                    "pack": "courier", "preset": "read", "account": "courier-existing",
                    "result": "approved, to be carried out"}}"#,
            );
            let approved = client
                .approve_setup("yd-crm", 41, 1, None, Some("courier-existing"), "jo", false)
                .unwrap();
            assert_eq!(approved.account, "courier-existing", "{way}");
            assert!(approved.given.is_empty(), "{way}");
            assert_eq!(
                yard.asked("ApproveSetup").last(),
                Some(&json!({"bead": "yd-crm", "ask": 41, "option": 1,
                             "account": "courier-existing", "actor": "jo"})),
                "{way}"
            );
            // A drift card's standing account is kept with a bare preset
            // pick: neither values nor a candidate goes.
            yard.answer(
                "ApproveSetup",
                r#"{"result": {"bead": "yd-crm", "ask": 41, "asker": "yardmaster",
                    "pack": "courier", "preset": "read", "keep": true,
                    "result": "approved, to be carried out"}}"#,
            );
            let approved = client
                .approve_setup("yd-crm", 41, 1, None, None, "jo", false)
                .unwrap();
            assert!(approved.keep, "{way}");
            assert!(approved.given.is_empty(), "{way}");
            assert_eq!(approved.account, "", "{way}");
            assert_eq!(
                yard.asked("ApproveSetup").last(),
                Some(&json!({"bead": "yd-crm", "ask": 41, "option": 1, "actor": "jo"})),
                "{way}"
            );
            // The yard says no, in its words, whole.
            let refusal = "ask 41 on yd-crm: the candidate courier-existing holds COURIER_ACCOUNT=ops@example.com no more: the card is from before that changed. Nothing ran. Decline it; it is asked again if it is still wanted.";
            yard.answer(
                "ApproveSetup",
                &json!({"error": {"code": "error", "message": refusal}}).to_string(),
            );
            let err = client
                .approve_setup("yd-crm", 41, 1, None, Some("courier-existing"), "jo", false)
                .unwrap_err();
            assert_eq!(err.to_string(), refusal, "{way}");
            // The yard went away over the call: no reply, and no word on
            // whether the approval landed.
            yard.answer_nothing("ApproveSetup");
            let err = client
                .approve_setup("yd-crm", 41, 1, Some(&values), None, "jo", false)
                .unwrap_err();
            assert!(matches!(err, Error::Down(_)), "{way}: {err:?}");
            without(&err);
        }
    }

    #[test]
    fn a_setup_ask_in_the_queue_and_on_the_bead() {
        let yard = Yard::start();
        let setup = json!({"pack": "courier", "label": "embedded in yardr doctest",
            "account": ["COURIER_ACCOUNT"], "account_file": "/home/jo/.yardr/sources/courier.env",
            "candidates": [{"source": "courier-existing",
                            "file": "/home/jo/.yardr/sources/courier-existing.env",
                            "values": {"COURIER_ACCOUNT": "jo@example.test"}}],
            "does": ["pack courier (embedded in yardr doctest), of kind source, into the yard: it adds 1 source and copies 1 command file (1 command), and changes nothing else. It makes no hook and sets no credential's file.",
                     "the account COURIER_ACCOUNT: already given for courier-existing (COURIER_ACCOUNT=jo@example.test, in /home/jo/.yardr/sources/courier-existing.env). Take it with --account courier-existing, or type another unseen (written to /home/jo/.yardr/sources/courier.env, named in the answer, never the value).",
                     "grants to crew yardmaster — pick one:"],
            "back": "yardr source rm courier; yardr source revoke courier <what> --crew yardmaster",
            "carried": [{"name": "courier", "text": "name: courier\n",
                         "files": [{"program": "courier-mint", "text": "#!/bin/sh\n", "sha256": "2752"}]}],
            "presets": [{"name": "read", "grants": [{"source": "courier", "calls": ["read"]}]},
                        {"name": "everything", "grants": [{"source": "courier", "calls": ["read", "put"]}]}],
            "to": [{"crew": "yardmaster"}]});
        yard.answer(
            "UserAsks",
            &json!({"result": {
                "open": [{"bead": "yd-crm", "ask": 41, "question": "Set up courier for the yardmaster?",
                          "asker": "yardmaster", "setup": setup}],
                "decided": [{"bead": "yd-crm", "ask": 40, "question": "Set up courier for the yardmaster?",
                             "asker": "yardmaster", "state": "answered", "option": 1,
                             "option_label": "read", "setup": setup,
                             "given": ["COURIER_ACCOUNT"],
                             "ran": {"argv": ["setup", "courier"], "result": "approved, to be carried out"}},
                            {"bead": "yd-crm", "ask": 16, "question": "Move them now?",
                             "asker": "yardmaster", "state": "answered", "option": 1}]
            }})
            .to_string(),
        );
        let asks = yard.client().user_asks("jo").unwrap();
        let asked = asks.open[0].setup.as_ref().unwrap();
        assert_eq!(asked.pack, "courier");
        assert_eq!(asked.account, ["COURIER_ACCOUNT"]);
        assert_eq!(asked.account_file, "/home/jo/.yardr/sources/courier.env");
        let candidate = asked.candidates.first().unwrap();
        assert_eq!(candidate.source, "courier-existing");
        assert_eq!(
            candidate.values.get("COURIER_ACCOUNT").map(String::as_str),
            Some("jo@example.test")
        );
        assert_eq!(asked.does.len(), 3);
        assert!(asked.back.starts_with("yardr source rm courier"));
        assert_eq!(asked.carried[0].name, "courier");
        assert_eq!(asked.carried[0].files[0].program, "courier-mint");
        assert_eq!(asked.presets[1].name, "everything");
        assert_eq!(asked.presets[1].grants[0].calls, ["read", "put"]);
        assert_eq!(asked.to[0].crew, "yardmaster");
        assert_eq!(asks.decided[0].setup.as_ref(), Some(asked));
        assert_eq!(asks.decided[0].given, ["COURIER_ACCOUNT"]);
        let did = asks.decided[0].ran.as_ref().unwrap();
        assert_eq!(did.line(), "yardr setup courier");
        assert_eq!(did.result, "approved, to be carried out");
        // Any other ask has none.
        assert_eq!(asks.decided[1].setup, None);

        yard.answer(
            "Bead",
            &json!({"result": {"id": "yd-crm", "asks": [
                {"ask": 41, "question": "Set up courier for the yardmaster?",
                 "options": [{"label": "read", "consequence": "may make every read call of the source courier"},
                             {"label": "You decide", "consequence": "the recommended preset, read, is granted"}],
                 "recommend": 1, "why": "reading is the job", "state": "open", "setup": setup}
            ]}})
            .to_string(),
        );
        let bead = yard.client().bead("yd-crm").unwrap();
        assert_eq!(bead.asks[0].setup.as_ref(), Some(asked));
        assert_eq!(bead.asks[0].options.len(), 2);
        assert_eq!(bead.asks[0].recommend, 1);
    }

    /// A setup ask on a pack already applied: its standing file holds
    /// every account name, or the card names the ones it misses.
    #[test]
    fn a_setup_ask_on_a_pack_already_applied_names_what_stands() {
        let yard = Yard::start();
        let drift = |given: bool, missing: &[&str]| {
            json!({"pack": "courier", "account": ["COURIER_ACCOUNT", "COURIER_REGION"],
                "account_file": "/home/jo/.yardr/sources/courier.env",
                "given": given, "missing": missing,
                "does": ["the account COURIER_ACCOUNT and COURIER_REGION: given, in /home/jo/.yardr/sources/courier.env."],
                "back": "yardr source rm courier",
                "presets": [{"name": "read", "grants": [{"source": "courier", "calls": ["read"]}]}],
                "to": [{"crew": "yardmaster"}]})
        };
        yard.answer(
            "UserAsks",
            &json!({"result": {
                "open": [{"bead": "yd-crm", "ask": 42, "question": "Update courier?",
                            "asker": "yardmaster", "setup": drift(true, &[])},
                         {"bead": "yd-crm", "ask": 43, "question": "Update courier?",
                            "asker": "yardmaster", "setup": drift(false, &["COURIER_REGION"])}],
                "decided": []}})
            .to_string(),
        );
        let asks = yard.client().user_asks("jo").unwrap();
        let kept = asks.open[0].setup.as_ref().unwrap();
        assert!(kept.given);
        assert!(kept.missing.is_empty());
        let missing = asks.open[1].setup.as_ref().unwrap();
        assert!(!missing.given);
        assert_eq!(missing.missing, ["COURIER_REGION"]);
    }

    #[test]
    fn a_run_ask_in_the_queue_and_on_the_bead() {
        let yard = Yard::start();
        let run = json!({"argv": ["peer", "allow", "airy", "--mail-limit", "300"],
            "command": "yardr peer allow airy --mail-limit 300",
            "does": ["the link with airy takes 300 mails and asks an hour from it. It takes 30 now."],
            "back": "yardr peer allow airy --mail-limit 30"});
        let ran = json!({"argv": ["peer", "allow", "airy", "--mail-limit", "300"],
            "result": "peer airy: takes 300 mails and asks an hour, and 2400 files"});
        yard.answer(
            "UserAsks",
            &json!({"result": {
                "open": [{"bead": "yd-link", "ask": 18, "question": "Take more mail from airy?",
                          "asker": "yardmaster", "run": run}],
                "decided": [{"bead": "yd-link", "ask": 17, "question": "Take more mail from airy?",
                             "asker": "yardmaster", "state": "answered", "run": run, "ran": ran},
                            {"bead": "yd-link", "ask": 16, "question": "Move them now?",
                             "asker": "yardmaster", "state": "answered", "option": 1}]
            }})
            .to_string(),
        );
        let asks = yard.client().user_asks("jo").unwrap();
        let asked = asks.open[0].run.as_ref().unwrap();
        assert_eq!(asked.line(), "yardr peer allow airy --mail-limit 300");
        assert_eq!(asked.does.len(), 1);
        assert_eq!(asked.back, "yardr peer allow airy --mail-limit 30");
        assert_eq!(asks.decided[0].run.as_ref(), Some(asked));
        let did = asks.decided[0].ran.as_ref().unwrap();
        assert_eq!(did.line(), "yardr peer allow airy --mail-limit 300");
        assert!(did.result.starts_with("peer airy: takes 300 mails"));
        // Any other ask has neither.
        assert_eq!((&asks.decided[1].run, &asks.decided[1].ran), (&None, &None));
        // The words alone are put together as the yard does it.
        let words = Run {
            argv: asked.argv.clone(),
            ..Run::default()
        };
        assert_eq!(words.line(), asked.line());

        yard.answer(
            "Bead",
            &json!({"result": {"id": "yd-link", "asks": [
                {"ask": 18, "question": "Take more mail from airy?", "options": [],
                 "state": "open", "run": run},
                {"ask": 17, "question": "Take more mail from airy?", "options": [],
                 "state": "answered", "run": run, "ran": ran},
                {"ask": 16, "question": "Move them now?", "state": "answered", "option": 1}
            ]}})
            .to_string(),
        );
        let bead = yard.client().bead("yd-link").unwrap();
        assert_eq!(bead.asks[0].run.as_ref(), Some(asked));
        assert_eq!(bead.asks[0].ran, None);
        assert_eq!(bead.asks[1].ran.as_ref(), Some(did));
        assert_eq!(bead.asks[2].run, None);
    }

    /// A run ask that adds a source carries its texts: the yard's one line
    /// for the command, who may take it back, each declaration with its
    /// command files, the pack with a depot plan's file texts and diffs.
    /// What the card does not show (the yard's stale holding) is ignored.
    /// A carrying out that failed says so beside what it printed.
    #[test]
    fn a_run_ask_that_adds_a_source_carries_its_texts_and_its_pack() {
        let run: Run = serde_json::from_value(json!({
            "argv": ["pack", "apply", "telegram"],
            "command": "yardr pack apply telegram",
            "description": "Import a pack into the yard: set the yard to what the pack says",
            "does": ["pack telegram (embedded in yardr ecbab09), of kind source, into the yard: it adds one source."],
            "back": "yardr source rm telegram",
            "back_by": "the operator",
            "carried": [{
                "name": "telegram",
                "text": "name: telegram\nbase: https://api.telegram.org\n",
                "files": [{"program": "telegram/normalize", "text": "#!/bin/sh\n", "sha256": "dc24"}],
                "old": [{"program": "telegram/thread", "old_sha256": "aaaa",
                          "new_sha256": "bbbb", "diff": "--- was\n+++ is\n"}],
                "removed": [{"path": "old-helper", "sha256": "cccc", "mode": "0700",
                              "size": 12, "text": "old bytes\n"}],
                "inventory": [{"path": "telegram/thread", "sha256": "aaaa",
                                "mode": "0700", "size": 10}]
            }],
            "pack": {"name": "telegram", "kind": "source", "label": "ecbab09"}
        }))
        .unwrap();
        assert_eq!(
            run.description,
            "Import a pack into the yard: set the yard to what the pack says"
        );
        assert_eq!(run.back_by, "the operator");
        assert_eq!(run.carried.len(), 1);
        let carried = &run.carried[0];
        assert_eq!(
            carried.text,
            "name: telegram\nbase: https://api.telegram.org\n"
        );
        assert_eq!(carried.files[0].program, "telegram/normalize");
        assert_eq!(carried.old[0].diff, "--- was\n+++ is\n");
        assert_eq!(carried.removed[0].path, "old-helper");
        assert_eq!(run.pack.as_ref().unwrap().name, "telegram");

        let depot: Run = serde_json::from_value(json!({
            "argv": ["pack", "apply", "starter"],
            "command": "yardr pack apply starter",
            "does": ["pack starter into the depot yard: it changes one file."],
            "back": "yardr pack check starter --depot yard",
            "back_by": "the operator",
            "pack": {"name": "starter", "kind": "dir", "depot": "yard", "label": "ecbab09",
                     "plan": {"objects": [
                         {"kind": "file", "name": "flows/default.md", "action": "change",
                          "file_old_sha256": "1111", "file_new_sha256": "2222",
                          "text": "new text\n", "diff": "--- was\n+++ is\n"},
                         {"kind": "crew", "name": "builders", "action": "unchanged"}
                     ]}}
        }))
        .unwrap();
        let plan = depot.pack.as_ref().unwrap().plan.as_ref().unwrap();
        assert_eq!(plan.objects.len(), 2);
        assert_eq!(plan.objects[0].diff, "--- was\n+++ is\n");
        assert_eq!(plan.objects[1].text, "");

        let failed: Ran = serde_json::from_value(json!({
            "argv": ["pack", "apply", "telegram"],
            "result": "pack telegram: nothing further was applied",
            "failed": true
        }))
        .unwrap();
        assert!(failed.failed);
        assert!(failed.result.contains("nothing further"));
        // Absent where the approval ran in its own write.
        let ran: Ran = serde_json::from_value(json!({
            "argv": ["pack", "apply", "telegram"],
            "result": "pack telegram: 1 source declared"
        }))
        .unwrap();
        assert!(!ran.failed);
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
                             "secret": secret, "concealed": false, "given": ["BOOKS_TOKEN"]},
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
        // An ask the yard says nothing of is concealed, in the queue and
        // on the bead; one it marks unconcealed is not.
        assert!(asks.open[0].concealed);
        assert!(!asks.decided[0].concealed);
        assert!(asks.decided[1].concealed);

        yard.answer(
            "Bead",
            &json!({"result": {"id": "yd-crm", "asks": [
                {"ask": 18, "question": "The token for the books", "options": [],
                 "state": "open", "secret": secret},
                {"ask": 17, "question": "The token for the books", "options": [],
                 "state": "answered", "secret": secret, "concealed": false, "given": ["BOOKS_TOKEN"]},
                {"ask": 16, "question": "Move them now?", "state": "answered", "option": 1}
            ]}})
            .to_string(),
        );
        let bead = yard.client().bead("yd-crm").unwrap();
        assert_eq!(bead.asks[0].secret.as_ref(), Some(asked));
        assert!(bead.asks[0].given.is_empty());
        assert_eq!(bead.asks[1].given, ["BOOKS_TOKEN"]);
        assert_eq!(bead.asks[2].secret, None);
        assert!(bead.asks[0].concealed);
        assert!(!bead.asks[1].concealed);
        assert!(bead.asks[2].concealed);
    }

    /// The same client, the same requests, the same answers, whichever way
    /// the yard is reached.
    #[test]
    fn a_call_an_image_and_the_stream_by_the_socket_and_at_an_address() {
        // On Windows Yard::start() is the TCP yard, so both arms would be
        // the same yard twice; the stopped-yard tail below is three refused
        // connects, two seconds each there, and is covered once.
        #[cfg(unix)]
        let yards = vec![Yard::start(), Yard::start_tcp()];
        #[cfg(not(unix))]
        let yards = vec![Yard::start()];
        for yard in yards {
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
            let mut events = client.events(3, QUIET).unwrap();
            assert_eq!(events.next().unwrap().unwrap().text("text"), "four");
            yard.emit(r#"{"seq": 5, "kind": "crew_started"}"#);
            assert_eq!(events.next().unwrap().unwrap().seq, 5, "{way}");
            assert_eq!(yard.streams_asked(), vec![3], "{way}");
            yard.stop();
            assert!(events.next().is_none(), "{way}");
            // And nothing listens.
            let head = client.head();
            assert!(matches!(head, Err(Error::Down(_))), "{way}: {head:?}");
            assert!(
                matches!(client.events(0, QUIET), Err(Error::Down(_))),
                "{way}"
            );
            assert!(matches!(client.blob(HASH), Err(Error::Down(_))), "{way}");
        }
    }

    /// Windows: what stopping a stream rests on. A read already waiting on
    /// a socket is ended by cancelling the reads of another handle of it,
    /// which is what a stream's stop handle holds (a copy of the socket
    /// the reader reads), and a cancel that finds nothing waiting leaves
    /// the socket as it was. The peer here neither writes nor closes while
    /// the read waits, so nothing but the cancel can end the wait.
    #[cfg(windows)]
    #[test]
    fn cancelling_the_reads_of_a_socket_ends_a_read_waiting_on_another_handle_of_it() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let handle = client.try_clone().unwrap();

        // With no read waiting the cancel has nothing to end, and the socket
        // reads on as before.
        cancel_reads(&handle);
        peer.write_all(b"x").unwrap();
        let mut byte = [0u8];
        assert_eq!(client.read(&mut byte).unwrap(), 1);
        assert_eq!(byte, *b"x");

        let (sent, received) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || sent.send(client.read(&mut byte)).unwrap());
        // The cancel is made again until the reader answers: one that came
        // before the reader waits finds nothing, which is harmless, and the
        // test then fails for want of a wait to end, not for the cancel.
        let deadline = Instant::now() + Duration::from_secs(2);
        let waited = loop {
            thread::sleep(Duration::from_millis(50));
            cancel_reads(&handle);
            match received.try_recv() {
                Ok(waited) => break waited,
                Err(_) if Instant::now() < deadline => {}
                Err(_) => panic!("the cancel ends the wait of a read on another handle"),
            }
        };
        assert!(waited.is_err(), "{waited:?}");
        reader.join().unwrap();
    }

    /// The reader that waits for the next event of a quiet stream is woken
    /// by stopping it, whatever the yard does about that: these yards keep
    /// the stream open when their client hangs up, as a proxy that takes
    /// the FIN for a half-close does. Stopped before the reader waits, its
    /// read would fail at once on any system, so it has read an event first.
    #[test]
    fn stopping_an_idle_event_stream_unblocks_its_reader() {
        for yard in [Yard::start(), Yard::start_tcp()] {
            yard.hold_streams();
            yard.emit(r#"{"seq": 1, "kind": "crew_started"}"#);
            let way = yard.client().place();
            let mut events = yard.client().events(0, QUIET).unwrap();
            let stop = events.stop_handle();
            let (read, first) = std::sync::mpsc::channel();
            let (sent, received) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                // The event that is there, and then the wait for another.
                let event = events.next();
                read.send(event).unwrap();
                sent.send(events.next()).unwrap();
            });
            let event = first.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(event.unwrap().unwrap().seq, 1, "{way}");
            // Nothing says that the read is waiting. A little time makes it
            // so; a stop that came earlier still ends the reader, and the
            // test is then no proof, but never a failure.
            std::thread::sleep(Duration::from_millis(100));

            stop.stop();

            let ended = received
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("{way}: stopping the stream wakes its reader"));
            assert!(!matches!(ended, Some(Ok(_))), "{way}: {ended:?}");
            reader.join().unwrap();
        }
    }

    #[cfg(unix)]
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
        let _stream = client.events(7, QUIET).unwrap();
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
        assert!(matches!(stranger.events(0, QUIET), Err(Error::NotLetIn)));
        assert_eq!(yard.requests(), before + 4);
        // The four left a line each: the yard's own 401, with no id, as no
        // edge stands in front of it.
        let refused = |method: &str, path: &str, status: u16, wait: &str| {
            format!(
                "call failed: request=none method={method} path={path} outcome=refused status={status} timeout={wait}"
            )
        };
        assert_eq!(
            take_said(),
            [
                refused("POST", "/call/Head", 401, "30s"),
                refused("GET", "/me", 401, "30s"),
                refused("GET", &format!("/blobs/{HASH}"), 401, "30s"),
                refused("GET", "/events?after=0", 401, "60s"),
            ]
        );
        // The call never reached the yard's store.
        assert_eq!(yard.asked("Head"), Vec::<Value>::new());

        // A device that is removed: its stream is closed, and its token is
        // refused from then on.
        let client = yard.client();
        assert_eq!(client.head().unwrap(), 12);
        let mut events = client.events(0, QUIET).unwrap();
        yard.add_device("yardr-dev-phone");
        yard.remove_device(TOKEN);
        assert!(events.next().is_none());
        assert!(matches!(client.head(), Err(Error::NotLetIn)));
        assert_eq!(take_said(), [refused("POST", "/call/Head", 401, "30s")]);

        // A yard with no device at all has nothing under /user/, with a
        // token or without: 404, which is said as what it is.
        yard.remove_device("yardr-dev-phone");
        for err in [client.me().unwrap_err(), client.head().unwrap_err()] {
            assert!(matches!(err, Error::NoDevice), "{err:?}");
        }
        assert_eq!(
            take_said(),
            [
                refused("GET", "/me", 404, "30s"),
                refused("POST", "/call/Head", 404, "30s"),
            ]
        );
        // An image that is gone is 404 too, and is no such thing: no
        // failure, so no line.
        yard.add_device(TOKEN);
        assert_eq!(client.blob(HASH).unwrap(), None);
        assert_eq!(take_said(), Vec::<String>::new());
    }

    /// Through the edge, a good pair reaches the yard: every call carries
    /// the session token beside the device's, and the yard answers the
    /// device as ever, without ever seeing the session.
    #[test]
    fn through_the_edge_a_good_pair_reaches_the_yard() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let edge = Router::start(&yard, SESSION);
        let client = edge.client_with(TOKEN, SESSION);
        assert_eq!(client.head().unwrap(), 12);
        assert_eq!(edge.proxied(), 1);
        // The call's body crossed the edge whole.
        assert_eq!(yard.asked("Head"), vec![serde_json::json!({})]);
        // Both tokens rode to the edge.
        let request = edge.last_request();
        assert!(
            request.starts_with("POST /user/call/Head HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request.contains(&format!("Authorization: Bearer {TOKEN}\r\n")),
            "{request}"
        );
        assert!(
            request.contains(&format!("X-Edge-Token: {SESSION}\r\n")),
            "{request}"
        );
        // And the yard saw the device alone.
        let request = yard.last_request();
        assert!(
            request.contains(&format!("Authorization: Bearer {TOKEN}\r\n")),
            "{request}"
        );
        assert!(
            !request.to_ascii_lowercase().contains("x-edge-token"),
            "{request}"
        );
        // The session is in no word a client says of itself.
        assert!(!format!("{client:?}").contains(SESSION));
        // Who the pair is, and the stream after it.
        assert_eq!(client.me().unwrap().user, "jo");
        let _stream = client.events(0, QUIET).unwrap();
        assert_eq!(edge.proxied(), 3);
        assert!(
            edge.last_request()
                .contains(&format!("X-Edge-Token: {SESSION}\r\n"))
        );

        // A direct client sends no session header.
        yard.client().head().unwrap();
        assert!(
            !yard
                .last_request()
                .to_ascii_lowercase()
                .contains("x-edge-token"),
            "{}",
            yard.last_request()
        );
    }

    /// Through the edge, a named yard rides beside the session on every
    /// call and on the pairing; an unnamed one sends no header, and the
    /// edge reaches the account's own yard.
    #[test]
    fn through_the_edge_a_named_yard_rides_beside_the_session() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let edge = Router::start(&yard, SESSION);
        // Named: every call says which of the account's yards, beside
        // the session.
        let named = edge.client_with_yard(TOKEN, SESSION, "airy");
        assert_eq!(named.head().unwrap(), 12);
        assert_eq!(named.me().unwrap().user, "jo");
        let request = edge.last_request();
        assert!(request.contains("X-Edge-Yard: airy\r\n"), "{request}");
        assert!(
            request.contains(&format!("X-Edge-Token: {SESSION}\r\n")),
            "{request}"
        );
        // And the pairing too.
        yard.pair("K7QM-2XPD");
        let address = Address::parse(edge.address()).unwrap();
        pair_hosted(&address, "K7QM-2XPD", SESSION, "airy").unwrap();
        assert!(
            edge.last_request().contains("X-Edge-Yard: airy\r\n"),
            "{}",
            edge.last_request()
        );
        // Unnamed: no header rides, as today.
        edge.client_with(TOKEN, SESSION).head().unwrap();
        assert!(
            !edge
                .last_request()
                .to_ascii_lowercase()
                .contains("x-edge-yard"),
            "{}",
            edge.last_request()
        );
        // One that is no token rides on no request either.
        edge.client_with_yard(TOKEN, SESSION, "airy farm")
            .head()
            .unwrap();
        assert!(
            !edge
                .last_request()
                .to_ascii_lowercase()
                .contains("x-edge-yard"),
            "{}",
            edge.last_request()
        );
    }

    /// Through the edge, a request written after the edge has accepted
    /// waits for its bytes: the edge blocks for them instead of hanging
    /// up on the empty read, as a nonblocking accept would.
    #[test]
    fn the_edge_waits_for_a_request_written_late() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let edge = Router::start(&yard, SESSION);
        let target = edge.address().strip_prefix("http://").unwrap().to_string();
        let mut conn = TcpStream::connect(target).unwrap();
        // The edge accepts and reaches its read with nothing written:
        // many of its accept polls pass first.
        thread::sleep(Duration::from_millis(100));
        write!(
            conn,
            "POST /user/call/Head HTTP/1.1\r\nHost: edge\r\nAuthorization: Bearer {TOKEN}\r\nX-Edge-Token: {SESSION}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
        conn.flush().unwrap();
        let mut reply = String::new();
        BufReader::new(conn).read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200 "), "{reply}");
        assert_eq!(edge.proxied(), 1);
    }

    /// The two 401s through the edge are told apart by the response's
    /// shape: the edge's own refuses the session, the yard's the device,
    /// and neither is asked twice. A third, which has neither's header, is
    /// no refusal of the device: it is said as what it is, and the calls
    /// beside it go on.
    #[test]
    fn a_refused_session_and_a_refused_device_are_told_apart() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let edge = Router::start(&yard, SESSION);

        // No session the edge knows: its own 401, and the yard hears
        // nothing of any of it.
        let stranger = edge.client_with(TOKEN, "kratos-dev-unknown");
        let err = stranger.head().unwrap_err();
        assert!(matches!(err, Error::NotSignedIn), "{err:?}");
        assert_eq!(err.to_string(), "this session is no longer signed in");
        assert_eq!(edge.proxied(), 0);
        assert_eq!(yard.requests(), 0);
        assert!(matches!(stranger.me(), Err(Error::NotSignedIn)));
        assert!(matches!(stranger.blob(HASH), Err(Error::NotSignedIn)));
        assert!(matches!(stranger.events(0, QUIET), Err(Error::NotSignedIn)));
        assert_eq!(edge.proxied(), 0);
        assert_eq!(yard.requests(), 0);

        // A session revoked at the edge is refused from then on.
        let client = edge.client_with(TOKEN, SESSION);
        assert_eq!(client.head().unwrap(), 12);
        edge.revoke(SESSION);
        assert!(matches!(client.head(), Err(Error::NotSignedIn)));
        assert_eq!(edge.proxied(), 1);
        assert_eq!(yard.asked("Head").len(), 1);

        // A device the yard does not know, with a good session: the
        // yard's own 401, passed through, which offers pairing again.
        edge.allow(SESSION);
        let wrong = edge.client_with("yardr-dev-unknown", SESSION);
        let err = wrong.head().unwrap_err();
        assert!(matches!(err, Error::NotLetIn), "{err:?}");
        assert_eq!(edge.proxied(), 2);
        // The call reached the yard, but never its store.
        assert_eq!(yard.asked("Head").len(), 1);

        // A known device with a good session, and the stream turned away
        // as a hosted vps yard's was (yardr-hosted-c0sb): a 401 of the
        // yard's web view, with neither header. It says nothing of the
        // device, which the calls beside it show, and it is not the words
        // of a device that is out.
        let known = edge.client_with(TOKEN, SESSION);
        edge.turn_away("/user/events");
        take_said();
        let Err(err) = known.events(0, QUIET) else {
            panic!("the stream opened");
        };
        assert!(matches!(err, Error::Gateway), "{err:?}");
        assert!(!err.to_string().contains("no longer"), "{err}");
        assert_eq!(
            take_said(),
            [
                "call failed: request=none method=GET path=/events?after=0 outcome=refused status=401 timeout=60s"
            ]
        );
        assert_eq!(known.head().unwrap(), 12);
        assert!(known.me().is_ok());
        // The turned-away request never reached the yard; the two calls did.
        assert_eq!(edge.proxied(), 4);
        // Passed again, the stream opens: the same device, the same session.
        edge.let_through();
        assert!(known.events(0, QUIET).is_ok());
    }

    /// A connection whose request has not arrived when the edge reads for
    /// it is held open for the request, not reset: the stranger's answer
    /// stays the edge's own 401, whatever its accept loop found first.
    #[test]
    fn a_request_that_is_not_there_when_the_edge_reads_is_waited_for() {
        use std::net::TcpStream;
        use std::time::Duration;

        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let at = edge.address().strip_prefix("http://").unwrap();
        let mut conn = TcpStream::connect(at).unwrap();
        // Nothing is sent, so the edge reads for the request before it
        // comes. A connection it closed would answer EOF; one it holds
        // open holds this read to its timeout instead.
        conn.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let waiting = conn.read(&mut [0u8; 1]).unwrap_err();
        assert!(
            matches!(
                waiting.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ),
            "expected the edge to keep waiting for a request, got {waiting:?}"
        );
        // And the request that comes then is refused as ever, at the edge.
        conn.set_read_timeout(Some(CALL_TIMEOUT)).unwrap();
        conn.write_all(
            b"POST /user/call HTTP/1.1\r\nHost: edge\r\nX-Edge-Token: kratos-dev-unknown\r\nContent-Length: 0\r\n\r\n",
        )
        .unwrap();
        let mut answer = String::new();
        conn.read_to_string(&mut answer).unwrap();
        assert!(
            answer.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
            "{answer}"
        );
        assert!(
            answer.contains("\r\nX-Edge-Refusal: session\r\n"),
            "{answer}"
        );
        assert_eq!(edge.proxied(), 0);
        assert_eq!(yard.requests(), 0);
    }

    /// The shapes the two refusals are told by: the edge's carries
    /// `X-Edge-Refusal: session` with a body naming the edge, the yard's
    /// is plain `Unauthorized` with `WWW-Authenticate: Bearer` and no
    /// edge header — through the edge byte for byte. A 401 of the yard's
    /// web view has the words of its gateway and neither header.
    #[test]
    fn the_two_refusals_have_the_shapes_they_are_told_by() {
        use std::net::TcpStream;

        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let ask = |at: &str, head: &str| -> String {
            let mut conn = TcpStream::connect(at.strip_prefix("http://").unwrap()).unwrap();
            conn.write_all(head.as_bytes()).unwrap();
            let mut answer = String::new();
            conn.read_to_string(&mut answer).unwrap();
            answer
        };
        // The edge, with no session it knows.
        let answer = ask(
            edge.address(),
            "GET /user/me HTTP/1.1\r\nHost: edge\r\nConnection: close\r\n\r\n",
        );
        assert!(
            answer.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
            "{answer}"
        );
        assert!(
            answer.contains("\r\nX-Edge-Refusal: session\r\n"),
            "{answer}"
        );
        assert!(
            answer.ends_with("{\"error\":\"edge: no Kratos session; sign in first\"}\n"),
            "{answer}"
        );
        // The yard, with no device it knows: direct, and through the edge
        // with a good session. The edge adds nothing.
        let head = "GET /user/me HTTP/1.1\r\nHost: yard\r\nAuthorization: Bearer unknown\r\nConnection: close\r\n\r\n";
        let direct = ask(yard.address(), head);
        assert!(
            direct.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
            "{direct}"
        );
        assert!(
            direct.contains("\r\nWWW-Authenticate: Bearer\r\n"),
            "{direct}"
        );
        assert!(
            !direct.to_ascii_lowercase().contains("x-edge-refusal"),
            "{direct}"
        );
        assert!(direct.ends_with("Unauthorized\n"), "{direct}");
        let head = format!(
            "GET /user/me HTTP/1.1\r\nHost: edge\r\nAuthorization: Bearer unknown\r\nX-Edge-Token: {SESSION}\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(ask(edge.address(), &head), direct);
        // The yard's web view, asked what is not its to answer: neither
        // header, so neither refusal.
        edge.turn_away("/user/me");
        let gateway = ask(edge.address(), &head);
        assert!(
            gateway.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
            "{gateway}"
        );
        let lower = gateway.to_ascii_lowercase();
        assert!(
            !lower.contains("www-authenticate") && !lower.contains("x-edge-refusal"),
            "{gateway}"
        );
        assert!(
            gateway.ends_with(
                "the gateway's token is missing: the X-Edge-JWT header is not on the request\n"
            ),
            "{gateway}"
        );
    }

    /// Pairing through the edge: with a good session the code reaches the
    /// yard and the device is the yard's; with none the edge answers, and
    /// the code is never spent.
    #[test]
    fn pairing_through_the_edge_takes_a_session_first() {
        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let address = Address::parse(edge.address()).unwrap();

        // Without a session the edge knows, pairing says to sign in
        // first, and the pairing stays open.
        yard.pair("K7QM-2XPD");
        let Err(Error::Refused { code, message }) = pair(&address, "K7QM-2XPD") else {
            panic!("the edge let no session in");
        };
        assert_eq!(code, "no_session");
        assert!(message.starts_with("Sign in first"), "{message}");
        assert!(yard.pairing_open());
        assert_eq!(edge.proxied(), 0);

        // With one, the code is the device's token, once.
        let paired = pair_hosted(&address, "K7QM-2XPD", SESSION, "").unwrap();
        assert!(!yard.pairing_open());
        assert_eq!(edge.proxied(), 1);
        assert!(yard.devices().contains(&paired.token));
        assert_eq!(
            edge.client_with(&paired.token, SESSION).me().unwrap().user,
            "jo"
        );
        // The code is good once, and the session rode beside it.
        let Err(Error::Refused { code, .. }) = pair_hosted(&address, "K7QM-2XPD", SESSION, "")
        else {
            panic!("the code was good once");
        };
        assert_eq!(code, "no_pairing");
        assert!(
            edge.last_request()
                .contains(&format!("X-Edge-Token: {SESSION}\r\n")),
            "{}",
            edge.last_request()
        );
    }

    /// Pairing through the edge tells no ready yard from a wrong code:
    /// the edge's 403 reads as the yard not ready, and the code is never
    /// spent on it, while the yard's own 403 past a ready yard stays a
    /// wrong one.
    #[test]
    fn pairing_through_the_edge_tells_no_ready_yard_from_a_wrong_code() {
        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let address = Address::parse(edge.address()).unwrap();

        // No yard is ready: the edge's 403 is the yard not ready, in the
        // edge's sentence, and the pairing stays open.
        yard.pair("K7QM-2XPD");
        edge.no_yard("the yard is building");
        let Err(Error::Refused { code, message }) = pair_hosted(&address, "K7QM-2XPD", SESSION, "")
        else {
            panic!("no yard answered the code");
        };
        assert_eq!(code, "no_yard");
        assert_eq!(message, "the yard is building");
        assert!(yard.pairing_open());
        assert_eq!(edge.proxied(), 0);
        assert_eq!(yard.requests(), 0);

        // Past a ready yard, the yard's own 403 stays a wrong code: the
        // code is counted, and the pairing stays open for the next try.
        let edge = Router::start(&yard, SESSION);
        let address = Address::parse(edge.address()).unwrap();
        let Err(Error::Refused { code, message }) = pair_hosted(&address, "WXYZ-2345", SESSION, "")
        else {
            panic!("a wrong code was taken");
        };
        assert_eq!(code, "wrong_code");
        assert!(message.starts_with("That is not the code."), "{message}");
        assert!(yard.pairing_open());
        assert_eq!(edge.proxied(), 1);
    }

    /// Calls through the edge read no ready yard as the yard not ready:
    /// a call, who this device is, an image and the stream alike, and
    /// the yard hears none of them. Past a ready yard a 403 of any other
    /// shape keeps today's arms.
    #[test]
    fn calls_through_the_edge_read_no_ready_yard() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let edge = Router::start(&yard, SESSION);
        edge.no_yard("the yard is building");
        let client = edge.client_with(TOKEN, SESSION);

        let not_ready = |err: Error| {
            let Error::Refused { code, message } = err else {
                panic!("{err:?}");
            };
            assert_eq!(code, "no_yard");
            assert_eq!(message, "the yard is building");
        };
        not_ready(client.head().unwrap_err());
        not_ready(client.me().unwrap_err());
        not_ready(client.blob(HASH).unwrap_err());
        let Err(err) = client.events(0, QUIET) else {
            panic!("no yard opened a stream");
        };
        not_ready(err);
        assert_eq!(edge.proxied(), 0);
        assert_eq!(yard.requests(), 0);
        // Each of the four left its line, in the order they were made: the
        // edge's own refusal, with the status it came with and no id, as
        // the edge of this test echoes none.
        let refused = |method: &str, path: &str, wait: &str| {
            format!(
                "call failed: request=none method={method} path={path} outcome=refused status=403 timeout={wait}"
            )
        };
        assert_eq!(
            take_said(),
            [
                refused("POST", "/call/Head", "30s"),
                refused("GET", "/me", "30s"),
                refused("GET", &format!("/blobs/{HASH}"), "30s"),
                refused("GET", "/events?after=0", "60s"),
            ]
        );

        // Past a ready yard, a 403 of any other shape keeps today's
        // arms: the yard's own refusal reads as its own, and a bare one
        // as its status.
        let edge = Router::start(&yard, SESSION);
        let client = edge.client_with(TOKEN, SESSION);
        yard.answer_status(
            "Head",
            403,
            r#"{"error": {"code": "no_such", "message": "the yard says no"}}"#,
        );
        let err = client.head().unwrap_err();
        assert!(
            matches!(err, Error::Refused { ref code, .. } if code == "no_such"),
            "{err:?}"
        );
        yard.answer_status("Head", 403, "{}");
        let err = client.head().unwrap_err();
        assert!(matches!(err, Error::Protocol(_)), "{err:?}");
    }

    /// Calls through the edge read away, no answer and busy in the edge's
    /// words: a 503 with away, a 504, and a 429 with busy, each with the
    /// edge's string-error body, never as what footplate cannot read. The
    /// yard hears none of them. Past a ready yard a 503 of the yard's own
    /// shape keeps today's arms: replacing is the yard's, and a bare one
    /// its status.
    #[test]
    fn calls_through_the_edge_read_away_no_answer_and_busy() {
        let yard = Yard::start_tcp();
        yard.answer("Head", r#"{"result": 12}"#);
        let refused = |method: &str, path: &str, status: u16, wait: &str| {
            format!(
                "call failed: request=none method={method} path={path} outcome=refused status={status} timeout={wait}"
            )
        };
        for (set, code, message, status) in [
            ("away", "away", "the yard is away", 503),
            (
                "no_answer",
                "no_answer",
                "the yard gave no answer in time",
                504,
            ),
            (
                "busy",
                "busy",
                "the yard's link holds too many open calls and streams",
                429,
            ),
        ] {
            let edge = Router::start(&yard, SESSION);
            match set {
                "away" => edge.away(message),
                "no_answer" => edge.no_answer(message),
                _ => edge.busy(message),
            }
            let client = edge.client_with(TOKEN, SESSION);
            let Err(Error::Refused {
                code: got,
                message: said,
            }) = client.head()
            else {
                panic!("{set}: an away edge answered");
            };
            assert_eq!(got, code, "{set}");
            assert_eq!(said, message, "{set}");
            assert!(!said.contains("cannot read"), "{set}: {said}");
            assert_eq!(edge.proxied(), 0, "{set}");
            assert_eq!(
                take_said(),
                [refused("POST", "/call/Head", status, "30s")],
                "{set}"
            );
        }
        assert_eq!(yard.requests(), 0);

        // Past a ready yard, the yard's own 503 keeps today's arms: its
        // refusal reads as its own, and a bare one as its status. The
        // edge's header alone, without its body, is none of the edge's.
        let edge = Router::start(&yard, SESSION);
        let client = edge.client_with(TOKEN, SESSION);
        yard.answer_status(
            "Head",
            503,
            r#"{"error": {"code": "no_such", "message": "the yard says no"}}"#,
        );
        let err = client.head().unwrap_err();
        assert!(
            matches!(err, Error::Refused { ref code, .. } if code == "no_such"),
            "{err:?}"
        );
        yard.answer_status("Head", 503, "{}");
        let err = client.head().unwrap_err();
        assert!(matches!(err, Error::Protocol(_)), "{err:?}");
        let _ = take_said();
    }

    /// The edge's noYard is told by its status and its body's key: a 403
    /// of `{"error": "<sentence>"}`, with one leading "edge: " stripped.
    /// Anything else is none, and the caller's own arms run.
    #[test]
    fn no_yard_reads_only_the_edge_s_403() {
        let some = |status: u16, body: &str| match no_yard(status, body.as_bytes()) {
            Some(Error::Refused { code, message }) => (code, message),
            answer => panic!("{answer:?}"),
        };
        for (body, message) in [
            (
                r#"{"error":"edge: the yard is building"}"#,
                "the yard is building",
            ),
            (
                "{\"error\":\"edge: the yard is building\"}\n",
                "the yard is building",
            ),
            (
                r#"{"error":"the yard is building"}"#,
                "the yard is building",
            ),
        ] {
            let (code, sentence) = some(403, body);
            assert_eq!(code, "no_yard", "{body}");
            assert_eq!(sentence, message, "{body}");
        }
        // Anything else is none: the yard's own refusal, an error of the
        // call's own shape, no JSON at all, and any other status.
        for body in [
            r#"{"refused":"wrong_code"}"#,
            r#"{"error":"edge: the yard is building","refused":"wrong_code"}"#,
            r#"{"error":"edge: the yard is building","extra":true}"#,
            r#"{"error":{"code":"no_such","message":"the yard says no"}}"#,
            "{}",
            "Forbidden",
            "",
        ] {
            assert!(no_yard(403, body.as_bytes()).is_none(), "{body}");
        }
        assert!(no_yard(200, r#"{"error":"edge: the yard is building"}"#.as_bytes()).is_none());
    }

    /// The edge's refuseCall is told by its status with its header and its
    /// body's key: a 503 with away, a 504, and a 429 with busy, each of
    /// `{"error": "<sentence>"}`, with one leading "edge: " stripped.
    /// Anything else is none, and the caller's own arms run.
    #[test]
    fn edge_call_refusal_reads_only_the_edge_s_503_504_and_429() {
        let some = |status: u16, refusal: Option<&str>, body: &str| match edge_call_refusal(
            status,
            refusal,
            body.as_bytes(),
        ) {
            Some(Error::Refused { code, message }) => (code, message),
            answer => panic!("{status} {refusal:?} {body}: {answer:?}"),
        };
        for (status, refusal, body, code, message) in [
            (
                503,
                Some("away"),
                r#"{"error":"edge: the yard is away"}"#,
                "away",
                "the yard is away",
            ),
            (
                503,
                Some("Away"),
                "{\"error\":\"edge: the yard is away\"}\n",
                "away",
                "the yard is away",
            ),
            (
                503,
                Some("away"),
                r#"{"error":"the yard is away"}"#,
                "away",
                "the yard is away",
            ),
            (
                504,
                None,
                r#"{"error":"edge: the yard gave no answer in time"}"#,
                "no_answer",
                "the yard gave no answer in time",
            ),
            (
                504,
                None,
                "{\"error\":\"edge: the yard gave no answer in time\"}\n",
                "no_answer",
                "the yard gave no answer in time",
            ),
            (
                429,
                Some("busy"),
                r#"{"error":"edge: the yard's link holds too many open calls and streams"}"#,
                "busy",
                "the yard's link holds too many open calls and streams",
            ),
            (
                429,
                Some("BUSY"),
                "{\"error\":\"edge: the yard's link holds too many open calls and streams\"}\n",
                "busy",
                "the yard's link holds too many open calls and streams",
            ),
        ] {
            let (got, said) = some(status, refusal, body);
            assert_eq!(got, code, "{body}");
            assert_eq!(said, message, "{body}");
        }
        for (status, refusal, body) in [
            (503, None, r#"{"error":"edge: the yard is away"}"#),
            (503, Some("busy"), r#"{"error":"edge: the yard is away"}"#),
            (
                503,
                Some("away"),
                r#"{"error": {"code": "x", "message": "y"}}"#,
            ),
            (503, Some("away"), "{}"),
            (503, Some("away"), "Service Unavailable"),
            (504, None, r#"{"error": {"code": "x", "message": "y"}}"#),
            (504, None, "{}"),
            (504, None, "Gateway Timeout"),
            (429, None, r#"{"error":"edge: busy"}"#),
            (429, Some("away"), r#"{"error":"edge: busy"}"#),
            (
                429,
                Some("busy"),
                r#"{"error": {"code": "x", "message": "y"}}"#,
            ),
            (429, Some("busy"), "{}"),
            (200, Some("away"), r#"{"error":"edge: the yard is away"}"#),
            (403, None, r#"{"error":"edge: the yard is away"}"#),
        ] {
            assert!(
                edge_call_refusal(status, refusal, body.as_bytes()).is_none(),
                "{status} {refusal:?} {body}"
            );
        }
    }

    #[test]
    fn signing_in_at_the_edge_answers_a_session_and_the_account() {
        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let address = Address::parse(edge.address()).unwrap();

        // One flow, then the password method: the session and the account
        // it signed in as. The flow's own action names another flow, and
        // is not followed: the submit answers the flow that was opened,
        // at the typed origin.
        let signed = sign_in(&address, KRATOS_EMAIL, KRATOS_PASSWORD).unwrap();
        assert!(is_token(&signed.session));
        assert_ne!(signed.session, SESSION);
        assert_eq!(signed.account, KRATOS_EMAIL);
        let submit = edge.last_request();
        assert!(
            submit.starts_with("POST /kratos/self-service/login?flow=test-flow-id HTTP/1.1\r\n"),
            "{submit}"
        );
        assert!(!submit.contains("another-flow"), "{submit}");
        // The minted session is the edge's from then on: every call with
        // it reaches the yard, and it opens no pairing's road by itself.
        assert_eq!(
            edge.client_with(TOKEN, &signed.session).me().unwrap().user,
            "jo"
        );
        assert!(!yard.pairing_open());
        assert!(yard.requests() > 0);
    }

    #[test]
    fn a_wrong_password_fails_the_sign_in_and_names_it() {
        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let address = Address::parse(edge.address()).unwrap();

        let Err(Error::Refused { code, message }) =
            sign_in(&address, KRATOS_EMAIL, "not-the-password")
        else {
            panic!("a wrong password signed in");
        };
        assert_eq!(code, "bad_sign_in");
        assert!(message.starts_with("The sign-in failed: "), "{message}");
        assert!(
            message.contains("provided credentials are invalid"),
            "{message}"
        );
        assert!(
            message.ends_with("Check the email and password, and sign in again."),
            "{message}"
        );
        // The password is in no error, and the refusal reached no yard.
        assert!(!message.contains("not-the-password"), "{message}");
        assert_eq!(edge.proxied(), 0);
        assert_eq!(yard.requests(), 0);
    }

    #[test]
    fn a_forgotten_flow_is_opened_once_more_then_said() {
        let yard = Yard::start_tcp();
        let edge = Router::start(&yard, SESSION);
        let address = Address::parse(edge.address()).unwrap();

        // Forgotten once: opened again, and the sign-in stands.
        edge.expire_logins(1);
        let signed = sign_in(&address, KRATOS_EMAIL, KRATOS_PASSWORD).unwrap();
        assert_eq!(signed.account, KRATOS_EMAIL);
        assert_eq!(
            edge.client_with(TOKEN, &signed.session).me().unwrap().user,
            "jo"
        );

        // Forgotten twice: said, not tried a third time, and the code for
        // it never leaves the client.
        edge.expire_logins(2);
        let Err(Error::Refused { code, message }) =
            sign_in(&address, KRATOS_EMAIL, KRATOS_PASSWORD)
        else {
            panic!("an expired sign-in stood");
        };
        assert_eq!(code, "bad_sign_in");
        assert_eq!(
            message,
            "The sign-in expired before it was sent. Sign in again."
        );
        assert_ne!(code, "expired_flow");
    }

    #[test]
    fn where_no_sign_in_answers_that_is_said() {
        let yard = Yard::start_tcp();
        let address = Address::parse(yard.address()).unwrap();

        // A yard with no edge has no such path: the sign-in names itself.
        let Err(Error::Refused { code, message }) =
            sign_in(&address, KRATOS_EMAIL, KRATOS_PASSWORD)
        else {
            panic!("no edge signed in");
        };
        assert_eq!(code, "no_sign_in");
        assert!(
            message.starts_with(&format!("No sign-in answers at {address}")),
            "{message}"
        );
        assert!(
            message.ends_with("Check the origin, and sign in again."),
            "{message}"
        );

        // And where nothing listens, the sign-in names itself too.
        yard.stop();
        let Err(Error::Refused { code, message }) =
            sign_in(&address, KRATOS_EMAIL, KRATOS_PASSWORD)
        else {
            panic!("nothing signed in");
        };
        assert_eq!(code, "no_sign_in");
        assert!(message.contains("nothing answers"), "{message}");
        assert!(!message.contains(KRATOS_PASSWORD), "{message}");
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

        // A timed out pairing is not tried again: it may already have
        // made the device. The timeout is still said without an OS error.
        yard.pair("WXYZ-2345");
        yard.stall("/user/pair");
        let request_before = yard.requests();
        let err = match pair_with(&address, "WXYZ-2345", None, "", Duration::from_millis(100)) {
            Err(err) => err,
            Ok(_) => panic!("the stalled pairing answered"),
        };
        assert!(
            matches!(
                &err,
                Error::Slow {
                    idempotent: false,
                    sent: true
                }
            ),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "the yard did not answer in time, and the action may or may not have happened"
        );
        assert_eq!(yard.requests(), request_before + 1);
        assert!(yard.pairing_open());

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

    /// A session at its harness's login screen says so on its login
    /// fields, which `Crew` serves only while true: what is present reads
    /// as served, and what is absent reads false, as on a yard from before
    /// it served them.
    #[test]
    fn crew_at_a_login_screen_carry_it_on_their_fields() {
        let yard = Yard::start();
        yard.answer(
            "Crew",
            r#"{"result": [
                {"name": "led", "enabled": true, "state": "running", "status": "idle",
                 "login_screen": true, "login_led": true},
                {"name": "screen", "enabled": true, "state": "running", "status": "idle",
                 "login_screen": true},
                {"name": "past", "enabled": true, "state": "running", "status": "idle",
                 "login_screen": false, "login_led": false},
                {"name": "plain", "enabled": true, "state": "running", "status": "working"},
                {"name": "bare", "enabled": true, "state": "running"}
            ]}"#,
        );
        let crew = yard.client().crew().unwrap();
        let logins: Vec<(&str, bool, bool)> = crew
            .iter()
            .map(|r| (r.name.as_str(), r.login_screen, r.login_led))
            .collect();
        assert_eq!(
            logins,
            [
                ("led", true, true),
                ("screen", true, false),
                ("past", false, false),
                ("plain", false, false),
                ("bare", false, false),
            ]
        );
    }

    #[test]
    fn arrivals_come_with_their_rows_in_arrival_order() {
        let yard = Yard::start();
        yard.answer(
            "Arrivals",
            r#"{"result": {"rows": [
                {"bead": "footplate-p512", "title": "The sign-in's password label",
                 "platform": "new", "line": "footplate", "expected": "—",
                 "remark": "held by jo", "tone": "red", "held_by": "jo"},
                {"bead": "yardr-bwm2", "title": "Hosted v0 trial",
                 "platform": "review", "line": "yardr", "expected": "20m–35m",
                 "remark": "", "tone": "none"},
                {"bead": "yd-links", "title": "Move the links",
                 "platform": "build", "line": "", "expected": "2h–3h",
                 "remark": "waits for decisions", "tone": "amber"}
            ], "platforms": ["new", "review", "build"]}}"#,
        );
        let board = yard.client().arrivals().unwrap();
        assert_eq!(yard.asked("Arrivals"), vec![json!({})]);
        let rows = &board.rows;
        assert_eq!(
            rows.iter().map(|row| row.bead.as_str()).collect::<Vec<_>>(),
            ["footplate-p512", "yardr-bwm2", "yd-links"]
        );
        assert_eq!(
            (
                rows[0].title.as_str(),
                rows[0].platform.as_str(),
                rows[0].line.as_str(),
                rows[0].expected.as_str(),
                rows[0].remark.as_str(),
                rows[0].tone.as_str(),
                rows[0].held_by.as_str()
            ),
            (
                "The sign-in's password label",
                "new",
                "footplate",
                "—",
                "held by jo",
                "red",
                "jo"
            )
        );
        assert_eq!(rows[1].remark.as_str(), "");
        assert_eq!(rows[1].tone.as_str(), "none");
        assert!(rows[1].held_by.is_empty());
        assert_eq!(rows[2].line.as_str(), "");
        assert_eq!(rows[2].remark.as_str(), "waits for decisions");
        assert_eq!(rows[2].tone.as_str(), "amber");
    }

    #[test]
    fn arrivals_from_a_yard_with_no_object_are_its_bare_rows() {
        let yard = Yard::start();
        // A yard not yet upgraded serves the bare list of rows.
        yard.answer(
            "Arrivals",
            r#"{"result": [
                {"bead": "yd-links", "title": "Move the links",
                 "platform": "new", "line": "", "expected": "2h–3h",
                 "remark": "", "tone": "none"}
            ]}"#,
        );
        let board = yard.client().arrivals().unwrap();
        assert_eq!(board.rows.len(), 1);
        assert_eq!(board.rows[0].bead.as_str(), "yd-links");
        // No answer at all is an empty board.
        yard.answer("Arrivals", r#"{"result": null}"#);
        assert_eq!(yard.client().arrivals().unwrap(), Arrivals::default());
    }

    #[test]
    fn flows_come_with_depots_flows_stages_beads_lamps_and_totals() {
        let yard = Yard::start();
        yard.answer(
            "Flows",
            r#"{"result": {"travelling": 4, "depots": [
                {"name": "yardr", "kind": "git", "travelling": 2, "flows": [
                    {"name": "default", "type": "", "source": "store", "stages": [
                        {"stage": "new", "group": "muse-builders", "human": false,
                         "lamp": "turn", "terminal": false, "limit": 3, "taken": 2,
                         "beads": [
                            {"bead": "yardr-bwm2", "title": "Hosted v0 trial",
                             "expected": "20m–35m", "remark": "waits for mckean",
                             "tone": "amber", "working": false}]},
                        {"stage": "review", "group": "mckean", "human": true,
                         "lamp": "none", "terminal": false, "limit": 1, "taken": 1,
                         "beads": []},
                        {"stage": "merged", "group": "", "human": false,
                         "lamp": "none", "terminal": true, "beads": []}
                    ]},
                    {"name": "design", "type": "design", "source": "repo@0f415ee",
                     "stages": []}
                ], "elsewhere": [
                    {"bead": "yardr-pinned", "title": "A pinned flow's bead",
                     "expected": "—", "remark": "", "tone": "none", "working": true,
                     "worker": "herdr yardr-pinned-new",
                     "group": "muse-builders", "kind": "muse",
                     "model": "muse-spark-1.3-contributor",
                     "entered_stage_at": "2026-10-08T09:30:00Z"}
                ]},
                {"name": "drafts", "kind": "dir", "travelling": 1, "flows": [],
                 "elsewhere": [
                    {"bead": "yd-greet", "title": "The greeting",
                     "expected": "—", "remark": "held", "tone": "red", "working": false}
                 ],
                 "error": ".yardr/flows/default.flow: flow line 1: want a stage"}
            ], "unassigned": [
                {"bead": "yd-shed", "title": "Paint the shed",
                 "expected": "2h–3h", "remark": "", "tone": "none", "working": false}
            ]}}"#,
        );
        let board = yard.client().flows().unwrap();
        assert_eq!(yard.asked("Flows"), vec![json!({})]);
        assert_eq!(board.travelling, 4);
        assert_eq!(board.depots.len(), 2);
        let depot = &board.depots[0];
        assert_eq!((depot.name.as_str(), depot.kind.as_str()), ("yardr", "git"));
        assert_eq!(depot.travelling, 2);
        assert_eq!(depot.flows.len(), 2);
        assert_eq!(depot.flows[0].name.as_str(), "default");
        assert_eq!(depot.flows[0].flow_type.as_str(), "");
        assert_eq!(depot.flows[0].source.as_str(), "store");
        assert_eq!(depot.flows[1].flow_type.as_str(), "design");
        let stages = &depot.flows[0].stages;
        assert_eq!(stages.len(), 3);
        assert_eq!(
            (
                stages[0].stage.as_str(),
                stages[0].group.as_str(),
                stages[0].human,
                stages[0].lamp.as_str()
            ),
            ("new", "muse-builders", false, "turn")
        );
        assert!(!stages[0].terminal);
        assert_eq!((stages[0].limit, stages[0].taken), (Some(3), Some(2)));
        assert_eq!(stages[0].beads.len(), 1);
        assert_eq!(stages[0].beads[0].bead.as_str(), "yardr-bwm2");
        assert_eq!(stages[0].beads[0].remark.as_str(), "waits for mckean");
        assert_eq!(stages[0].beads[0].tone.as_str(), "amber");
        assert!(!stages[0].beads[0].working);
        assert!(stages[0].beads[0].worker.is_empty());
        assert!(stages[0].beads[0].group.is_empty());
        assert!(stages[0].beads[0].kind.is_empty());
        assert!(stages[0].beads[0].model.is_empty());
        assert!(stages[0].beads[0].entered_stage_at.is_empty());
        assert!(stages[0].beads[0].held_by.is_empty());
        assert_eq!(stages[1].lamp.as_str(), "none");
        assert_eq!((stages[1].limit, stages[1].taken), (Some(1), Some(1)));
        assert!(stages[1].beads.is_empty());
        // The terminal stage closes its beads; with no route it serves
        // no seats, absent rather than zero.
        assert!(stages[2].terminal);
        assert_eq!((stages[2].limit, stages[2].taken), (None, None));
        assert_eq!(depot.elsewhere.len(), 1);
        assert_eq!(depot.elsewhere[0].bead.as_str(), "yardr-pinned");
        assert!(depot.elsewhere[0].working);
        assert_eq!(depot.elsewhere[0].worker.as_str(), "herdr yardr-pinned-new");
        assert_eq!(depot.elsewhere[0].group.as_str(), "muse-builders");
        assert_eq!(depot.elsewhere[0].kind.as_str(), "muse");
        assert_eq!(
            depot.elsewhere[0].model.as_str(),
            "muse-spark-1.3-contributor"
        );
        assert_eq!(
            depot.elsewhere[0].entered_stage_at.as_str(),
            "2026-10-08T09:30:00Z"
        );
        assert!(depot.error.is_empty());
        let drafts = &board.depots[1];
        assert!(drafts.flows.is_empty());
        assert_eq!(drafts.elsewhere.len(), 1);
        // A hold from before the yard kept the holder reads `held`
        // with no holder: absent is unknown.
        assert_eq!(drafts.elsewhere[0].remark.as_str(), "held");
        assert!(drafts.elsewhere[0].held_by.is_empty());
        assert!(!drafts.error.is_empty());
        assert_eq!(board.unassigned.len(), 1);
        assert_eq!(board.unassigned[0].bead.as_str(), "yd-shed");
        // No answer at all is an empty view.
        yard.answer("Flows", r#"{"result": null}"#);
        assert_eq!(yard.client().flows().unwrap(), Flows::default());
    }

    #[test]
    fn source_grants_come_with_each_call_s_own_grant() {
        let yard = Yard::start();
        yard.answer(
            "SourceGrants",
            r#"{"result": [
                {"source": "crm", "crew": "yardmaster", "as": "jo",
                 "calls": ["read"], "at": "2026-10-08T09:30:00Z", "by": "jo",
                 "given": {"read": {"at": "2026-10-08T09:30:00Z", "by": "jo"}}},
                {"source": "crm", "group": "builders",
                 "calls": ["put", "read"], "at": "2026-10-08T09:30:00Z", "by": "jo",
                 "given": {
                    "put": {"at": "2026-10-08T09:30:00Z", "by": "jo",
                            "bead": "yd-crm", "ask": 57, "asker": "yardmaster"},
                    "read": {"at": "2026-10-08T09:30:00Z", "by": "jo"}}},
                {"source": "mail", "group": "builders",
                 "calls": ["read"], "at": "2026-10-12T10:15:00Z", "by": "mckean",
                 "kinds": null},
                {"source": "crm", "crew": "yardmaster",
                 "calls": ["customer", "note", "old"],
                 "at": "2026-10-12T10:15:00Z", "by": "mckean",
                 "kinds": {"customer": "read", "note": "update"}}
            ]}"#,
        );
        let grants = yard.client().source_grants().unwrap();
        assert_eq!(yard.asked("SourceGrants"), vec![json!({})]);
        assert_eq!(grants.len(), 4);
        assert_eq!(grants[0].source.as_str(), "crm");
        assert_eq!(grants[0].crew.as_str(), "yardmaster");
        assert!(grants[0].group.is_empty());
        assert_eq!(grants[0].as_user.as_str(), "jo");
        assert_eq!(grants[0].calls, ["read"]);
        assert_eq!(grants[0].given["read"].by.as_str(), "jo");
        assert!(grants[0].given["read"].bead.is_empty());
        assert_eq!(grants[1].group.as_str(), "builders");
        assert!(grants[1].crew.is_empty());
        assert!(grants[1].as_user.is_empty());
        assert_eq!(grants[1].calls, ["put", "read"]);
        let put = &grants[1].given["put"];
        assert_eq!(
            (
                put.by.as_str(),
                put.bead.as_str(),
                put.ask,
                put.asker.as_str()
            ),
            ("jo", "yd-crm", 57, "yardmaster")
        );
        // A record with no given has none: its calls were granted before
        // their history was kept.
        assert!(grants[2].given.is_empty());
        assert_eq!(grants[2].at.as_str(), "2026-10-12T10:15:00Z");
        // The kinds the yard served, one to each call it could read; a call
        // with none is left out, and a record from a yard before it served
        // kinds has no `kinds`, nor has one with a null.
        assert_eq!(grants[3].calls, ["customer", "note", "old"]);
        assert_eq!(grants[3].kinds["customer"], "read");
        assert_eq!(grants[3].kinds["note"], "update");
        assert!(!grants[3].kinds.contains_key("old"));
        assert!(grants[..3].iter().all(|grant| grant.kinds.is_empty()));
        // No answer at all is none.
        yard.answer("SourceGrants", r#"{"result": null}"#);
        assert!(yard.client().source_grants().unwrap().is_empty());
    }

    #[test]
    fn session_screen_asks_by_session_and_answers_the_styled_screen() {
        let yard = Yard::start();
        yard.answer(
            "SessionScreen",
            r#"{"result": {"screen": "\u001b[1mYardr yardmaster\u001b[0m\n❯ "}}"#,
        );
        let screen = yard.client().session_screen("yardmaster").unwrap();
        assert_eq!(
            yard.asked("SessionScreen"),
            vec![json!({"session": "yardmaster"})]
        );
        assert_eq!(screen.screen, "\x1b[1mYardr yardmaster\x1b[0m\n❯ ");
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
                 "since": "2026-10-08T09:30:00Z",
                 "identity": {"source": "talk", "sender": "U999"},
                 "weight": [{"class": "outside", "line": "lets the yard change something outside it: crew member yardmaster may call send of google-mail (an update)"}]},
                {"bead": "yd-greet", "bead_title": "The greeting", "topic": "",
                 "ask": 5, "question": "What tone?", "asker": "yardmaster",
                 "since": "2026-10-09T09:30:00Z"}
            ], "decided": [
                {"bead": "yd-links", "bead_title": "Move the links", "topic": "topic:linear-out",
                 "ask": 6, "question": "Send them to build?", "asker": "yardmaster",
                 "since": "2026-10-07T09:30:00Z", "state": "answered",
                 "option": 1, "option_label": "Send them", "words": "tonight",
                 "weight": [{"class": "trust", "line": "widens who the yard listens to: user kim"}]},
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
        // The card's weight, one entry a class; absent on a routine card.
        assert_eq!(asks.open[0].weight.len(), 1);
        assert_eq!(asks.open[0].weight[0].class, "outside");
        assert!(
            asks.open[0].weight[0]
                .line
                .starts_with("lets the yard change")
        );
        assert!(asks.open[1].weight.is_empty());
        assert_eq!(asks.decided[0].weight.len(), 1);
        assert_eq!(asks.decided[0].weight[0].class, "trust");
        assert!(asks.decided[1].weight.is_empty());
        // What a mapping ask maps; absent on any other ask.
        let identity = asks.open[0].identity.as_ref().unwrap();
        assert_eq!(
            (identity.source.as_str(), identity.sender.as_str()),
            ("talk", "U999")
        );
        assert!(asks.open[1].identity.is_none());
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
                     "text": "the test passed on a copy", "at": "2026-10-08T09:30:00Z",
                     "files": [{"hash": "aa", "type": "image/png", "bytes": 70}]},
                    {"seq": 3, "kind": "note", "author": "yardmaster",
                     "text": "shipped", "at": "2026-10-08T09:31:00Z", "files": null}
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
                     "state": "open",
                     "identity": {"source": "talk", "sender": "U999", "channel": "general"},
                     "weight": [{"class": "as-user", "line": "lets calls or words count as a person's: as ada"}]},
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
        assert_eq!(bead.notes.len(), 2);
        assert_eq!(bead.notes[0].author, "yardmaster");
        assert_eq!(
            bead.notes[0].files,
            [BlobRef {
                hash: "aa".into(),
                kind: "image/png".into(),
                bytes: 70,
                ..BlobRef::default()
            }]
        );
        // `null` where a note's images are promised is none.
        assert_eq!(bead.notes[1].files, []);
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
        // The card's weight, as the queue's own.
        assert_eq!(ask.weight.len(), 1);
        assert_eq!(ask.weight[0].class, "as-user");
        assert!(bead.asks[1].weight.is_empty());
        // What a mapping ask maps; absent on any other ask.
        let identity = ask.identity.as_ref().unwrap();
        assert_eq!(
            (
                identity.source.as_str(),
                identity.sender.as_str(),
                identity.channel.as_str()
            ),
            ("talk", "U999", "general")
        );
        assert!(bead.asks[1].identity.is_none());
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
            .answer_ask("yd-links", 3, 1, "move them tonight", "jo", false)
            .unwrap();
        // A pick alone sends no words.
        client
            .answer_ask("yd-links", 3, 2, "", "jo", false)
            .unwrap();
        // Words answer an ask with no options.
        client
            .answer_ask("yd-greet", 5, 0, "warmly", "jo", false)
            .unwrap();
        // A mapping pick sends weighed, the client's word its person saw
        // the weight and passed the gate; a yard from before takes the
        // call without it as it always did.
        client.answer_ask("yd-links", 3, 1, "", "jo", true).unwrap();
        assert_eq!(
            yard.asked("AnswerAsk"),
            vec![
                json!({"bead": "yd-links", "ask": 3, "option": 1, "words": "move them tonight", "actor": "jo"}),
                json!({"bead": "yd-links", "ask": 3, "option": 2, "actor": "jo"}),
                json!({"bead": "yd-greet", "ask": 5, "option": 0, "words": "warmly", "actor": "jo"}),
                json!({"bead": "yd-links", "ask": 3, "option": 1, "actor": "jo", "weighed": true}),
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
            .add_note("yd-greet", "the newcomer arrives Monday", "jo", &[])
            .unwrap();
        assert_eq!(
            yard.asked("AddNote"),
            vec![json!({"bead": "yd-greet", "text": "the newcomer arrives Monday", "actor": "jo"})]
        );
        // A note with files names each hash with its name; without, no
        // argument goes, and a yard from before takes the call as it
        // always did.
        client
            .add_note(
                "yd-greet",
                "the mockup, third pass",
                "jo",
                &[
                    BlobFile {
                        hash: "aa".into(),
                        name: "mockup.html".into(),
                    },
                    BlobFile {
                        hash: "bb".into(),
                        name: "fence.png".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            yard.asked("AddNote").last(),
            Some(
                &json!({"bead": "yd-greet", "text": "the mockup, third pass", "actor": "jo",
                    "files": [{"hash": "aa", "name": "mockup.html"}, {"hash": "bb", "name": "fence.png"}]})
            )
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

    #[test]
    fn a_reaction_is_put_and_taken_away() {
        let yard = Yard::start();
        yard.answer("React", "{}");
        let client = yard.client();
        client.react(13, "🎉", "jo").unwrap();
        client.react(13, "", "jo").unwrap();
        assert_eq!(
            yard.asked("React"),
            vec![
                json!({"seq": 13, "emoji": "🎉", "actor": "jo"}),
                json!({"seq": 13, "emoji": "", "actor": "jo"}),
            ]
        );

        // What the yard refuses is said in its words.
        yard.answer(
            "React",
            r#"{"error": {"code": "error", "message": "emoji 🍌 is none of 👍 👎 ❤️ 🎉 👀 ✅"}}"#,
        );
        let err = client.react(13, "🍌", "jo").unwrap_err();
        assert_eq!(err.to_string(), "emoji 🍌 is none of 👍 👎 ❤️ 🎉 👀 ✅");

        // A yard from before reactions has no such call.
        let old = Yard::start();
        let err = old.client().react(13, "👍", "jo").unwrap_err();
        assert_eq!(err.to_string(), "no such method React");
    }

    #[test]
    fn a_letter_carries_its_parties_reactions_or_none() {
        let yard = Yard::start();
        yard.answer(
            "Conversation",
            r#"{"result": [
                {"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo",
                 "text": "Shall I land it?", "read": false,
                 "reactions": [{"from": "jo", "emoji": "👍"}]},
                {"seq": 13, "at": "2026-10-08T09:31:00Z", "from": "jo", "to": "yardmaster",
                 "text": "Land it.", "read": true}]}"#,
        );
        let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
        assert_eq!(
            letters[0].reactions,
            vec![Reaction {
                from: "jo".into(),
                emoji: "👍".into()
            }]
        );
        // A yard from before reactions sends no such field, and the mail
        // reads as it always has.
        assert_eq!(letters[1].reactions, vec![]);
    }

    /// The PNG of docs/api.md: one pixel, 70 bytes.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    const HASH: &str = "497790947d4666760ce38f3c00e852c71fdb66cae849bae8e9ede352719e1581";

    #[test]
    fn the_cap_on_a_file_is_the_yard_s_or_8_mib() {
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
    fn the_count_on_a_letter_is_the_yard_s_or_64() {
        let yard = Yard::start();
        yard.answer(
            "Settings",
            r#"{"result": {"files_max": "32", "operator": "jo"}}"#,
        );
        assert_eq!(yard.client().settings().unwrap().files_cap(), 32);
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        assert_eq!(yard.client().settings().unwrap().files_cap(), 64);
        yard.answer("Settings", r#"{"result": {"files_max": "many"}}"#);
        assert_eq!(yard.client().settings().unwrap().files_cap(), 64);
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
                name: String::new(),
                not_taken: String::new()
            }
        );
        yard.client()
            .mail(
                "yardmaster",
                "",
                "jo",
                &[BlobFile {
                    hash: blob.hash,
                    name: "fence.png".into(),
                }],
            )
            .unwrap();
        assert_eq!(
            yard.asked("Mail"),
            vec![json!({"to": "yardmaster", "text": "", "actor": "jo",
                "files": [{"hash": HASH, "name": "fence.png"}]})]
        );

        yard.answer(
            "PutBlob",
            r#"{"error": {"code": "error", "message": "the file is 9437184 bytes, over the yard's limit of 8388608 (yardr settings set blob_max_bytes <bytes>)"}}"#,
        );
        let err = yard.client().put_blob(&[7u8; 9 * 1024 * 1024]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the file is 9437184 bytes, over the yard's limit of 8388608 (yardr settings set blob_max_bytes <bytes>)"
        );
    }

    #[cfg(unix)]
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
                 "files": [{"hash": HASH, "type": "image/png", "bytes": 70, "name": "fence.png"}]},
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
            name: "fence.png".into(),
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
                     "files": [{"hash": HASH, "type": "image/png", "bytes": 70, "name": "fence.png"}]}}))
        .unwrap();
        assert_eq!(event.files(), vec![png]);
        let plain: Event =
            serde_json::from_value(json!({"seq": 13, "kind": "mail", "data": {"files": "none"}}))
                .unwrap();
        assert_eq!(plain.files(), vec![]);
        // A hook delivery's own files are the source's refs, with no hash:
        // read as a mail's they would be images of nothing the yard holds.
        let hook: Event = serde_json::from_value(json!({"seq": 139, "kind": "hook",
            "actor": "hook:telegram",
            "data": {"source": "telegram", "files": [{"id": "AgAD", "name": "photo.jpg",
                                                      "type": "image/jpeg", "size": 51200}]}}))
        .unwrap();
        assert_eq!(hook.files(), vec![]);
    }

    /// The files of a hook delivery are asked by the delivery's seq, by the
    /// socket and at an address alike: each as the yard holds it or with
    /// the reason it did not take it, and none while it has fetched none.
    #[test]
    fn a_delivery_s_files_are_asked_by_the_socket_and_at_an_address() {
        for yard in [Yard::start(), Yard::start_tcp()] {
            let client = yard.client();
            let way = client.place();
            // A yard from before the call says so, as of any method.
            let err = client.delivery_files(139).unwrap_err();
            assert_eq!(err.to_string(), "no such method DeliveryFiles", "{way}");
            // One that has it answers in the order the source named the
            // files, as docs/api.md shows.
            yard.answer(
                "DeliveryFiles",
                &json!({"result": [
                    {"hash": HASH, "type": "image/png", "bytes": 70},
                    {"hash": "", "type": "", "bytes": 1048576, "name": "shed-plans.pdf",
                     "not_taken": "document of 1048576 bytes, over the yard's limit of 8192"}
                ]})
                .to_string(),
            );
            let files = client.delivery_files(139).unwrap();
            assert_eq!(
                yard.asked("DeliveryFiles").last(),
                Some(&json!({"seq": 139}))
            );
            let under = if client.is_remote() { "/user" } else { "" };
            let request = yard.last_request();
            assert!(
                request.starts_with(&format!("POST {under}/call/DeliveryFiles HTTP/1.1\r\n")),
                "{way}: {request}"
            );
            assert_eq!(
                files,
                vec![
                    BlobRef {
                        hash: HASH.into(),
                        kind: "image/png".into(),
                        bytes: 70,
                        name: String::new(),
                        not_taken: String::new(),
                    },
                    BlobRef {
                        hash: String::new(),
                        kind: String::new(),
                        bytes: 1048576,
                        name: "shed-plans.pdf".into(),
                        not_taken: "document of 1048576 bytes, over the yard's limit of 8192"
                            .into(),
                    },
                ],
                "{way}"
            );
            // A delivery whose files are not fetched answers null, and a
            // list with nothing in it reads the same.
            yard.answer("DeliveryFiles", r#"{"result": null}"#);
            assert_eq!(client.delivery_files(140).unwrap(), vec![], "{way}");
            yard.answer("DeliveryFiles", r#"{"result": []}"#);
            assert_eq!(client.delivery_files(140).unwrap(), vec![], "{way}");
        }
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

        // With files the mail names each hash with its name, and needs no
        // text.
        yard.client()
            .mail_peer(
                "clerk",
                "north",
                "",
                "jo",
                &[BlobFile {
                    hash: HASH.to_string(),
                    name: "ledger.csv".into(),
                }],
            )
            .unwrap();
        assert_eq!(
            yard.asked("MailPeer")[1],
            json!({"to": "clerk", "peer": "north", "text": "", "actor": "jo",
                "files": [{"hash": HASH, "name": "ledger.csv"}]})
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

    /// A yard that holds the conversation with the yardmaster, the mails
    /// 1 to `n`.
    fn long_conversation(n: u64) -> Yard {
        let yard = Yard::start();
        yard.converse(
            "yardmaster",
            (1..=n)
                .map(|seq| json!({"seq": seq, "from": "yardmaster", "to": "jo", "text": "x"}))
                .collect(),
        );
        yard
    }

    #[test]
    fn a_conversation_is_read_from_its_newest_end_a_page_at_a_time() {
        let yard = long_conversation(120);
        let client = yard.client();
        let newest = client.conversation_page("jo", "yardmaster", 50, 0).unwrap();
        assert_eq!(
            (newest.letters.len(), newest.letters[0].seq, newest.more),
            (50, 71, true)
        );
        let before = client
            .conversation_page("jo", "yardmaster", 50, 71)
            .unwrap();
        assert_eq!(
            (before.letters[0].seq, before.letters[49].seq, before.more),
            (21, 70, true)
        );
        let oldest = client
            .conversation_page("jo", "yardmaster", 50, 21)
            .unwrap();
        assert_eq!((oldest.letters.len(), oldest.more), (20, false));
        assert_eq!(
            yard.asked("Conversation"),
            vec![
                json!({"a": "jo", "b": "yardmaster", "limit": 50}),
                json!({"a": "jo", "b": "yardmaster", "limit": 50, "before": 71}),
                json!({"a": "jo", "b": "yardmaster", "limit": 50, "before": 21}),
            ]
        );
        // What is new is read after a seq, as ever.
        assert_eq!(
            client.conversation("jo", "yardmaster", 118).unwrap().len(),
            2
        );
    }

    #[test]
    fn a_yard_that_refuses_pages_is_asked_for_the_conversation_whole() {
        let yard = long_conversation(120);
        yard.old();
        let page = yard
            .client()
            .conversation_page("jo", "yardmaster", 50, 0)
            .unwrap();
        assert_eq!((page.letters.len(), page.more), (120, false));
        assert_eq!(
            yard.asked("Conversation"),
            vec![
                json!({"a": "jo", "b": "yardmaster", "limit": 50}),
                json!({"a": "jo", "b": "yardmaster", "after": 0}),
            ]
        );
        // What it refuses whole as well is the refusal that is told.
        yard.answer(
            "Conversation",
            r#"{"error": {"code": "error", "message": "user nobody: not found"}}"#,
        );
        let err = yard
            .client()
            .conversation_page("jo", "nobody", 50, 0)
            .unwrap_err();
        assert!(err.to_string().contains("user nobody"), "{err}");
        assert_eq!(yard.asked("Conversation").len(), 4);
    }

    #[test]
    fn a_yard_that_answers_all_to_a_page_has_no_more() {
        let yard = Yard::start();
        let letters: Vec<Value> = (1..=120)
            .map(|seq| json!({"seq": seq, "from": "yardmaster", "to": "jo", "text": "x"}))
            .collect();
        yard.answer("Conversation", &json!({ "result": letters }).to_string());
        let page = yard
            .client()
            .conversation_page("jo", "yardmaster", 50, 0)
            .unwrap();
        assert_eq!((page.letters.len(), page.more), (120, false));
        assert_eq!(yard.asked("Conversation").len(), 1);
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

    #[cfg(unix)]
    #[test]
    fn no_yard_is_an_error_and_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        // The socket is missing.
        let missing = Client::new(dir.path().join("yard.sock"));
        assert!(matches!(missing.head(), Err(Error::Down(_))));
        assert!(matches!(missing.events(0, QUIET), Err(Error::Down(_))));
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
        let said = |status: u16| {
            format!(
                "call failed: request=none method=POST path=/call/Head outcome=unreadable status={status} timeout=30s"
            )
        };
        yard.answer("Head", "not JSON");
        assert!(matches!(yard.client().head(), Err(Error::Protocol(_))));
        assert_eq!(take_said(), [said(200)]);
        // A reply that came whole and is not the shape asked for is no
        // failed exchange: the call leaves no line.
        yard.answer("Head", r#"{"result": "twelve"}"#);
        assert!(matches!(yard.client().head(), Err(Error::Protocol(_))));
        assert_eq!(take_said(), Vec::<String>::new());
        yard.answer_status("Head", 500, "{}");
        assert!(matches!(yard.client().head(), Err(Error::Protocol(_))));
        assert_eq!(take_said(), [said(500)]);
    }

    /// As [`answering`], but each connection is held open after `reply`, as
    /// one whose rest is slow to come.
    fn holding(reply: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let finished = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut asked = [0u8; 1024];
            let _ = conn.read(&mut asked);
            let _ = conn.write_all(&reply);
            // The client closes after its timeout; waiting for that is both
            // the fixture's release and the end of this one-request server.
            let mut rest = [0u8; 1];
            while conn.read(&mut rest).unwrap_or(0) != 0 {}
        });
        (address, finished)
    }

    /// A call that fails leaves one line, with the edge's id where the head
    /// of the answer carried one, and `none` where it did not (a yard with
    /// no edge in front, an edge from before the header, a value that is no
    /// id): refused by the yard or by the edge, cut off, slow after its
    /// head, answered by the edge's own gateway error, and not answered at
    /// all. One that answers leaves none. Each line is kept in the log as
    /// well, led by the time, and a log at its cap is moved aside for a new
    /// one ([`log`]).
    #[test]
    fn a_failed_call_leaves_one_line_with_the_edge_s_request_id() {
        use chrono::SubsecRound as _;
        let dir = tempfile::tempdir().unwrap();
        let file = keep_in(dir.path());
        let started = chrono::Local::now().trunc_subsecs(3).fixed_offset();
        let mut said_all = Vec::new();
        let answer = |status: &str, headers: &str, body: &str| {
            format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_bytes()
        };
        // Says more than it sends, as an answer cut off or slow in its body.
        let short = b"HTTP/1.1 200 OK\r\nX-Edge-Request: r-5\r\nContent-Length: 50\r\n\r\nshort";
        let refusal = r#"{"error": {"code": "no-such", "message": "no"}}"#;
        let long = format!("X-Edge-Request: r-{}\r\n", "0".repeat(EDGE_REQUEST_MOST));
        // What the listener sends, whether it holds the connection after,
        // and the id, outcome and status the line says.
        let cases = [
            (
                "the yard's refusal",
                answer("200 OK", "X-Edge-Request: r-1\r\n", refusal),
                false,
                "r-1",
                "refused",
                "200",
            ),
            (
                "the device out",
                answer(
                    "401 Unauthorized",
                    "WWW-Authenticate: Bearer\r\nX-Edge-Request: r-2\r\n",
                    "Unauthorized\n",
                ),
                false,
                "r-2",
                "refused",
                "401",
            ),
            (
                "turned away by a gateway, neither header",
                answer(
                    "401 Unauthorized",
                    "X-Edge-Request: r-9\r\n",
                    "the gateway's token is missing: the X-Edge-JWT header is not on the request\n",
                ),
                false,
                "r-9",
                "refused",
                "401",
            ),
            (
                "the session out, at the edge",
                answer(
                    "401 Unauthorized",
                    "X-Edge-Refusal: session\r\nX-Edge-Request: r-3\r\n",
                    "",
                ),
                false,
                "r-3",
                "refused",
                "401",
            ),
            (
                "the edge's own gateway error",
                answer("504 Gateway Timeout", "X-Edge-Request: r-4\r\n", "too slow"),
                false,
                "r-4",
                "unreadable",
                "504",
            ),
            (
                "cut off in its body",
                short.to_vec(),
                false,
                "r-5",
                "down",
                "200",
            ),
            (
                "slow in its body",
                short.to_vec(),
                true,
                "r-5",
                "timeout",
                "200",
            ),
            (
                "no id: an edge from before the header",
                answer("401 Unauthorized", "", ""),
                false,
                "none",
                "refused",
                "401",
            ),
            (
                "no id: two words",
                answer("401 Unauthorized", "X-Edge-Request: r 6\r\n", ""),
                false,
                "none",
                "refused",
                "401",
            ),
            (
                "no id: too long",
                answer("401 Unauthorized", &long, ""),
                false,
                "none",
                "refused",
                "401",
            ),
            (
                "hung up before any answer",
                Vec::new(),
                false,
                "none",
                "down",
                "none",
            ),
            (
                "no answer in time",
                Vec::new(),
                true,
                "none",
                "timeout",
                "none",
            ),
        ];
        // Only the held ones are meant to run out of their wait: the rest
        // are given so long a one that a busy machine never decides them.
        let (brief, ample) = (Duration::from_millis(100), Duration::from_secs(5));
        for (name, reply, held, id, outcome, status) in cases {
            let (address, wait, finished) = match held {
                true => {
                    let (address, finished) = holding(reply);
                    (address, brief, Some(finished))
                }
                false => (answering(reply), ample, None),
            };
            let client = Client::remote(Address::parse(&address).unwrap(), TOKEN);
            let Err(err) = client.call_once("Head", &json!({}), wait) else {
                panic!("{name}: the call answered");
            };
            if let Some(finished) = finished {
                finished.join().unwrap();
            }
            assert_eq!(err.outcome(), outcome, "{name}: {err:?}");
            let said = take_said();
            assert_eq!(
                said,
                [format!(
                    "call failed: request={id} method=POST path=/call/Head outcome={outcome} status={status} timeout={wait:?}"
                )],
                "{name}"
            );
            said_all.extend(said);
        }

        let address = answering(answer(
            "200 OK",
            "X-Edge-Request: r-8\r\n",
            r#"{"result": 4}"#,
        ));
        let client = Client::remote(Address::parse(&address).unwrap(), TOKEN);
        assert!(client.call_once("Head", &json!({}), ample).is_ok());
        assert_eq!(take_said(), Vec::<String>::new());

        // The log has each line said, in order, led by the time it was
        // said, and nothing of the call that answered.
        let kept = std::fs::read_to_string(&file).unwrap();
        let mut kept_lines = Vec::new();
        for entry in kept.lines() {
            let (at, line) = entry.split_once(' ').unwrap();
            let at = chrono::DateTime::parse_from_rfc3339(at).unwrap();
            assert!(started <= at && at <= chrono::Local::now(), "{entry}");
            kept_lines.push(line.to_string());
        }
        assert_eq!(kept_lines, said_all);

        // A log at its cap is moved aside whole when a line comes, and the
        // line begins another: the history is there, and the log is not
        // left to grow.
        let full = "a".repeat(usize::try_from(log::CAP).unwrap());
        std::fs::write(&file, &full).unwrap();
        let address = answering(Vec::new());
        let client = Client::remote(Address::parse(&address).unwrap(), TOKEN);
        assert!(client.call_once("Head", &json!({}), ample).is_err());
        let said = take_said();
        assert_eq!(said.len(), 1, "{said:?}");
        let kept = std::fs::read_to_string(&file).unwrap();
        assert_eq!(kept.lines().count(), 1, "{kept}");
        assert!(kept.ends_with(&format!(" {}\n", said[0])), "{kept}");
        assert_eq!(
            std::fs::read_to_string(file.with_extension("log.1")).unwrap(),
            full
        );
    }

    /// A 401 is told by its head, never by its words. Only the yard's own
    /// challenge, `WWW-Authenticate: Bearer`, says the device is out, and
    /// the edge's header says the session is. A 401 with neither, as the
    /// yard's web view answers a stream the edge sent it by mistake
    /// (yardr-hosted-c0sb), says nothing of the device: it is not said in
    /// the words of a device that is out.
    #[test]
    fn a_401_says_the_device_is_out_only_with_the_yard_s_challenge() {
        let gateway =
            "the gateway's token is missing: the X-Edge-JWT header is not on the request\n";
        for (name, headers, body, want) in [
            (
                "the yard's own",
                "WWW-Authenticate: Bearer\r\n",
                "Unauthorized\n",
                "device",
            ),
            (
                "in any case, with parameters",
                "www-authenticate: bearer realm=\"yardr\"\r\n",
                "",
                "device",
            ),
            (
                "among several challenges",
                "WWW-Authenticate: Basic realm=\"x\", Bearer\r\n",
                "",
                "device",
            ),
            (
                "in a header of its own",
                "WWW-Authenticate: Basic realm=\"x\"\r\nWWW-Authenticate: Bearer\r\n",
                "",
                "device",
            ),
            (
                "another scheme",
                "WWW-Authenticate: Basic realm=\"Bearer\"\r\n",
                "",
                "gateway",
            ),
            ("the web gateway's, neither header", "", gateway, "gateway"),
            (
                "the edge's, whatever else the head offers",
                "X-Edge-Refusal: session\r\nWWW-Authenticate: Bearer\r\n",
                "",
                "session",
            ),
        ] {
            let reply = format!(
                "HTTP/1.1 401 Unauthorized\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_bytes();
            let client = Client::remote(Address::parse(&answering(reply)).unwrap(), TOKEN);
            let err = client.head().unwrap_err();
            let (kind, said) = match &err {
                Error::NotLetIn => ("device", "this device is no longer let in"),
                Error::NotSignedIn => ("session", "this session is no longer signed in"),
                Error::Gateway => (
                    "gateway",
                    "a gateway turned the request away (401), which does not mean this device is out",
                ),
                other => panic!("{name}: {other:?}"),
            };
            assert_eq!((kind, err.to_string().as_str()), (want, said), "{name}");
            // The words of a device that is out are said of that alone.
            assert_eq!(
                err.to_string().contains("this device is no longer let in"),
                want == "device",
                "{name}"
            );
        }
        take_said();
    }

    /// The id is one short token, so the line stays one line of words; what
    /// is not known stands as `none`, and every outcome has its word.
    #[test]
    fn the_edge_s_id_is_one_short_token_and_the_line_says_what_it_does_not_know() {
        assert_eq!(edge_request("r-0123abcd").as_deref(), Some("r-0123abcd"));
        for no in ["", "r 1", "r\t1", "r-é", &"x".repeat(EDGE_REQUEST_MOST + 1)] {
            assert_eq!(edge_request(no), None, "{no:?}");
        }
        let sent = Sent {
            verb: "GET",
            path: "/me",
            limit: None,
            request: None,
            status: None,
        };
        assert_eq!(
            sent.line(&Error::NoDevice),
            "call failed: request=none method=GET path=/me outcome=refused status=none timeout=none"
        );
        for (err, outcome) in [
            (Error::Down(io::ErrorKind::ConnectionReset.into()), "down"),
            (Error::Down(io::ErrorKind::TimedOut.into()), "timeout"),
            (Error::Down(io::ErrorKind::WouldBlock.into()), "timeout"),
            (
                Error::Slow {
                    idempotent: true,
                    sent: true,
                },
                "timeout",
            ),
            (
                Error::Slow {
                    idempotent: false,
                    sent: false,
                },
                "timeout",
            ),
            (Error::Silent, "timeout"),
            (Error::NotLetIn, "refused"),
            (Error::NotSignedIn, "refused"),
            (Error::Gateway, "refused"),
            (Error::Protocol("no".into()), "unreadable"),
        ] {
            assert_eq!(err.outcome(), outcome, "{err:?}");
        }
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

    #[cfg(unix)]
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
        let mut events = yard.client().events(11, QUIET).unwrap();

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
        let mut events = yard.client().events(3, QUIET).unwrap();
        assert_eq!(events.next().unwrap().unwrap().seq, 4);
    }

    /// How long a test's stream is read with nothing said: long enough
    /// that the tests of what it says never see it run out.
    const QUIET: Duration = Duration::from_secs(60);

    /// A stream at a port of this machine: the answer's head, then each
    /// part after its pause, written as it is. It is held open a while
    /// after the last part, as a stream that says nothing more is.
    fn streaming(parts: Vec<(Duration, Vec<u8>)>) -> Client {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut asked = [0u8; 1024];
            let _ = conn.read(&mut asked);
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\n\r\n",
            );
            for (pause, part) in parts {
                thread::sleep(pause);
                let _ = conn.write_all(&part);
            }
            thread::sleep(Duration::from_secs(2));
        });
        Client::remote(Address::parse(&address).unwrap(), TOKEN)
    }

    /// An event as a chunk of the stream: its size line, the line, and
    /// the line break that ends the chunk.
    fn chunk(event: &str) -> Vec<u8> {
        let line = format!("{event}\n");
        format!("{:x}\r\n{line}\r\n", line.len()).into_bytes()
    }

    /// A stream that says nothing for as long as it is read is said to be
    /// silent between two events, and read on: what comes next is read
    /// whole, even where the silence fell inside a chunk's size line.
    #[test]
    fn a_silent_stream_is_said_so_between_two_events_and_read_on() {
        let first = r#"{"seq": 21, "kind": "mail"}"#;
        let second = chunk(r#"{"seq": 22, "kind": "noted", "bead": "yd-shed"}"#);
        let pause = Duration::from_millis(400);
        let client = streaming(vec![
            (Duration::ZERO, chunk(first)),
            (pause, second[..1].to_vec()),
            (pause, second[1..].to_vec()),
        ]);
        let mut events = client.events(20, Duration::from_millis(100)).unwrap();
        assert_eq!(events.next().unwrap().unwrap().seq, 21);
        let mut silences = 0;
        let event = loop {
            match events.next().unwrap() {
                Err(Error::Silent) => silences += 1,
                other => break other.unwrap(),
            }
        };
        assert!(silences >= 2, "{silences}");
        assert_eq!((event.seq, event.bead.as_str()), (22, "yd-shed"));
    }

    /// A stream that times out in the middle of an event has ended: what
    /// of the event came cannot be read on from, and the timeout is said
    /// plainly.
    #[test]
    fn a_stream_that_stops_in_the_middle_of_an_event_has_ended() {
        let whole = chunk(r#"{"seq": 21, "kind": "mail"}"#);
        let client = streaming(vec![(Duration::ZERO, whole[..12].to_vec())]);
        let mut events = client.events(20, Duration::from_millis(100)).unwrap();
        let Err(err) = events.next().unwrap() else {
            panic!("the partial event was complete");
        };
        assert!(
            matches!(
                &err,
                Error::Slow {
                    idempotent: true,
                    sent: true
                }
            ),
            "{err:?}"
        );
        assert_eq!(err.to_string(), "the yard is slow to answer; trying again");
    }

    /// A stream whose answer does not come is asked once more, then said
    /// to be slow in plain words; the wait stays bounded, as the follower's
    /// stop could not reach it before.
    #[test]
    fn a_stream_that_does_not_answer_is_not_waited_for_without_end() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let held = thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(2));
            drop(conn);
        });
        let client = Client::remote(Address::parse(&address).unwrap(), TOKEN);
        let asked = Instant::now();
        let Err(err) = client.events(20, Duration::from_millis(100)) else {
            panic!("a held stream opened");
        };
        assert!(
            matches!(
                err,
                Error::Slow {
                    idempotent: true,
                    sent: true
                }
            ),
            "{err:?}"
        );
        assert_eq!(err.to_string(), "the yard is slow to answer; trying again");
        assert!(asked.elapsed() < Duration::from_secs(1));
        held.join().unwrap();
    }

    /// A yard that never answers within the timeout is said to be slow, in
    /// plain words, and not the raw OS error (on macOS a slow unix socket
    /// reads as EAGAIN, "os error 35"): a GET is asked again, and one that
    /// answers then shows no error, while anything else is asked once,
    /// since it may have run.
    #[test]
    fn a_slow_yard_is_said_plainly_and_a_get_is_asked_again() {
        // On Windows Yard::start() is the TCP yard, so the socket arm is
        // unix's alone.
        #[cfg(unix)]
        let yards = vec![Yard::start(), Yard::start_tcp()];
        #[cfg(not(unix))]
        let yards = vec![Yard::start()];
        for yard in yards {
            let client = yard.client();
            let way = client.place();
            let slow = Duration::from_millis(100);
            // Each try that gave up leaves a line, with no edge id: no head
            // came to carry one.
            let gave_up = |method: &str, path: &str| {
                format!(
                    "call failed: request=none method={method} path={path} outcome=timeout status=none timeout=100ms"
                )
            };
            yard.answer("Head", r#"{"result": 4}"#);
            yard.answer("Crew", r#"{"result": []}"#);
            // A GET the yard never answers is asked again, then said
            // plainly.
            yard.stall("/call/Head");
            let Err(err) = client.send("GET", "/call/Head", "", "", Some(slow), true) else {
                panic!("{way}: a stalled yard answered");
            };
            assert!(
                matches!(
                    err,
                    Error::Slow {
                        idempotent: true,
                        sent: true
                    }
                ),
                "{way}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "the yard is slow to answer; trying again",
                "{way}"
            );
            assert!(!err.to_string().contains("os error"), "{way}: {err}");
            assert_eq!(yard.asked("Head").len(), 2, "{way}");
            assert_eq!(
                take_said(),
                [gave_up("GET", "/call/Head"), gave_up("GET", "/call/Head")],
                "{way}"
            );
            // A GET that answers the second time shows no error.
            yard.stall_once("/call/Crew");
            let Ok((status, mut body)) = client.send("GET", "/call/Crew", "", "", Some(slow), true)
            else {
                panic!("{way}: the second try failed");
            };
            assert_eq!(status, 200, "{way}");
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, br#"{"result": []}"#, "{way}");
            assert_eq!(yard.asked("Crew").len(), 2, "{way}");
            // The try that gave up said so, though the call went on to answer.
            assert_eq!(take_said(), [gave_up("GET", "/call/Crew")], "{way}");
            // Anything else is asked once, and says it may have run.
            yard.stall("/call/Mail");
            let Err(err) = client.call_once("Mail", &json!({}), slow) else {
                panic!("{way}: a stalled yard answered");
            };
            assert!(
                matches!(
                    err,
                    Error::Slow {
                        idempotent: false,
                        sent: true
                    }
                ),
                "{way}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "the yard did not answer in time, and the action may or may not have happened",
                "{way}"
            );
            assert!(!err.to_string().contains("os error"), "{way}: {err}");
            assert_eq!(yard.asked("Mail").len(), 1, "{way}");
            assert_eq!(take_said(), [gave_up("POST", "/call/Mail")], "{way}");
            // A read is a POST too, and changed nothing: it is said to be
            // slow, as a GET is, not to have maybe run. Only a GET is asked
            // again, so it is asked once.
            yard.stall("/call/Flows");
            let Err(err) = client.call_once("Flows", &json!({}), slow) else {
                panic!("{way}: a stalled yard answered");
            };
            assert!(
                matches!(
                    err,
                    Error::Slow {
                        idempotent: true,
                        sent: true
                    }
                ),
                "{way}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "the yard is slow to answer; trying again",
                "{way}"
            );
            assert_eq!(yard.asked("Flows").len(), 1, "{way}");
            assert_eq!(take_said(), [gave_up("POST", "/call/Flows")], "{way}");
            // A stream whose head never comes is said plainly too.
            yard.stall("/events");
            let before = yard.requests();
            let Err(err) = client.events(20, slow) else {
                panic!("{way}: a stalled stream opened");
            };
            assert!(
                matches!(
                    err,
                    Error::Slow {
                        idempotent: true,
                        sent: true
                    }
                ),
                "{way}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "the yard is slow to answer; trying again",
                "{way}"
            );
            assert_eq!(yard.requests(), before + 2, "{way}");
            assert_eq!(
                take_said(),
                [
                    gave_up("GET", "/events?after=20"),
                    gave_up("GET", "/events?after=20")
                ],
                "{way}"
            );
        }

        // The same retry covers a response body that starts but times out
        // before it is complete. The second answer is read whole.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let reply = br#"{"user":"jo","said_by":"laptop","yard":"south"}"#;
        let server = thread::spawn(move || {
            let read_head = |conn: &std::net::TcpStream| {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
            };
            let (mut first, _) = listener.accept().unwrap();
            read_head(&first);
            write!(
                first,
                "HTTP/1.1 200 OK\r\nX-Edge-Request: r-first\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            )
            .unwrap();
            first.write_all(&reply[..2]).unwrap();
            first.flush().unwrap();

            let (mut second, _) = listener.accept().unwrap();
            read_head(&second);
            write!(
                second,
                "HTTP/1.1 200 OK\r\nX-Edge-Request: r-second\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            )
            .unwrap();
            second.write_all(reply).unwrap();
        });
        let client = Client::remote(Address::parse(&address).unwrap(), TOKEN);
        let (status, bytes, request) = client
            .get_body("/me", "", Some(Duration::from_millis(100)), |_| true)
            .unwrap();
        assert_eq!(status, 200);
        assert_eq!(bytes, reply);
        // The answer read whole is the second's, and the try that timed out
        // in its body left its line, with the id its head carried.
        assert_eq!(request.as_deref(), Some("r-second"));
        assert_eq!(
            take_said(),
            [
                "call failed: request=r-first method=GET path=/me outcome=timeout status=200 timeout=100ms"
            ]
        );
        server.join().unwrap();

        // A call whose head came and whose body stops is told by the call
        // too: a read is slow, a write may have run. Neither is asked again.
        let short = b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort".to_vec();
        for (method, idempotent) in [("Flows", true), ("Mail", false)] {
            let (address, finished) = holding(short.clone());
            let client = Client::remote(Address::parse(&address).unwrap(), TOKEN);
            let Err(err) = client.call_once(method, &json!({}), Duration::from_millis(100)) else {
                panic!("{method}: a body that stops answered");
            };
            finished.join().unwrap();
            assert!(
                matches!(
                    &err,
                    Error::Slow {
                        idempotent: said,
                        sent: true
                    } if *said == idempotent
                ),
                "{method}: {err:?}"
            );
            assert_eq!(
                take_said(),
                [format!(
                    "call failed: request=none method=POST path=/call/{method} outcome=timeout status=200 timeout=100ms"
                )],
                "{method}"
            );
        }
    }

    /// A connect that gave up for its timeout sent nothing: a write says
    /// so, and that it is safe to send again, while a read is slow, as any
    /// other of its timeouts. A connect refused any other way says the
    /// yard is down, as ever. (No test waits out the connect's own ten
    /// seconds against a network that drops: this drives the mapping a
    /// failed connect takes.)
    #[test]
    fn a_connect_that_gave_up_says_nothing_was_sent() {
        for kind in [io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock] {
            let err = connect_error(Error::Down(io::Error::new(kind, "the test gave up")), false);
            assert!(
                matches!(
                    err,
                    Error::Slow {
                        idempotent: false,
                        sent: false
                    }
                ),
                "{kind:?}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "the yard did not answer in time, and nothing was sent; it is safe to send again",
                "{kind:?}"
            );
            assert!(!err.to_string().contains("gave up"), "{kind:?}: {err}");
            let err = connect_error(Error::Down(io::Error::new(kind, "the test gave up")), true);
            assert!(
                matches!(
                    err,
                    Error::Slow {
                        idempotent: true,
                        sent: false
                    }
                ),
                "{kind:?}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "the yard is slow to answer; trying again",
                "{kind:?}"
            );
        }
        // Anything else of a connect is down, as ever.
        let err = connect_error(
            Error::Down(io::Error::new(io::ErrorKind::ConnectionRefused, "refused")),
            false,
        );
        assert!(matches!(err, Error::Down(_)), "{err:?}");
        let err = connect_error(Error::Protocol("no".into()), false);
        assert!(matches!(err, Error::Protocol(_)), "{err:?}");
    }

    /// The connection to a yard at an address asks the other end after
    /// half a minute's silence, then every ten seconds, and is dead after
    /// three asked with no answer: within about a minute. Windows takes the
    /// time and the interval in one call and has none to read them back
    /// with, so this is the other systems' to check.
    #[cfg(not(windows))]
    #[test]
    fn a_connection_to_an_address_is_found_dead_within_a_minute() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address =
            Address::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let Conn::Tcp(tcp) = address.connect(Some(CALL_TIMEOUT)).unwrap() else {
            panic!("an address with no TLS is reached by TCP alone")
        };
        let socket = socket2::SockRef::from(&tcp);
        assert!(socket.keepalive().unwrap());
        assert_eq!(socket.tcp_keepalive_time().unwrap(), KEEPALIVE);
        assert_eq!(socket.tcp_keepalive_interval().unwrap(), KEEPALIVE_INTERVAL);
        assert_eq!(socket.tcp_keepalive_retries().unwrap(), KEEPALIVE_RETRIES);
        assert_eq!(
            (KEEPALIVE + KEEPALIVE_INTERVAL * KEEPALIVE_RETRIES).as_secs(),
            60
        );
    }

    fn bead_news(seq: u64, kind: &str, bead: &str, originator: &str) -> Event {
        serde_json::from_value(json!({
            "seq": seq, "kind": kind, "actor": "yardmaster", "bead": bead,
            "data": {"originator": originator},
        }))
        .unwrap()
    }

    #[test]
    fn timetable_news_is_one_of_mine_added_or_closed() {
        assert!(bead_news(21, "created", "yd-shed", "jo").timetable_news("jo"));
        assert!(bead_news(22, "closed", "yd-shed", "jo").timetable_news("jo"));
        assert!(!bead_news(21, "created", "yd-shed", "ada").timetable_news("jo"));
        assert!(!bead_news(22, "closed", "yd-shed", "ada").timetable_news("jo"));
        assert!(!bead_news(23, "advanced", "yd-shed", "jo").timetable_news("jo"));
        assert!(!bead_news(24, "noted", "yd-shed", "jo").timetable_news("jo"));
        assert!(!bead_news(25, "created", "", "jo").timetable_news("jo"));
        assert!(!bead_news(26, "created", "yd-shed", "jo").timetable_news(""));
        // A close that names no originator is nobody's.
        let bare: Event = serde_json::from_value(json!({
            "seq": 27, "kind": "closed", "actor": "yardmaster", "bead": "yd-shed",
            "data": {"reason": "merged"},
        }))
        .unwrap();
        assert!(!bare.timetable_news("jo"));
    }

    #[test]
    fn timetable_news_reads_the_gap_for_mine_added_or_closed() {
        let yard = Yard::start();
        yard.emit(r#"{"seq": 21, "kind": "created", "actor": "yardmaster", "bead": "yd-shed", "data": {"originator": "ada"}}"#);
        yard.emit(r#"{"seq": 22, "kind": "created", "actor": "yardmaster", "bead": "yd-barn", "data": {"originator": "jo"}}"#);
        yard.emit(r#"{"seq": 23, "kind": "noted", "actor": "yardmaster", "bead": "yd-barn", "data": {"note": 1}}"#);
        yard.emit(r#"{"seq": 24, "kind": "closed", "actor": "yardmaster", "bead": "yd-barn", "data": {"reason": "merged", "originator": "jo"}}"#);
        let client = yard.client();
        // One bead added and closed counts once.
        assert_eq!(
            client.timetable_news("jo", 20, 24).unwrap(),
            ["yd-barn".to_string()]
        );
        assert_eq!(
            client.timetable_news("jo", 20, 22).unwrap(),
            ["yd-barn".to_string()]
        );
        assert_eq!(
            client.timetable_news("ada", 20, 21).unwrap(),
            ["yd-shed".to_string()]
        );
        assert!(client.timetable_news("jo", 20, 21).unwrap().is_empty());
        assert!(client.timetable_news("jo", 22, 23).unwrap().is_empty());
        assert_eq!(
            client.timetable_news("jo", 20, 24).unwrap(),
            ["yd-barn".to_string()]
        );
    }

    #[test]
    fn timetable_news_asks_nothing_where_there_is_no_gap() {
        let yard = Yard::start();
        assert!(
            yard.client()
                .timetable_news("jo", 20, 20)
                .unwrap()
                .is_empty()
        );
        assert!(
            yard.client()
                .timetable_news("jo", 21, 20)
                .unwrap()
                .is_empty()
        );
        assert!(yard.streams_asked().is_empty());
    }

    #[test]
    fn timetable_news_leaves_the_stream_past_its_seq() {
        let yard = Yard::start();
        // Past `until` the stream is left, not waited on: what happens
        // next comes down the follower's own.
        yard.emit(r#"{"seq": 30, "kind": "created", "actor": "yardmaster", "bead": "yd-shed", "data": {"originator": "jo"}}"#);
        assert!(
            yard.client()
                .timetable_news("jo", 20, 25)
                .unwrap()
                .is_empty()
        );
        assert_eq!(yard.streams_asked(), vec![20]);
    }

    #[test]
    fn timetable_news_leaves_a_stream_that_outruns_its_gap() {
        let yard = Yard::start();
        // The gap 20..22 holds two seqs at most. A stream that sends
        // more events than that is not describing the gap — it replays
        // one seq and holds the catch-up open — and is left unread past
        // the gap's width, so the match it dangles later never lands.
        for _ in 0..3 {
            yard.emit(r#"{"seq": 21, "kind": "noted", "actor": "yardmaster", "bead": "yd-shed", "data": {}}"#);
        }
        yard.emit(r#"{"seq": 22, "kind": "created", "actor": "yardmaster", "bead": "yd-shed", "data": {"originator": "jo"}}"#);
        assert!(
            yard.client()
                .timetable_news("jo", 20, 22)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn timetable_news_reads_past_ten_thousand_events() {
        let yard = Yard::start();
        for seq in 21..=10_021 {
            let event = if seq == 10_021 {
                format!(
                    r#"{{"seq": {seq}, "kind": "closed", "bead": "yd-shed", "data": {{"originator": "jo"}}}}"#
                )
            } else {
                format!(r#"{{"seq": {seq}, "kind": "noted"}}"#)
            };
            yard.emit(&event);
        }
        assert_eq!(
            yard.client().timetable_news("jo", 20, 10_021).unwrap(),
            ["yd-shed".to_string()]
        );
    }

    /// Every literal method name passed through this file's call helpers has
    /// a row in README's tables of what the window reads and writes, saying
    /// when it is made: a new call ships its row. The names are found in the
    /// text of this file, passed through one of four helpers, so there is no
    /// list of them to keep. One way only: `GET /blobs/<hash>` has a row and
    /// no name to find. The row is in the table of reads exactly when
    /// `READS` has the name, which is what a timeout on the call says.
    #[test]
    fn the_readme_rows_every_literal_helper_call() {
        let source = include_str!("api.rs");
        let mut calls = Vec::new();
        for helper in ["answer", "list", "call", "reply"] {
            // Put together here so that this test is not one of the calls it
            // looks for. rustfmt breaks a long call after its bracket, with
            // the name on the next line.
            let opening = format!("self.{helper}(");
            for (at, _) in source.match_indices(&opening) {
                let after = source[at + opening.len()..].trim_start();
                if let Some(name) = after.strip_prefix('"').and_then(|s| s.split('"').next()) {
                    calls.push(name);
                }
            }
        }
        // Few found means the calls are no longer written as they are looked
        // for, and every row would pass unread.
        assert!(calls.len() > 20, "found {calls:?}");
        calls.sort();
        calls.dedup();

        // From the reads' header to the end of the writes' table, whether the
        // checkout ends its lines with LF or CR LF.
        let mut writes = false;
        let tables = include_str!("../README.md")
            .lines()
            .skip_while(|line| !line.starts_with("| It reads |"))
            .take_while(|line| {
                writes |= line.starts_with("| It writes |");
                !writes || line.starts_with('|')
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            tables.contains("| It writes |"),
            "README's tables of what it reads and writes were not found"
        );
        // All at once, for a change that adds several.
        let rowless: Vec<_> = calls
            .iter()
            .filter(|call| !tables.contains(&format!("`{call}`")))
            .collect();
        assert!(
            rowless.is_empty(),
            "called in src/api.rs and with no row in README's tables: {rowless:?}"
        );

        // The table a call's row is in says whether a timeout on it is a
        // read's (`reads`). A row's first cell names its calls: the second
        // may speak of others.
        let named = |table: &str| -> Vec<String> {
            table
                .lines()
                .filter_map(|line| line.trim_start_matches('|').split('|').next())
                .flat_map(|cell| cell.split('`').skip(1).step_by(2))
                .map(str::to_string)
                .collect()
        };
        let (read_rows, write_rows) = tables.split_once("| It writes |").unwrap();
        let (read_rows, write_rows) = (named(read_rows), named(write_rows));
        let misfiled: Vec<_> = calls
            .iter()
            .filter(|call| {
                let rows = if reads(call) { &read_rows } else { &write_rows };
                !rows.iter().any(|row| row == **call)
            })
            .collect();
        assert!(
            misfiled.is_empty(),
            "in README's table of what it reads, and not in `READS` (or the other way about): {misfiled:?}"
        );
        // A name nothing calls says a timeout of nothing.
        let stale: Vec<_> = READS.iter().filter(|name| !calls.contains(name)).collect();
        assert!(
            stale.is_empty(),
            "in `READS` and called by nothing: {stale:?}"
        );
    }
}
