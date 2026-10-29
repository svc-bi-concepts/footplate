//! What the window shows, without the window: who there is to talk with,
//! which conversation is open, what is unread, and what the yard has to be
//! asked next. It follows `yardr chat` (internal/chat of the yardr
//! repository): the state is read once, and events are applied from then on.
//!
//! Nothing here calls the yard. A change that needs a call answers it as an
//! [`Effect`], and the window makes it and hands the answer back.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use crate::api::{
    self, AboutBead, Arrival, Arrivals, AskWeight, Asks, Bead, BeadAsk, BeadNote, BlobRef,
    CrewMember, DecidedAsk, Event, Flow, FlowBead, FlowDepot, Flows, GrantGiven, Identity, Letter,
    Me, Notification, OpenAsk, Peer, Ran, Reaction, Run, Secret, Setup, SourceGrant,
    TelemetryPreview, User,
};
use crate::markdown;
use crate::select;

/// The kind of message a link must carry out for mail to be written.
const PEER_MAIL: &str = "mail";

/// The prefix of a bead label that makes it a topic; shown without it.
const TOPIC_PREFIX: &str = "topic:";

/// The option yardr adds to every ask with options: picking it leaves the
/// question to the asker.
const YOU_DECIDE: &str = "You decide";

/// Where asks with no topic stand.
pub const EVERYTHING_ELSE: &str = "Everything else";

/// The row below the conversations: what the yard reports.
pub const YARD: &str = "yard";

/// The row below the yard row: the beads in transit.
pub const ARRIVALS: &str = "arrivals";

/// The timetable's row for the beads travelling with no line: it stands
/// last, and only while any travel. The details and the back link name
/// it in lowercase, as the design quotes them (`no line`).
pub const NO_LINE: &str = "No line";

/// What a window for the user `me` starts from, read at the seq `head`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub me: String,
    pub head: u64,
    pub crew: Vec<CrewMember>,
    /// The users of the yard, the window's own among them.
    pub users: Vec<User>,
    pub peers: Vec<Peer>,
    /// How far the user's mail is acknowledged.
    pub cursor: u64,
    /// The user's unread mail.
    pub unread: Vec<Letter>,
    /// How far the user's notifications are acknowledged.
    pub notification_cursor: u64,
    /// What the yard reports, oldest first.
    pub notifications: Vec<Notification>,
    /// The person's open asks with what they dealt with.
    pub asks: Asks,
    /// The arrivals board as read: the rows as served, ordered and
    /// remarked.
    pub arrivals: Arrivals,
    /// Why the arrivals board is empty, in the window's words; `None`
    /// where the board was read.
    pub arrivals_notice: Option<String>,
    /// The flows view as read: the lines with their flows, stages, beads
    /// and lamps, the unassigned rows, and the travelling totals.
    pub flows: Flows,
    /// Why the flows view is empty, in the window's words; `None` where
    /// it was read.
    pub flows_notice: Option<String>,
    /// The yard has no Arrivals call: one from before the Lines view.
    /// The Lines pane says so, and no notice is raised.
    pub arrivals_too_old: bool,
    /// The yard has no Flows call: as above, for the flows half of the
    /// Lines view.
    pub flows_too_old: bool,
    /// The yard's operator, as its settings name them: who the grants
    /// view is offered to, and no one else. Empty where the yard names
    /// none, or its settings could not be read.
    pub operator: String,
    /// Every grant of the yard's sources, as read: empty where the
    /// window is not the operator's, which reads none.
    pub grants: Vec<SourceGrant>,
    /// Why the grants are empty, in the window's words; `None` where
    /// they were read.
    pub grants_notice: Option<String>,
    /// The yard has no SourceGrants call: one from before the grants
    /// view. The grants pane says so, and no notice is raised.
    pub grants_too_old: bool,
    /// The yard serves the grant list to no device that comes down the
    /// link from its edge: it is read on the yard's machine. The grants
    /// pane says so, and the call is not made, nor a notice raised.
    pub grants_on_the_yard: bool,
    /// The yard reads a session's screen to no device that comes down the
    /// link from its edge: it is read on the yard's machine. The window
    /// offers no Screen, and the call is not made.
    pub screen_on_the_yard: bool,
    /// The yard's telemetry switch with the install id and the next
    /// request, as read: `None` where the window is not the operator's,
    /// which reads none, where the yard serves the calls to no such
    /// window, and where the read failed.
    pub telemetry: Option<TelemetryPreview>,
    /// Why the telemetry preview is empty, in the window's words; `None`
    /// where it was read, or the yard serves no such call here.
    pub telemetry_notice: Option<String>,
    /// The yard serves the telemetry calls to no such window: one from
    /// before them, or one reached down the link from its edge. The
    /// Telemetry section is not offered, and the calls are not made.
    pub telemetry_unserved: bool,
    /// Of a yard at an address: who it says this device is. The window
    /// names the yard by it; the model reads nothing of it.
    pub yard: Option<Me>,
}

/// One whom a linked yard exposes to this one: `<name>@<peer>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Party {
    pub name: String,
    pub peer: String,
    /// A person there, and no crew member.
    pub person: bool,
    /// One line of what they do, in the peer's words.
    pub role: String,
    /// Whether this yard's link carries mail out.
    pub sends: bool,
    /// Whether the peer's side of the link takes mail for them.
    pub mail: bool,
}

impl Party {
    pub fn address(&self) -> String {
        format!("{}@{}", self.name, self.peer)
    }
}

/// Those the linked yards expose, in the order the yard lists them.
pub fn parties(peers: &[Peer]) -> Vec<Party> {
    let mut out = Vec::new();
    for p in peers {
        let Some(said) = &p.said else { continue };
        for e in &said.roster {
            out.push(Party {
                name: e.name.clone(),
                peer: p.name.clone(),
                person: e.kind == "user",
                role: e.role.clone(),
                sends: p.send.iter().any(|kind| kind == PEER_MAIL),
                mail: e.mail,
            });
        }
    }
    out
}

/// The names the links trust as a user of this yard, `<name>@<peer>` with
/// the user: mail from such a name is that user speaking, and stands in the
/// user's row.
pub fn trusted(peers: &[Peer]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for p in peers {
        for (name, user) in &p.trust {
            out.insert(format!("{name}@{}", p.name), user.clone());
        }
    }
    out
}

/// What the arrivals row says is in transit under it: the number, or
/// that none is.
fn transit_words(transit: usize) -> String {
    match transit {
        0 => "none in transit".to_string(),
        transit => format!("{transit} in transit"),
    }
}

/// What a line's row says is in transit under it: the number, or that
/// nothing is.
fn line_transit_words(transit: usize) -> String {
    match transit {
        0 => "nothing in transit".to_string(),
        transit => format!("{transit} in transit"),
    }
}

/// Whether the remark is a wait that names `me`: `waits for` with `me`
/// among the comma-separated names. Nobody where `me` is empty. The rows
/// draw with this; what counts toward "waiting on you" is alone or asked
/// ([`Model::waiting_on_you`]), not this.
pub fn waits_on_you(remark: &str, me: &str) -> bool {
    waits_names(remark).is_some_and(|names| !me.is_empty() && names.contains(&me))
}

/// Whether the remark is a hold: `held` where the holder is unknown,
/// `held by <name>` where the yard knows them. A stall is no hold.
pub fn is_held(remark: &str) -> bool {
    remark == "held" || remark.starts_with("held by ")
}

/// Whether the served holder is the reader: `held_by` names `me`.
/// Nobody where `me` is empty, and an unknown holder is nobody's.
pub fn held_by_you(held_by: &str, me: &str) -> bool {
    !me.is_empty() && held_by == me
}

/// Whether the remark names `me` alone: `waits for jo` where `me` is `jo`.
/// A wait for several names no one alone, even where `me` is among them;
/// the yard prints every member of the stage's group, so alone means the
/// group has only them. Nobody where `me` is empty.
fn waits_on_you_alone(remark: &str, me: &str) -> bool {
    match waits_names(remark) {
        Some(names) => !me.is_empty() && names.len() == 1 && names[0] == me,
        None => false,
    }
}

/// The remark with `you` where it names the reader: `waits for
/// mckean, yardmaster` reads `waits for you, yardmaster`, and `held by
/// mckean` reads `held by you`. Any other remark stands as the yard
/// printed it.
pub fn you_first(remark: &str, me: &str) -> String {
    if let Some(holder) = remark.strip_prefix("held by ") {
        if !me.is_empty() && holder.trim() == me {
            return "held by you".to_string();
        }
        return remark.to_string();
    }
    let Some(names) = waits_names(remark) else {
        return remark.to_string();
    };
    if me.is_empty() || !names.contains(&me) {
        return remark.to_string();
    }
    let others: Vec<&str> = names.into_iter().filter(|n| *n != me).collect();
    match others.is_empty() {
        true => "waits for you".to_string(),
        false => format!("waits for you, {}", others.join(", ")),
    }
}

/// The names a `waits for` remark waits on, None for any other remark.
fn waits_names(remark: &str) -> Option<Vec<&str>> {
    remark.strip_prefix("waits for ").map(|names| {
        names
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .collect()
    })
}

/// What a crew member's session does, in the words of `yardr chat`. One at
/// its harness's login screen reads no mail, however its runtime reads it
/// (often idle): the yard's words for it, whatever its status.
pub fn state_of(member: &CrewMember) -> &'static str {
    match (member.state.as_str(), member.status.as_str()) {
        ("failed", _) => "failed to start",
        ("running", _) if member.login_screen => "not logged in",
        ("running", "working") => "working",
        ("running", "idle") => "idle",
        ("running", "blocked") => "blocked",
        ("running", _) => "running",
        _ => "not running",
    }
}

/// A crew member's state for the dot beside their name in the list: what
/// runs, what rests, what waits, what stopped, what failed. A session at
/// its harness's login screen waits, for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Working,
    Idle,
    Blocked,
    Stopped,
    Failed,
}

/// The session a worker names, as `SessionScreen` takes it: the worker
/// without its runner word where the runner is herdr, since the yard
/// resolves a session id or a crew member's name, and neither carries
/// one. Any other worker stays whole: the yard refuses what is no
/// session, naming it.
pub fn worker_session(worker: &str) -> &str {
    worker.strip_prefix("herdr ").unwrap_or(worker)
}

/// What dot a crew member's session carries, from the same state and
/// status [`state_of`] words. A running session whose status the daemon
/// has not looked at yet still runs.
pub fn dot_of(member: &CrewMember) -> Dot {
    match (member.state.as_str(), member.status.as_str()) {
        ("failed", _) => Dot::Failed,
        ("running", _) if member.login_screen => Dot::Blocked,
        ("running", "idle") => Dot::Idle,
        ("running", "blocked") => Dot::Blocked,
        ("running", _) => Dot::Working,
        _ => Dot::Stopped,
    }
}

/// A row of the list on the left.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// A crew member's name, a user's, or `<name>@<peer>`.
    pub name: String,
    /// Under the name: a crew member's state, that a user is a person or
    /// the window's own, or what one of a linked yard is there and their
    /// role line.
    pub line: String,
    pub unread: usize,
    pub selected: bool,
    /// The dot beside the name: a crew member's state, and none on any
    /// other row. A linked yard says nothing of its people's state, and
    /// people and the yard have none.
    pub dot: Option<Dot>,
}

/// A mail of the open conversation, as it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub at: String,
    pub from: String,
    pub text: String,
    /// The files it carries, shown under its text.
    pub files: Vec<BlobRef>,
    /// The user's own, also what they wrote from a linked yard under a
    /// name trusted as them, in a row that is not their own.
    pub mine: bool,
    /// Where a mail the user sent stands. Mails that stand alike share one
    /// line, under the last of them.
    pub standing: Option<String>,
    /// Whether the name and the time stand above it: the first of its
    /// sender's mails within five minutes does, and the rest read under it.
    pub head: bool,
    /// The day's separator above it, where the day changes: `Today`,
    /// `Yesterday`, or the weekday with the day and the month.
    pub day: Option<String>,
    /// The mail amends what came before: its first word is `correction`,
    /// and it carries the label, so it reads as no new thought. Its `text`
    /// is what follows the word, which the label says.
    pub correction: bool,
    /// The asks and bead ids its text names that the yard said, each
    /// where it stands in its block: the chips, and what opening one
    /// shows.
    pub refs: Vec<select::RefSpan>,
    /// The open asks its text names, each once, in order: the previews
    /// under it, each with its card a tap away.
    pub asks: Vec<AskPreview>,
    /// The parties' emoji on the mail, by who put them, as the yard
    /// answered them: shown under its text.
    pub reactions: Vec<Reaction>,
}

/// The reactions a tap puts: the yard's fixed set, in its order.
pub const REACTIONS: [&str; 6] = ["👍", "👎", "❤️", "🎉", "👀", "✅"];

/// An open ask a mail names, as the preview under it shows it: the
/// question's first line, who asked and how long it waits. The card
/// itself stays in Decide.
#[derive(Debug, Clone, PartialEq)]
pub struct AskPreview {
    pub bead: String,
    pub ask: u64,
    pub question: String,
    pub asker: String,
    pub waits: String,
    /// What the head calls it: `ask`, `run ask` or `secret ask`.
    pub kind: String,
}

/// Where what is typed goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// `Mail` to a crew member or a user of this yard.
    Crew(String),
    /// `MailPeer` to one of a linked yard.
    Peer { name: String, peer: String },
}

/// What the left list has open: a bead with open asks, one dealt with,
/// the grants, a conversation, what the yard reports, the arrivals
/// board, a line's flows, the beads with no line, or one travelling bead
/// whole.
#[derive(Debug, Clone, PartialEq)]
pub enum Selected {
    /// A bead with open asks, by its id. A bead with several is one row.
    Decision(String),
    /// One dealt with, by bead and ask note seq.
    Decided(String, u64),
    /// The grants: every source × holder × call, for the operator's
    /// window alone. It has a row where the window is the operator's.
    Grants,
    /// A conversation, by crew name, user name or `<name>@<peer>`.
    Conversation(String),
    Yard,
    /// The arrivals pane: the panel of tracks, or the departures
    /// board under the head's toggle.
    Arrivals,
    /// One line's flows, by depot name, in the served order.
    Line(String),
    /// The beads travelling with no line, newest change first. It stands
    /// only while any travel.
    NoLine,
    /// One travelling bead whole, by its id. It has no row of the list:
    /// it opens from the board, a flow, Elsewhere or No line, and that
    /// row stands open for it.
    Bead(String),
}

impl Selected {
    /// The mode whose list holds the row: the conversations and the yard
    /// row are chat's, the beads with open asks, the dealt with and the
    /// grants are decisions', the timetable's rows and a travelling bead
    /// timetable's.
    pub fn mode(&self) -> Mode {
        match self {
            Selected::Conversation(_) | Selected::Yard => Mode::Chat,
            Selected::Decision(_) | Selected::Decided(_, _) | Selected::Grants => Mode::Decisions,
            Selected::Arrivals | Selected::Line(_) | Selected::NoLine | Selected::Bead(_) => {
                Mode::Timetable
            }
        }
    }
}

/// One of the rail's three modes: the crew and the conversation, the open
/// asks with their cards, or the timetable with the board and the lines.
/// Kept per yard in the settings, chat where none was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Chat,
    Decisions,
    Timetable,
}

impl Mode {
    /// The three, as the rail lists them.
    pub const ALL: [Mode; 3] = [Mode::Chat, Mode::Decisions, Mode::Timetable];

    /// The mode's place among the three.
    pub fn index(self) -> usize {
        match self {
            Mode::Chat => 0,
            Mode::Decisions => 1,
            Mode::Timetable => 2,
        }
    }

    /// The mode `by` steps on from this one, around the ends.
    pub fn step(self, by: isize) -> Mode {
        let n = Self::ALL.len() as isize;
        Self::ALL[(self.index() as isize + by).rem_euclid(n) as usize]
    }

    /// The mode's name, as its tooltip says it.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Decisions => "decisions",
            Mode::Timetable => "timetable",
        }
    }

    /// The mode's label, as the rail shows it under its glyph.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Chat => "Crew",
            Mode::Decisions => "Decide",
            Mode::Timetable => "Lines",
        }
    }
}

/// What Enter sends for an ask with the line's words.
#[derive(Debug, Clone, PartialEq)]
pub enum AnswerAction {
    /// A pick with the words in the one answer; option 0 answers an ask
    /// with no options with words.
    Answer { option: u64, words: String },
    /// Words with no option chosen go as a plain note; the ask stays open.
    Note { words: String },
}

/// One bead's row among the decisions: the question in plain words, or
/// the bead's title with how many it asks, with every ask expanded
/// beneath it.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRow {
    /// The bead's id, whole; `short` is what the row ends with.
    pub bead: String,
    /// The question in plain words, or `<title>: <n> questions`.
    pub head: String,
    /// Every open ask expanded, in ask order: empty where one ask is the
    /// row, whose head and sub already say it.
    pub asks: Vec<DecisionRowAsk>,
    /// `<asker> · <wants> · <waits>`, or `<waits>` when several ask, whose
    /// own lines say the rest.
    pub sub: String,
    /// The bead's id small, at the row's end: past its last `-`.
    pub short: String,
    pub selected: bool,
}

/// One ask of a bead's open row, as its expanded row shows it: the
/// question with whose it is, what it wants and how long it waits. The
/// options themselves are the open card's alone.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRowAsk {
    /// The ask note's seq: what an answer names.
    pub ask: u64,
    /// The question in plain words.
    pub head: String,
    /// `<asker> · <wants> · <waits>`.
    pub meta: String,
}

/// One topic's beads, in the order their oldest ask waits.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicGroup {
    /// Without the `topic:` prefix, or "Everything else".
    pub topic: String,
    pub rows: Vec<DecisionRow>,
}

/// What became of a dealt with ask, as the history's icons and chips
/// name it: a pick approved, words or a secret given, taken back, put
/// aside, or declined. A declined ask has no chip of its own: it shows
/// under All.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    Words,
    TakenBack,
    PutAside,
    Declined,
}

impl Outcome {
    /// The chips after All, in their order.
    pub const CHIPS: [Outcome; 4] = [
        Outcome::Approved,
        Outcome::Words,
        Outcome::TakenBack,
        Outcome::PutAside,
    ];

    /// The history's icon, first on its rows: each draws in the
    /// system's fonts on macOS and Linux. A put-aside stands as two
    /// bars, not the pause mark, which Linux's fonts have no glyph for.
    pub fn icon(self) -> &'static str {
        match self {
            Outcome::Approved => "✓",
            Outcome::Words => "✎",
            Outcome::TakenBack => "↩",
            Outcome::PutAside => "‖",
            Outcome::Declined => "✕",
        }
    }

    /// The chip's word.
    pub fn chip(self) -> &'static str {
        match self {
            Outcome::Approved => "Approved",
            Outcome::Words => "Words",
            Outcome::TakenBack => "Taken back",
            Outcome::PutAside => "Put aside",
            Outcome::Declined => "Declined",
        }
    }

    /// The outcome's word on a folded row, where no one ask's words stand.
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Approved => "approved",
            Outcome::Words => "words",
            Outcome::TakenBack => "taken back",
            Outcome::PutAside => "put aside",
            Outcome::Declined => "declined",
        }
    }
}

/// One dealt with, with its outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct DecidedRow {
    pub bead: String,
    pub ask: u64,
    /// The question's first line in plain words.
    pub head: String,
    /// The outcome's icon with its words: `✓approved`.
    pub outcome: String,
    /// `<asker> · <asked at>`.
    pub detail: String,
    pub short: String,
    pub selected: bool,
}

/// One row of a decided day: one dealt with, or several look-alikes
/// folded into one row with their count, opened to show them.
#[derive(Debug, Clone, PartialEq)]
pub enum DecidedEntry {
    One(DecidedRow),
    Fold(DecidedFold),
}

/// Several consecutive look-alikes folded: the first's question, with
/// the count at the row's end, past what the head's cut would hide.
#[derive(Debug, Clone, PartialEq)]
pub struct DecidedFold {
    /// The first folded ask: the fold's key, what opens it.
    pub bead: String,
    pub ask: u64,
    /// The first's head in plain words.
    pub head: String,
    /// The shared outcome's icon with its word: `↩taken back`.
    pub outcome: String,
    /// The shared asker.
    pub detail: String,
    /// Whether the fold stands open, with its rows under it.
    pub open: bool,
    pub rows: Vec<DecidedRow>,
}

/// One day of dealt with asks, newest day first: its head with its rows,
/// look-alikes folded.
#[derive(Debug, Clone, PartialEq)]
pub struct DecidedGroup {
    pub day: String,
    pub rows: Vec<DecidedEntry>,
}

/// One bead's route, as its details' strip shows it: its flow's stages
/// in travel order, each with whether a bead there waits on a person,
/// and which one the bead stands at.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub stages: Vec<RouteStage>,
    pub current: usize,
}

/// One stage of a route: the stage with whether its group is a manual
/// one, whose turn is a person's.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteStage {
    pub name: String,
    pub human: bool,
}

/// How the panel's worked beads split, as its head line draws them:
/// the served kinds with their counts, most first with ties by name,
/// and the groups working them with their seats.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingSplit {
    pub kinds: Vec<(String, usize)>,
    pub groups: Vec<WorkingGroup>,
}

/// One group with a worked bead on the Flows view, with its seats
/// where a current stage names the group. A group no current stage
/// names (an old claim after a route change) carries no seats, and
/// the tooltip lists its name alone.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingGroup {
    pub name: String,
    pub limit: Option<usize>,
    pub taken: Option<usize>,
}

/// One arrival's bead whole, as the details pane shows it: what the
/// Bead call returned, with the line, expected arrival and remark of
/// the arrivals row it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrivalDetail {
    pub bead: String,
    pub title: String,
    pub stage: String,
    /// From the arrivals row: empty when the yard serves the bead no
    /// longer, and the pane shows what the Bead call returned alone.
    pub line: String,
    pub expected: String,
    pub remark: String,
    pub tone: String,
    /// Who holds the bead, from the row it came from. Empty where no
    /// hold is on, and where the holder is unknown.
    pub held_by: String,
    pub labels: Vec<String>,
    pub body: String,
    /// The notes oldest first, so the newest stands last.
    pub notes: Vec<CardNote>,
    /// How many of the bead's asks are open: the Decisions rows' job,
    /// counted here for the line that says so.
    pub open_asks: usize,
}

/// One notice of the yard row, as it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub seq: u64,
    pub at: String,
    pub bead: String,
    pub text: String,
    pub read: bool,
    /// The files of a hook delivery, shown under its text: each as the
    /// yard holds it, or with why it did not take it. None for any other
    /// notice, and for a delivery until the yard has fetched its files,
    /// which it does after it told of the delivery.
    pub files: Vec<BlobRef>,
}

/// One option of a card, numbered from 1, `you decide` last.
#[derive(Debug, Clone, PartialEq)]
pub struct CardOption {
    pub number: u64,
    pub label: String,
    pub consequence: String,
}

/// One note of the bead's body, as the expanded card and the arrival's
/// details show it.
#[derive(Debug, Clone, PartialEq)]
pub struct CardNote {
    pub author: String,
    /// When it was written (RFC 3339, as the yard gives it): the
    /// details show it beside the author, the cards do not.
    pub at: String,
    pub text: String,
    /// The files the note carries, drawn below its words.
    pub files: Vec<BlobRef>,
}

/// One ask of a bead's open row, as its card shows it: the question,
/// the asker and the recommendation, the context, the numbered options,
/// the recommended one with its why, and whether the card stands expanded.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionAsk {
    pub ask: u64,
    pub question: String,
    pub asker: String,
    pub waits: String,
    /// The recommended option's label; empty when nothing is recommended.
    pub recommend_label: String,
    pub context: Vec<String>,
    pub options: Vec<CardOption>,
    /// The recommended option's number; 0 when none.
    pub recommend: u64,
    pub why: String,
    /// The option chosen with a number key or the arrows; 0 when none.
    pub chosen: u64,
    /// Whether this ask is the bead's current one: the keys act on it.
    pub current: bool,
    pub expanded: bool,
    pub about: Vec<AboutBead>,
    /// `open`, `answered`, `put aside` or `declined`.
    pub state: String,
    /// What a secret ask asks for: its card has the yard's line and a field
    /// that shows marks unless its toggle is pressed, where another has
    /// options or a line for words.
    pub secret: Option<Secret>,
    /// The command a run ask proposes: its card has the yard's block and
    /// the control that approves, and its question and context are marked
    /// as the asker's.
    pub run: Option<Run>,
    /// The setup a setup ask proposes: its card has the yard's block with
    /// the account choice and the presets to pick from, and the control
    /// that approves the pick with the account together. Its question and
    /// context are marked as the asker's, as a run ask's are.
    pub setup: Option<Setup>,
    /// What a mapping ask maps: its picks write a user's identity, so each
    /// passes the weighty gate; absent on any other ask.
    pub identity: Option<Identity>,
    /// The card's weight, one entry a class; empty on a routine card. A
    /// card with any passes the Mac's unlock before its approval goes, or
    /// takes the ask's number typed back where this window has no gate.
    pub weight: Vec<AskWeight>,
    /// The candidate taken with a tap, by source name; empty where none
    /// is. A filled secret field types another instead.
    pub account: String,
}

/// A bead's open asks, as the open row shows them: one card each.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionDetail {
    pub bead: String,
    pub title: String,
    pub topic: String,
    pub body: String,
    pub notes: Vec<CardNote>,
    /// A design bead's document, named by path and branch, not fetched;
    /// empty otherwise.
    pub doc: String,
    pub asks: Vec<DecisionAsk>,
}

/// One dealt with, as the open row shows it: the card with its pick, and
/// the same options to pick again from there.
#[derive(Debug, Clone, PartialEq)]
pub struct DecidedDetail {
    pub bead: String,
    pub title: String,
    pub ask: u64,
    pub question: String,
    pub asker: String,
    /// `answered`, `put aside`, `declined` or `taken back`.
    pub state: String,
    /// What was decided, in plain words.
    pub state_line: String,
    /// The picked option's number, 0 on a words answer and elsewhere.
    pub picked: u64,
    /// The picked option's label, empty on a words answer and elsewhere.
    pub pick: String,
    pub words: String,
    /// When a put-aside ask comes back, empty otherwise.
    pub return_at: String,
    pub recommend_label: String,
    pub context: Vec<String>,
    pub options: Vec<CardOption>,
    pub recommend: u64,
    pub why: String,
    pub chosen: u64,
    pub expanded: bool,
    pub about: Vec<AboutBead>,
    pub body: String,
    pub notes: Vec<CardNote>,
    pub doc: String,
    /// What a secret ask asks for, as on its open card.
    pub secret: Option<Secret>,
    /// The command a run ask proposes, as on its open card.
    pub run: Option<Run>,
    /// The setup a setup ask proposes, as on its open card.
    pub setup: Option<Setup>,
    /// What a mapping ask maps, as on its open card.
    pub identity: Option<Identity>,
    /// The card's weight, one entry a class, as on its open card.
    pub weight: Vec<AskWeight>,
    /// The candidate taken with a tap, by source name; empty where none
    /// is. A filled secret field types another instead.
    pub account: String,
    /// What the yard ran when the ask was approved; none until it is.
    pub ran: Option<Ran>,
}

/// A call the yard has to be asked, with what the model does on its answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// `Conversation` with `name` after a seq; answered to
    /// [`Model::conversation`].
    Read { name: String, after: u64 },
    /// `Conversation` with `name`, its newest [`PAGE`] mails older than the
    /// seq `before` (0: the newest of all); answered to [`Model::page`] or
    /// [`Model::page_failed`].
    Page { name: String, before: u64 },
    /// `AckUserInbox` through a seq; answered to [`Model::acked`] or
    /// [`Model::ack_failed`].
    Ack { through: u64 },
    /// `Head`, then `Crew`; answered to [`Model::listed`].
    Crew,
    /// `Peers`; answered to [`Model::linked`].
    Peers,
    /// `Users`; answered to [`Model::users`].
    Users,
    /// `UserNotifications`; answered to [`Model::notifications`].
    Notifications,
    /// `AckUserNotifications` through a seq; answered to
    /// [`Model::notifications_acked`] or [`Model::notifications_ack_failed`].
    AckNotifications { through: u64 },
    /// `UserAsks`; answered to [`Model::asks`].
    Asks,
    /// `Arrivals`; answered to [`Model::arrivals`].
    Arrivals,
    /// `Flows`; answered to [`Model::flows`].
    Flows,
    /// `SourceGrants`; answered to [`Model::grants`].
    Grants,
    /// `TelemetryShow`; answered to [`Model::telemetry`].
    Telemetry,
    /// Which of the person's beads were added or closed after `from`
    /// through `until`, by bead; answered to [`Model::timetable_seeded`].
    Timetable { from: u64, until: u64 },
    /// `Bead` with `bead`; answered to [`Model::bead_loaded`] or
    /// [`Model::bead_failed`].
    Bead { bead: String },
    /// `Bead` with each of `beads`, one after another, to learn which are
    /// beads of the yard: ids mail names that no list the window reads
    /// holds, as a closed bead's is; answered to [`Model::beads_looked`].
    Beads { beads: Vec<String> },
    /// `DeliveryFiles` of each hook delivery `seqs` names, one after
    /// another, newest first; answered to [`Model::delivery_files`].
    DeliveryFiles { seqs: Vec<u64> },
}

/// A read of the yard made again and again, whose answer replaces what
/// the model holds of it: the effect of that name. Reads of one kind may
/// answer out of the order they were made in, since each is a connection
/// of its own and `UserAsks` takes seconds on a large yard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Read {
    Crew,
    Peers,
    Users,
    Notifications,
    Asks,
    Arrivals,
    Flows,
    Grants,
    Telemetry,
}

impl Read {
    /// The effect that makes the read.
    pub fn effect(self) -> Effect {
        match self {
            Read::Crew => Effect::Crew,
            Read::Peers => Effect::Peers,
            Read::Users => Effect::Users,
            Read::Notifications => Effect::Notifications,
            Read::Asks => Effect::Asks,
            Read::Arrivals => Effect::Arrivals,
            Read::Flows => Effect::Flows,
            Read::Grants => Effect::Grants,
            Read::Telemetry => Effect::Telemetry,
        }
    }
}

/// The reads of one following, each stamped when it is made: an answer is
/// applied only when it is newer than the last of its kind applied, and
/// than the state the model was read from. A read that failed is made
/// again once, after a while, unless a newer one answered meanwhile.
#[derive(Debug, Clone, Default)]
pub struct Reads {
    /// The last stamp given.
    made: u64,
    /// What was made up to this stamp is older than the state read last.
    floor: u64,
    /// The newest stamp applied, by kind.
    applied: HashMap<Read, u64>,
    /// The reads to make again, by kind, with the stamp of the one that
    /// failed.
    again: HashMap<Read, u64>,
}

impl Reads {
    /// The stamp of a read made now.
    pub fn make(&mut self) -> u64 {
        self.made += 1;
        self.made
    }

    /// Whether the answer of the read of `read` stamped `stamp` is to be
    /// applied, which it is from here on.
    pub fn fresh(&mut self, read: Read, stamp: u64) -> bool {
        let applied = self.applied.entry(read).or_default();
        if stamp <= self.floor || stamp <= *applied {
            return false;
        }
        *applied = stamp;
        true
    }

    /// The read of `read` stamped `stamp` failed: whether it is to be made
    /// again after a while. Not where one is to be already, nor where a
    /// newer one answered.
    pub fn failed(&mut self, read: Read, stamp: u64) -> bool {
        if !self.accepts_failure(read, stamp) {
            return false;
        }
        if self.again.contains_key(&read) {
            return false;
        }
        self.again.insert(read, stamp);
        true
    }

    /// Whether a failed answer is still newer than the state read and any
    /// answer already applied for its kind.
    pub fn accepts_failure(&self, read: Read, stamp: u64) -> bool {
        stamp > self.floor && self.applied.get(&read).is_none_or(|&a| a <= stamp)
    }

    /// The while is up: whether the read that failed is still to be made
    /// again, with nothing newer of its kind applied since.
    pub fn again(&mut self, read: Read) -> bool {
        let Some(stamp) = self.again.remove(&read) else {
            return false;
        };
        stamp > self.floor && self.applied.get(&read).is_none_or(|&a| a < stamp)
    }

    /// The state was read again: every read made before is older than it,
    /// and none that failed is made again.
    fn restarted(&mut self) {
        self.floor = self.made;
        self.again.clear();
    }
}

/// How many mails of a conversation the yard is asked for at once: the
/// newest when it opens, and as many older each time its top is reached.
pub const PAGE: u64 = 50;

/// What stands above the oldest mail the open conversation shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Top {
    /// The yard has not answered yet whether there is older mail.
    Unread,
    /// The yard has older mail: reaching the top asks for it.
    Older,
    /// This is the oldest mail of the conversation.
    Start,
}

/// What a piece of news is: the settings choose which kinds the system
/// tells of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mail,
    Decision,
    Notice,
}

/// Something that arrived for the person since the window started: a mail
/// to them, a new ask, a notice of the yard. Never what they wrote
/// themselves, and never what was unread or open when the window started.
/// The window takes it ([`Model::take_news`]) and decides whether the
/// system tells of it (notify.rs).
#[derive(Debug, Clone, PartialEq)]
pub struct News {
    pub kind: Kind,
    /// Who it is from: a mail's sender, an ask's asker, the yard.
    pub from: String,
    /// The mail's text, the ask's question, the notice.
    pub text: String,
    /// The row it belongs to.
    pub target: Selected,
    /// Whether that row was open as it arrived.
    pub open: bool,
}

/// One unread mail to the user: who sent it, and whether the window has
/// shown it.
#[derive(Debug, Clone, PartialEq)]
struct Waiting {
    from: String,
    seen: bool,
}

/// The hour a linked yard refused mail over: what its link takes in an
/// hour and when the hour ends.
#[derive(Debug, Clone, PartialEq)]
struct Hour {
    limit: u64,
    until: String,
}

impl Hour {
    /// When the hour ends as `HH:MM`, in the zone of the person reading.
    /// Empty for a time that cannot be read.
    fn end(&self) -> String {
        match DateTime::parse_from_rfc3339(&self.until) {
            Ok(t) => t.with_timezone(&chrono::Local).format("%H:%M").to_string(),
            Err(_) => String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    me: String,
    crew: Vec<CrewMember>,
    /// The arrivals board as read: the rows as served, ordered and
    /// remarked.
    arrivals: Vec<Arrival>,
    /// The flows view as read: the lines with their flows, stages, beads
    /// and lamps, the unassigned rows, and the travelling totals.
    flows: Flows,
    /// The yard has no Arrivals call: one from before the Lines view.
    /// The Lines pane says so, and no notice is raised. A board read
    /// again clears it, as one read at the start never sets it.
    arrivals_too_old: bool,
    /// The yard has no Flows call: as above, for the flows half of the
    /// Lines view.
    flows_too_old: bool,
    /// The yard's operator, as its settings name them: who the grants
    /// view is offered to, and no one else.
    operator: String,
    /// Every grant of the yard's sources, as read: empty where the
    /// window is not the operator's, which reads none.
    grants: Vec<SourceGrant>,
    /// The yard has no SourceGrants call: one from before the grants
    /// view. The grants pane says so, and no notice is raised. Grants
    /// read again clear it, as ones read at the start never set it.
    grants_too_old: bool,
    /// The grants are read on the yard's machine, since the yard serves
    /// them down no link: the pane says so, and no call is made for them.
    /// A window reached by the edge starts so; a refusal that says the
    /// request came down the link sets it, and grants read clear it.
    grants_on_the_yard: bool,
    /// A session's screen is read on the yard's machine, since the yard
    /// serves it down no link: the window offers no Screen, and no call is
    /// made for one. A window reached by the edge starts so; a refusal that
    /// says the request came down the link sets it, and the model of a state
    /// read again keeps it.
    screen_on_the_yard: bool,
    /// The yard's telemetry switch with the install id and the next
    /// request, as read: `None` where the window is not the operator's,
    /// which reads none, where the yard serves the calls to no such
    /// window, and where the read failed.
    telemetry: Option<TelemetryPreview>,
    /// Why the telemetry preview is empty, in the window's words; `None`
    /// where it was read, or the yard serves no such call here. A preview
    /// read again clears it.
    telemetry_notice: Option<String>,
    /// The yard serves the telemetry calls to no such window: one from
    /// before them, or one reached down the link from its edge. The
    /// Telemetry section is not offered, and no call is made for it. A
    /// preview read again clears it, as one read at the start never sets
    /// it.
    telemetry_unserved: bool,
    /// The switch is read-only in this window: the yard refused its change
    /// from where this window reaches it, and the switch is changed in the
    /// yard's web view instead. Set again clears nothing: the section
    /// stays read-only until the yard is read whole.
    telemetry_read_only: bool,
    /// The yard has no React call: one from before reactions. The row
    /// the reactions are tapped on is hidden, and no notice is raised.
    /// A reaction answered clears it.
    react_too_old: bool,
    /// The timetable row a travelling bead was opened from: the board, a
    /// line or No line. The list's open row and the details' back link
    /// while the bead is open.
    bead_from: Option<Selected>,
    /// The users of the yard by name, the window's own among them.
    users: Vec<String>,
    parties: Vec<Party>,
    /// The names the links trust as a user of this yard, with the user.
    trust: BTreeMap<String, String>,
    /// The list and pane the open row belongs to. A mode switch must not
    /// leave a row from another list selected when a queue update removes it.
    mode: Mode,
    selected: Option<Selected>,
    /// The conversation that was open last: the one opened again when the
    /// open row is gone, so that a decision dealt with leads back to where
    /// it was opened from.
    back: Option<String>,
    /// The conversations by whom they are with, each in order of seq, and
    /// those the yard was asked for whole.
    convo: HashMap<String, Vec<Letter>>,
    loaded: HashSet<String>,
    /// The conversations of which the yard has older mail than the window
    /// was given, and those an older page of which is asked for and not
    /// answered.
    older: HashSet<String>,
    paging: HashSet<String>,
    /// The hour a linked yard refused mail over, by peer, from the mail
    /// the yard wrote that one of this yard's was not delivered: `limit`
    /// and `until` of an undelivered event. Said under failed mail to the
    /// peer instead of the inbox's why; a refusal the yard tells without
    /// them clears the peer's.
    hours: HashMap<String, Hour>,
    /// The peer of a message the yard gave up, by its outbox id, from the
    /// failed event the yard logs before the mail that tells of it: that
    /// mail names the message alone.
    failed_ids: HashMap<String, String>,
    /// The user's unread mail by seq. `cursor` is how far the yard has it
    /// acknowledged, `asked` how far the window has asked for.
    unread: BTreeMap<u64, Waiting>,
    cursor: u64,
    asked: u64,
    /// What the yard reports, oldest first.
    notifications: Vec<Notification>,
    /// The open asks, oldest first, with what was dealt with, newest first.
    open: Vec<OpenAsk>,
    decided: Vec<DecidedAsk>,
    /// Whether the folded "Decided" stands open.
    decided_expanded: bool,
    /// The history's chip: `None` shows every outcome.
    decided_chip: Option<Outcome>,
    /// The history's filter, as typed: matches question, asker and outcome.
    decided_filter: String,
    /// The folded history rows standing open, by their first ask.
    decided_folds: HashSet<(String, u64)>,
    /// Whether the timetable's quiet lines stand unfolded.
    quiet_expanded: bool,
    /// Whether the arrivals pane shows the departures board instead of
    /// the panel: the head's toggle, the panel where false.
    departures: bool,
    /// One bead whole, by id, as the cards show it: read when a decision
    /// row opens, and again on every event whose bead is one shown.
    beads: HashMap<String, Bead>,
    /// The ids mail names that look like a bead's, and whether each is one:
    /// as the yard said when asked, or because a list the window reads
    /// held it when mail named it. A bead that closed falls out of every
    /// list, and its id in mail chips by this; no id is asked twice.
    looked: HashMap<String, bool>,
    /// The ids asked about and not answered yet.
    looking: HashSet<String>,
    /// The files of the hook deliveries the yard row shows, by the
    /// delivery's seq, as `DeliveryFiles` answered them. A delivery has an
    /// entry once the yard has fetched its files, and the yard records a
    /// delivery's files once: an entry is never replaced.
    deliveries: HashMap<u64, Vec<BlobRef>>,
    /// Deliveries whose `DeliveryFiles` ask has not failed: a successful
    /// empty answer is not asked again when the row opens.
    delivering: HashSet<u64>,
    /// The hook delivery notices shown in the yard row. A failed first ask
    /// still needs its one refetch when `hook_files` comes.
    delivery_notices: HashSet<u64>,
    /// The deliveries whose `hook_files` event has already asked for a
    /// refetch, so replaying that event does not ask twice.
    delivery_refetched: HashSet<u64>,
    /// The yard has no DeliveryFiles call: one from before it. The notices
    /// show no files, and no notice is raised.
    deliveries_too_old: bool,
    /// The bead the drawer was opened on last. The queue read again keeps
    /// it read, as it keeps the open travelling bead: it is none of the
    /// queue's, and a closed bead never is.
    drawn: Option<String>,
    /// The option chosen with a number key or the arrows, by bead and ask.
    chosen: HashMap<(String, u64), u64>,
    /// The asks whose card stands expanded, by bead and ask.
    expanded: HashSet<(String, u64)>,
    /// The run asks whose carried texts stand shown, by bead and ask: the
    /// tap behind the yard's sentences.
    texts: HashSet<(String, u64)>,
    /// The setup ask's candidate taken with a tap, by bead and ask and
    /// source name. A filled secret field types another instead, and
    /// clears this.
    account: HashMap<(String, u64), String>,
    /// Which ask of a bead with several the keys act on, by bead.
    current: HashMap<String, u64>,
    /// The seq of the last `crew_status` applied to each crew member: a
    /// read of the yard that is older does not put an older status back.
    status_at: HashMap<String, u64>,
    /// The newest event seq applied, from the head the yard was read at.
    events_seen: u64,
    /// The person's beads added or closed since they last had
    /// Timetable open, by bead: the rail's Lines badge counts them. By
    /// bead and not by event, so a bead added and closed counts once,
    /// and a late catch-up cannot count a bead the stream told of twice.
    timetable_news: HashSet<String>,
    /// The newest change seen when Timetable was last open, an event
    /// seq, kept per yard in the settings.
    timetable_looked: u64,
    /// What arrived and the window has not taken yet, oldest first.
    news: Vec<News>,
    /// The window shows another yard: no row of this one is open, and none
    /// is opened by what arrives, so nothing of it is seen or marked read
    /// until the window is back ([`Model::back_to`]).
    aside: bool,
    /// The stamps of the reads made of the yard, and what they answered.
    reads: Reads,
    /// What went wrong last, in words; empty when nothing.
    pub notice: String,
}

impl Model {
    /// The model of a state just read. `selected` is what was open before
    /// the yard was lost, if any: it stays open when its row is still
    /// there. Else the mode's own default is.
    pub fn new(state: State, mode: Mode, selected: Option<Selected>) -> (Model, Vec<Effect>) {
        Self::new_returning(state, mode, selected, false, false, false)
    }

    /// The model of a state read when returning to a yard. The folded
    /// "Decided" list and the quiet lines must be open before `selected`
    /// is checked, since their rows do not exist in the order while
    /// folded.
    pub fn new_returning(
        state: State,
        mode: Mode,
        selected: Option<Selected>,
        decided_expanded: bool,
        quiet_expanded: bool,
        departures: bool,
    ) -> (Model, Vec<Effect>) {
        let mut m = Self::of(state, decided_expanded, quiet_expanded, departures);
        m.mode = mode;
        m.selected = m.row_in(mode, selected);
        m.opened();
        let effects = m.open();
        (m, effects)
    }

    /// The model of a state just read of a yard the window does not show:
    /// it has what waits there, and no row of it is open.
    pub fn new_aside(state: State) -> Model {
        Model {
            aside: true,
            ..Self::of(state, false, false, false)
        }
    }

    /// What a state just read has, with no row open yet.
    fn of(state: State, decided_expanded: bool, quiet_expanded: bool, departures: bool) -> Model {
        let mut m = Model {
            me: state.me,
            parties: parties(&state.peers),
            trust: trusted(&state.peers),
            crew: state.crew,
            arrivals: state.arrivals.rows,
            flows: state.flows,
            arrivals_too_old: state.arrivals_too_old,
            flows_too_old: state.flows_too_old,
            operator: state.operator,
            grants: state.grants,
            grants_too_old: state.grants_too_old,
            grants_on_the_yard: state.grants_on_the_yard,
            screen_on_the_yard: state.screen_on_the_yard,
            telemetry: state.telemetry,
            telemetry_notice: state.telemetry_notice,
            telemetry_unserved: state.telemetry_unserved,
            cursor: state.cursor,
            asked: state.cursor,
            notifications: state.notifications,
            open: state.asks.open,
            decided: state.asks.decided,
            decided_expanded,
            quiet_expanded,
            departures,
            events_seen: state.head,
            ..Model::default()
        };
        m.set_users(state.users);
        for l in state.unread.iter().filter(|l| !l.read && l.seq > m.cursor) {
            m.unread.insert(
                l.seq,
                Waiting {
                    from: l.from.clone(),
                    seen: false,
                },
            );
        }
        m
    }

    /// The row to open in `mode`: `selected` where it is still there,
    /// else the mode's own default. In chat that is the yardmaster's
    /// conversation, as in `yardr chat`, else the first row from the top;
    /// in decisions the first open decision, and none where there is
    /// none; in timetable the arrivals row. A travelling bead has no row
    /// of the list, and stays open as a row that is still there does: it
    /// shows what the Bead call returns, whether the yard serves the row
    /// still or not.
    fn row_in(&self, mode: Mode, selected: Option<Selected>) -> Option<Selected> {
        let order = self.order_in(mode);
        selected
            .filter(|s| {
                (mode == Mode::Timetable && matches!(s, Selected::Bead(_))) || order.contains(s)
            })
            .or_else(|| match mode {
                Mode::Chat => order
                    .iter()
                    .find_map(|s| match s {
                        Selected::Conversation(name) if name == "yardmaster" => Some(s.clone()),
                        _ => None,
                    })
                    .or_else(|| order.into_iter().next()),
                // A kept expansion makes dealt-with rows visible, but a
                // saved row that disappeared falls back as it did before
                // that list was kept: to an open decision, or none. The
                // grants stand in the list, but never take a card's place:
                // an answer must come back to its card, not jump aside.
                Mode::Decisions => order
                    .iter()
                    .find(|s| matches!(s, Selected::Decision(_)))
                    .cloned(),
                Mode::Timetable => Some(Selected::Arrivals),
            })
    }

    /// The window shows another yard from here on: the open row is shut,
    /// and what arrives opens none. What it holds stays as it is, and goes
    /// on with the yard; what went wrong last was said while it showed.
    pub fn leave(&mut self) {
        self.selected = None;
        self.aside = true;
        self.notice.clear();
    }

    /// The window shows this yard again: on `selected`, the row it was left
    /// on, where that is still there, as [`Model::new_returning`] opens
    /// one.
    pub fn back_to(
        &mut self,
        mode: Mode,
        selected: Option<Selected>,
        decided_expanded: bool,
        quiet_expanded: bool,
        departures: bool,
    ) -> Vec<Effect> {
        self.aside = false;
        self.mode = mode;
        self.decided_expanded |= decided_expanded;
        self.quiet_expanded |= quiet_expanded;
        self.departures |= departures;
        self.selected = self.row_in(mode, selected);
        self.opened();
        self.open()
    }

    /// Opens the mode's row: `selected` where it is still there, else the
    /// mode's own default. What the mode is opened on when the person
    /// steps to it.
    pub fn show(&mut self, mode: Mode, selected: Option<Selected>) -> Vec<Effect> {
        self.mode = mode;
        self.selected = self.row_in(mode, selected);
        self.notice.clear();
        self.opened();
        self.open()
    }

    /// The model of a state read again after the yard was lost: what was
    /// open stays open as in [`Model::new`], and the conversation to come
    /// back to from a decision stays that one.
    pub fn resumed(&self, state: State, mode: Mode) -> (Model, Vec<Effect>) {
        let head = state.head;
        let unread = state.unread.clone();
        // Of a yard the window does not show, no row is open afterwards
        // either.
        let (mut m, mut effects) = match self.aside {
            true => (Model::new_aside(state), Vec::new()),
            false => Model::new(state, mode, self.selected.clone()),
        };
        if m.back.is_none() {
            m.back = self.back.clone();
        }
        if m.bead_from.is_none() {
            m.bead_from = self.bead_from.clone();
        }
        // A read made before the state was read again is older than it.
        m.reads = self.reads.clone();
        m.reads.restarted();
        // What a refusal taught of the screen stays true: a client cannot
        // always tell that the yard is reached by its edge, so the state
        // read again may not say it.
        m.screen_on_the_yard |= self.screen_on_the_yard;
        // What the yard said of the ids mail names stays true: asking
        // them again for every reconnection would make a call each.
        m.looked = self.looked.clone();
        // The timetable's news and when it was last looked at carry
        // over, and what changed meanwhile is caught up with.
        m.timetable_news = self.timetable_news.clone();
        m.timetable_looked = self.timetable_looked;
        if self.events_seen < head {
            effects.push(Effect::Timetable {
                from: self.events_seen,
                until: head,
            });
        }
        // What was shown before the yard was lost is not unseen again.
        for (seq, w) in m.unread.iter_mut() {
            w.seen |= self.unread.get(seq).is_some_and(|before| before.seen);
        }
        // What the yard has now and had not before it was lost arrived
        // meanwhile, and is news as if the stream had told of it.
        for l in unread {
            let arrived = l.seq > self.cursor && !self.unread.contains_key(&l.seq);
            if arrived && m.unread.contains_key(&l.seq) {
                m.mail_news(&l.from, &l.text);
            }
        }
        m.ask_news(&self.open, &self.decided);
        m.notice_news(&self.notifications);
        effects.extend(m.ack());
        (m, effects)
    }

    /// The stamps of the reads made of the yard.
    pub fn reads(&mut self) -> &mut Reads {
        &mut self.reads
    }

    /// What arrived since the window last took it, oldest first.
    pub fn take_news(&mut self) -> Vec<News> {
        std::mem::take(&mut self.news)
    }

    /// A mail to the user arrived from `from`: news of the row that shows
    /// its sender, which is the yard row for the yard's own mail and for
    /// anyone who has no row.
    fn mail_news(&mut self, from: &str, text: &str) {
        let target = match self.row_of(from) {
            Some(row) => Selected::Conversation(row.to_string()),
            None => Selected::Yard,
        };
        self.news.push(News {
            kind: Kind::Mail,
            from: from.to_string(),
            text: text.to_string(),
            open: self.shows(from),
            target,
        });
    }

    /// The asks that are open now and were not in `open` are news. One the
    /// person dealt with is not when it is open again (an answer the yard
    /// did not take), unless it was put aside: that one has come back.
    fn ask_news(&mut self, open: &[OpenAsk], decided: &[DecidedAsk]) {
        let news: Vec<News> = self
            .open
            .iter()
            .filter(|a| !open.iter().any(|o| o.bead == a.bead && o.ask == a.ask))
            .filter(|a| {
                !decided
                    .iter()
                    .any(|d| d.bead == a.bead && d.ask == a.ask && d.state != "put aside")
            })
            .map(|a| News {
                kind: Kind::Decision,
                from: a.asker.clone(),
                text: a.question.clone(),
                open: self.selected == Some(Selected::Decision(a.bead.clone())),
                target: Selected::Decision(a.bead.clone()),
            })
            .collect();
        self.news.extend(news);
    }

    /// The unread notices that are newer than all of `before` are news.
    fn notice_news(&mut self, before: &[Notification]) {
        let newest = before.iter().map(|n| n.seq).max().unwrap_or(0);
        let open = self.selected == Some(Selected::Yard);
        let news: Vec<News> = self
            .notifications
            .iter()
            .filter(|n| !n.read && n.seq > newest)
            .map(|n| News {
                kind: Kind::Notice,
                from: YARD.to_string(),
                text: n.text.clone(),
                target: Selected::Yard,
                open,
            })
            .collect();
        self.news.extend(news);
    }

    pub fn me(&self) -> &str {
        &self.me
    }

    /// Whether the window is the operator's: it runs as the yard's
    /// operator, who alone is offered the grants view and the Telemetry
    /// section. A yard that names no operator offers them to no one.
    pub fn is_operator(&self) -> bool {
        !self.operator.is_empty() && self.me == self.operator
    }

    pub fn selected(&self) -> Option<&Selected> {
        self.selected.as_ref()
    }

    /// The open conversation's name, when a conversation is open.
    pub fn selected_conversation(&self) -> Option<&str> {
        match self.selected.as_ref() {
            Some(Selected::Conversation(name)) => Some(name),
            _ => None,
        }
    }

    /// Whether the open conversation is one whose mail says where the next
    /// answer goes, and not all of it is read yet: the row of a user whom
    /// names of linked yards are trusted as. Who wrote last is not known
    /// until then, and a guess would be the user in this yard.
    pub fn reading(&self) -> bool {
        let Some(sel @ Selected::Conversation(_)) = self.selected.as_ref() else {
            return false;
        };
        let sources = self.sources(sel);
        sources.len() > 1 && sources.iter().any(|s| !self.loaded.contains(s))
    }

    /// Whether the yard has answered for all the mail the open row shows:
    /// before that the row is shorter than it will be. A row that shows no
    /// mail waits for none.
    pub fn answered(&self) -> bool {
        self.selected.as_ref().is_some_and(|sel| {
            self.sources(sel)
                .iter()
                .all(|name| self.loaded.contains(name))
        })
    }

    /// The names of the conversations, in order: the crew, the addresses
    /// of those the linked yards expose, then the users of this yard.
    fn conversation_names(&self) -> Vec<String> {
        self.crew
            .iter()
            .map(|r| r.name.clone())
            .chain(
                self.parties
                    .iter()
                    .map(Party::address)
                    .filter(|a| self.folded(a).is_none()),
            )
            .chain(self.people().cloned())
            .collect()
    }

    /// The users of this yard who have a row, the window's own among
    /// them. One a crew member has the name of has none: that row is the
    /// crew member's.
    fn people(&self) -> impl Iterator<Item = &String> {
        self.users.iter().filter(|u| self.member(u).is_none())
    }

    /// The user in whose row the mail of `address` stands: the one a link
    /// trusts that name as, when that user has a row. Such a name has no
    /// row of its own, as in `yardr chat`: it is how the user spoke.
    fn folded(&self, address: &str) -> Option<&str> {
        let user = self.trust.get(address)?;
        (self.member(user).is_some() || self.people().any(|u| u == user)).then_some(user.as_str())
    }

    /// The conversation that shows the mail `from` sent: their own row, or
    /// the row of the user they are trusted as. `None` is the yard row: it
    /// has the yard's own mail and that of anyone left without a row, so
    /// that no mail is nowhere.
    fn row_of<'a>(&'a self, from: &'a str) -> Option<&'a str> {
        if let Some(user) = self.folded(from) {
            return Some(user);
        }
        let row = self.member(from).is_some()
            || self.people().any(|u| u == from)
            || self.party(from).is_some();
        row.then_some(from)
    }

    /// Whether the open row shows the mail `from` sent.
    fn shows(&self, from: &str) -> bool {
        match self.selected.as_ref() {
            Some(Selected::Conversation(name)) => self.row_of(from) == Some(name.as_str()),
            Some(Selected::Yard) => self.row_of(from).is_none(),
            _ => false,
        }
    }

    /// Whose mail the row `sel` shows, each a party a conversation is read
    /// with. A conversation's is the one it is with, and in a user's row
    /// each name trusted as them; the yard row's is the yard and whoever
    /// wrote and has no row.
    fn sources(&self, sel: &Selected) -> Vec<String> {
        match sel {
            Selected::Conversation(name) => std::iter::once(name.clone())
                .chain(
                    self.trust
                        .keys()
                        .filter(|a| self.folded(a) == Some(name.as_str()))
                        .cloned(),
                )
                .collect(),
            Selected::Yard => {
                let mut rest: Vec<&String> = self
                    .unread
                    .values()
                    .map(|w| &w.from)
                    .chain(self.convo.keys())
                    .filter(|from| *from != YARD && self.row_of(from).is_none())
                    .collect();
                rest.sort();
                rest.dedup();
                let yard = self.row_of(YARD).is_none().then(|| YARD.to_string());
                yard.into_iter().chain(rest.into_iter().cloned()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The seq from which on the row `sel` has all its mail. A row of
    /// several conversations is read a page of each, and the pages do not
    /// reach equally far back: what is older than the oldest mail of one
    /// that has older still may have mail of that one missing in between,
    /// and is not shown until it was asked for.
    fn horizon(&self, sel: &Selected) -> u64 {
        self.sources(sel)
            .iter()
            .filter(|name| self.older.contains(*name))
            .filter_map(|name| self.convo.get(name)?.first())
            .map(|l| l.seq)
            .max()
            .unwrap_or(0)
    }

    /// The mail the row `sel` shows, oldest first.
    fn letters(&self, sel: &Selected) -> Vec<&Letter> {
        let horizon = self.horizon(sel);
        let mut out: Vec<&Letter> = self
            .sources(sel)
            .iter()
            .filter_map(|name| self.convo.get(name))
            .flatten()
            .filter(|l| l.seq >= horizon)
            .collect();
        out.sort_by_key(|l| l.seq);
        out
    }

    /// The beads with open asks, each with its asks oldest first, in the
    /// order their oldest ask waits.
    fn decision_beads(&self) -> Vec<(String, Vec<&OpenAsk>)> {
        let mut beads: Vec<(String, Vec<&OpenAsk>)> = Vec::new();
        for ask in &self.open {
            match beads.iter_mut().find(|(bead, _)| bead == &ask.bead) {
                Some((_, asks)) => asks.push(ask),
                None => beads.push((ask.bead.clone(), vec![ask])),
            }
        }
        beads
    }

    /// Every row of one mode's list from the top: the conversations and
    /// the yard row in chat, the beads with open asks with the dealt with
    /// when "Decided" stands open and the grants last where the window is
    /// the operator's in decisions, the arrivals row, the lines in the
    /// served order and No line last in timetable, while unassigned beads
    /// travel. A travelling bead is open from the board, a flow,
    /// Elsewhere or No line, and has no row here.
    fn order_in(&self, mode: Mode) -> Vec<Selected> {
        let mut out: Vec<Selected> = Vec::new();
        match mode {
            Mode::Chat => {
                out.extend(
                    self.conversation_names()
                        .into_iter()
                        .map(Selected::Conversation),
                );
                out.push(Selected::Yard);
            }
            Mode::Decisions => {
                for (bead, _) in self.decision_beads() {
                    out.push(Selected::Decision(bead));
                }
                if self.decided_expanded {
                    for ask in self.visible_decided() {
                        out.push(Selected::Decided(ask.bead.clone(), ask.ask));
                    }
                }
                if self.is_operator() {
                    out.push(Selected::Grants);
                }
            }
            Mode::Timetable => {
                out.push(Selected::Arrivals);
                out.extend(
                    self.flows
                        .depots
                        .iter()
                        .filter(|depot| depot.travelling > 0)
                        .map(|depot| Selected::Line(depot.name.clone())),
                );
                if !self.flows.unassigned.is_empty() {
                    out.push(Selected::NoLine);
                }
                if self.quiet_expanded {
                    out.extend(
                        self.flows
                            .depots
                            .iter()
                            .filter(|depot| depot.travelling == 0)
                            .map(|depot| Selected::Line(depot.name.clone())),
                    );
                }
            }
        }
        out
    }

    /// Every row of every list: what a dealt-with decision leads back
    /// through, the open decisions first.
    fn order(&self) -> Vec<Selected> {
        [Mode::Decisions, Mode::Chat, Mode::Timetable]
            .into_iter()
            .flat_map(|mode| self.order_in(mode))
            .collect()
    }

    fn member(&self, name: &str) -> Option<&CrewMember> {
        self.crew.iter().find(|r| r.name == name)
    }

    /// What the open conversation's header says beside the name, for a
    /// crew member of this yard: what its session does. None for anyone
    /// else: people and a linked yard's people have no state, as the
    /// rows have it.
    pub fn crew_state(&self, name: &str) -> Option<&'static str> {
        self.member(name).map(state_of)
    }

    /// The one of a linked yard with the address, when a peer exposes it.
    pub fn party(&self, address: &str) -> Option<&Party> {
        self.parties.iter().find(|p| p.address() == address)
    }

    /// The conversation `asker` is written to in, when there is one: with
    /// one of this yard's crew or its users, with one a linked yard
    /// exposes, or with the user such a one is trusted as. Anyone else has
    /// no conversation.
    pub fn conversation_with<'a>(&'a self, asker: &'a str) -> Option<&'a str> {
        self.row_of(asker)
    }

    /// Whether `asker` can be written to from here.
    pub fn can_write(&self, asker: &str) -> bool {
        self.row_of(asker).is_some()
    }

    /// Why `asker` cannot be written to from here, when they cannot: a
    /// name the link does not expose, or none of this yard's crew or users.
    pub fn unwritable(&self, asker: &str) -> Option<String> {
        if self.can_write(asker) {
            return None;
        }
        match asker.split_once('@') {
            Some(_) => Some(format!(
                "{asker} cannot be written to from here: the link does not expose them"
            )),
            None => Some(format!(
                "{asker} cannot be written to from here: no member of this yard's crew and no user of it is called that"
            )),
        }
    }

    /// How many mails of the row `name` the user has not seen. Mail that
    /// was shown does not count, also while the yard has it unread still:
    /// one position stands for all senders, so unseen mail of another holds
    /// the acknowledgement back, and the count would stay on a row with
    /// nothing left to read.
    pub fn unread(&self, name: &str) -> usize {
        self.unread
            .values()
            .filter(|w| !w.seen && self.row_of(&w.from) == Some(name))
            .count()
    }

    /// The conversations of the list on the left: the crew, the linked
    /// yards' people, and this yard's people, in that order.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = self.crew_rows();
        rows.extend(self.linked_rows());
        rows.extend(self.people_rows());
        rows
    }

    /// This yard's crew, with their states and dots, for the sidebar's
    /// first group.
    pub fn crew_rows(&self) -> Vec<Row> {
        self.crew
            .iter()
            .map(|r| {
                self.conversation_row(r.name.clone(), state_of(r).to_string(), Some(dot_of(r)))
            })
            .collect()
    }

    /// Those a linked yard exposes, for the sidebar's second group. One
    /// folded into a user's row stands there, not here.
    pub fn linked_rows(&self) -> Vec<Row> {
        self.parties
            .iter()
            .filter(|p| self.folded(&p.address()).is_none())
            .map(|p| {
                let is = if p.person { "person" } else { "crew member" };
                let line = if p.role.is_empty() {
                    is.to_string()
                } else {
                    format!("{is}: {}", plain(&p.role))
                };
                self.conversation_row(p.address(), line, None)
            })
            .collect()
    }

    /// This yard's users who are no crew member, for the sidebar's third
    /// group.
    pub fn people_rows(&self) -> Vec<Row> {
        self.people()
            .map(|u| {
                let line = if *u == self.me {
                    "you, notes to self"
                } else {
                    "person"
                };
                self.conversation_row(u.clone(), line.to_string(), None)
            })
            .collect()
    }

    /// One conversation's row: its unread mail, whether it is open, and
    /// the dot beside its name, if any.
    fn conversation_row(&self, name: String, line: String, dot: Option<Dot>) -> Row {
        Row {
            unread: self.unread(&name),
            selected: self.selected_conversation() == Some(name.as_str()),
            name,
            line,
            dot,
        }
    }

    /// How many beads have open asks: the Decisions head's count.
    pub fn decisions_count(&self) -> usize {
        self.decision_beads().len()
    }

    /// The open asks above the conversations, grouped by topic, each group
    /// in the order its oldest ask waits. A bead with several open asks is
    /// one row.
    pub fn decision_groups<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> Vec<TopicGroup>
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut groups: Vec<TopicGroup> = Vec::new();
        for (bead, asks) in self.decision_beads() {
            let first = asks[0];
            let cached = self.beads.get(&bead);
            let options_of = |ask: &OpenAsk| {
                cached
                    .and_then(|b| b.asks.iter().find(|full| full.ask == ask.ask))
                    .map_or(0, |full| full.options.len())
            };
            // One ask is the row: its head and sub say it, and nothing
            // stands expanded beneath. Several expand, each with its own
            // question and line.
            let expanded = asks.len() > 1;
            let row = DecisionRow {
                bead: bead.clone(),
                head: decision_head(first, &asks),
                asks: asks
                    .iter()
                    .filter(|_| expanded)
                    .map(|ask| DecisionRowAsk {
                        ask: ask.ask,
                        head: plain(&ask.question),
                        meta: decision_meta(ask, options_of(ask), now),
                    })
                    .collect(),
                sub: decision_sub(first, &asks, options_of(first), now),
                short: short_bead(&bead),
                selected: self.selected.as_ref() == Some(&Selected::Decision(bead)),
            };
            let topic = display_topic(&first.topic);
            match groups.iter_mut().find(|g| g.topic == topic) {
                Some(g) => g.rows.push(row),
                None => groups.push(TopicGroup {
                    topic,
                    rows: vec![row],
                }),
            }
        }
        groups
    }

    /// How many asks were dealt with: the folded "Decided (n)".
    pub fn decided_count(&self) -> usize {
        self.decided.len()
    }

    pub fn decided_expanded(&self) -> bool {
        self.decided_expanded
    }

    pub fn toggle_decided(&mut self) {
        self.decided_expanded = !self.decided_expanded;
    }

    /// Whether the timetable's quiet lines stand unfolded.
    pub fn quiet_expanded(&self) -> bool {
        self.quiet_expanded
    }

    pub fn toggle_quiet(&mut self) {
        self.quiet_expanded = !self.quiet_expanded;
    }

    /// Whether the arrivals pane shows the departures board: the head's
    /// toggle, the panel where false.
    pub fn departures(&self) -> bool {
        self.departures
    }

    /// The head's toggle sets which arrivals view stands: the departures
    /// board where true, the panel where false.
    pub fn set_departures(&mut self, departures: bool) {
        self.departures = departures;
    }

    /// The arrivals board as read: the rows as served, ordered and
    /// remarked.
    pub fn arrival_rows(&self) -> &[Arrival] {
        &self.arrivals
    }

    /// The arrivals board as read again. The yard has the call, so it
    /// is not too old for it.
    pub fn arrivals(&mut self, board: Arrivals) {
        self.arrivals = board.rows;
        self.arrivals_too_old = false;
    }

    /// Whether the yard has no Arrivals call: one from before the Lines
    /// view, which the Lines pane says.
    pub fn arrivals_too_old(&self) -> bool {
        self.arrivals_too_old
    }

    /// The arrivals board could not be read again. A yard without the
    /// call has no rows to show: the Lines pane says it is too old, and
    /// no notice is raised. Anything else is said as a notice, as ever.
    pub fn arrivals_failed(&mut self, err: &api::Error) {
        match lines_too_old(err) {
            true => {
                self.arrivals.clear();
                self.arrivals_too_old = true;
            }
            false => {
                self.notice = format!("the arrivals could not be read: {err}");
            }
        }
    }

    /// One bead's route, as the details' strip shows it: the stages in
    /// travel order of the served flow the bead stands in, with which
    /// one it stands at. None where the bead stands in no stage of a
    /// served flow: Elsewhere, No line, or a flow the yard reads no
    /// longer. The arrivals answer carries no stages of its own, so the
    /// strip reads them here, where every arrival row is placed.
    pub fn route_for(&self, bead: &str) -> Option<Route> {
        for depot in &self.flows.depots {
            for flow in &depot.flows {
                let at = flow
                    .stages
                    .iter()
                    .position(|s| s.beads.iter().any(|b| b.bead == bead));
                if let Some(current) = at {
                    return Some(Route {
                        stages: flow
                            .stages
                            .iter()
                            .map(|s| RouteStage {
                                name: s.stage.clone(),
                                human: s.human,
                            })
                            .collect(),
                        current,
                    });
                }
            }
        }
        None
    }

    /// The beads that wait on the person reading, in the board's order
    /// first and then the complete Flows order. Arrivals is capped, so
    /// the flows also supply waits beyond the rows the table can show. A
    /// bead waits on them where its remark names them alone, or where
    /// they have an open ask on it; a wait naming several people counts
    /// only with an open ask on that bead.
    pub fn waiting_on_you(&self) -> Vec<(&str, &str)> {
        fn include<'a>(
            waiting: &mut Vec<(&'a str, &'a str)>,
            seen: &mut HashSet<&'a str>,
            me: &str,
            asked: &HashSet<&str>,
            bead: &'a str,
            title: &'a str,
            remark: &str,
        ) {
            if (waits_on_you_alone(remark, me) || asked.contains(bead)) && seen.insert(bead) {
                waiting.push((bead, title));
            }
        }

        let asked: HashSet<&str> = self.open.iter().map(|ask| ask.bead.as_str()).collect();
        let mut waiting = Vec::new();
        let mut seen = HashSet::new();
        for row in &self.arrivals {
            include(
                &mut waiting,
                &mut seen,
                &self.me,
                &asked,
                &row.bead,
                &row.title,
                &row.remark,
            );
        }
        for bead in self.flow_beads() {
            include(
                &mut waiting,
                &mut seen,
                &self.me,
                &asked,
                &bead.bead,
                &bead.title,
                &bead.remark,
            );
        }
        waiting
    }

    /// Every bead of the Flows view: each stage's in travel order, each
    /// depot's Elsewhere after its flows, and those with no line last,
    /// all in the served order.
    fn flow_beads(&self) -> impl Iterator<Item = &FlowBead> {
        self.flows
            .depots
            .iter()
            .flat_map(|depot| {
                depot
                    .flows
                    .iter()
                    .flat_map(|flow| flow.stages.iter())
                    .flat_map(|stage| stage.beads.iter())
                    .chain(depot.elsewhere.iter())
            })
            .chain(self.flows.unassigned.iter())
    }

    /// How many of the Flows beads are being worked: the panel's
    /// summary, beside what waits on the reader.
    pub fn working_count(&self) -> usize {
        self.flow_beads().filter(|bead| bead.working).count()
    }

    /// How the worked Flows beads split by served kind, and which
    /// groups work them with their seats: the panel's head line and
    /// its tooltip. Kinds count the worked beads with a kind served,
    /// most first with ties by name; worked beads with no kind (a
    /// command or a peer send) count in the total alone. Groups are
    /// the worked beads' groups, most beads first with ties by name,
    /// with the seats their stages serve; the yard serves one taken
    /// number across the whole yard, so the first stage naming the
    /// group speaks for all. The count matches the underlying worked
    /// rows, not the visible dots (a stage shows at most two headcodes
    /// then +N), so a worked bead hidden by overflow counts.
    pub fn working_split(&self) -> WorkingSplit {
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        let mut groups: BTreeMap<&str, usize> = BTreeMap::new();
        for bead in self.flow_beads().filter(|bead| bead.working) {
            if !bead.kind.is_empty() {
                *kinds.entry(bead.kind.as_str()).or_default() += 1;
            }
            if !bead.group.is_empty() {
                *groups.entry(bead.group.as_str()).or_default() += 1;
            }
        }
        let mut kinds: Vec<(String, usize)> = kinds
            .into_iter()
            .map(|(kind, n)| (kind.to_string(), n))
            .collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut seats: HashMap<&str, (Option<usize>, Option<usize>)> = HashMap::new();
        for depot in &self.flows.depots {
            for flow in &depot.flows {
                for stage in &flow.stages {
                    if !stage.group.is_empty() {
                        seats
                            .entry(stage.group.as_str())
                            .or_insert((stage.limit, stage.taken));
                    }
                }
            }
        }
        let mut counted: Vec<(&str, usize)> = groups.into_iter().collect();
        counted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let groups = counted
            .into_iter()
            .map(|(name, _)| {
                let (limit, taken) = seats.get(name).copied().unwrap_or((None, None));
                WorkingGroup {
                    name: name.to_string(),
                    limit,
                    taken,
                }
            })
            .collect();
        WorkingSplit { kinds, groups }
    }

    /// How many of the Flows beads are held or stalled: the red tone,
    /// the panel's summary beside what is worked and what waits.
    pub fn held_count(&self) -> usize {
        self.flow_beads().filter(|bead| bead.tone == "red").count()
    }

    /// The yard-wide travelling total the Flows call serves: the open
    /// beads outside the backlog, complete, not capped.
    pub fn travelling(&self) -> usize {
        self.flows.travelling
    }

    /// The flows view as read again. A line or No line that is gone
    /// falls back to the board; a travelling bead is never gone. The yard
    /// has the call, so it is not too old for it.
    pub fn flows(&mut self, board: Flows) -> Vec<Effect> {
        self.flows = board;
        self.flows_too_old = false;
        self.reselect()
    }

    /// Whether the yard has no Flows call: one from before the Lines
    /// view, which the Lines pane says.
    pub fn flows_too_old(&self) -> bool {
        self.flows_too_old
    }

    /// The flows view could not be read again. A yard without the call
    /// has no rows to show: the Lines pane says it is too old, and no
    /// notice is raised. An open line falls back to the board. Anything
    /// else is said as a notice, as ever.
    pub fn flows_failed(&mut self, err: &api::Error) -> Vec<Effect> {
        match lines_too_old(err) {
            true => {
                self.flows = Flows::default();
                self.flows_too_old = true;
                self.reselect()
            }
            false => {
                self.notice = format!("the flows could not be read: {err}");
                Vec::new()
            }
        }
    }

    /// Every grant of the yard's sources, as read: who may call what of
    /// each, and as whom.
    pub fn grant_records(&self) -> &[SourceGrant] {
        &self.grants
    }

    /// The grants as read again. The yard has the call and serves it to
    /// this window, so it is neither too old for it nor read elsewhere.
    pub fn grants(&mut self, grants: Vec<SourceGrant>) {
        self.grants = grants;
        self.grants_too_old = false;
        self.grants_on_the_yard = false;
    }

    /// Whether the yard has no SourceGrants call: one from before the
    /// grants view, which the grants pane says.
    pub fn grants_too_old(&self) -> bool {
        self.grants_too_old
    }

    /// Whether the grants are read on the yard's machine: the yard serves
    /// them down no link, and this window comes by one. The grants pane
    /// says so.
    pub fn grants_on_the_yard(&self) -> bool {
        self.grants_on_the_yard
    }

    /// The call for the grants where the yard serves them to this window,
    /// and none where they are read on its machine.
    fn read_grants(&self) -> Vec<Effect> {
        match self.grants_on_the_yard {
            true => Vec::new(),
            false => vec![Effect::Grants],
        }
    }

    /// The grants could not be read again. A yard without the call has
    /// none to show: the grants pane says it is too old, and no notice
    /// is raised. A yard that says the request came down the link has
    /// none to give this window: the pane says they are read on its
    /// machine, again with no notice. Anything else is said as a notice,
    /// as ever.
    pub fn grants_failed(&mut self, err: &api::Error) {
        match grants_too_old(err) {
            true => {
                self.grants.clear();
                self.grants_too_old = true;
            }
            false if grants_over_the_link(err) => {
                self.grants.clear();
                self.grants_on_the_yard = true;
            }
            false => {
                self.notice = format!("the grants could not be read: {err}");
            }
        }
    }

    /// Whether the settings offer the Telemetry section: the window runs
    /// as the yard's operator, and the yard serves the calls to such a
    /// window. A read that failed is offered still: the section says so,
    /// and tries again.
    pub fn telemetry_offered(&self) -> bool {
        self.is_operator() && !self.telemetry_unserved
    }

    /// The telemetry preview as read: the switch, the install id and the
    /// next request. `None` where none was read.
    pub fn telemetry_preview(&self) -> Option<&TelemetryPreview> {
        self.telemetry.as_ref()
    }

    /// Why the telemetry preview is empty, in the window's words; `None`
    /// where it was read, or the yard serves no such call here.
    pub fn telemetry_notice(&self) -> Option<&str> {
        self.telemetry_notice.as_deref()
    }

    /// Whether the switch is read-only in this window: the yard refused
    /// its change from where this window reaches it.
    pub fn telemetry_read_only(&self) -> bool {
        self.telemetry_read_only
    }

    /// The preview as read again. The yard has the calls and serves them
    /// to this window, so it is neither unserved nor unread: a preview
    /// read again clears both, and keeps the section read-only where the
    /// switch's change was refused.
    pub fn telemetry(&mut self, preview: TelemetryPreview) {
        self.telemetry = Some(preview);
        self.telemetry_notice = None;
        self.telemetry_unserved = false;
    }

    /// The call for the preview where the yard serves it to this window,
    /// and none where it serves no such call here.
    fn read_telemetry(&self) -> Vec<Effect> {
        match self.telemetry_unserved {
            true => Vec::new(),
            false => vec![Effect::Telemetry],
        }
    }

    /// The preview could not be read again. A yard without the calls, or
    /// one that says the request came down the link, has none to give
    /// this window: the section is not offered, and no notice is raised.
    /// Anything else drops the old preview and is said in the section,
    /// which tries again.
    pub fn telemetry_failed(&mut self, err: &api::Error) {
        match telemetry_too_old(err) || telemetry_over_the_link(err) {
            true => {
                self.telemetry = None;
                self.telemetry_notice = None;
                self.telemetry_unserved = true;
            }
            false => {
                self.telemetry = None;
                self.telemetry_notice =
                    Some(format!("the telemetry switch could not be read: {err}"));
            }
        }
    }

    /// The switch's change was refused. One refused from where this window
    /// reaches the yard is read-only here: the section keeps the state,
    /// the id and the payload, and says the switch is changed in the
    /// yard's web view. One refused down the link, or one with no such
    /// call, hides the section, as a preview refused so does. Whether
    /// either was: anything else is the window's to say of the press.
    pub fn telemetry_set_refused(&mut self, err: &api::Error) -> bool {
        if telemetry_too_old(err) || telemetry_over_the_link(err) {
            self.telemetry = None;
            self.telemetry_notice = None;
            self.telemetry_unserved = true;
            return true;
        }
        if telemetry_router_refused(err) {
            self.telemetry_read_only = true;
            return true;
        }
        false
    }

    /// One line's flows as served, when the yard serves the line.
    pub fn depot(&self, name: &str) -> Option<&FlowDepot> {
        self.flows.depots.iter().find(|d| d.name == name)
    }

    /// The lines of the Flows view in the served order, travelling or
    /// not, as the panel draws their tracks.
    pub fn depots(&self) -> &[FlowDepot] {
        &self.flows.depots
    }

    /// The beads travelling with no line, newest change first, as served.
    pub fn unassigned(&self) -> &[FlowBead] {
        &self.flows.unassigned
    }

    /// How many of the person's beads were added or closed since they
    /// last had Timetable open: the rail's Lines badge.
    pub fn timetable_news(&self) -> usize {
        self.timetable_news.len()
    }

    /// Whether the person looks at Timetable now: its mode open on the
    /// shown yard.
    fn looking(&self) -> bool {
        !self.aside && self.mode == Mode::Timetable
    }

    /// The yard was read at `head`, Timetable last open at `seen`: what
    /// the dot starts from. A yard never looked at starts from the head,
    /// with no dot: the first run lights nothing. The answer is what the
    /// catch-up reads from, where it is behind the head.
    pub fn adopt_timetable_seen(&mut self, seen: Option<u64>, head: u64) -> u64 {
        self.timetable_looked = seen.map_or(head, |looked| looked.min(head));
        self.timetable_looked
    }

    /// Timetable was opened or left: what was seen is the newest event
    /// applied. Opening clears the news; the answer is kept in the
    /// settings, per yard.
    pub fn timetable_visited(&mut self, open: bool) -> u64 {
        if open {
            self.timetable_news.clear();
        }
        self.timetable_looked = self.events_seen;
        self.timetable_looked
    }

    /// The catch-up over the gap to `until` found the person's beads in
    /// `news` added or closed: the badge, unless Timetable is open, which
    /// saw them instead.
    pub fn timetable_seeded(&mut self, news: Vec<String>, until: u64) {
        // Timetable may have opened while this answer was in flight. Its
        // visit advanced the watermark through the same gap, so a late
        // answer must not bring those changes back as news.
        if until <= self.timetable_looked {
            return;
        }
        match self.looking() {
            true => self.timetable_looked = self.timetable_looked.max(until),
            false => self.timetable_news.extend(news),
        }
    }

    /// The timetable row a travelling bead was opened from: the board, a
    /// line or No line. The board where none was, or where the line is
    /// gone or nothing travels without one anymore.
    pub fn bead_back(&self) -> Selected {
        match self.bead_from.as_ref() {
            Some(Selected::Line(name)) if self.depot(name).is_some() => {
                Selected::Line(name.clone())
            }
            Some(Selected::NoLine) if !self.flows.unassigned.is_empty() => Selected::NoLine,
            Some(Selected::Arrivals) => Selected::Arrivals,
            _ => Selected::Arrivals,
        }
    }

    /// The arrivals row of the timetable list: with the yard-wide
    /// travelling total the Flows call serves. It stands open for a
    /// travelling bead from the board too, which has no row of its own.
    pub fn arrivals_row(&self) -> Row {
        Row {
            name: ARRIVALS.to_string(),
            line: transit_words(self.flows.travelling),
            unread: 0,
            selected: matches!(self.selected.as_ref(), Some(Selected::Arrivals))
                || matches!(self.selected.as_ref(), Some(Selected::Bead(_)))
                    && self.bead_back() == Selected::Arrivals,
            dot: None,
        }
    }

    /// The lines of the timetable list in the served order, each with
    /// its served travelling total. Every served line stands here,
    /// travelling or not. A line stands open for a travelling bead from
    /// its flows or its Elsewhere too.
    pub fn line_rows(&self) -> Vec<Row> {
        self.flows
            .depots
            .iter()
            .map(|depot| self.line_row(depot))
            .collect()
    }

    /// One line's row: its served travelling total, standing open for a
    /// travelling bead from its flows or its Elsewhere too.
    fn line_row(&self, depot: &FlowDepot) -> Row {
        let back = Selected::Line(depot.name.clone());
        Row {
            name: depot.name.clone(),
            line: line_transit_words(depot.travelling),
            unread: 0,
            selected: self.selected.as_ref() == Some(&back)
                || matches!(self.selected.as_ref(), Some(Selected::Bead(_)))
                    && self.bead_back() == back,
            dot: None,
        }
    }

    /// The timetable's active lines in the served order: those with
    /// beads in transit, each with its count. The quiet ones stand
    /// folded under Quiet.
    pub fn active_line_rows(&self) -> Vec<Row> {
        self.flows
            .depots
            .iter()
            .filter(|depot| depot.travelling > 0)
            .map(|depot| self.line_row(depot))
            .collect()
    }

    /// The timetable's quiet lines in the served order: those with
    /// nothing in transit. They stand under Quiet while it is unfolded.
    pub fn quiet_rows(&self) -> Vec<Row> {
        self.flows
            .depots
            .iter()
            .filter(|depot| depot.travelling == 0)
            .map(|depot| self.line_row(depot))
            .collect()
    }

    /// The No line row of the timetable list: with how many beads travel
    /// with no line. It stands last, and only while any travel. It stands
    /// open for a travelling bead from under it too.
    pub fn no_line_row(&self) -> Option<Row> {
        if self.flows.unassigned.is_empty() {
            return None;
        }
        Some(Row {
            name: NO_LINE.to_string(),
            line: transit_words(self.flows.unassigned.len()),
            unread: 0,
            selected: matches!(self.selected.as_ref(), Some(Selected::NoLine))
                || matches!(self.selected.as_ref(), Some(Selected::Bead(_)))
                    && self.bead_back() == Selected::NoLine,
            dot: None,
        })
    }

    /// A travelling bead's row of the flows view with the depot it stands
    /// in, `None` for one travelling with no line. The stages first, in
    /// the served order, then the depot's Elsewhere, then the unassigned.
    /// The window reads it for what Arrivals does not serve: who works
    /// the bead, on hover and in its details.
    pub fn flow_bead(&self, bead: &str) -> Option<(&FlowBead, Option<&str>)> {
        for depot in &self.flows.depots {
            for flow in &depot.flows {
                for stage in &flow.stages {
                    if let Some(found) = stage.beads.iter().find(|b| b.bead == bead) {
                        return Some((found, Some(depot.name.as_str())));
                    }
                }
            }
            if let Some(found) = depot.elsewhere.iter().find(|b| b.bead == bead) {
                return Some((found, Some(depot.name.as_str())));
            }
        }
        self.flows
            .unassigned
            .iter()
            .find(|b| b.bead == bead)
            .map(|found| (found, None))
    }

    /// The session whose screen opens for the bead: the one working it.
    /// The operator's alone to open: none where the window is not the
    /// operator's, where the yard reads no screen down the link this
    /// window comes by, where no session works the bead, or where the yard
    /// names none.
    pub fn screen_for_bead(&self, bead: &str) -> Option<String> {
        if !self.is_operator() || self.screen_on_the_yard {
            return None;
        }
        let (found, _) = self.flow_bead(bead)?;
        match found.worker.is_empty() {
            true => None,
            false => Some(worker_session(&found.worker).to_string()),
        }
    }

    /// The session whose screen opens for the crew member: their own.
    /// The operator's alone to open: none where the window is not the
    /// operator's, where the yard reads no screen down the link this
    /// window comes by, or where the name is none of this yard's crew.
    pub fn screen_for_crew(&self, name: &str) -> Option<String> {
        if !self.is_operator() || self.screen_on_the_yard {
            return None;
        }
        self.member(name).map(|member| member.name.clone())
    }

    /// Whether a session's screen is read on the yard's machine: the yard
    /// serves it down no link, and this window comes by one. The window
    /// offers no Screen then, and asks for none.
    pub fn screen_on_the_yard(&self) -> bool {
        self.screen_on_the_yard
    }

    /// The yard refused a screen as a request come down the link from its
    /// edge: it reads none for this window, whatever the client could tell
    /// of the way it comes. The Screen goes from the window.
    pub fn screen_refused_down_the_link(&mut self) {
        self.screen_on_the_yard = true;
    }

    /// One travelling bead whole, as the details pane shows it: what the
    /// Bead call returned, with the line, expected arrival, remark and
    /// holder of the row it came from. The board's row where the yard
    /// serves the bead still, else the flows view's; `no line` for a
    /// bead with none. None until the bead is read.
    pub fn arrival_detail(&self, bead: &str) -> Option<ArrivalDetail> {
        let b = self.beads.get(bead)?;
        let row = self.arrivals.iter().find(|r| r.bead == bead);
        let flowed = self.flow_bead(bead);
        let (line, expected, remark, tone, held_by) = match (row, flowed) {
            (Some(r), _) if !r.line.is_empty() => (
                r.line.clone(),
                r.expected.clone(),
                r.remark.clone(),
                r.tone.clone(),
                r.held_by.clone(),
            ),
            (Some(r), _) => (
                "no line".to_string(),
                r.expected.clone(),
                r.remark.clone(),
                r.tone.clone(),
                r.held_by.clone(),
            ),
            (None, Some((found, Some(depot)))) => (
                depot.to_string(),
                found.expected.clone(),
                found.remark.clone(),
                found.tone.clone(),
                found.held_by.clone(),
            ),
            (None, Some((found, None))) => (
                "no line".to_string(),
                found.expected.clone(),
                found.remark.clone(),
                found.tone.clone(),
                found.held_by.clone(),
            ),
            (None, None) => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        };
        let mut notes: Vec<&BeadNote> = b.notes.iter().collect();
        // The notes newest last. The yard serves them oldest first; the
        // order by seq stands even where it serves them otherwise, and a
        // stable sort keeps the served order within one seq.
        notes.sort_by_key(|n| n.seq);
        Some(ArrivalDetail {
            bead: b.id.clone(),
            title: b.title.clone(),
            stage: b.stage.clone(),
            line,
            expected,
            remark,
            tone,
            held_by,
            labels: b.labels.iter().map(|l| plain(l)).collect(),
            body: plain(&b.body),
            notes: notes
                .into_iter()
                .map(|n| CardNote {
                    author: n.author.clone(),
                    at: n.at.clone(),
                    text: plain(&n.text),
                    files: n.files.clone(),
                })
                .collect(),
            open_asks: b.asks.iter().filter(|a| a.state == "open").count(),
        })
    }

    /// The dealt with, newest first, with their picks.
    /// The history's chip: `None` shows every outcome.
    pub fn decided_chip(&self) -> Option<Outcome> {
        self.decided_chip
    }

    pub fn set_decided_chip(&mut self, chip: Option<Outcome>) {
        self.decided_chip = chip;
    }

    /// Types the history's filter. Whether it changed: the window draws
    /// again only then.
    pub fn set_decided_filter(&mut self, filter: &str) -> bool {
        if self.decided_filter == filter {
            return false;
        }
        self.decided_filter = filter.to_string();
        true
    }

    /// The folded history rows standing open, by their first ask.
    pub fn decided_folds(&self) -> &HashSet<(String, u64)> {
        &self.decided_folds
    }

    /// Opens a folded history row, or folds it shut again.
    pub fn toggle_decided_fold(&mut self, bead: &str, ask: u64) {
        let key = (bead.to_string(), ask);
        if !self.decided_folds.remove(&key) {
            self.decided_folds.insert(key);
        }
    }

    /// The folds of `asks` stand open too: those that did before the yard
    /// was left for another.
    pub fn show_folds(&mut self, asks: &HashSet<(String, u64)>) {
        self.decided_folds.extend(asks.iter().cloned());
    }

    /// How many dealt with asks the filter leaves for each chip: All with
    /// the four outcomes after it, in their order. The text counts, the
    /// chip does not: each chip says what choosing it would show.
    pub fn decided_chip_counts(&self) -> Vec<(Option<Outcome>, usize)> {
        let mut counts = vec![(None, 0)];
        counts.extend(Outcome::CHIPS.iter().map(|chip| (Some(*chip), 0)));
        for ask in &self.decided {
            if !decided_matches(ask, None, &self.decided_filter) {
                continue;
            }
            counts[0].1 += 1;
            let outcome = outcome_of(ask);
            if let Some((_, n)) = counts.iter_mut().find(|(chip, _)| *chip == Some(outcome)) {
                *n += 1;
            }
        }
        counts
    }

    /// The dealt with asks matching the chip, in the order the rows step
    /// through them. The text counts too; folds do not: every member
    /// stays in reach of the keys, and opening one opens its fold.
    fn visible_decided(&self) -> Vec<&DecidedAsk> {
        self.decided
            .iter()
            .filter(|ask| decided_matches(ask, self.decided_chip, &self.decided_filter))
            .collect()
    }

    /// The dealt with asks as the history shows them: matching the chip
    /// and the filter, grouped by the day they were asked — the yard
    /// serves no decided-at time — newest day first, look-alikes folded.
    /// A fold with the open ask in it stands open.
    pub fn decided_groups<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> Vec<DecidedGroup>
    where
        Tz::Offset: std::fmt::Display,
    {
        let zone = now.timezone();
        let today = now.date_naive();
        // First-appearance order is the served one, newest first; the sort
        // below keeps it within each day.
        let mut days: Vec<(Option<NaiveDate>, Vec<&DecidedAsk>)> = Vec::new();
        for ask in self.visible_decided() {
            let day = day_of(&ask.since, &zone);
            match days.iter_mut().find(|(d, _)| d == &day) {
                Some((_, asks)) => asks.push(ask),
                None => days.push((day, vec![ask])),
            }
        }
        // Newest day first; what has no day stands last, on its own.
        days.sort_by(|a, b| match (&a.0, &b.0) {
            (Some(a), Some(b)) => b.cmp(a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        days.into_iter()
            .map(|(day, asks)| DecidedGroup {
                day: decided_day(day, today),
                rows: self.fold_decided(asks, &zone),
            })
            .collect()
    }

    /// One day's dealt with asks as its rows: consecutive look-alikes as
    /// one folded row, the rest one by one.
    fn fold_decided<Tz: TimeZone>(&self, asks: Vec<&DecidedAsk>, zone: &Tz) -> Vec<DecidedEntry>
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut rows: Vec<DecidedEntry> = Vec::new();
        let mut run: Vec<&DecidedAsk> = Vec::new();
        let flush = |run: &mut Vec<&DecidedAsk>, rows: &mut Vec<DecidedEntry>| {
            if run.len() < 2 {
                rows.extend(
                    run.drain(..)
                        .map(|ask| DecidedEntry::One(self.decided_row(ask, zone))),
                );
                return;
            }
            let first = run[0];
            let members: Vec<DecidedRow> = run
                .drain(..)
                .map(|ask| self.decided_row(ask, zone))
                .collect();
            let open = self
                .decided_folds
                .contains(&(first.bead.clone(), first.ask))
                || members.iter().any(|row| row.selected);
            let outcome = outcome_of(first);
            rows.push(DecidedEntry::Fold(DecidedFold {
                bead: first.bead.clone(),
                ask: first.ask,
                head: row_head(&first.question),
                outcome: format!("{}{}", outcome.icon(), outcome.word()),
                detail: first.asker.clone(),
                open,
                rows: members,
            }));
        };
        for ask in asks {
            let alike = run.last().is_some_and(|last| look_alike(last, ask));
            if !alike && !run.is_empty() {
                flush(&mut run, &mut rows);
            }
            run.push(ask);
        }
        flush(&mut run, &mut rows);
        rows
    }

    /// One dealt with as its history row: the question's first line, the
    /// outcome's icon with its words, and whose it was with when it was
    /// asked.
    fn decided_row<Tz: TimeZone>(&self, ask: &DecidedAsk, zone: &Tz) -> DecidedRow
    where
        Tz::Offset: std::fmt::Display,
    {
        let outcome = outcome_of(ask);
        let words = decided_outcome(ask);
        // `left to <asker>` already names whose: the asker is not said
        // twice.
        let mut detail = match ask.state.as_str() {
            "answered" if ask.option_label == YOU_DECIDE => Vec::new(),
            _ => vec![ask.asker.as_str()],
        };
        let at = clock_time(&ask.since, zone);
        if !at.is_empty() {
            detail.push(&at);
        }
        DecidedRow {
            bead: ask.bead.clone(),
            ask: ask.ask,
            head: row_head(&ask.question),
            outcome: format!("{}{}", outcome.icon(), words),
            detail: detail.join(" · "),
            short: short_bead(&ask.bead),
            selected: self.selected.as_ref() == Some(&Selected::Decided(ask.bead.clone(), ask.ask)),
        }
    }

    /// One bead whole, as the cards read it, when it was read.
    fn bead_ask(&self, bead: &str, ask: u64) -> Option<&BeadAsk> {
        self.beads
            .get(bead)
            .and_then(|b| b.asks.iter().find(|a| a.ask == ask))
    }

    /// Which ask of the bead the keys act on: the one chosen with a click,
    /// else the oldest open one.
    fn current_of(&self, bead: &str, asks: &[&OpenAsk]) -> u64 {
        let first = asks.first().map(|a| a.ask).unwrap_or(0);
        self.current
            .get(bead)
            .copied()
            .filter(|seq| asks.iter().any(|a| a.ask == *seq))
            .unwrap_or(first)
    }

    /// A bead's open asks, oldest first, each as its card.
    pub fn decision_detail<Tz: TimeZone>(
        &self,
        bead: &str,
        now: &DateTime<Tz>,
    ) -> Option<DecisionDetail>
    where
        Tz::Offset: std::fmt::Display,
    {
        let (_, asks) = self.decision_beads().into_iter().find(|(b, _)| b == bead)?;
        let first = asks[0];
        let current = self.current_of(bead, &asks);
        let cached = self.beads.get(bead);
        let (body, notes, doc) = match cached {
            Some(b) => (
                plain(&b.body),
                b.notes
                    .iter()
                    .map(|n| CardNote {
                        author: n.author.clone(),
                        at: n.at.clone(),
                        text: plain(&n.text),
                        files: n.files.clone(),
                    })
                    .collect(),
                design_doc(&b.id, &b.body, &b.notes),
            ),
            None => (String::new(), Vec::new(), String::new()),
        };
        Some(DecisionDetail {
            bead: bead.to_string(),
            title: first.bead_title.clone(),
            topic: display_topic(&first.topic),
            body,
            notes,
            doc,
            asks: asks
                .iter()
                .map(|ask| {
                    let full = self.bead_ask(bead, ask.ask);
                    DecisionAsk {
                        ask: ask.ask,
                        question: card_question(
                            &ask.question,
                            &ask.asker,
                            ask.run.is_some() || ask.setup.is_some(),
                        ),
                        asker: ask.asker.clone(),
                        waits: waits(&ask.since, now),
                        recommend_label: full.map(recommend_label).unwrap_or_default(),
                        context: full.map_or(Vec::new(), |f| {
                            card_context(
                                &f.context,
                                &ask.asker,
                                ask.run.is_some() || ask.setup.is_some(),
                            )
                        }),
                        options: full.map_or(Vec::new(), card_options),
                        recommend: full.map_or(0, |f| f.recommend),
                        why: full.map_or(String::new(), |f| plain(&f.why)),
                        chosen: self
                            .chosen
                            .get(&(bead.to_string(), ask.ask))
                            .copied()
                            .unwrap_or(0),
                        current: ask.ask == current,
                        expanded: self.expanded.contains(&(bead.to_string(), ask.ask)),
                        about: full.map_or(Vec::new(), |f| f.about.clone()),
                        state: full.map_or("open".to_string(), |f| f.state.clone()),
                        secret: ask.secret.clone(),
                        run: ask.run.clone(),
                        setup: ask.setup.clone(),
                        identity: ask.identity.clone(),
                        weight: ask.weight.clone(),
                        account: self.account_choice(bead, ask.ask),
                    }
                })
                .collect(),
        })
    }

    /// One dealt with, as its card: the pick with the same options to pick
    /// again from there.
    pub fn decided_detail<Tz: TimeZone>(
        &self,
        bead: &str,
        ask: u64,
        zone: &Tz,
    ) -> Option<DecidedDetail>
    where
        Tz::Offset: std::fmt::Display,
    {
        let a = self
            .decided
            .iter()
            .find(|a| a.bead == bead && a.ask == ask)?;
        let full = self.bead_ask(bead, ask);
        let cached = self.beads.get(bead);
        let (body, notes, doc) = match cached {
            Some(b) => (
                plain(&b.body),
                b.notes
                    .iter()
                    .map(|n| CardNote {
                        author: n.author.clone(),
                        at: n.at.clone(),
                        text: plain(&n.text),
                        files: n.files.clone(),
                    })
                    .collect(),
                design_doc(&b.id, &b.body, &b.notes),
            ),
            None => (String::new(), Vec::new(), String::new()),
        };
        Some(DecidedDetail {
            bead: a.bead.clone(),
            title: a.bead_title.clone(),
            ask: a.ask,
            question: card_question(&a.question, &a.asker, a.run.is_some() || a.setup.is_some()),
            asker: a.asker.clone(),
            state: a.state.clone(),
            state_line: decided_state(a, zone),
            picked: a.option,
            pick: a.option_label.clone(),
            words: plain(&a.words),
            return_at: a.return_at.clone(),
            recommend_label: full.map(recommend_label).unwrap_or_default(),
            context: full.map_or(Vec::new(), |f| {
                card_context(&f.context, &a.asker, a.run.is_some() || a.setup.is_some())
            }),
            options: full.map_or(Vec::new(), card_options),
            recommend: full.map_or(0, |f| f.recommend),
            why: full.map_or(String::new(), |f| plain(&f.why)),
            chosen: self
                .chosen
                .get(&(bead.to_string(), ask))
                .copied()
                .unwrap_or(0),
            expanded: self.expanded.contains(&(bead.to_string(), ask)),
            about: full.map_or(Vec::new(), |f| f.about.clone()),
            body,
            notes,
            doc,
            secret: a.secret.clone(),
            run: a.run.clone(),
            setup: a.setup.clone(),
            identity: a.identity.clone(),
            weight: a.weight.clone(),
            account: self.account_choice(bead, ask),
            ran: a.ran.clone(),
        })
    }

    /// Makes `ask` the bead's current one: the keys act on it. An ask that
    /// is not open on the bead changes nothing.
    pub fn select_ask(&mut self, bead: &str, ask: u64) {
        if self.open.iter().any(|a| a.bead == bead && a.ask == ask) {
            self.current.insert(bead.to_string(), ask);
        }
    }

    /// The bead's current ask, when a decision row is open.
    pub fn current_ask(&self) -> Option<(String, u64)> {
        let Selected::Decision(bead) = self.selected.as_ref()? else {
            return None;
        };
        Some((bead.clone(), self.current_for(bead)?))
    }

    /// Which ask of the bead the keys act on, when it has open asks.
    pub fn current_for(&self, bead: &str) -> Option<u64> {
        let (_, asks) = self.decision_beads().into_iter().find(|(b, _)| b == bead)?;
        Some(self.current_of(bead, &asks))
    }

    /// Chooses an option with a number key. One past the last changes
    /// nothing, as does an ask the bead does not show, and one taken back.
    pub fn choose(&mut self, bead: &str, ask: u64, option: u64) {
        if option == 0 || self.taken_back(bead, ask) {
            return;
        }
        let last = match self.bead_ask(bead, ask) {
            Some(full) => full.options.len() as u64,
            None => return,
        };
        if option <= last {
            self.chosen.insert((bead.to_string(), ask), option);
        }
    }

    /// Leaves the pick: nothing is chosen.
    pub fn clear_choice(&mut self, bead: &str, ask: u64) {
        self.chosen.remove(&(bead.to_string(), ask));
    }

    /// Moves the chosen option with the arrows, around the ends: down from
    /// none chooses the first, up from none the last.
    pub fn move_choice(&mut self, bead: &str, ask: u64, by: isize) {
        let last = match self.bead_ask(bead, ask) {
            Some(full) => full.options.len() as u64,
            None => return,
        };
        if last == 0 || self.taken_back(bead, ask) {
            return;
        }
        let key = (bead.to_string(), ask);
        let at = self.chosen.get(&key).copied().unwrap_or(0) as isize;
        let next = if at == 0 {
            if by >= 0 { 1 } else { last as isize }
        } else {
            (at - 1 + by).rem_euclid(last as isize) + 1
        };
        self.chosen.insert(key, next as u64);
    }

    /// Expands the card in place to the bead's body, notes and about-beads,
    /// or folds it again.
    pub fn toggle_expand(&mut self, bead: &str, ask: u64) {
        let key = (bead.to_string(), ask);
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    /// The asks whose card stands expanded, by bead and ask.
    pub fn expanded(&self) -> &HashSet<(String, u64)> {
        &self.expanded
    }

    /// The cards of `asks` stand expanded too: those that did before the
    /// yard was left for another.
    pub fn expand(&mut self, asks: &HashSet<(String, u64)>) {
        self.expanded.extend(asks.iter().cloned());
    }

    /// Shows the run ask's carried texts behind the yard's sentences, or
    /// hides them again.
    pub fn toggle_texts(&mut self, bead: &str, ask: u64) {
        let key = (bead.to_string(), ask);
        if !self.texts.remove(&key) {
            self.texts.insert(key);
        }
    }

    /// Whether the run ask's carried texts stand shown behind the
    /// yard's sentences.
    pub fn texts_shown(&self, bead: &str, ask: u64) -> bool {
        self.texts.contains(&(bead.to_string(), ask))
    }

    /// The run asks whose carried texts stand shown, by bead and ask.
    pub fn texts(&self) -> &HashSet<(String, u64)> {
        &self.texts
    }

    /// The carried texts of `asks` stand shown too: those that did before
    /// the yard was left for another.
    pub fn show_texts(&mut self, asks: &HashSet<(String, u64)>) {
        self.texts.extend(asks.iter().cloned());
    }

    /// One bead whole, as read. What is no longer open or dealt with leaves
    /// the cards it kept.
    pub fn bead_loaded(&mut self, bead: Bead) {
        self.beads.insert(bead.id.clone(), bead);
    }

    /// Whether the bead was read: the card reads its options with it.
    pub fn has_bead(&self, bead: &str) -> bool {
        self.beads.contains_key(bead)
    }

    /// The bead could not be read: the card says why.
    pub fn bead_failed(&mut self, bead: &str, why: &str) {
        self.notice = format!("the bead {bead} could not be read: {why}");
    }

    /// What Enter sends for the ask with the line's words: the option
    /// chosen, or the recommended one when none is, or words alone. `None`
    /// sends nothing: neither an option, a recommendation, nor words. One
    /// dealt with takes only a new pick: nothing is chosen by default, and
    /// Enter alone sends nothing, not even the recommendation. One taken
    /// back takes nothing at all: the yard refuses an answer to an ask that
    /// is asked no more.
    pub fn answer_action(&self, bead: &str, ask: u64, words: &str) -> Option<AnswerAction> {
        let full = self.bead_ask(bead, ask)?;
        if self.taken_back(bead, ask) {
            return None;
        }
        // A secret ask takes no words and no pick: its answer is typed in
        // its own field and goes in its own call.
        if self.secret(bead, ask).is_some() || full.secret.is_some() {
            return None;
        }
        // Nor does a run ask: Enter approves nothing, and no words or pick
        // stand in for the approval, which has a key and a call of its own.
        if self.run(bead, ask).is_some() || full.run.is_some() {
            return None;
        }
        // Nor a setup ask: its preset pick goes with the account in the
        // approval, which has a key and a call of its own.
        if self.setup(bead, ask).is_some() || full.setup.is_some() {
            return None;
        }
        let words = words.trim().to_string();
        // An ask with no options wants words: they are the answer.
        if full.options.is_empty() {
            return (!words.is_empty()).then_some(AnswerAction::Answer { option: 0, words });
        }
        let chosen = self
            .chosen
            .get(&(bead.to_string(), ask))
            .copied()
            .unwrap_or(0);
        if chosen != 0 {
            return Some(AnswerAction::Answer {
                option: chosen,
                words,
            });
        }
        if self.decided.iter().any(|a| a.bead == bead && a.ask == ask) {
            return None;
        }
        if full.recommend != 0 {
            return Some(AnswerAction::Answer {
                option: full.recommend,
                words,
            });
        }
        (!words.is_empty()).then_some(AnswerAction::Note { words })
    }

    /// A pick is answered: the ask moves to "Decided" at once, and a later
    /// pick on it from there replaces the earlier one for display. Answers
    /// the calls that read the queue and the bead again, for what the yard
    /// says.
    pub fn picked(&mut self, bead: &str, ask: u64, option: u64, words: &str) -> Vec<Effect> {
        let words = words.trim().to_string();
        let label = self
            .bead_ask(bead, ask)
            .and_then(|full| full.options.get(option.saturating_sub(1) as usize))
            .map(|o| o.label.clone())
            .unwrap_or_default();
        // A later pick on a dealt with replaces the earlier one.
        if let Some(dealt) = self
            .decided
            .iter_mut()
            .find(|a| a.bead == bead && a.ask == ask)
        {
            dealt.state = "answered".to_string();
            dealt.option = option;
            dealt.option_label = label;
            dealt.words = words;
            dealt.return_at.clear();
        } else if let Some(at) = self
            .open
            .iter()
            .position(|a| a.bead == bead && a.ask == ask)
        {
            let open = self.open.remove(at);
            self.decided.insert(
                0,
                DecidedAsk {
                    bead: open.bead,
                    bead_title: open.bead_title,
                    topic: open.topic,
                    ask: open.ask,
                    question: open.question,
                    asker: open.asker,
                    since: open.since,
                    state: "answered".to_string(),
                    option,
                    option_label: label,
                    words,
                    by: String::new(),
                    return_at: String::new(),
                    secret: open.secret,
                    concealed: open.concealed,
                    given: Vec::new(),
                    run: open.run,
                    setup: open.setup,
                    identity: open.identity,
                    weight: open.weight,
                    ran: None,
                },
            );
        } else {
            return Vec::new();
        }
        self.chosen.remove(&(bead.to_string(), ask));
        self.account.remove(&(bead.to_string(), ask));
        let mut effects = self.reselect();
        effects.push(Effect::Asks);
        effects.push(Effect::Bead {
            bead: bead.to_string(),
        });
        effects
    }

    /// What the ask asks for when it is a secret ask, open or dealt with.
    pub fn secret(&self, bead: &str, ask: u64) -> Option<&Secret> {
        let open = self.open.iter().filter(|a| a.bead == bead && a.ask == ask);
        let decided = self
            .decided
            .iter()
            .filter(|a| a.bead == bead && a.ask == ask);
        open.map(|a| &a.secret)
            .chain(decided.map(|a| &a.secret))
            .next()?
            .as_ref()
    }

    /// What the ask maps when it is a mapping ask, open or dealt with.
    pub fn identity(&self, bead: &str, ask: u64) -> Option<&Identity> {
        let open = self.open.iter().filter(|a| a.bead == bead && a.ask == ask);
        let decided = self
            .decided
            .iter()
            .filter(|a| a.bead == bead && a.ask == ask);
        open.map(|a| &a.identity)
            .chain(decided.map(|a| &a.identity))
            .next()?
            .as_ref()
    }

    /// Whether the ask's secret value is drawn hidden, open or dealt
    /// with. An ask the yard says nothing of, and one that is no ask
    /// here, is concealed.
    pub fn concealed(&self, bead: &str, ask: u64) -> bool {
        let open = self.open.iter().filter(|a| a.bead == bead && a.ask == ask);
        let decided = self
            .decided
            .iter()
            .filter(|a| a.bead == bead && a.ask == ask);
        open.map(|a| a.concealed)
            .chain(decided.map(|a| a.concealed))
            .next()
            .unwrap_or(true)
    }

    /// Whether its asker took the ask back: its card reads only, with
    /// nothing to pick and no line for words.
    pub fn taken_back(&self, bead: &str, ask: u64) -> bool {
        self.decided
            .iter()
            .any(|a| a.bead == bead && a.ask == ask && a.state == "taken back")
    }

    /// What a field on the ask's card takes: the secret of an ask that is
    /// open, put aside or given already, where giving again replaces. A
    /// declined one takes none, and neither does one taken back: the yard
    /// refuses a value for an ask that is asked no more.
    pub fn giving(&self, bead: &str, ask: u64) -> Option<&Secret> {
        let closed = self.decided.iter().any(|a| {
            a.bead == bead && a.ask == ask && matches!(a.state.as_str(), "declined" | "taken back")
        });
        self.secret(bead, ask).filter(|_| !closed)
    }

    /// The yard took the values of a secret ask and answered the names it
    /// wrote: the ask moves to "Decided" at once, as one that `gave` them,
    /// and the queue and the bead are read again for what the yard says.
    pub fn gave(&mut self, bead: &str, ask: u64, names: &[String]) -> Vec<Effect> {
        if let Some(dealt) = self
            .decided
            .iter_mut()
            .find(|a| a.bead == bead && a.ask == ask)
        {
            dealt.state = "answered".to_string();
            dealt.given = names.to_vec();
            dealt.return_at.clear();
        } else if let Some(at) = self
            .open
            .iter()
            .position(|a| a.bead == bead && a.ask == ask)
        {
            let open = self.open.remove(at);
            self.decided.insert(
                0,
                DecidedAsk {
                    bead: open.bead,
                    bead_title: open.bead_title,
                    topic: open.topic,
                    ask: open.ask,
                    question: open.question,
                    asker: open.asker,
                    since: open.since,
                    state: "answered".to_string(),
                    secret: open.secret,
                    concealed: open.concealed,
                    given: names.to_vec(),
                    identity: open.identity,
                    weight: open.weight,
                    ..DecidedAsk::default()
                },
            );
        } else {
            return Vec::new();
        }
        let mut effects = self.reselect();
        effects.push(Effect::Asks);
        effects.push(Effect::Bead {
            bead: bead.to_string(),
        });
        effects
    }

    /// The card's weight, one entry a class, when the ask is open or
    /// dealt with: empty on a routine card, and on one a yard from before
    /// cards were weighed asked.
    pub fn weight(&self, bead: &str, ask: u64) -> &[AskWeight] {
        let open = self.open.iter().filter(|a| a.bead == bead && a.ask == ask);
        let decided = self
            .decided
            .iter()
            .filter(|a| a.bead == bead && a.ask == ask);
        open.map(|a| a.weight.as_slice())
            .chain(decided.map(|a| a.weight.as_slice()))
            .next()
            .unwrap_or(&[])
    }

    /// The command the ask proposes when it is a run ask, open or dealt
    /// with.
    pub fn run(&self, bead: &str, ask: u64) -> Option<&Run> {
        let open = self.open.iter().filter(|a| a.bead == bead && a.ask == ask);
        let decided = self
            .decided
            .iter()
            .filter(|a| a.bead == bead && a.ask == ask);
        open.map(|a| &a.run)
            .chain(decided.map(|a| &a.run))
            .next()?
            .as_ref()
    }

    /// What the control on the ask's card approves: the command of a run
    /// ask that is open or put aside. One that was approved, declined or
    /// taken back has no control: a command runs once for one ask, and the
    /// yard refuses an approval of an ask that is asked no more.
    pub fn approving(&self, bead: &str, ask: u64) -> Option<&Run> {
        let closed = self
            .decided
            .iter()
            .any(|a| a.bead == bead && a.ask == ask && a.state != "put aside");
        self.run(bead, ask).filter(|_| !closed)
    }

    /// The yard approved a run ask and ran its command: the ask moves to
    /// "Decided" at once, as one that `ran` it, and the queue and the bead
    /// are read again for what the yard says. An approval the yard carries
    /// out after the call answers with `CARRY_PENDING`, and reads as being
    /// carried out until the queue is read again.
    pub fn approved(&mut self, bead: &str, ask: u64, ran: &api::Approved) -> Vec<Effect> {
        let ran = Some(Ran {
            argv: ran.argv.clone(),
            result: ran.result.clone(),
            failed: false,
        });
        if let Some(dealt) = self
            .decided
            .iter_mut()
            .find(|a| a.bead == bead && a.ask == ask)
        {
            dealt.state = "answered".to_string();
            dealt.ran = ran;
            dealt.return_at.clear();
        } else if let Some(at) = self
            .open
            .iter()
            .position(|a| a.bead == bead && a.ask == ask)
        {
            let open = self.open.remove(at);
            self.decided.insert(
                0,
                DecidedAsk {
                    bead: open.bead,
                    bead_title: open.bead_title,
                    topic: open.topic,
                    ask: open.ask,
                    question: open.question,
                    asker: open.asker,
                    since: open.since,
                    state: "answered".to_string(),
                    concealed: open.concealed,
                    run: open.run,
                    identity: open.identity,
                    weight: open.weight,
                    ran,
                    ..DecidedAsk::default()
                },
            );
        } else {
            return Vec::new();
        }
        let mut effects = self.reselect();
        effects.push(Effect::Asks);
        effects.push(Effect::Bead {
            bead: bead.to_string(),
        });
        effects
    }

    /// The setup the ask proposes when it is a setup ask, open or dealt
    /// with.
    pub fn setup(&self, bead: &str, ask: u64) -> Option<&Setup> {
        let open = self.open.iter().filter(|a| a.bead == bead && a.ask == ask);
        let decided = self
            .decided
            .iter()
            .filter(|a| a.bead == bead && a.ask == ask);
        open.map(|a| &a.setup)
            .chain(decided.map(|a| &a.setup))
            .next()?
            .as_ref()
    }

    /// What the control on the ask's card approves: the setup of a setup
    /// ask that is open or put aside. One that was approved, declined or
    /// taken back has no control: a setup runs once for one ask, and the
    /// yard refuses an approval of an ask that is asked no more.
    pub fn setup_approving(&self, bead: &str, ask: u64) -> Option<&Setup> {
        let closed = self
            .decided
            .iter()
            .any(|a| a.bead == bead && a.ask == ask && a.state != "put aside");
        self.setup(bead, ask).filter(|_| !closed)
    }

    /// The preset the approval picks: the option chosen, else the
    /// recommended one. None where the bead is not read yet, where the
    /// ask sets up nothing, or where nothing is chosen and nothing is
    /// recommended.
    pub fn setup_pick(&self, bead: &str, ask: u64) -> Option<u64> {
        let full = self.bead_ask(bead, ask)?;
        if full.options.is_empty() || full.setup.is_none() {
            return None;
        }
        let chosen = self
            .chosen
            .get(&(bead.to_string(), ask))
            .copied()
            .unwrap_or(0);
        let pick = if chosen != 0 { chosen } else { full.recommend };
        (pick != 0 && pick <= full.options.len() as u64).then_some(pick)
    }

    /// The yard approved a setup ask with the preset picked: the ask moves
    /// to "Decided" at once, and the queue and the bead are read again for
    /// what the yard says. The server carries the setup out after the
    /// call, so the approval reads as being carried out until the queue is
    /// read again. `option` is the preset pick the call carried.
    pub fn setup_approved(
        &mut self,
        bead: &str,
        ask: u64,
        option: u64,
        approved: &api::SetupApproved,
    ) -> Vec<Effect> {
        // `You decide` is an option on the card, but the yard records the
        // recommended preset it resolved to. Keep the dealt card on that
        // preset when the approval arrives, before the queue is read again.
        let option = self
            .setup(bead, ask)
            .and_then(|setup| {
                setup
                    .presets
                    .iter()
                    .position(|preset| preset.name == approved.preset)
            })
            .map_or(option, |preset| preset as u64 + 1);
        let ran = Some(Ran {
            argv: vec!["setup".to_string(), approved.pack.clone()],
            result: approved.result.clone(),
            failed: false,
        });
        if let Some(dealt) = self
            .decided
            .iter_mut()
            .find(|a| a.bead == bead && a.ask == ask)
        {
            dealt.state = "answered".to_string();
            dealt.option = option;
            dealt.option_label = approved.preset.clone();
            dealt.given = approved.given.clone();
            dealt.ran = ran;
            dealt.return_at.clear();
        } else if let Some(at) = self
            .open
            .iter()
            .position(|a| a.bead == bead && a.ask == ask)
        {
            let open = self.open.remove(at);
            self.decided.insert(
                0,
                DecidedAsk {
                    bead: open.bead,
                    bead_title: open.bead_title,
                    topic: open.topic,
                    ask: open.ask,
                    question: open.question,
                    asker: open.asker,
                    since: open.since,
                    state: "answered".to_string(),
                    option,
                    option_label: approved.preset.clone(),
                    words: String::new(),
                    by: String::new(),
                    return_at: String::new(),
                    secret: open.secret,
                    concealed: open.concealed,
                    given: approved.given.clone(),
                    run: open.run,
                    setup: open.setup,
                    identity: open.identity,
                    weight: open.weight,
                    ran,
                },
            );
        } else {
            return Vec::new();
        }
        self.chosen.remove(&(bead.to_string(), ask));
        self.account.remove(&(bead.to_string(), ask));
        let mut effects = self.reselect();
        effects.push(Effect::Asks);
        effects.push(Effect::Bead {
            bead: bead.to_string(),
        });
        effects
    }

    /// The setup ask's candidate taken with a tap, by source name; empty
    /// where none is taken, or where the yard lists it no more.
    pub fn account_choice(&self, bead: &str, ask: u64) -> String {
        let taken = self.account.get(&(bead.to_string(), ask));
        let listed = self.setup(bead, ask).is_some_and(|setup| {
            taken.is_some_and(|taken| setup.candidates.iter().any(|c| &c.source == taken))
        });
        match (taken, listed) {
            (Some(taken), true) => taken.clone(),
            _ => String::new(),
        }
    }

    /// Takes the setup ask's candidate with a tap: a tap on the taken one
    /// takes it back, and a tap on another takes that one instead. One
    /// the yard lists no more is not taken.
    pub fn take_account(&mut self, bead: &str, ask: u64, source: &str) {
        let listed = self
            .setup(bead, ask)
            .is_some_and(|setup| setup.candidates.iter().any(|c| c.source == source));
        if !listed {
            return;
        }
        let key = (bead.to_string(), ask);
        if self.account.get(&key).is_some_and(|taken| taken == source) {
            self.account.remove(&key);
        } else {
            self.account.insert(key, source.to_string());
        }
    }

    /// Takes back whatever candidate the setup ask's card took: a filled
    /// secret field types another instead.
    pub fn clear_account(&mut self, bead: &str, ask: u64) {
        self.account.remove(&(bead.to_string(), ask));
    }

    /// The ask is put aside: it leaves the list and returns as the yard
    /// says.
    pub fn deferred(&mut self, bead: &str, ask: u64) -> Vec<Effect> {
        let Some(at) = self
            .open
            .iter()
            .position(|a| a.bead == bead && a.ask == ask)
        else {
            return Vec::new();
        };
        let open = self.open.remove(at);
        self.decided.insert(
            0,
            DecidedAsk {
                bead: open.bead,
                bead_title: open.bead_title,
                topic: open.topic,
                ask: open.ask,
                question: open.question,
                asker: open.asker,
                since: open.since,
                state: "put aside".to_string(),
                option: 0,
                option_label: String::new(),
                words: String::new(),
                by: String::new(),
                return_at: String::new(),
                secret: open.secret,
                concealed: open.concealed,
                given: Vec::new(),
                run: open.run,
                setup: open.setup,
                identity: open.identity,
                weight: open.weight,
                ran: None,
            },
        );
        self.chosen.remove(&(bead.to_string(), ask));
        self.account.remove(&(bead.to_string(), ask));
        let mut effects = self.reselect();
        effects.push(Effect::Asks);
        effects.push(Effect::Bead {
            bead: bead.to_string(),
        });
        effects
    }

    /// The ask is declined with words: not a pick and not "you decide". A
    /// secret ask is declined with none: the yard takes no words on it.
    pub fn declined(&mut self, bead: &str, ask: u64, words: &str) -> Vec<Effect> {
        let words = words.trim().to_string();
        if words.is_empty() && self.secret(bead, ask).is_none() {
            return Vec::new();
        }
        let Some(at) = self
            .open
            .iter()
            .position(|a| a.bead == bead && a.ask == ask)
        else {
            return Vec::new();
        };
        let open = self.open.remove(at);
        self.decided.insert(
            0,
            DecidedAsk {
                bead: open.bead,
                bead_title: open.bead_title,
                topic: open.topic,
                ask: open.ask,
                question: open.question,
                asker: open.asker,
                since: open.since,
                state: "declined".to_string(),
                option: 0,
                option_label: String::new(),
                words,
                by: String::new(),
                return_at: String::new(),
                secret: open.secret,
                concealed: open.concealed,
                given: Vec::new(),
                run: open.run,
                setup: open.setup,
                identity: open.identity,
                weight: open.weight,
                ran: None,
            },
        );
        self.chosen.remove(&(bead.to_string(), ask));
        self.account.remove(&(bead.to_string(), ask));
        let mut effects = self.reselect();
        effects.push(Effect::Asks);
        effects.push(Effect::Bead {
            bead: bead.to_string(),
        });
        effects
    }

    /// The `yard` row below the conversations, with its own unread count.
    pub fn yard_row(&self) -> Row {
        Row {
            name: YARD.to_string(),
            line: "notifications".to_string(),
            unread: self.yard_unread(),
            selected: self.selected.as_ref() == Some(&Selected::Yard),
            dot: None,
        }
    }

    /// How many notifications the user has not read, with the mail of the
    /// yard row not seen: the yard's own, and that of anyone without a row.
    pub fn yard_unread(&self) -> usize {
        let mail = self
            .unread
            .values()
            .filter(|w| !w.seen && self.row_of(&w.from).is_none())
            .count();
        self.notifications.iter().filter(|n| !n.read).count() + mail
    }

    /// The mail of the yard row, oldest first, shown among the notices:
    /// what the yard itself sent, and what anyone sent who has no row.
    /// Opening the row reads it, so no mail holds the inbox back unseen.
    pub fn yard_mail(&self) -> Vec<Entry> {
        self.entries_of(&Selected::Yard)
    }

    /// The notices of the open yard row, oldest first.
    pub fn notices(&self) -> Vec<Notice> {
        self.notifications
            .iter()
            .map(|n| Notice {
                seq: n.seq,
                at: n.at.clone(),
                bead: n.bead.clone(),
                text: plain(&n.text),
                read: n.read,
                files: self.delivered(n).to_vec(),
            })
            .collect()
    }

    /// The files of the hook delivery `n` tells of, as the yard held them
    /// when it was last asked. A notice of any other kind has none, and a
    /// delivery has none until the yard has fetched them.
    fn delivered(&self, n: &Notification) -> &[BlobRef] {
        match n.kind.as_str() {
            "hook" => self.deliveries.get(&n.seq).map_or(&[], Vec::as_slice),
            _ => &[],
        }
    }

    /// How many files the hook deliveries of the notices have: what makes
    /// the yard row taller after its notices were laid out.
    pub fn delivered_count(&self) -> usize {
        self.notifications
            .iter()
            .map(|n| self.delivered(n).len())
            .sum()
    }

    /// The hook deliveries the open yard row shows that `DeliveryFiles` was
    /// not asked of, newest first, as one [`Effect::DeliveryFiles`]: each
    /// once, when its notice is first shown. The yard fetches a delivery's
    /// files after it told of the delivery, so the answer may be none; the
    /// delivery's `hook_files` asks again.
    fn deliveries_wanted(&mut self) -> Vec<Effect> {
        if self.selected != Some(Selected::Yard) || self.deliveries_too_old {
            return Vec::new();
        }
        let wanted: Vec<u64> = self
            .notifications
            .iter()
            .rev()
            .filter(|n| n.kind == "hook")
            .map(|n| n.seq)
            .filter(|seq| !self.delivering.contains(seq) && !self.deliveries.contains_key(seq))
            .collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        self.delivering.extend(wanted.iter().copied());
        self.delivery_notices.extend(wanted.iter().copied());
        vec![Effect::DeliveryFiles { seqs: wanted }]
    }

    /// What the yard said of the deliveries [`Effect::DeliveryFiles`] asked
    /// about: the files of each it has fetched show with its notice from
    /// here on, and one with none yet shows none until its `hook_files`
    /// asks again. The yard records a delivery's files once and whole, so
    /// the first answer with files stands, and a later one with none is of
    /// a call made before they came. What was not answered is asked again
    /// when the yard row next shows it. A yard from before the call has
    /// none to show: no notice is raised. Any other failure is said as a
    /// notice, as ever.
    pub fn delivery_files(
        &mut self,
        asked: &[u64],
        answers: Vec<(u64, Result<Vec<BlobRef>, api::Error>)>,
    ) {
        let shown: HashSet<u64> = self
            .notifications
            .iter()
            .filter(|n| n.kind == "hook")
            .map(|n| n.seq)
            .collect();
        let mut answered = HashSet::new();
        let mut failed = None;
        for (seq, answer) in answers {
            match answer {
                Ok(files) => {
                    answered.insert(seq);
                    if shown.contains(&seq) && !files.is_empty() {
                        self.deliveries.entry(seq).or_insert(files);
                    }
                }
                Err(err) => {
                    failed.get_or_insert((seq, err));
                }
            }
        }
        // The batch ends at its first failure, and what it did not reach
        // was never asked.
        for seq in asked.iter().filter(|seq| !answered.contains(*seq)) {
            self.delivering.remove(seq);
        }
        match failed {
            Some((_, err)) if deliveries_too_old(&err) => self.deliveries_too_old = true,
            Some((seq, err)) if shown.contains(&seq) => {
                self.notice = format!("the files of a hook delivery could not be read: {err}");
            }
            Some(_) => {}
            None => {}
        }
    }

    /// What stands above the open conversation: nothing for a crew member,
    /// for a user of this yard who that is and where what is typed goes,
    /// and for one of a linked yard who that is there, then whatever keeps
    /// the mail from arriving.
    pub fn about(&self) -> Vec<String> {
        let Some(name) = self.selected_conversation() else {
            return Vec::new();
        };
        let Some(p) = self.party(name) else {
            return self.about_person(name).into_iter().collect();
        };
        let is = if p.person {
            "a person"
        } else {
            "a crew member"
        };
        let mut first = format!("{is} of the linked yard {}", p.peer);
        if !p.role.is_empty() {
            first = format!("{first}: {}", plain(&p.role));
        }
        let mut out = vec![first];
        if !p.sends {
            out.push(format!("This yard's link sends no mail to {0}. The operator allows it: yardr peer allow {0} --send mail", p.peer));
        } else if !p.mail {
            out.push(format!(
                "{} takes no mail from this yard yet, so what you write is refused there.",
                p.peer
            ));
        }
        out
    }

    /// The line above the conversation with a user of this yard, the
    /// window's own too: with names trusted as them it says that their
    /// mail is here, and whom the next mail goes to.
    fn about_person(&self, name: &str) -> Option<String> {
        if !self.people().any(|u| u == name) {
            return None;
        }
        let is = if name == self.me {
            "you: what you write here is mail to yourself".to_string()
        } else {
            "a person of this yard".to_string()
        };
        let sources = self.sources(&Selected::Conversation(name.to_string()));
        if sources.len() < 2 {
            return Some(is);
        }
        let goes = match self.target() {
            Some(Target::Peer { name, peer }) => format!("{name}@{peer}, who wrote last"),
            _ => format!("{name} in this yard"),
        };
        Some(format!(
            "{is}, also trusted as {}: mail from that name is in this conversation, and what you write goes to {goes}",
            sources[1..].join(", ")
        ))
    }

    /// The beads an id named in mail opens as: the open and dealt-with
    /// asks' beads, the travelling beads, the beads read whole, and those
    /// the yard said it has when mail named them ([`Model::lookups`]), a
    /// closed bead among them. Only these chip: any other id stays plain
    /// text, as `re-ask` does.
    fn known_beads(&self) -> HashSet<String> {
        let mut known = HashSet::new();
        known.extend(self.open.iter().map(|a| a.bead.clone()));
        known.extend(self.decided.iter().map(|a| a.bead.clone()));
        known.extend(self.arrivals.iter().map(|r| r.bead.clone()));
        for depot in &self.flows.depots {
            for flow in &depot.flows {
                for stage in &flow.stages {
                    known.extend(stage.beads.iter().map(|b| b.bead.clone()));
                }
            }
            known.extend(depot.elsewhere.iter().map(|b| b.bead.clone()));
        }
        known.extend(self.flows.unassigned.iter().map(|b| b.bead.clone()));
        known.extend(self.beads.keys().cloned());
        known.extend(
            self.looked
                .iter()
                .filter(|(_, bead)| **bead)
                .map(|(id, _)| id.clone()),
        );
        known
    }

    /// Asks the yard which of the ids the open row's mail names are beads,
    /// of those no list the window reads holds: a bead that closed falls
    /// out of every list, and its id in old mail would stand plain. All in
    /// one effect, each id once: the mail is drawn at every frame and the
    /// drawing asks nothing, only mail that comes into view does. An id a
    /// list holds is no question, and stays a bead after it leaves the
    /// list. The yard can answer for `bead@peer` too: a closed question
    /// another yard asked falls out of the same lists.
    fn lookups(&mut self) -> Vec<Effect> {
        let Some(sel @ (Selected::Conversation(_) | Selected::Yard)) = self.selected.clone() else {
            return Vec::new();
        };
        if self.aside {
            return Vec::new();
        }
        let known = self.known_beads();
        let mut held = Vec::new();
        let mut wanted: Vec<String> = Vec::new();
        for letter in self.letters(&sel) {
            for id in named_beads(&plain(&letter.text)) {
                if let Some(bead) = self.resolve_bead(&known, &id) {
                    held.push(bead);
                } else if !self.looked.contains_key(&id)
                    && !self.looking.contains(&id)
                    && !wanted.contains(&id)
                {
                    wanted.push(id);
                }
            }
        }
        self.looked
            .extend(held.into_iter().map(|bead| (bead, true)));
        if wanted.is_empty() {
            return Vec::new();
        }
        self.looking.extend(wanted.iter().cloned());
        vec![Effect::Beads { beads: wanted }]
    }

    /// What the yard said of the ids [`Effect::Beads`] asked about: those
    /// it has, under the very id, are beads; those it refused, and those
    /// it took for a longer id or a UUID, are none. What it did not answer
    /// is asked again when mail next comes into view.
    pub fn beads_looked(
        &mut self,
        asked: &[String],
        answers: Vec<(String, Result<String, api::Error>)>,
    ) {
        for id in asked {
            self.looking.remove(id);
        }
        for (id, answer) in answers {
            match answer {
                Ok(found) => {
                    self.looked.insert(id.clone(), found == id);
                }
                // The edge's own words about its link say nothing of the
                // id; it is asked again.
                Err(err) if err.edge_link() => {}
                Err(api::Error::Refused { .. }) => {
                    self.looked.insert(id, false);
                }
                Err(_) => {}
            }
        }
    }

    /// The bead `id` opens as, when footplate knows it: itself, or the
    /// one known bead of another yard it names without its peer. None
    /// where none is known, or where several yards' are.
    fn resolve_bead(&self, known: &HashSet<String>, id: &str) -> Option<String> {
        if known.contains(id) {
            return Some(id.to_string());
        }
        if id.contains('@') {
            return None;
        }
        let mut peered = known.iter().filter(|bead| {
            bead.strip_prefix(id)
                .is_some_and(|peer| peer.starts_with('@') && peer.len() > 1)
        });
        let bead = peered.next()?;
        peered.next().is_none().then(|| bead.clone())
    }

    /// Whether the ask is open for the viewer or dealt with: what its
    /// chip opens. None where it is neither.
    fn ask_state(&self, bead: &str, ask: u64) -> Option<bool> {
        if self.open.iter().any(|a| a.bead == bead && a.ask == ask) {
            return Some(true);
        }
        if self.decided.iter().any(|a| a.bead == bead && a.ask == ask) {
            return Some(false);
        }
        None
    }

    /// The card the number `ask` opens as, when one ask of that number is
    /// the viewer's. None where none is, or where several beads' are.
    fn resolve_ask(&self, ask: u64) -> Option<(String, bool)> {
        let mut matches = self
            .open
            .iter()
            .filter(|a| a.ask == ask)
            .map(|a| (a.bead.as_str(), true))
            .chain(
                self.decided
                    .iter()
                    .filter(|a| a.ask == ask)
                    .map(|a| (a.bead.as_str(), false)),
            );
        let (bead, open) = matches.next()?;
        matches.next().is_none().then(|| (bead.to_string(), open))
    }

    /// The open ask's preview, as it stands under a mail that names it.
    fn ask_preview<Tz: TimeZone>(
        &self,
        bead: &str,
        ask: u64,
        now: &DateTime<Tz>,
    ) -> Option<AskPreview>
    where
        Tz::Offset: std::fmt::Display,
    {
        let open = self.open.iter().find(|a| a.bead == bead && a.ask == ask)?;
        Some(AskPreview {
            bead: bead.to_string(),
            ask,
            question: row_head(&open.question),
            asker: open.asker.clone(),
            waits: waits(&open.since, now),
            kind: match (&open.run, &open.secret) {
                (Some(_), _) => "run ask".to_string(),
                (_, Some(_)) => "secret ask".to_string(),
                _ => "ask".to_string(),
            },
        })
    }

    /// The references `text` names that the yard said, each where it
    /// stands in its block, with the open asks among them for the
    /// previews under it. `known` is [`Model::known_beads`].
    fn entry_refs<Tz: TimeZone>(
        &self,
        text: &str,
        known: &HashSet<String>,
        now: &DateTime<Tz>,
    ) -> (Vec<select::RefSpan>, Vec<AskPreview>)
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut spans = Vec::new();
        let mut previews = Vec::new();
        let mut previewed: HashSet<(String, u64)> = HashSet::new();
        for (ix, block) in markdown::blocks(text).iter().enumerate() {
            for found in select::refs(&block.text) {
                // An ask's whole reference chips, or the bead's own
                // where the ask is none the yard said.
                let (range, target) = match &found.kind {
                    select::RefKind::Ask(ask) => match self.resolve_ask(*ask) {
                        Some((bead, open)) => (
                            found.range.clone(),
                            select::RefTarget::Ask {
                                bead,
                                ask: *ask,
                                open,
                            },
                        ),
                        None => continue,
                    },
                    select::RefKind::AskOn {
                        ask,
                        bead,
                        bead_range,
                    } => {
                        let Some(id) = self.resolve_bead(known, bead) else {
                            continue;
                        };
                        match self.ask_state(&id, *ask) {
                            Some(open) => (
                                found.range.clone(),
                                select::RefTarget::Ask {
                                    bead: id,
                                    ask: *ask,
                                    open,
                                },
                            ),
                            None => (bead_range.clone(), select::RefTarget::Bead { bead: id }),
                        }
                    }
                    select::RefKind::Bead(bead) => match self.resolve_bead(known, bead) {
                        Some(id) => (found.range.clone(), select::RefTarget::Bead { bead: id }),
                        None => continue,
                    },
                };
                if let select::RefTarget::Ask {
                    bead,
                    ask,
                    open: true,
                } = &target
                    && previewed.insert((bead.clone(), *ask))
                    && let Some(preview) = self.ask_preview(bead, *ask, now)
                {
                    previews.push(preview);
                }
                spans.push(select::RefSpan {
                    block: ix,
                    range,
                    target,
                });
            }
        }
        (spans, previews)
    }

    /// The open conversation, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        match self.selected.as_ref() {
            Some(sel @ Selected::Conversation(_)) => self.entries_of(sel),
            _ => Vec::new(),
        }
    }

    /// The mail the row `sel` shows, oldest first.
    fn entries_of(&self, sel: &Selected) -> Vec<Entry> {
        let letters = self.letters(sel);
        let zone = chrono::Local;
        let now = chrono::Local::now();
        let today = now.date_naive();
        let known = self.known_beads();
        let mut out: Vec<Entry> = Vec::with_capacity(letters.len());
        let mut before: Option<&Letter> = None;
        for l in letters {
            let mine = self.own(&l.from, sel);
            let standing = mine.then(|| self.standing(l));
            // The line is under the last of those that stand alike.
            if let Some(shown) = out.last_mut()
                && shown.mine
                && shown.standing == standing
            {
                shown.standing = None;
            }
            // A mail reads under the sender's name and time above it while
            // it comes within five minutes on the same day; every other one
            // heads its own group, and a new day breaks above it.
            let head = match before {
                None => true,
                Some(b) => !grouped(b, l, &zone),
            };
            let day = match (day_of(&l.at, &zone), before) {
                (Some(day), None) => Some(day_words(day, today)),
                (Some(day), Some(b)) if day_of(&b.at, &zone) != Some(day) => {
                    Some(day_words(day, today))
                }
                _ => None,
            };
            before = Some(l);
            let text = plain(&l.text);
            // A correction's label says its word: its text is what follows
            // it, and the references stand in that.
            let (correction, text) = match correction(&text) {
                Some(said) => (true, said.to_string()),
                None => (false, text),
            };
            let (refs, asks) = self.entry_refs(&text, &known, &now);
            out.push(Entry {
                seq: l.seq,
                at: l.at.clone(),
                from: l.from.clone(),
                correction,
                text,
                files: l.files.clone(),
                mine,
                standing,
                head,
                day,
                refs,
                asks,
                reactions: l.reactions.clone(),
            });
        }
        out
    }

    /// The hash of every image shown, each once: the open conversation's
    /// or the open yard row's mail's, the yard row's hook deliveries', then
    /// the open row's expanded cards' notes'. One this yard did not take,
    /// from a linked yard or for a delivery, is none it has.
    pub fn images(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let letters = match self.selected.as_ref() {
            Some(sel @ (Selected::Conversation(_) | Selected::Yard)) => self.letters(sel),
            _ => Vec::new(),
        };
        for f in letters.into_iter().flat_map(|l| &l.files) {
            push_image(&mut out, f);
        }
        if self.selected == Some(Selected::Yard) {
            for f in self.notifications.iter().flat_map(|n| self.delivered(n)) {
                push_image(&mut out, f);
            }
        }
        for note in self.expanded_notes() {
            for f in &note.files {
                push_image(&mut out, f);
            }
        }
        out
    }

    /// The notes the open row's expanded cards show: the bead's, where
    /// one of its cards stands expanded, and none anywhere else. An
    /// arrival's bead shows its notes whole, so its are here too.
    fn expanded_notes(&self) -> Vec<&BeadNote> {
        let bead = match self.selected.as_ref() {
            Some(Selected::Decision(bead)) if self.expanded.iter().any(|(b, _)| b == bead) => {
                Some(bead)
            }
            Some(Selected::Decided(bead, ask))
                if self.expanded.iter().any(|(b, a)| b == bead && a == ask) =>
            {
                Some(bead)
            }
            Some(Selected::Bead(bead)) => Some(bead),
            _ => None,
        };
        bead.and_then(|bead| self.beads.get(bead))
            .map(|b| b.notes.iter().collect())
            .unwrap_or_default()
    }

    /// Where a mail the user sent stands: read or not for a crew member,
    /// and for one of a linked yard its delivery, since that yard does not
    /// say what was read.
    fn standing(&self, l: &Letter) -> String {
        if let Some((_, peer)) = l.to.split_once('@') {
            return match l.delivery.as_str() {
                "queued" => format!("queued for {peer}: the daemon delivers it"),
                "failed" => self.not_delivered(peer),
                _ => format!("delivered to {peer}, which does not say when it is read"),
            };
        }
        if l.read {
            return "read".to_string();
        }
        let name = &l.to;
        let member = self.member(name);
        match member.map(state_of) {
            Some("idle") => format!("sent; {name} is idle"),
            Some("working") => format!("sent; {name} is working and reads mail when idle"),
            Some("blocked") => format!("{name} waits at a prompt in its pane and cannot read mail"),
            // Where the yard leads the login, the person answers it at the
            // asks: footplate's Decisions.
            Some("not logged in") if member.is_some_and(|r| r.login_led) => format!(
                "{name} is not logged in and cannot read mail: the yard leads its login, and waits for the operator at its asks (yardr asks)"
            ),
            Some("not logged in") => format!("{name} is not logged in and cannot read mail"),
            Some("not running") => format!("{name} is not running: yardr crew start {name}"),
            Some("failed to start") => format!("{name} failed to start: yardr crew show {name}"),
            _ => "sent".to_string(),
        }
    }

    /// Why mail to the peer was not delivered: what the peer takes in an
    /// hour and when the hour ends, where the yard refused one of this
    /// yard's over the hour, else that the inbox says why.
    fn not_delivered(&self, peer: &str) -> String {
        if let Some(hour) = self.hours.get(peer) {
            let end = hour.end();
            if !end.is_empty() {
                return format!(
                    "not delivered: {peer} takes {} mails an hour from this yard; send again after {end}",
                    hour.limit
                );
            }
        }
        "not delivered: yardr inbox says why".to_string()
    }

    /// Where what is typed goes, when a conversation is open. In the row
    /// of a user whom a name of a linked yard is trusted as it answers
    /// whoever wrote last, as `yardr chat` does: that name at its yard when
    /// the newest mail that is not the user's own came from there, else the
    /// user in this yard. Only a name trusted as the row's user is answered
    /// at its yard: any other row goes to the one it is with, whoever else
    /// wrote in it.
    pub fn target(&self) -> Option<Target> {
        let sel = self.selected.as_ref()?;
        let name = self.selected_conversation()?;
        if let Some(p) = self.party(name) {
            return Some(Target::Peer {
                name: p.name.clone(),
                peer: p.peer.clone(),
            });
        }
        let last = self
            .letters(sel)
            .into_iter()
            .rev()
            .find(|l| !self.own(&l.from, sel))
            .filter(|l| self.trust.get(&l.from).is_some_and(|user| user == name));
        Some(match last.and_then(|l| l.from.split_once('@')) {
            Some((name, peer)) => Target::Peer {
                name: name.to_string(),
                peer: peer.to_string(),
            },
            None => Target::Crew(name.to_string()),
        })
    }

    /// Whether the mail `from` sent, as the row `sel` shows it, is the
    /// user's own: their name, or a name a link trusts as them, which is
    /// how they spoke from that yard. In their own row such a name is the
    /// other end of the conversation instead: it wrote to them, a note to
    /// themselves from there, and an answer goes back to it.
    fn own(&self, from: &str, sel: &Selected) -> bool {
        let theirs = matches!(sel, Selected::Conversation(row) if *row == self.me);
        from == self.me || (!theirs && self.trust.get(from).is_some_and(|user| *user == self.me))
    }

    /// Whether the yard has the row `sel` to open.
    pub fn has_row(&self, sel: &Selected) -> bool {
        matches!(sel, Selected::Bead(_)) || self.order().contains(sel)
    }

    /// Opens a row of a list: a bead, one dealt with, the grants, a
    /// conversation, the yard row, a timetable row, or one travelling bead
    /// from the board, a flow, Elsewhere or No line, which has no row of
    /// the list. A row that is not there changes nothing.
    pub fn select(&mut self, sel: Selected) -> Vec<Effect> {
        if !self.has_row(&sel) {
            return Vec::new();
        }
        self.selected = Some(sel);
        self.notice.clear();
        self.opened();
        self.open()
    }

    /// Opens the row `by` after the open one in the mode's list, around
    /// the ends. From a travelling bead, from the row it was opened from.
    pub fn step_in(&mut self, mode: Mode, by: isize) -> Vec<Effect> {
        let order = self.order_in(mode);
        if order.is_empty() {
            return Vec::new();
        }
        let n = order.len() as isize;
        let at = self
            .selected
            .as_ref()
            .and_then(|s| {
                let from = match s {
                    Selected::Bead(_) => self.bead_back(),
                    _ => s.clone(),
                };
                order.iter().position(|o| *o == from)
            })
            .unwrap_or(0) as isize;
        let next = (at + by).rem_euclid(n) as usize;
        self.select(order[next].clone())
    }

    /// The unread mail the open row shows is seen: that of a conversation
    /// the yard has answered, from where the row has all its mail. Unread
    /// mail older than the pages read was never shown, so it stays unseen,
    /// and holds the acknowledgement back until its page is read.
    fn see(&mut self) {
        let horizon = self.selected.as_ref().map_or(0, |sel| self.horizon(sel));
        let shown: Vec<u64> = self
            .unread
            .iter()
            .filter(|(seq, w)| {
                !w.seen && **seq >= horizon && self.loaded.contains(&w.from) && self.shows(&w.from)
            })
            .map(|(seq, _)| *seq)
            .collect();
        for seq in shown {
            if let Some(w) = self.unread.get_mut(&seq) {
                w.seen = true;
            }
        }
    }

    /// What opening a row does here: the unread mail it shows is seen, a
    /// conversation is the one to come back to, and a timetable row is
    /// the one a travelling bead opened next comes back to.
    fn opened(&mut self) {
        self.see();
        if let Some(Selected::Conversation(sel)) = &self.selected {
            self.back = Some(sel.clone());
        }
        if let Some(sel @ (Selected::Arrivals | Selected::Line(_) | Selected::NoLine)) =
            &self.selected
        {
            self.bead_from = Some(sel.clone());
        }
    }

    /// Reads the open conversation if it was not read before, its newest
    /// page, with each whose mail it shows, and acknowledges what opening
    /// it showed; the yard row does so with its mail, whole, and its
    /// notices need no call. A bead, one dealt with and one travelling
    /// bead read the bead whole when it was not read before. The grants
    /// are read again whenever their row opens, so a revoke, which the
    /// yard tells no event of, is seen. The board, a line and No line
    /// need no call: their rows came with the state, and events re-read
    /// them.
    fn open(&mut self) -> Vec<Effect> {
        match self.selected.clone() {
            Some(sel @ Selected::Conversation(_)) => {
                let mut effects: Vec<Effect> = self
                    .sources(&sel)
                    .iter()
                    .filter(|from| !self.loaded.contains(*from))
                    .map(|from| self.read(from))
                    .collect();
                effects.extend(self.ack());
                effects.extend(self.lookups());
                effects
            }
            // The yard row has no top to reach among its notices: the mail
            // it shows is read whole, and the files of its hook deliveries
            // are asked for.
            Some(sel @ Selected::Yard) => {
                let mut effects: Vec<Effect> = self
                    .sources(&sel)
                    .into_iter()
                    .filter(|from| !self.loaded.contains(from))
                    .map(|name| Effect::Read { name, after: 0 })
                    .collect();
                effects.extend(self.ack());
                effects.extend(self.lookups());
                effects.extend(self.deliveries_wanted());
                effects
            }
            Some(Selected::Decision(bead))
            | Some(Selected::Decided(bead, _))
            | Some(Selected::Bead(bead)) => {
                if self.beads.contains_key(&bead) {
                    Vec::new()
                } else {
                    vec![Effect::Bead { bead }]
                }
            }
            Some(Selected::Grants) => match self.is_operator() {
                true if !self.grants_too_old => self.read_grants(),
                false => Vec::new(),
                true => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The call for the conversation with `name`: what came after the last
    /// mail the window has of it, or its newest page when the yard was not
    /// asked for it before.
    fn read(&self, name: &str) -> Effect {
        match self.convo.get(name) {
            Some(c) if self.loaded.contains(name) => Effect::Read {
                name: name.to_string(),
                after: c.last().map_or(0, |l| l.seq),
            },
            _ => Effect::Page {
                name: name.to_string(),
                before: 0,
            },
        }
    }

    /// Reads the bead whole when it was not read before: what the
    /// drawer opens with, without selecting it, as opening the row does.
    pub fn read_bead(&mut self, bead: &str) -> Vec<Effect> {
        self.drawn = Some(bead.to_string());
        if self.beads.contains_key(bead) {
            Vec::new()
        } else {
            vec![Effect::Bead {
                bead: bead.to_string(),
            }]
        }
    }

    /// Mail was sent to `to`. It also comes from the stream; reading it
    /// shows it when the stream is lost at this moment, and costs what is
    /// new.
    pub fn sent(&mut self, to: &str) -> Vec<Effect> {
        self.sources(&Selected::Conversation(to.to_string()))
            .iter()
            .map(|with| self.read(with))
            .collect()
    }

    /// A reaction was put on the mail `seq`, or taken away from it. It
    /// also comes from the stream; reading the conversation that holds
    /// the mail shows it when the stream is lost at this moment, and
    /// costs what is new. The yard answered, so it has the call.
    pub fn reacted(&mut self, seq: u64) -> Vec<Effect> {
        self.react_too_old = false;
        let name = self
            .convo
            .iter()
            .find(|(_, c)| c.iter().any(|l| l.seq == seq))
            .map(|(name, _)| name.clone());
        name.map(|name| {
            // `after` is exclusive. Start just before the target so the
            // reread returns the mail whose reactions may have changed.
            vec![Effect::Read {
                name,
                after: seq.saturating_sub(1),
            }]
        })
        .unwrap_or_default()
    }

    /// Whether the yard has no React call: one from before reactions,
    /// which hides the row the reactions are tapped on.
    pub fn react_too_old(&self) -> bool {
        self.react_too_old
    }

    /// The user's own reaction on the mail `seq`, if they put one and the
    /// window holds the mail: a tap on it takes it away, and a tap on
    /// another puts that one instead.
    pub fn own_reaction(&self, seq: u64) -> Option<String> {
        self.convo
            .values()
            .flat_map(|c| c.iter())
            .find(|l| l.seq == seq)?
            .reactions
            .iter()
            .find(|r| r.from == self.me)
            .map(|r| r.emoji.clone())
    }

    /// A reaction was not put. A yard without the call hides the row
    /// the reactions are tapped on, and no notice is raised. Anything
    /// else is said as a notice, as ever.
    pub fn react_failed(&mut self, err: &api::Error) {
        // The yard's one answer with no code of its own: its words are
        // read here, as `not_given` reads them, and when they change
        // this is shown as any refusal.
        match err {
            api::Error::Refused { message, .. } if message.starts_with("no such method") => {
                self.react_too_old = true;
            }
            _ => {
                self.notice = format!("not reacted: {err}");
            }
        }
    }

    /// How many mails the window holds, of every conversation.
    fn letters_held(&self) -> usize {
        self.convo.values().map(Vec::len).sum()
    }

    /// Puts `l` into the conversation with `name`, in order. A mail it has
    /// already (the stream and the yard both tell of each) only gains what
    /// `l` knows more: that it was read, its reactions, or where it stands.
    fn add(&mut self, name: &str, l: Letter, reactions_are_current: bool) {
        let c = self.convo.entry(name.to_string()).or_default();
        match c.binary_search_by_key(&l.seq, |have| have.seq) {
            Ok(i) => {
                c[i].read |= l.read;
                // Conversation reads carry the current state, including
                // no reactions after a clear. A mail stream event has no
                // reaction field, so it must leave what is already held.
                if reactions_are_current {
                    c[i].reactions = l.reactions;
                }
                // Mail to another yard only goes on from queued, and the
                // stream tells of it as queued whenever it tells.
                if !l.delivery.is_empty() && l.delivery != "queued" {
                    c[i].delivery = l.delivery;
                }
            }
            Err(i) => c.insert(i, l),
        }
    }

    /// The conversation with `name` as the yard answered it.
    pub fn conversation(&mut self, name: &str, letters: Vec<Letter>) -> Vec<Effect> {
        self.loaded.insert(name.to_string());
        for l in letters {
            self.add(name, l, true);
        }
        self.see();
        let mut effects = self.ack();
        effects.extend(self.lookups());
        effects
    }

    /// A page of the conversation with `name` as the yard answered it, and
    /// whether the yard has older mail of it still. A yard that knows no
    /// pages answers all of it and says nothing of more: that is the whole
    /// conversation then, and nothing is asked for after it. `before` is
    /// what the page was asked with.
    pub fn page(
        &mut self,
        name: &str,
        before: u64,
        letters: Vec<Letter>,
        more: bool,
    ) -> Vec<Effect> {
        self.paging.remove(name);
        // An older page for a conversation whose newest was not read is
        // one asked for before the yard was lost: with it alone the
        // conversation would stand read, and its newest mail missing.
        if before > 0 && !self.loaded.contains(name) {
            return Vec::new();
        }
        if more {
            self.older.insert(name.to_string());
        } else {
            self.older.remove(name);
        }
        self.conversation(name, letters)
    }

    /// The yard did not answer the page: it is asked for again when the
    /// conversation is opened or its top reached again.
    pub fn page_failed(&mut self, name: &str, why: &str) {
        self.paging.remove(name);
        self.notice = format!("the conversation with {name} could not be read: {why}");
    }

    /// What stands above the oldest mail of the open conversation.
    pub fn top(&self) -> Top {
        let Some(sel @ Selected::Conversation(_)) = self.selected.as_ref() else {
            return Top::Unread;
        };
        let sources = self.sources(sel);
        if sources.iter().any(|name| self.older.contains(name)) {
            Top::Older
        } else if sources.iter().all(|name| self.loaded.contains(name)) {
            Top::Start
        } else {
            Top::Unread
        }
    }

    /// The seq of the oldest and of the newest mail the open conversation
    /// shows.
    pub fn span(&self) -> Option<(u64, u64)> {
        let letters = match self.selected.as_ref() {
            Some(sel @ Selected::Conversation(_)) => self.letters(sel),
            _ => Vec::new(),
        };
        Some((letters.first()?.seq, letters.last()?.seq))
    }

    /// The rows the mails above `oldest` add to the shown conversation:
    /// one for each mail, group head and day break, less the head and break
    /// the oldest shown mail loses where it joins them. The window keeps
    /// its place by it when an older page arrives above the view.
    pub fn rows_above(&self, oldest: u64) -> usize {
        let entries = self.entries();
        let mut rows = 0;
        let mut under = true;
        for e in &entries {
            if e.seq < oldest {
                rows += 1 + usize::from(e.head) + usize::from(e.day.is_some());
            } else if under {
                // The oldest shown mail stood first: its head, and its
                // break where its time reads, stood as rows; what it lost
                // to the group above it takes those rows back.
                under = false;
                rows -= usize::from(!e.head);
                rows -= usize::from(e.day.is_none() && day_of(&e.at, &chrono::Local).is_some());
            }
        }
        rows
    }

    /// The top of the open conversation was reached: asks for the page
    /// before the oldest mail it shows, of each conversation in the row
    /// that stops there and has older. Nothing while that is asked for
    /// already, and nothing once the yard said there is no more.
    pub fn older(&mut self) -> Vec<Effect> {
        let Some(sel @ Selected::Conversation(_)) = self.selected.clone() else {
            return Vec::new();
        };
        let horizon = self.horizon(&sel);
        let mut effects = Vec::new();
        for name in self.sources(&sel) {
            let first = self
                .convo
                .get(&name)
                .and_then(|c| c.first())
                .map_or(0, |l| l.seq);
            if self.older.contains(&name) && first == horizon && self.paging.insert(name.clone()) {
                effects.push(Effect::Page {
                    name,
                    before: first,
                });
            }
        }
        effects
    }

    /// Acknowledges the user's mail as far as the window has shown it:
    /// through the newest seen mail before the oldest that was not. A
    /// user's inbox has one position for all its senders, so mail of a
    /// conversation that was not opened holds back what came after it; it
    /// is never marked read unseen.
    fn ack(&mut self) -> Vec<Effect> {
        let through = self
            .unread
            .iter()
            .take_while(|(_, w)| w.seen)
            .map(|(seq, _)| *seq)
            .last()
            .unwrap_or(0);
        if through <= self.asked {
            return Vec::new();
        }
        self.asked = through;
        vec![Effect::Ack { through }]
    }

    /// The user's mail through `through` is read: by this window, or by
    /// anything else that acknowledged it (`yardr chat`, `yardr inbox`).
    pub fn acked(&mut self, through: u64) {
        self.cursor = self.cursor.max(through);
        self.asked = self.asked.max(self.cursor);
        self.unread.retain(|seq, _| *seq > through);
    }

    /// The yard did not take the acknowledgement: it is asked again with
    /// the next mail seen.
    pub fn ack_failed(&mut self, why: &str) {
        self.asked = self.cursor;
        self.notice = format!("your mail could not be marked read: {why}");
    }

    /// Applies one event of the stream. Mail and notices never cover or
    /// move the open card: only the queue leaving its bead moves it.
    pub fn event(&mut self, e: &Event) -> Vec<Effect> {
        // An event that became a notification for the user carries less
        // than what changed: the notices are read again.
        let mut effects = Vec::new();
        self.events_seen = self.events_seen.max(e.seq);
        // One of the person's beads added or closed since they last had
        // Timetable open is its news, unless Timetable is open, which
        // saw it instead.
        if e.timetable_news(&self.me) && e.seq > self.timetable_looked {
            match self.looking() {
                true => self.timetable_looked = self.timetable_looked.max(e.seq),
                false => {
                    self.timetable_news.insert(e.bead.clone());
                }
            }
        }
        if e.notified(&self.me) {
            effects.push(Effect::Notifications);
        }
        effects.extend(match e.kind.as_str() {
            "mail" => {
                let held = self.letters_held();
                let mut arrived = self.mail(e);
                // The stream tells of the yard's other mail too, and of
                // what it told before: those add nothing to ask about.
                if self.letters_held() > held {
                    arrived.extend(self.lookups());
                }
                arrived
            }
            "mail_read" => {
                let (reader, through) = (e.text("reader"), e.number("through"));
                if reader == self.me {
                    self.acked(through);
                }
                // The user's own row has what they sent themselves.
                if let Some(c) = self.convo.get_mut(reader) {
                    for l in c.iter_mut().filter(|l| l.to == reader && l.seq <= through) {
                        l.read = true;
                    }
                }
                Vec::new()
            }
            "reaction" => {
                self.reaction(e);
                Vec::new()
            }
            "notifications_read" => {
                let (reader, through) = (e.text("reader"), e.number("through"));
                if reader == self.me {
                    self.notifications_acked(through);
                }
                Vec::new()
            }
            // A hook delivery's files were fetched and recorded, after the
            // delivery and its notice: a delivery asked about before then
            // is asked again. One never shown is asked with its notice,
            // when the row shows it. It is no notification, so it never
            // reads the notices again.
            "hook_files" => {
                let seq = e.number("seq");
                if !self.deliveries_too_old
                    && self.delivery_notices.contains(&seq)
                    && self.delivery_refetched.insert(seq)
                {
                    self.delivering.insert(seq);
                    vec![Effect::DeliveryFiles { seqs: vec![seq] }]
                } else {
                    Vec::new()
                }
            }
            // A note on a bead: a question asked, answered, put aside or
            // declined, or the asker's next note. It says no more than
            // that, so the queue is read again.
            "noted" => vec![Effect::Asks],
            "crew_status" => {
                let name = e.text("crew").to_string();
                if let Some(r) = self
                    .crew
                    .iter_mut()
                    .find(|r| r.name == name && r.state == "running")
                {
                    r.status = e.text("status").to_string();
                    self.status_at.insert(name, e.seq);
                }
                Vec::new()
            }
            // A session started, stopped or failed, the list itself
            // changed, its login changed, or a crew member's first prompt
            // arrived: the event carries less than what changed. `crew_login`
            // arrives through this arm and needs none of its own.
            kind if kind.starts_with("crew_") => vec![Effect::Crew],
            kind if kind.starts_with("session_prompt") && !e.text("crew").is_empty() => {
                vec![Effect::Crew]
            }
            // Removing a user also removes any link trust that named them;
            // unlike a direct grant change, this is not a `peer_` event.
            "user_removed" => vec![Effect::Peers],
            kind if kind.starts_with("peer_") => self.link(e),
            // The operator is the only person whose device may read the
            // grant graph. Drop the cached rows as soon as that setting
            // changes, then read them again only if this window is still
            // the operator's. A remote listener enforces the same rule on
            // the call itself; this also clears what a former operator's
            // window already had on screen.
            "setting_changed" if e.text("key") == "operator" => {
                self.operator = e.text("value").to_string();
                self.grants.clear();
                self.grants_too_old = false;
                // A former operator's window keeps none of the preview.
                // Unserved stays as it was: a yard that serves the calls
                // to no such window is not asked for them on a change of
                // operator either, and one read whole reads them again.
                self.telemetry = None;
                self.telemetry_notice = None;
                self.telemetry_read_only = false;
                if self.is_operator() {
                    let mut effects = self.read_grants();
                    effects.extend(self.read_telemetry());
                    effects
                } else if self.selected == Some(Selected::Grants) {
                    self.selected = self.row_in(Mode::Decisions, None);
                    self.opened();
                    self.open()
                } else {
                    Vec::new()
                }
            }
            // The telemetry switch moved: the preview carries less than
            // what changed, so it is read again, where the window is the
            // operator's and the yard serves the calls to it.
            "setting_changed" if e.text("key") == "telemetry" => match self.is_operator() {
                true => self.read_telemetry(),
                false => Vec::new(),
            },
            // A grant given: the event carries less than what changed, so
            // the grants are read again, where the window is the
            // operator's. A revoke tells no event; opening the grants
            // reads them again.
            "source_granted" => match self.is_operator() {
                true => self.read_grants(),
                false => Vec::new(),
            },
            _ => Vec::new(),
        });
        // A bead shown is read again on every event whose bead it is: the
        // card's options, recommendation and state may have changed.
        if !e.bead.is_empty() && self.beads.contains_key(&e.bead) {
            effects.push(Effect::Bead {
                bead: e.bead.clone(),
            });
        }
        // Arrivals move with every event about a bead, and with the
        // changes that move their other inputs though they name none:
        // docs/api.md names them with the call. The flows view moves with
        // those and one more: a removed depot's line leaves the timetable,
        // and only `depot_removed` says so.
        let arrivals = !e.bead.is_empty()
            || matches!(
                e.kind.as_str(),
                "group_set"
                    | "group_removed"
                    | "route_added"
                    | "route_removed"
                    | "flow_set"
                    | "flow_removed"
                    | "flow_pointer_set"
                    | "flow_pointer_cleared"
                    | "depot_updated"
            )
            || e.kind.as_str() == "setting_changed"
                && matches!(e.text("key"), "max_sessions" | "operator");
        if arrivals {
            effects.push(Effect::Arrivals);
        }
        if arrivals || e.kind.as_str() == "depot_removed" {
            effects.push(Effect::Flows);
        }
        effects
    }

    /// A mail event: one the user sent, or one to the user. Mail the user
    /// wrote to another yard is the conversation with `<name>@<peer>`,
    /// queued as the event tells of it; mail from there has that address as
    /// its sender already.
    fn mail(&mut self, e: &Event) -> Vec<Effect> {
        self.undelivered(e);
        let to = e.text("to");
        let mut l = Letter {
            seq: e.seq,
            at: e.at.clone(),
            from: e.actor.clone(),
            to: to.to_string(),
            text: e.text("text").to_string(),
            files: e.files(),
            ..Letter::default()
        };
        let peer = e.text("to_peer");
        if !peer.is_empty() {
            if e.actor == self.me {
                l.to = format!("{to}@{peer}");
                l.delivery = "queued".to_string();
                self.add(&l.to.clone(), l, false);
            }
            return Vec::new();
        }
        // Mail the user wrote to themselves is both: their own, and unread
        // in their inbox until their row shows it. So is what they wrote
        // from a linked yard under a name trusted as them, to anyone but
        // themselves: it stands in the row of whoever it was written to,
        // as yardr chat's mail has it, and being theirs it is not
        // counted, acknowledged or notified.
        let mine = e.actor == self.me
            || (to != self.me
                && self
                    .trust
                    .get(&e.actor)
                    .is_some_and(|user| *user == self.me));
        let for_me = to == self.me && e.flag("to_user");
        if !mine && !for_me {
            return Vec::new();
        }
        if for_me {
            l.read = e.seq <= self.cursor;
        }
        let read = l.read;
        self.add(if mine { to } else { &e.actor }, l, false);
        if !for_me || read || self.unread.contains_key(&e.seq) {
            return Vec::new();
        }
        // On the open row that shows it, it is seen as it arrives.
        let seen = self.shows(&e.actor);
        self.unread.insert(
            e.seq,
            Waiting {
                from: e.actor.clone(),
                seen,
            },
        );
        if !mine {
            self.mail_news(&e.actor, e.text("text"));
        }
        let mut effects = self.ack();
        // A name of this yard with no row may be a user added since the
        // users were read: they are read again, and the mail has its row.
        if !mine && e.actor != YARD && !e.actor.contains('@') && self.row_of(&e.actor).is_none() {
            effects.push(Effect::Users);
        }
        effects
    }

    /// A reaction event: the target letter's reactions gain the
    /// reactor's emoji, or lose it on a clear. The letter is in one
    /// conversation; the others are left as they are. One the window
    /// does not hold is none read yet, and its read carries them.
    fn reaction(&mut self, e: &Event) {
        let target = e.number("seq");
        let emoji = e.text("emoji");
        for c in self.convo.values_mut() {
            let Ok(i) = c.binary_search_by_key(&target, |l| l.seq) else {
                continue;
            };
            c[i].reactions.retain(|r| r.from != e.actor);
            if !emoji.is_empty() {
                c[i].reactions.push(Reaction {
                    from: e.actor.clone(),
                    emoji: emoji.to_string(),
                });
                c[i].reactions.sort_by(|a, b| a.from.cmp(&b.from));
            }
            break;
        }
    }

    /// A mail the yard wrote that one of this yard's was not delivered:
    /// the hour the peer refused over, where it did. The mail names the
    /// outbox message alone; the failed event before it named its peer.
    /// One the yard tells without the hour clears the peer's.
    fn undelivered(&mut self, e: &Event) {
        let gone = e.text("undelivered");
        if e.actor != YARD || gone.is_empty() {
            return;
        }
        let Some(peer) = self.failed_ids.remove(gone) else {
            return;
        };
        let (limit, until) = (e.number("limit"), e.text("until"));
        if limit > 0 && !until.is_empty() {
            self.hours.insert(
                peer,
                Hour {
                    limit,
                    until: until.to_string(),
                },
            );
        } else {
            self.hours.remove(&peer);
        }
    }

    /// An event of a link. The roster a peer says changes without an event
    /// of its own, so every one is the occasion to read the linked yards
    /// again, and one that tells how a message to a peer ended to read the
    /// conversations with that yard again, whole: where the user's mail
    /// stands is what changed.
    fn link(&mut self, e: &Event) -> Vec<Effect> {
        let mut effects = vec![Effect::Peers];
        let peer = e.text("peer");
        if e.kind.as_str() == "peer_message_failed"
            && e.text("kind") == PEER_MAIL
            && !peer.is_empty()
        {
            let id = e.text("id");
            if !id.is_empty() {
                self.failed_ids.insert(id.to_string(), peer.to_string());
            }
        }
        if matches!(e.kind.as_str(), "peer_message_sent" | "peer_message_failed")
            && !peer.is_empty()
        {
            let mut names: Vec<&String> = self
                .loaded
                .iter()
                .filter(|n| n.split_once('@').is_some_and(|(_, of)| of == peer))
                .collect();
            names.sort();
            // Whole as far as the window has it: where the yard has older
            // mail than that, from the oldest it was given on.
            effects.extend(names.into_iter().map(|name| {
                Effect::Read {
                    name: name.clone(),
                    after: self
                        .convo
                        .get(name)
                        .and_then(|c| c.first())
                        .filter(|_| self.older.contains(name))
                        .map_or(0, |l| l.seq.saturating_sub(1)),
                }
            }));
        }
        effects
    }

    /// The crew as read at `head`. A status the stream told of since is
    /// newer than the read and stays. When the open row is gone the
    /// conversation open last is opened, else the first from the top.
    pub fn listed(&mut self, head: u64, mut crew: Vec<CrewMember>) -> Vec<Effect> {
        for r in crew.iter_mut().filter(|r| r.state == "running") {
            if let Some(old) = self.member(&r.name)
                && self.status_at.get(&r.name).is_some_and(|at| *at > head)
            {
                r.status = old.status.clone();
            }
        }
        self.crew = crew;
        self.reselect()
    }

    /// The linked yards as read again. Whom they trust as a user changes
    /// which row has whose mail, so the open row is opened again: it reads
    /// what it newly shows.
    pub fn linked(&mut self, peers: &[Peer]) -> Vec<Effect> {
        let before = self.open_sources();
        self.parties = parties(peers);
        self.trust = trusted(peers);
        self.regroup(before)
    }

    /// The users of the yard as read again.
    pub fn users(&mut self, users: Vec<User>) -> Vec<Effect> {
        let before = self.open_sources();
        self.set_users(users);
        self.regroup(before)
    }

    /// Whose mail the open row shows.
    fn open_sources(&self) -> Vec<String> {
        self.selected
            .as_ref()
            .map_or(Vec::new(), |sel| self.sources(sel))
    }

    /// The window's own user is one of the yard's whatever was read: their
    /// row is where the mail they wrote themselves is shown.
    fn set_users(&mut self, users: Vec<User>) {
        self.users = users.into_iter().map(|u| u.name).collect();
        if !self.me.is_empty() && !self.users.contains(&self.me) {
            self.users.push(self.me.clone());
        }
    }

    /// Who has a row changed; `before` is whose mail the open row showed.
    /// The open row stays when it is still there, and when it shows other
    /// mail than before that is seen and read. One of a name newly trusted
    /// as a user goes to that user's row.
    fn regroup(&mut self, before: Vec<String>) -> Vec<Effect> {
        if let Some(user) = self.selected_conversation().and_then(|s| self.folded(s)) {
            self.selected = Some(Selected::Conversation(user.to_string()));
        }
        let effects = self.reselect();
        if !effects.is_empty() || self.open_sources() == before {
            return effects;
        }
        self.opened();
        self.open()
    }

    /// The notices as read again. The yard row stands whatever they are,
    /// and where it is open a hook delivery that came is asked for its
    /// files.
    pub fn notifications(&mut self, list: Vec<Notification>) -> Vec<Effect> {
        let shown: HashSet<u64> = list
            .iter()
            .filter(|n| n.kind == "hook")
            .map(|n| n.seq)
            .collect();
        self.deliveries.retain(|seq, _| shown.contains(seq));
        self.delivering.retain(|seq| shown.contains(seq));
        self.delivery_notices.retain(|seq| shown.contains(seq));
        self.delivery_refetched.retain(|seq| shown.contains(seq));
        let before = std::mem::replace(&mut self.notifications, list);
        self.notice_news(&before);
        self.deliveries_wanted()
    }

    /// Marks one notice read: the oldest unread, one key. Elsewhere than
    /// the open yard row it does nothing.
    pub fn dismiss(&mut self) -> Vec<Effect> {
        if self.selected.as_ref() != Some(&Selected::Yard) {
            return Vec::new();
        }
        match self.notifications.iter().find(|n| !n.read) {
            Some(n) => vec![Effect::AckNotifications { through: n.seq }],
            None => Vec::new(),
        }
    }

    /// Marks every notice read: one key and the button in the yard row's
    /// head, through the newest unread. Elsewhere than the open yard row it
    /// does nothing.
    pub fn dismiss_all(&mut self) -> Vec<Effect> {
        if self.selected.as_ref() != Some(&Selected::Yard) {
            return Vec::new();
        }
        let unread = self.notifications.iter().filter(|n| !n.read);
        match unread.map(|n| n.seq).max() {
            Some(through) => vec![Effect::AckNotifications { through }],
            None => Vec::new(),
        }
    }

    /// The user's notifications through `through` are read: by this window,
    /// or by anything else that acknowledged them (`yardr inbox --ack`).
    pub fn notifications_acked(&mut self, through: u64) {
        for n in self.notifications.iter_mut().filter(|n| n.seq <= through) {
            n.read = true;
        }
    }

    /// The yard did not take the acknowledgement: the notices stay unread,
    /// and the next key asks again.
    pub fn notifications_ack_failed(&mut self, why: &str) {
        self.notice = format!("your notices could not be marked read: {why}");
    }

    /// The queue as read again. When the open bead or the open dealt with
    /// is gone the conversation open last is opened, else the first row
    /// from the top. A bead no longer in
    /// the queue leaves what the cards kept of it; the open bead is read
    /// when it is not yet.
    pub fn asks(&mut self, asks: Asks) -> Vec<Effect> {
        let open = std::mem::replace(&mut self.open, asks.open);
        let decided = std::mem::replace(&mut self.decided, asks.decided);
        let news = self.news.len();
        self.ask_news(&open, &decided);
        let mut keep = HashSet::new();
        for bead in self
            .open
            .iter()
            .map(|a| &a.bead)
            .chain(self.decided.iter().map(|a| &a.bead))
        {
            keep.insert(bead.clone());
        }
        // The open travelling bead is none of the queue's, and stays
        // what the details show. So does the drawer's, a closed bead
        // among them.
        if let Some(Selected::Bead(bead)) = self.selected.as_ref() {
            keep.insert(bead.clone());
        }
        keep.extend(self.drawn.iter().cloned());
        self.beads.retain(|bead, _| keep.contains(bead));
        self.chosen.retain(|(bead, _), _| keep.contains(bead));
        self.expanded.retain(|(bead, _)| keep.contains(bead));
        self.current.retain(|bead, _| keep.contains(bead));
        let mut effects = self.reselect();
        // Which row is open is known only now.
        let selected = self.selected.clone();
        for n in &mut self.news[news..] {
            n.open = selected.as_ref() == Some(&n.target);
        }
        if effects.is_empty() {
            // The open bead stayed: it is newly in the queue, or newly without
            // a read bead. It is read when it is not yet.
            if let Some(Selected::Decision(_) | Selected::Decided(_, _) | Selected::Bead(_)) =
                self.selected.as_ref()
            {
                effects.extend(self.open());
            }
        }
        // Every bead with open asks is read for its rows' counts, once:
        // the open bead is already asked above, and what the cache has is
        // not asked again. None of a yard the window does not show: its
        // rows are not drawn.
        if self.aside {
            return effects;
        }
        let open_bead = match selected {
            Some(Selected::Decision(bead) | Selected::Decided(bead, _) | Selected::Bead(bead)) => {
                Some(bead)
            }
            _ => None,
        };
        let mut read: HashSet<&str> = HashSet::new();
        for ask in &self.open {
            if self.beads.contains_key(&ask.bead)
                || open_bead.as_deref() == Some(ask.bead.as_str())
                || !read.insert(ask.bead.as_str())
            {
                continue;
            }
            effects.push(Effect::Bead {
                bead: ask.bead.clone(),
            });
        }
        effects
    }

    /// When a put-aside ask comes back next, if one waits: the window reads
    /// the queue again then, since that return arrives with no event. With
    /// `after`, the returns up to that moment are left out: the window has
    /// read the queue for them already.
    pub fn next_return(
        &self,
        after: Option<DateTime<chrono::Utc>>,
    ) -> Option<DateTime<chrono::Utc>> {
        self.decided
            .iter()
            .filter(|a| a.state == "put aside")
            .filter_map(|a| DateTime::parse_from_rfc3339(&a.return_at).ok())
            .map(|at| at.with_timezone(&chrono::Utc))
            .filter(|at| after.is_none_or(|after| *at > after))
            .min()
    }

    /// When the open row is gone, falls back within its mode. In chat, a
    /// decision opened from a conversation leads back there; decisions
    /// stay among their own rows, and the timetable keeps its board,
    /// line, No line or bead. A travelling bead is never gone: it shows
    /// what the Bead call returns, whether the yard serves the row still
    /// or not.
    fn reselect(&mut self) -> Vec<Effect> {
        // No row is open of a yard the window does not show.
        if self.aside {
            return Vec::new();
        }
        let order = self.order_in(self.mode);
        if self.selected.as_ref().is_some_and(|s| {
            (self.mode == Mode::Timetable && matches!(s, Selected::Bead(_))) || order.contains(s)
        }) {
            return Vec::new();
        }
        self.selected = match self.mode {
            Mode::Chat => self
                .back
                .clone()
                .map(Selected::Conversation)
                .filter(|s| order.contains(s))
                .or_else(|| order.into_iter().next()),
            Mode::Decisions | Mode::Timetable => self.row_in(self.mode, None),
        };
        self.opened();
        self.open()
    }
}

/// Text as it reaches the window: letters, line breaks and tabs, and no
/// other control character. U+2028 and U+2029 go with the control
/// characters: a viewer that breaks a line on them would let a title start
/// a line again. What another yard or a crew member wrote is shown as text
/// and nothing else.
pub fn plain(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\t' => Some(c),
            '\u{2028}' | '\u{2029}' => None,
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// The bead ids a text names, as its chips find them: an id alone, or the
/// bead an ask is named on. Each as often as it stands.
fn named_beads(text: &str) -> Vec<String> {
    markdown::blocks(text)
        .iter()
        .flat_map(|block| select::refs(&block.text))
        .filter_map(|found| match found.kind {
            select::RefKind::Bead(bead) | select::RefKind::AskOn { bead, .. } => Some(bead),
            select::RefKind::Ask(_) => None,
        })
        .collect()
}

/// A mail's time (RFC 3339, as the yard gives it) as `yardr chat` shows it,
/// in the zone of the person reading: `10-05 13:20`. Nothing for a time
/// that cannot be read.
pub fn clock<Tz: TimeZone>(at: &str, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match DateTime::parse_from_rfc3339(at) {
        Ok(t) => t.with_timezone(zone).format("%m-%d %H:%M").to_string(),
        Err(_) => String::new(),
    }
}

/// A time (RFC 3339, as the yard gives it) as the time of day alone, in the
/// zone of the person reading: `13:20`. For a time that stands under a day
/// the screen names already: the conversation's separator, the history's
/// day group. Nothing for a time that cannot be read.
pub fn clock_time<Tz: TimeZone>(at: &str, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match DateTime::parse_from_rfc3339(at) {
        Ok(t) => t.with_timezone(zone).format("%H:%M").to_string(),
        Err(_) => String::new(),
    }
}

/// Whether the mail `after` reads under `before`'s name and time: from the
/// same sender, within five minutes of it, on the same day in the zone of
/// the person reading. A time that cannot be read stands on its own.
fn grouped<Tz: TimeZone>(before: &Letter, after: &Letter, zone: &Tz) -> bool {
    if before.from != after.from {
        return false;
    }
    let (Ok(first), Ok(next)) = (
        DateTime::parse_from_rfc3339(&before.at),
        DateTime::parse_from_rfc3339(&after.at),
    ) else {
        return false;
    };
    if (next.timestamp() - first.timestamp()).abs() > 5 * 60 {
        return false;
    }
    first.with_timezone(zone).date_naive() == next.with_timezone(zone).date_naive()
}

/// What the mail says under its `correction` label, where it amends what
/// came before: its first word is `correction`, however it is capitalized,
/// and whatever follows it. A longer word that starts that way is none:
/// `correctional` is no correction.
///
/// The label says the word, so the text starts after it: the marks before
/// the word (`**`, `(`), those stuck to it (`**:`, `)`), the space after it
/// and a dash set off by spaces go with it. What opens the text itself
/// stays, so that its Markdown does. Where the word does not cut out
/// cleanly the text stays whole, as it was drawn before: a word joined to
/// letters by an apostrophe or hyphen (`correction's`) is a word of the
/// text, and a mark stuck before it that nothing closes beside the word
/// wraps the whole text, which would be left with its closing alone.
fn correction(text: &str) -> Option<&str> {
    let from = text.find(char::is_alphanumeric)?;
    let (before, from_word) = text.split_at(from);
    let (word, after) = from_word.split_at(
        from_word
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(from_word.len()),
    );
    if !word.eq_ignore_ascii_case("correction") {
        return None;
    }
    let rest = after.trim_start_matches(|c: char| !c.is_alphanumeric() && !c.is_whitespace());
    let opening = &before[before.trim_end_matches(|c: char| !c.is_whitespace()).len()..];
    let closing = &after[..after.len() - rest.len()];
    // A colon can touch the body (`correction:the`); an apostrophe or a
    // hyphen joining letters keeps the whole word (`correction's`).
    let joins_word = closing
        .chars()
        .all(|c| matches!(c, '\'' | '’' | '-' | '‐' | '‑'));
    if (rest.starts_with(char::is_alphanumeric) && joins_word) || !closes(opening, closing) {
        return Some(text);
    }
    // A dash between spaces parts the word from the text; one after a line
    // break is a list's bullet, and stays.
    let spaced = rest.trim_start_matches([' ', '\t']);
    let rest = match spaced.strip_prefix(['-', '–', '—']) {
        Some(dashed) if dashed.is_empty() || dashed.starts_with(char::is_whitespace) => dashed,
        _ => rest,
    };
    Some(rest.trim_start())
}

/// Whether the marks `closing`, stuck after a word, close the marks
/// `opening` stuck before it, each in the order it was opened: `(` by `)`,
/// `**` by `:**`.
fn closes(opening: &str, closing: &str) -> bool {
    let mut left = closing;
    opening.chars().rev().all(|open| {
        let close = match open {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '<' => '>',
            other => other,
        };
        left.find(close).is_some_and(|at| {
            left = &left[at + close.len_utf8()..];
            true
        })
    })
}

/// The day `at` (RFC 3339, as the yard gives it) falls on in the zone of
/// the person reading. Nothing for a time that cannot be read.
fn day_of<Tz: TimeZone>(at: &str, zone: &Tz) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(zone).date_naive())
}

/// The day's separator: `Today`, `Yesterday`, or the weekday with
/// the day and the month (`Saturday 17 Oct`), as the review's sketch
/// has it.
fn day_words(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        return "Today".to_string();
    }
    if day == today - chrono::Duration::days(1) {
        return "Yesterday".to_string();
    }
    format!("{} {} {}", day.format("%A"), day.day(), day.format("%b"))
}

/// A topic as the decisions show it: the label without its `topic:`
/// prefix, or "Everything else" for an ask with none.
fn display_topic(topic: &str) -> String {
    match topic.strip_prefix(TOPIC_PREFIX) {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ if topic.is_empty() => EVERYTHING_ELSE.to_string(),
        _ => topic.to_string(),
    }
}

/// A bead's id small, at a row's end: past its last `-`.
pub fn short_bead(bead: &str) -> String {
    bead.rsplit('-').next().unwrap_or(bead).to_string()
}

/// How many of the flow's beads stand on its `backlog` stage: the
/// panel's backlog cell counts them, and draws no stage cell for
/// them, so the queue reads as a count and never as chips.
pub fn backlog_count(flow: &Flow) -> usize {
    flow.stages
        .iter()
        .filter(|stage| stage.stage == "backlog")
        .map(|stage| stage.beads.len())
        .sum()
}

/// Whether the bead is another yard's question: `<id>@<peer>`, as the
/// queue names it. The yard refuses a plain note on it: notes stay local,
/// and words for the asker go as mail.
pub fn foreign_bead(bead: &str) -> bool {
    bead.contains('@')
}

/// A bead's row head: the question in plain words, wrapped to two lines
/// where it is drawn, or the bead's title with how many it asks.
fn decision_head(first: &OpenAsk, asks: &[&OpenAsk]) -> String {
    if asks.len() == 1 {
        return plain(&first.question);
    }
    let title = if first.bead_title.is_empty() {
        first.bead.clone()
    } else {
        first.bead_title.clone()
    };
    format!("{}: {} questions", row_head(&title), asks.len())
}

/// A decisions or Decided row's head: the first line of the question in
/// plain words. No later line, and no indent of the asker's, is drawn in
/// the list: the row shows one line, and its truncate() cuts it with an
/// ellipsis where it does not fit. Whose words they are, the row's sub says.
fn row_head(question: &str) -> String {
    asker_lines(question).into_iter().next().unwrap_or_default()
}

/// A bead's row sub: whose one ask is, what it wants and how long it
/// waits, or how long the oldest of several waits, whose own lines say
/// the rest. `options` is what the Bead read holds for the ask, 0 where
/// it was not read yet.
fn decision_sub<Tz: TimeZone>(
    first: &OpenAsk,
    asks: &[&OpenAsk],
    options: usize,
    now: &DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if asks.len() == 1 {
        return decision_meta(first, options, now);
    }
    waits(&first.since, now)
}

/// One open ask's line: whose it is, what it wants and how long it waits.
/// `options` is what the Bead read holds for the ask: 0 where it was not
/// read yet, where an ask with options reads as wanting words until then.
fn decision_meta<Tz: TimeZone>(ask: &OpenAsk, options: usize, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let wants = match options {
        1 => "1 option".to_string(),
        options if options > 1 => format!("{options} options"),
        _ if ask.secret.is_some() => "secret wanted".to_string(),
        _ if ask.run.is_some() => "approval wanted".to_string(),
        _ if ask.setup.is_some() => "approval wanted".to_string(),
        _ => "words wanted".to_string(),
    };
    [
        ask.asker.as_str(),
        wants.as_str(),
        waits(&ask.since, now).as_str(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// What became of a dealt with ask, as the history's icons and chips
/// name it. A pick is approved, even `you decide`, which leaves it to
/// the asker; a secret given and a words answer are words.
pub fn outcome_of(ask: &DecidedAsk) -> Outcome {
    match ask.state.as_str() {
        // A setup approval holds the names given too, with the preset
        // and what the yard runs: it is approved, not words.
        "answered" if !ask.given.is_empty() && ask.setup.is_none() => Outcome::Words,
        "answered" if ask.run.is_some() || ask.ran.is_some() => Outcome::Approved,
        "answered" if ask.option != 0 => Outcome::Approved,
        "answered" => Outcome::Words,
        "taken back" => Outcome::TakenBack,
        "declined" => Outcome::Declined,
        _ => Outcome::PutAside,
    }
}

/// A dealt with row's outcome in words, after its icon: `approved`, the
/// words answer, `gave <names>`, what took it back, `put aside`, or what
/// declined it.
fn decided_outcome(ask: &DecidedAsk) -> String {
    match ask.state.as_str() {
        "answered" if ask.option_label == YOU_DECIDE => format!("left to {}", ask.asker),
        "answered" if ask.option != 0 || ask.run.is_some() || ask.ran.is_some() => {
            "approved".to_string()
        }
        "answered" if !ask.given.is_empty() => format!("gave {}", ask.given.join(", ")),
        "answered" if !ask.words.is_empty() => plain(&ask.words),
        "answered" => "answered".to_string(),
        "declined" if ask.words.is_empty() => "declined".to_string(),
        "declined" => format!("declined: {}", plain(&ask.words)),
        "taken back" => taken_back(ask),
        _ => "put aside".to_string(),
    }
}

/// Whether a dealt with ask matches the history's filter: the chip's
/// outcome, and every word of the text in its question, asker or outcome,
/// in any case.
fn decided_matches(ask: &DecidedAsk, chip: Option<Outcome>, filter: &str) -> bool {
    if chip.is_some_and(|chip| outcome_of(ask) != chip) {
        return false;
    }
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {}",
        ask.question,
        ask.asker,
        decided_outcome(ask),
        outcome_of(ask).word(),
    )
    .to_lowercase();
    words.iter().all(|word| hay.contains(word))
}

/// A question's shape: digit runs as one `#`, so questions differing only
/// in numbers share it.
fn shape_of(question: &str) -> String {
    let mut shape = String::with_capacity(question.len());
    let mut in_number = false;
    for c in question.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                shape.push('#');
                in_number = true;
            }
        } else {
            shape.push(c);
            in_number = false;
        }
    }
    shape
}

/// Whether two dealt with asks fold into one history row: the same asker,
/// the same outcome, and questions differing only in numbers.
fn look_alike(a: &DecidedAsk, b: &DecidedAsk) -> bool {
    a.asker == b.asker
        && outcome_of(a) == outcome_of(b)
        && shape_of(&a.question) == shape_of(&b.question)
}

/// A decided day's head: `Today`, `Yesterday`, else the day and the month
/// (`17 Oct`, with the year where it is not this one); `Earlier` for an
/// ask whose time cannot be read.
fn decided_day(day: Option<NaiveDate>, today: NaiveDate) -> String {
    let Some(day) = day else {
        return "Earlier".to_string();
    };
    if day == today {
        return "Today".to_string();
    }
    if day == today.pred_opt().unwrap_or(day) {
        return "Yesterday".to_string();
    }
    if day.year() == today.year() {
        return format!("{} {}", day.day(), day.format("%b"));
    }
    format!("{} {} {}", day.day(), day.format("%b"), day.year())
}

/// What became of an ask its asker asks no more: `taken back by <by>:
/// <words>`. One another yard took back names nobody, and its words alone
/// say where it ended. It is never the person's own doing, so no "you".
fn taken_back(ask: &DecidedAsk) -> String {
    let words = plain(&ask.words);
    match (ask.by.is_empty(), words.is_empty()) {
        (true, true) => "taken back".to_string(),
        (true, false) => words,
        (false, true) => format!("taken back by {}", ask.by),
        (false, false) => format!("taken back by {}: {words}", ask.by),
    }
}

/// What one dealt with did with it, in plain words: `you picked 1 · Move
/// them now`, `put aside, back 10-11 09:30`, `you declined: <words>`,
/// `left to <asker>`, `taken back by <by>: <words>`. A pick carries no time: the yard serves none of its
/// own for an answer, and the ask's `since` is when it arrived, not when
/// it was decided.
fn decided_state<Tz: TimeZone>(ask: &DecidedAsk, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match ask.state.as_str() {
        // A secret ask's answer is the names, never a value.
        "answered" if !ask.given.is_empty() && ask.setup.is_none() => {
            format!("you gave {}", ask.given.join(", "))
        }
        "declined" if ask.secret.is_some() => "you declined".to_string(),
        // A setup ask's answer is the pack with the preset picked, and
        // the names given where the account was typed.
        "answered" if ask.setup.is_some() && ask.ran.is_some() => {
            setup_state(ask, ask.setup.as_ref().unwrap(), ask.ran.as_ref().unwrap())
        }
        // A run ask's answer is the command the yard ran, whole; what it
        // printed stands under it on the card. One the yard has not
        // carried out yet reads as being carried out, and one whose
        // carrying out failed as failed, with the failure under it.
        "answered" if ask.ran.is_some() => {
            let ran = ask.ran.as_ref().unwrap();
            let line = plain(&ran.line());
            if ran.failed {
                format!("approved, failed: {line}")
            } else if ran.result == CARRY_PENDING {
                format!("approved, being carried out: {line}")
            } else {
                format!("approved, ran: {line}")
            }
        }
        "answered" if ask.option == 0 => {
            format!("you answered: {}", plain(&ask.words))
        }
        "answered" if ask.option_label == YOU_DECIDE => {
            format!("left to {}", ask.asker)
        }
        "answered" => format!("you picked {} · {}", ask.option, plain(&ask.option_label)),
        "put aside" if ask.return_at.is_empty() => "put aside".to_string(),
        "put aside" => format!("put aside, back {}", clock(&ask.return_at, zone)),
        "declined" => format!("you declined: {}", plain(&ask.words)),
        "taken back" => taken_back(ask),
        _ => format!("{} · {}", ask.asker, plain(&ask.state)),
    }
}

/// What one dealt with did with a setup ask, in plain words: `approved,
/// being carried out: setup of pack courier, preset read to crew
/// yardmaster; gave COURIER_ACCOUNT`, `approved, ran: …` once the server
/// has carried it out, `approved, failed: …` where that failed, with what
/// it printed under it on the card. As a run ask's three readings are.
fn setup_state(ask: &DecidedAsk, setup: &Setup, ran: &Ran) -> String {
    let preset = match ask.option_label.is_empty() {
        true => format!("preset {}", ask.option),
        false => format!("preset {}", plain(&ask.option_label)),
    };
    let mut what = format!(
        "setup of pack {}, {} to {}",
        plain(&setup.pack),
        preset,
        setup_grantees(&setup.to)
    );
    if !ask.given.is_empty() {
        what.push_str(&format!("; gave {}", ask.given.join(", ")));
    }
    let reading = if ran.failed {
        "approved, failed"
    } else if ran.result == CARRY_PENDING {
        "approved, being carried out"
    } else {
        "approved, ran"
    };
    format!("{reading}: {what}")
}

/// Who a setup ask's grants are for, in words: each crew member and each
/// group by name, the last two joined with "and", as the yard's card has
/// it (`granteesWord` in internal/store/runask_setup.go of the yardr
/// repository).
pub fn setup_grantees(to: &[api::SetupGrantee]) -> String {
    let words: Vec<String> = to
        .iter()
        .filter_map(|t| {
            if !t.crew.is_empty() {
                Some(format!("crew {}", plain(&t.crew)))
            } else if !t.group.is_empty() {
                Some(format!("group {}", plain(&t.group)))
            } else {
                None
            }
        })
        .collect();
    match words.len() {
        0 => String::new(),
        1 | 2 => words.join(" and "),
        _ => format!(
            "{} and {}",
            words[..words.len() - 1].join(", "),
            words[words.len() - 1]
        ),
    }
}

/// The account names a setup ask's answer types: the names the update
/// newly needs where its standing file misses them, else the whole
/// account, as the yard's approval holds it (`approveSetup` in
/// internal/store/runask_setup.go of the yardr repository).
pub fn setup_typed_names(setup: &Setup) -> &[String] {
    match setup.missing.is_empty() {
        true => &setup.account,
        false => &setup.missing,
    }
}

/// Whether a setup ask's standing account is kept with a bare preset
/// pick: a drift card whose file holds every account name.
pub fn setup_keepable(setup: &Setup) -> bool {
    setup.given && setup.missing.is_empty()
}

/// What approving a setup ask does, in one line: the apply, the account
/// and the grants, as the yard's card has it (`Summary` in
/// internal/store/runask_setup.go of the yardr repository).
pub fn setup_summary(setup: &Setup) -> String {
    format!(
        "yardr pack apply {}, the account {}, and the preset's grants to {}",
        plain(&setup.pack),
        setup
            .account
            .iter()
            .map(|name| plain(name))
            .collect::<Vec<_>>()
            .join(" and "),
        setup_grantees(&setup.to)
    )
}

/// What the yard answered to a secret that was given, in one line: the
/// names, and the yard's words for where the source's credential is set
/// now or what it still wants. It ends with what the yard says is left to
/// do, where it says so: the card that named it leaves with the giving.
pub fn given_line(given: &api::Given) -> String {
    let state = [&given.credential, &given.missing]
        .into_iter()
        .find(|words| !words.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("written to {}", given.file));
    let mut line = format!("gave {} · {state}", given.given.join(", "));
    if !given.next.is_empty() {
        line.push_str(" · ");
        line.push_str(&given.next);
    }
    line
}

/// Why a secret was not given, for the card. A call that ended without an
/// answer may have written the value all the same, so it is never said to
/// be sent or not sent, unless the connect gave up, which sent nothing; a
/// yard that does not have the call sends its person to a terminal. The
/// words hold what the yard said and never a value.
pub fn not_given(err: &api::Error, bead: &str, ask: u64) -> String {
    match err {
        api::Error::Slow {
            idempotent: false,
            sent: false,
        } => "the yard did not answer. The value was not written. Give it again.".to_string(),
        api::Error::Down(_)
        | api::Error::Protocol(_)
        | api::Error::Slow {
            idempotent: false,
            sent: true,
        } => "the yard did not answer. The value may be written. Give it again.".to_string(),
        err if err.may_have_run() => {
            "the yard did not answer. The value may be written. Give it again.".to_string()
        }
        // The yard's one answer with no code of its own: its words are
        // read here, and when they change this is shown as any refusal.
        api::Error::Refused { message, .. } if message.starts_with("no such method") => format!(
            "this yard takes no secret from footplate. Answer in a terminal on the yard's machine: yardr bead answer {bead} {ask} --secret"
        ),
        _ => format!("not given: {err}"),
    }
}

/// One class of a weighty card's weight as its head shows it: as
/// `yardr bead ask` prints it, with the class and the yard's line for
/// what this card does.
pub fn weight_line(class: &str, line: &str) -> String {
    format!("WEIGHTY · {}: {}", plain(class), plain(line))
}

/// The command that approves a run ask in a terminal on the yard's machine:
/// the way for a window the yard takes no approval from.
pub fn approve_command(bead: &str, ask: u64) -> String {
    format!("yardr bead answer {bead} {ask} --approve")
}

/// What an approval the yard carries out after the call holds until the
/// carrying out completes it: `CarryPending` in internal/store/runask.go of
/// the yardr repository, which the card reads as being carried out.
pub const CARRY_PENDING: &str = "approved, to be carried out";

/// What the yard answered to an approval, in one line: the command as it
/// ran, whole, and what it printed; or that it is being carried out, where
/// the yard answers with `CARRY_PENDING`. The card that proposed it
/// leaves with the approval.
pub fn approved_line(approved: &api::Approved) -> String {
    let ran = Ran {
        argv: approved.argv.clone(),
        result: approved.result.clone(),
        failed: false,
    };
    if ran.result == CARRY_PENDING {
        return format!("approved, being carried out: {}", plain(&ran.line()));
    }
    match plain(&ran.result) {
        result if result.is_empty() => format!("approved, ran: {}", plain(&ran.line())),
        result => format!("approved, ran: {} · {result}", plain(&ran.line())),
    }
}

/// Whether the yard has no approval call for this window: one from before
/// the call, or one whose listener does not serve it to this device, which
/// it says in the same words. The card then sends its person to a terminal.
pub fn approves_elsewhere(err: &api::Error) -> bool {
    // The yard's one answer with no code of its own: its words are read
    // here, and when they change this is shown as any refusal.
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("no such method"))
}

/// Whether the yard refused the files of a note because its `AddNote`
/// takes none: a yardr from before notes carried files. Its words are
/// read here, as above, and when they change this is shown as any
/// refusal.
pub fn note_files_too_old(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("argument \"files\""))
}

/// Whether the yard has no such Lines call: one from before Arrivals
/// and Flows, which says it has no such method. That is no error: the
/// Lines pane says the yard is too old, in plain words, and no notice
/// is raised. Its words are read here, as above, and when they change
/// this is shown as any refusal.
pub fn lines_too_old(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("no such method"))
}

/// What the Lines pane says of a yard from before the Lines view, in
/// place of the arrivals or the flows it would draw: plain words, never
/// the wire's method names, and never in red.
pub const LINES_TOO_OLD: &str =
    "this yard's yardr is older than the Lines view (it needs yardr from 16 Oct 2026 or later)";

/// Whether the yard has no SourceGrants call: one from before the grants
/// view, which says it has no such method. That is no error: the grants
/// pane says the yard is too old, in plain words, and no notice is
/// raised. Its words are read here, as above, and when they change this
/// is shown as any refusal.
pub fn grants_too_old(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("no such method"))
}

/// What the grants pane says of a yard from before the grants view, in
/// place of the grants it would list: plain words, never the wire's
/// method name, and never in red.
pub const GRANTS_TOO_OLD: &str =
    "this yard's yardr is older than the grants view (it needs yardr from 21 Oct 2026 or later)";

/// Whether the yard refused the grant list because the request came down
/// the link from its edge: it serves the list to no device that comes so,
/// whoever holds it. That is no fault of the device's, whatever the rest of
/// the refusal says of it: the grants pane says they are read on the
/// yard's machine, and no notice is raised. Its words are read here, as
/// above, and when they change this is shown as any refusal.
pub fn grants_over_the_link(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("the operator's grant list is not read over the link"))
}

/// What the grants pane says of a yard that serves the grant list on its
/// own machine alone, in place of the grants it would list: one line of
/// plain words, never in red, since nothing is wrong. A window reaches a
/// hosted yard by its edge, and the yard serves no such call down the link.
pub const GRANTS_ON_THE_YARD: &str =
    "the grants are read on the yard's machine: yardr source grants";

/// Whether the yard refused a session's screen because the request came
/// down the link from its edge: it reads none to a device that comes so,
/// whoever holds it. That is no fault of the device's, whatever the rest of
/// the refusal says of it: the card says the screen is read on the yard's
/// machine, in no red. Its words are read here, as above, and when they
/// change this is shown as any refusal. The yard's other refusals of a
/// screen (plain HTTP from another machine) are the address's fault, and
/// stay red.
pub fn screen_over_the_link(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("a session's screen is not read over the link"))
}

/// What the screen's card says of a yard that reads a session's screen on
/// its own machine alone, in place of the screen it would draw: one line of
/// plain words, never in red, since nothing is wrong. A window reaches a
/// hosted yard by its edge, and the yard serves no such call down the link.
pub const SCREEN_ON_THE_YARD: &str = "the screen is read on the yard's machine";

/// Whether the yard has no telemetry calls: one from before them, or one
/// whose listener serves them to no such device, which it says in the
/// same words. That is no error: the Telemetry section is not offered,
/// and no notice is raised. Its words are read here, as above, and when
/// they change this is shown as any refusal.
pub fn telemetry_too_old(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("no such method"))
}

/// Whether the yard refused the telemetry preview or its change because
/// the request came down the link from its edge: it serves neither to a
/// device that comes so, whoever holds it. That is no error either: the
/// section is not offered, and no notice is raised. Its words are read
/// here, as above, and when they change this is shown as any refusal.
pub fn telemetry_over_the_link(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. }
        if message.starts_with("the telemetry switch is not read over the link")
            || message.starts_with("the telemetry switch is not changed over the link"))
}

/// Whether the yard refused the switch's change from where this window
/// reaches it, while it reads the preview there: the yard's router takes
/// the read over https and no change. That is no fault of the device's:
/// the section keeps the state, the id and the payload read-only, and
/// says the switch is changed in the yard's web view. Its words are read
/// here, as above, and when they change this is shown as any refusal.
pub fn telemetry_router_refused(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("the telemetry switch is taken over https only"))
}

/// What the Telemetry section says where the yard refused the switch's
/// change from where the window reaches it: one sentence, never in red,
/// since nothing is wrong, with Open beside it where the yard's web view
/// answers.
pub const TELEMETRY_READ_ONLY: &str = "the switch is changed in the yard's web view";

/// The consent words the switch is turned on under, as every surface
/// shares them: copied from yardr's `internal/telemetry` (`Questions`,
/// `Profile`, `IDSentence`, `DeletionSentence`), which the on and off
/// help and the web control say too. The four questions the batch
/// answers, and where it goes.
pub const TELEMETRY_QUESTIONS: &str = "When telemetry is on, the daemon sends one batch a day to https://eu.i.posthog.com/batch/ answering four questions: which yardr versions and OS are running; which harnesses and models do the work; where beads fail, as counts by kind; whether yards keep running after 1, 7 and 30 days.";

/// The id-only PostHog profile, as the consent words say it.
pub const TELEMETRY_PROFILE: &str = "The batch carries the yard's random install id as PostHog's distinct_id, and PostHog keeps an id-only person profile for it — no name, no email — so a yard's data is deleted by install id.";

/// The pseudonymous sentence, as the consent words say it.
pub const TELEMETRY_ID_SENTENCE: &str = "This id is pseudonymous, not anonymous: anyone you give it to can find this yard's events, and giving it is your act.";

/// How the operator has the yard's data deleted, said wherever a batch
/// is previewed.
pub const TELEMETRY_DELETION: &str =
    "To have this yard's data deleted, send the project this install id.";

/// The switch as it stands, in the section's words: `on (since …)` with
/// the grant's date, else `off — nothing is sent`. As `yardr telemetry
/// show` and the web control say it.
pub fn telemetry_state(preview: &TelemetryPreview) -> String {
    match preview.telemetry.as_str() {
        "on" if preview.since.is_empty() => "on".to_string(),
        "on" => format!("on (since {})", preview.since),
        _ => "off — nothing is sent".to_string(),
    }
}

/// The next request's line, as `yardr telemetry show` renders it: the
/// sending day and the source days the batch holds.
pub fn telemetry_next_request(preview: &TelemetryPreview) -> String {
    format!(
        "sending day {} (UTC); {}",
        preview.sending_day,
        telemetry_source_days(&preview.source_days)
    )
}

/// The source days of the next request line, as `yardr telemetry show`
/// renders them.
pub fn telemetry_source_days(days: &[String]) -> String {
    match days.len() {
        0 => "no eligible source days".to_string(),
        1 => format!("source day {}", days[0]),
        _ => format!("source days {}..{}", days[0], days[days.len() - 1]),
    }
}

/// Why a preview has no batch, in words: the same line the CLI and the
/// web view print.
pub fn telemetry_no_batch(reason: &str) -> String {
    match reason {
        "off" => "no batch available until telemetry is enabled".to_string(),
        "attempted" => "no batch available until the next UTC day".to_string(),
        "empty" => "no batch available: nothing to send".to_string(),
        _ => "no batch available: no install id (run yardr telemetry on again)".to_string(),
    }
}

/// The next payload as the yard returned it, as indented JSON, as show
/// renders it; `None` where there is no batch to send.
pub fn telemetry_payload(preview: &TelemetryPreview) -> Option<String> {
    preview
        .request
        .as_ref()
        .and_then(|request| serde_json::to_string_pretty(request).ok())
}

/// Whether the yard has no DeliveryFiles call: one from before a device
/// could learn a hook delivery's files, which says it has no such method.
/// That is no error: the notices show no files, and no notice is raised.
/// Its words are read here, as above, and when they change this is shown
/// as any refusal.
pub fn deliveries_too_old(err: &api::Error) -> bool {
    matches!(err, api::Error::Refused { message, .. } if message.starts_with("no such method"))
}

/// Who holds a grant, with the user called as where there is one:
/// `crew member yardmaster`, `group builders as ada`.
pub fn grant_holder(grant: &SourceGrant) -> String {
    let holder = match grant.crew.is_empty() {
        false => format!("crew member {}", plain(&grant.crew)),
        true => format!("group {}", plain(&grant.group)),
    };
    match grant.as_user.is_empty() {
        true => holder,
        false => format!("{holder} as {}", plain(&grant.as_user)),
    }
}

/// Whether the call reads as an update, by the kind the yard served for it
/// (`SourceGrant::kinds`): `read` is a read and `update` an update. A call
/// with no kind served (its source is gone or unreadable, it is no longer
/// declared, or the yard is from before it served kinds) is taken for an
/// update, the weightier reading, but for the word `read`: the grant of
/// every read call is a read whatever the yard could read of its source.
pub fn grant_is_update(call: &str, grant: &SourceGrant) -> bool {
    match grant.kinds.get(call).map(String::as_str) {
        Some("read") => false,
        Some("update") => true,
        _ => call != "read",
    }
}

/// The line that takes one grant back, to copy: `yardr source revoke`
/// with the source, the call and the holder, with `--as` where the grant
/// has one.
pub fn grant_revoke(source: &str, call: &str, grant: &SourceGrant) -> String {
    // Identifiers from SourceGrants are yard text. Keep ordinary names in
    // the CLI's familiar form, and quote other values before a person
    // copies the line into a shell. Put flags before `--` in that case so
    // a value beginning with `-` cannot become another option.
    let simple = |word: &str| {
        word.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
    };
    let shell_word = |word: &str| {
        let visible = plain(word).replace(['\n', '\t'], " ");
        if simple(&visible) {
            visible
        } else {
            format!("'{}'", visible.replace('\'', "'\\''"))
        }
    };
    if ![
        source,
        call,
        grant.group.as_str(),
        grant.crew.as_str(),
        grant.as_user.as_str(),
    ]
    .into_iter()
    .all(|word| word.is_empty() || simple(word))
    {
        let mut args = vec![
            "yardr".to_string(),
            "source".to_string(),
            "revoke".to_string(),
        ];
        if !grant.crew.is_empty() {
            args.push(format!("--crew={}", shell_word(&grant.crew)));
        } else if !grant.group.is_empty() {
            args.push(format!("--group={}", shell_word(&grant.group)));
        }
        if !grant.as_user.is_empty() {
            args.push(format!("--as={}", shell_word(&grant.as_user)));
        }
        args.push("--".to_string());
        args.push(shell_word(source));
        args.push(shell_word(call));
        return args.join(" ");
    }
    let mut line = format!("yardr source revoke {source} {call}");
    if !grant.crew.is_empty() {
        line.push_str(&format!(" --crew {}", grant.crew));
    } else if !grant.group.is_empty() {
        line.push_str(&format!(" --group {}", grant.group));
    }
    if !grant.as_user.is_empty() {
        line.push_str(&format!(" --as {}", grant.as_user));
    }
    line
}

/// How the grants view says a time: `17 Oct 19:02`, in the zone of the
/// person reading, as `yardr source grants` says it. Empty for a time
/// that cannot be read.
pub fn grant_time(at: &str) -> String {
    match DateTime::parse_from_rfc3339(at) {
        Ok(t) => t
            .with_timezone(&chrono::Local)
            .format("%-d %b %H:%M")
            .to_string(),
        Err(_) => String::new(),
    }
}

/// One call's own grant, as its row shows it: when it was granted and by
/// whom, with the bead, the ask and who asked it where it was approved
/// on a card. A call with no entry reads `not kept` and `-`: it was
/// granted before the yard kept each call's own.
pub fn grant_given(given: Option<&GrantGiven>) -> (String, String) {
    let Some(given) = given else {
        return ("not kept".to_string(), "-".to_string());
    };
    let granted = grant_time(&given.at);
    let mut by = plain(&given.by);
    if !given.bead.is_empty() {
        let asker = match given.asker.is_empty() {
            true => "unknown".to_string(),
            false => plain(&given.asker),
        };
        by.push_str(&format!(
            ", on {} ask {} asked by {asker}",
            plain(&given.bead),
            given.ask
        ));
    }
    (granted, by)
}

/// What the grants view says is granted, as `yardr source grants` prints
/// it: how many calls of how many sources to how many holders, with how
/// many may change something outside the yard and how many call as a
/// user. A holder's plain grant and its grant as a user are one holder.
pub fn grant_summary(grants: &[SourceGrant]) -> String {
    use std::collections::HashSet;
    let mut sources = HashSet::new();
    let mut holders = HashSet::new();
    let mut rows = 0;
    let mut updates = 0;
    let mut as_user = 0;
    for grant in grants {
        sources.insert(grant.source.as_str());
        holders.insert((grant.group.as_str(), grant.crew.as_str()));
        for call in &grant.calls {
            rows += 1;
            if grant_is_update(call, grant) {
                updates += 1;
            }
            if !grant.as_user.is_empty() {
                as_user += 1;
            }
        }
    }
    let calls = count_phrase(rows, "call", "calls");
    let sources = count_phrase(sources.len(), "source", "sources");
    let holders = count_phrase(holders.len(), "holder", "holders");
    // The command says `1 calls` and `2 call`, its verb swapped; the view
    // says what reads.
    let as_words = count_phrase(as_user, "call", "calls");
    format!(
        "{calls} granted of {sources} to {holders}: {updates} may change something outside the yard, {as_words} as a user."
    )
}

/// `n` things in words: `1 call`, `2 calls`, as the command says them.
fn count_phrase(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// One line for each record holding a call whose grant was not kept, as
/// the command prints them below its summary: when the record last gained
/// a call and by whom, for no call in particular.
pub fn grant_unkept(grants: &[SourceGrant]) -> Vec<String> {
    grants
        .iter()
        .filter(|grant| grant.calls.iter().any(|call| !grant.given.contains_key(call)))
        .map(|grant| {
            format!(
                "{} to {}: granted before its history was kept; the record last gained a call {} by {}.",
                plain(&grant.source),
                grant_holder(grant),
                grant_time(&grant.at),
                plain(&grant.by)
            )
        })
        .collect()
}

/// The weighty lines of one grant row, as a card shows them: one for an
/// update call, one where the grant is as a user, both where both hold,
/// none for a read, named or plain. Drawn in rust as text, never red.
pub fn grant_weight_lines(source: &str, call: &str, grant: &SourceGrant) -> Vec<String> {
    let mut lines = Vec::new();
    if grant_is_update(call, grant) {
        lines.push(weight_line(
            "outside",
            &format!(
                "lets the yard change something outside it: {} may call {} of {} (an update)",
                grant_holder(grant),
                plain(call),
                plain(source)
            ),
        ));
    }
    if !grant.as_user.is_empty() {
        lines.push(weight_line(
            "as-user",
            &format!(
                "lets calls or words count as a person's: as {}",
                plain(&grant.as_user)
            ),
        ));
    }
    lines
}

/// The grants in the order the view lists them: by source, then holder,
/// then the user called as, as served within each record's calls.
pub fn ordered_grants(grants: &[SourceGrant]) -> Vec<&SourceGrant> {
    let mut ordered: Vec<&SourceGrant> = grants.iter().collect();
    ordered.sort_by(|a, b| {
        (
            a.source.as_str(),
            a.crew.as_str(),
            a.group.as_str(),
            a.as_user.as_str(),
        )
            .cmp(&(
                b.source.as_str(),
                b.crew.as_str(),
                b.group.as_str(),
                b.as_user.as_str(),
            ))
    });
    ordered
}

/// What the yard answered to a setup approval, in one line: the pack with
/// the preset picked and who the grants are for, with the names given,
/// the candidate taken or the standing account kept, and that the server
/// carries the setup out after the call. The card that proposed it leaves
/// with the approval.
pub fn setup_approved_line(setup: &Setup, approved: &api::SetupApproved) -> String {
    let mut what = format!(
        "setup of pack {}, preset {} to {}",
        plain(&approved.pack),
        plain(&approved.preset),
        setup_grantees(&setup.to)
    );
    if !approved.account.is_empty() {
        what.push_str(&format!("; took {}", plain(&approved.account)));
    } else if !approved.given.is_empty() {
        what.push_str(&format!("; gave {}", approved.given.join(", ")));
    } else if approved.keep {
        what.push_str("; kept");
    }
    format!("approved, being carried out: {what}")
}

/// The command that approves a setup ask in a terminal on the yard's
/// machine: the way for a window the yard takes no approval from. Where
/// the card lists candidates, the account goes by name or typed; a drift
/// card whose file holds every name is kept with a bare preset pick.
pub fn setup_approve_command(bead: &str, ask: u64, setup: &Setup) -> String {
    let head = format!("yardr bead answer {bead} {ask}");
    match (setup_keepable(setup), setup.candidates.is_empty()) {
        (true, true) => format!("{head} <n> or <n> --secret"),
        (true, false) => format!("{head} <n>, <n> --secret, or <n> --account <source>"),
        (false, true) => format!("{head} <n> --secret"),
        (false, false) => format!("{head} <n> --secret, or <n> --account <source>"),
    }
}

/// Why a run or setup ask was not approved, for the card. A call that
/// ended without an answer may have run all the same, so it is never said
/// to be approved or not: the card, read again, says which, unless the
/// connect gave up, which sent nothing. A refusal is the yard's words,
/// whole.
pub fn not_approved(err: &api::Error) -> String {
    match err {
        api::Error::Slow {
            idempotent: false,
            sent: false,
        } => "the yard did not answer; nothing was approved. Approve it again.".to_string(),
        api::Error::Down(_)
        | api::Error::Protocol(_)
        | api::Error::Slow {
            idempotent: false,
            sent: true,
        } => "the yard did not answer; read the card again".to_string(),
        err if err.may_have_run() => "the yard did not answer; read the card again".to_string(),
        _ => format!("not approved: {err}"),
    }
}

/// One asker's text as the lines a run ask's card marks as theirs: split
/// on whatever breaks a line, so that what follows a break is marked too,
/// without the control characters and the empty.
fn asker_lines(text: &str) -> Vec<String> {
    text.split([
        '\n', '\r', '\u{b}', '\u{c}', '\u{85}', '\u{2028}', '\u{2029}',
    ])
    .map(|line| plain(line).trim().to_string())
    .filter(|line| !line.is_empty())
    .collect()
}

/// A run ask's question as its card shows it: each line marked as the
/// asker's, `yardmaster says: …`, so that none can begin as a line of the
/// yard's does, whatever its words are. Another ask's is as it was written.
fn card_question(question: &str, asker: &str, run: bool) -> String {
    if !run {
        return plain(question);
    }
    asker_lines(question)
        .iter()
        .map(|line| format!("{asker} says: {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A card's context lines. A run ask's are marked as the asker's, each line
/// of them: `yardmaster says: …`, so that none can begin as a line of the
/// yard's does, whatever its words are.
fn card_context(context: &[String], asker: &str, run: bool) -> Vec<String> {
    if !run {
        return context.iter().map(|c| plain(c)).collect();
    }
    context
        .iter()
        .flat_map(|c| asker_lines(c))
        .map(|line| format!("{asker} says: {line}"))
        .collect()
}

/// An ask's options as the card numbers them, from 1, `you decide` last
/// as the yard lists it.
fn card_options(ask: &BeadAsk) -> Vec<CardOption> {
    ask.options
        .iter()
        .enumerate()
        .map(|(i, o)| CardOption {
            number: i as u64 + 1,
            label: plain(&o.label),
            consequence: plain(&o.consequence),
        })
        .collect()
}

/// The recommended option's label; empty when nothing is recommended.
fn recommend_label(ask: &BeadAsk) -> String {
    if ask.recommend == 0 {
        return String::new();
    }
    ask.options
        .get(ask.recommend.saturating_sub(1) as usize)
        .map(|o| plain(&o.label))
        .unwrap_or_default()
}

/// A design bead's document, named by path and branch, not fetched: the
/// first `docs/design/…` the body or a note names, on the bead's branch.
/// Empty when none is named.
fn design_doc(bead: &str, body: &str, notes: &[crate::api::BeadNote]) -> String {
    const PREFIX: &str = "docs/design/";
    let mut texts = vec![body];
    texts.extend(notes.iter().map(|n| n.text.as_str()));
    for text in texts {
        let Some(at) = text.find(PREFIX) else {
            continue;
        };
        let rest = &text[at..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '"' | '\''))
            .unwrap_or(rest.len());
        let mut path = rest[..end].trim_end_matches([',', ';', ':']).to_string();
        // `Doc: <path>.` ends the sentence past the path's `.md`.
        if path.ends_with(".md.") {
            path.pop();
        }
        path = path.trim_end_matches('.').to_string();
        if path.len() > PREFIX.len() {
            return format!("{path} on branch yard/{bead}");
        }
    }
    String::new()
}

/// How long a bead has stood at its stage, from its served entry to
/// now: `41 min at new`. A stay under a minute reads a minute; longer
/// ones read hours, then days. Nothing for a time that cannot be read.
pub fn stage_age<Tz: TimeZone>(entered: &str, stage: &str, now: &DateTime<Tz>) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let Ok(entered) = DateTime::parse_from_rfc3339(entered) else {
        return None;
    };
    let minutes = ((now.timestamp() - entered.timestamp()) / 60).max(1);
    let span = if minutes < 60 {
        format!("{minutes} min")
    } else if minutes < 24 * 60 {
        format!("{} h", minutes / 60)
    } else {
        match minutes / (24 * 60) {
            1 => "1 day".to_string(),
            days => format!("{days} days"),
        }
    };
    Some(format!("{span} at {stage}"))
}

/// How long an ask waits, from its `since` to now: `now`, `5m`, `2h`, `3d`.
/// Nothing for a time that cannot be read.
pub fn waits<Tz: TimeZone>(since: &str, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Ok(since) = DateTime::parse_from_rfc3339(since) else {
        return String::new();
    };
    let minutes = (now.timestamp() - since.timestamp()) / 60;
    if minutes < 1 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 24 * 60 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}d", minutes / (24 * 60))
    }
}

/// Why what is typed is not sent as it is, if it is not: only a text with
/// nothing in it is held back, and its reason is empty, since there is
/// nothing to say. How long a text may be is the yard's to say: it sends
/// one past a mail as numbered mails, and refuses one past what they hold,
/// in its own words.
pub fn unsendable(text: &str) -> Option<String> {
    text.trim().is_empty().then(String::new)
}

/// The type of an image, read from its bytes as the yard reads it:
/// `image/png` or `image/jpeg`, and nothing for anything else. The yard
/// takes any bytes; an image is what the window draws.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// The tags `http.DetectContentType` answers `text/html` for, in upper
/// case: what the yard takes a page by.
const PAGE_TAGS: &[&[u8]] = &[
    b"<!DOCTYPE HTML",
    b"<HTML",
    b"<HEAD",
    b"<SCRIPT",
    b"<IFRAME",
    b"<H1",
    b"<DIV",
    b"<FONT",
    b"<TABLE",
    b"<A",
    b"<STYLE",
    b"<TITLE",
    b"<B",
    b"<BODY",
    b"<BR",
    b"<P",
    b"<!--",
];

/// The type of a page, read from its bytes as the yard reads it:
/// `text/html; charset=utf-8`, and nothing for anything else. Past the
/// leading whitespace, one of the tags above, the case aside, ended by a
/// space or `>`: what `http.DetectContentType` answers. The yard takes
/// any bytes; a page is what the browser opens from a row with run.
pub fn page_type(bytes: &[u8]) -> Option<&'static str> {
    let start = bytes
        .iter()
        .position(|b| !matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        .unwrap_or(bytes.len());
    let head = &bytes[start..];
    PAGE_TAGS
        .iter()
        .any(|tag| {
            head.len() > tag.len()
                && head[..tag.len()].eq_ignore_ascii_case(tag)
                && matches!(head[tag.len()], b' ' | b'>')
        })
        .then_some("text/html; charset=utf-8")
}

/// Why an image of `bytes` bytes is not sent in a yard whose cap on one is
/// `cap`, if it is not.
pub fn over_cap(bytes: usize, cap: u64) -> Option<String> {
    over_cap_as("image", bytes, cap)
}

/// Why a page of `bytes` bytes is not sent in a yard whose cap on one is
/// `cap`, if it is not: the cap is the yard's, whatever the file.
pub fn page_over_cap(bytes: usize, cap: u64) -> Option<String> {
    over_cap_as("page", bytes, cap)
}

/// Why a file of `bytes` bytes, an image, a page or any other, is not
/// sent in a yard whose cap on one is `cap`, if it is not.
fn over_cap_as(what: &str, bytes: usize, cap: u64) -> Option<String> {
    (bytes as u64 > cap).then(|| {
        format!(
            "the {what} is {}, and this yard takes one of {} at most (yardr settings: blob_max_bytes)",
            size(bytes as u64),
            size(cap)
        )
    })
}

/// Why bytes with `head` of `size` bytes are not sent in a yard whose cap
/// on one file is `cap`, if they are not: the refusal names what they
/// are, an image, a page or a file, as the yard tells them apart.
pub fn over_cap_for(head: &[u8], size: usize, cap: u64) -> Option<String> {
    match (image_type(head), page_type(head)) {
        (Some(_), _) => over_cap(size, cap),
        (None, Some(_)) => page_over_cap(size, cap),
        (None, None) => over_cap_as("file", size, cap),
    }
}

/// Whether `file` is an image, a PNG or a JPEG: what its type says, as
/// the yard told it apart. Anything else is a page or a file, which the
/// window never asks the yard for.
pub fn is_image(file: &BlobRef) -> bool {
    matches!(file.kind.as_str(), "image/png" | "image/jpeg")
}

/// Whether `file` is a page, an HTML blob: what its type says before any
/// `;`.
pub fn is_page(file: &BlobRef) -> bool {
    file.kind
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim() == "text/html")
}

/// The hash of `file` among the images to ask for, once: one this yard
/// did not take from a linked yard is none it has, and a page or a file
/// is opened in the browser, not asked for.
fn push_image<'a>(out: &mut Vec<&'a str>, file: &'a BlobRef) {
    if is_image(file) && file.not_taken.is_empty() && !out.contains(&file.hash.as_str()) {
        out.push(&file.hash);
    }
}

/// The line for a file of mail from a linked yard or of a hook delivery
/// that this yard did not take, with the yard's reason: under its name
/// where it has one, as a file that was taken is, else as what it is. The
/// name and the reason of a delivery's file can carry the source's own
/// words, so they are shown as text and nothing else.
pub fn not_taken(file: &BlobRef) -> String {
    let what = match (is_page(file), is_image(file)) {
        (true, _) => "page",
        (false, true) => "image",
        (false, false) => "file",
    };
    let named = match file.name.is_empty() {
        true => what.to_string(),
        false => plain(&file.name),
    };
    format!(
        "{named}, {}, not taken: {}",
        size(file.bytes),
        plain(&file.not_taken)
    )
}

/// A number of bytes for a person: `70 bytes`, `812 KiB`, `8.5 MiB`.
pub fn size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    match bytes {
        n if n < KIB => format!("{n} bytes"),
        n if n < MIB => format!("{} KiB", n / KIB),
        n => format!("{:.1} MiB", n as f64 / MIB as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AboutBead, AskOption, BeadNote, RosterEntry, Said};
    use serde_json::{Value, json};

    fn member(name: &str, state: &str, status: &str) -> CrewMember {
        CrewMember {
            name: name.into(),
            enabled: true,
            state: state.into(),
            status: status.into(),
            login_screen: false,
            login_led: false,
        }
    }

    /// `member` sitting at its harness's login screen, which the yard
    /// leads where `led` says.
    fn at_login(member: CrewMember, led: bool) -> CrewMember {
        CrewMember {
            login_screen: true,
            login_led: led,
            ..member
        }
    }

    fn north() -> Peer {
        Peer {
            name: "north".into(),
            send: vec!["mail".into()],
            receive: vec!["mail".into()],
            said: Some(Said {
                roster: vec![
                    RosterEntry {
                        name: "clerk".into(),
                        kind: "crew".into(),
                        role: "Keeps the books".into(),
                        mail: true,
                    },
                    RosterEntry {
                        name: "mckean".into(),
                        kind: "user".into(),
                        role: String::new(),
                        mail: true,
                    },
                ],
            }),
            ..Peer::default()
        }
    }

    fn letter(seq: u64, from: &str, to: &str, text: &str) -> Letter {
        Letter {
            seq,
            from: from.into(),
            to: to.into(),
            text: text.into(),
            ..Letter::default()
        }
    }

    /// The mail `seq` of `from` to `to` at `at` (RFC 3339, as the yard
    /// gives it).
    fn letter_at(seq: u64, from: &str, to: &str, at: &str, text: &str) -> Letter {
        Letter {
            seq,
            at: at.into(),
            from: from.into(),
            to: to.into(),
            text: text.into(),
            ..Letter::default()
        }
    }

    fn event(seq: u64, kind: &str, actor: &str, data: Value) -> Event {
        serde_json::from_value(json!({"seq": seq, "kind": kind, "actor": actor, "at": "2026-10-08T09:30:00Z", "data": data})).unwrap()
    }

    fn mail_to_jo(seq: u64, from: &str) -> Event {
        event(
            seq,
            "mail",
            from,
            json!({"to": "jo", "to_user": true, "text": format!("mail {seq}")}),
        )
    }

    fn bead_event(seq: u64, kind: &str, bead: &str, data: Value) -> Event {
        serde_json::from_value(json!({"seq": seq, "kind": kind, "actor": "yardmaster", "at": "2026-10-08T09:30:00Z", "bead": bead, "data": data})).unwrap()
    }

    /// A yard with the yardmaster and assembly, north linked, and jo with
    /// the unread mail given as (seq, from).
    fn yard(unread: &[(u64, &str)]) -> State {
        State {
            me: "jo".into(),
            head: 20,
            crew: vec![
                member("assembly", "running", "idle"),
                member("yardmaster", "running", "working"),
            ],
            peers: vec![north()],
            cursor: 5,
            unread: unread
                .iter()
                .map(|(seq, from)| letter(*seq, from, "jo", "unread"))
                .collect(),
            ..State::default()
        }
    }

    fn names(m: &Model) -> Vec<String> {
        m.rows().into_iter().map(|r| r.name).collect()
    }

    fn read(name: &str, after: u64) -> Effect {
        Effect::Read {
            name: name.into(),
            after,
        }
    }

    fn page(name: &str, before: u64) -> Effect {
        Effect::Page {
            name: name.into(),
            before,
        }
    }

    #[test]
    fn the_rows_are_the_crew_then_those_the_linked_yards_expose() {
        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            names(&m),
            [
                "assembly",
                "yardmaster",
                "clerk@north",
                "mckean@north",
                "jo"
            ]
        );
        let rows = m.rows();
        assert_eq!(rows[0].line, "idle");
        assert_eq!(rows[1].line, "working");
        assert_eq!(rows[2].line, "crew member: Keeps the books");
        assert_eq!(rows[3].line, "person");
    }

    /// The sidebar's groups are the rows split three ways: the crew with
    /// their dots, the linked yards' people, and this yard's people. Only
    /// the crew carries dots; the yard row carries none either.
    #[test]
    fn the_sidebar_s_groups_are_the_rows_split_three_ways() {
        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let crew = m.crew_rows();
        let dots: Vec<(&str, Option<Dot>)> =
            crew.iter().map(|r| (r.name.as_str(), r.dot)).collect();
        assert_eq!(
            dots,
            [
                ("assembly", Some(Dot::Idle)),
                ("yardmaster", Some(Dot::Working))
            ]
        );
        let linked = m.linked_rows();
        let names: Vec<&str> = linked.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["clerk@north", "mckean@north"]);
        assert!(linked.iter().all(|r| r.dot.is_none()));
        let people = m.people_rows();
        let names: Vec<&str> = people.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["jo"]);
        assert!(people.iter().all(|r| r.dot.is_none()));
        assert!(m.yard_row().dot.is_none());
    }

    /// Every crew state carries its dot: what runs, what rests, what
    /// waits, what stopped, what failed. A running session the daemon has
    /// not looked at yet still runs.
    #[test]
    fn every_crew_state_carries_its_dot() {
        let mut state = yard(&[]);
        state.crew = vec![
            member("working", "running", "working"),
            member("idle", "running", "idle"),
            member("blocked", "running", "blocked"),
            member("unknown", "running", "unknown"),
            member("stopped", "stopped", ""),
            member("failed", "failed", ""),
        ];
        let (m, _) = Model::new(state, Mode::Chat, None);
        let dots: Vec<Dot> = m.crew_rows().iter().map(|r| r.dot.unwrap()).collect();
        assert_eq!(
            dots,
            [
                Dot::Working,
                Dot::Idle,
                Dot::Blocked,
                Dot::Working,
                Dot::Stopped,
                Dot::Failed,
            ]
        );
    }

    /// A yard the window does not show has no open row: its mail counts,
    /// is news of a row that is not open, and is never seen or marked read
    /// until the window is back on it, whatever arrives or is read again.
    #[test]
    fn a_yard_held_aside_sees_nothing_until_the_window_is_back_on_its_row() {
        let (mut m, _) = Model::new(
            yard(&[]),
            Mode::Chat,
            Some(Selected::Conversation("assembly".into())),
        );
        m.conversation("assembly", Vec::new());
        m.leave();
        assert_eq!(m.selected(), None);
        assert_eq!(m.event(&mail_to_jo(21, "assembly")), []);
        assert_eq!(m.unread("assembly"), 1);
        let news = m.take_news();
        assert_eq!(news.len(), 1);
        assert!(!news[0].open);
        assert_eq!(news[0].target, Selected::Conversation("assembly".into()));

        // The stream ends and the yard is read again: still no row, and
        // what arrived meanwhile is news as it is of an open yard.
        let (mut m, effects) = m.resumed(yard(&[(21, "assembly"), (22, "yardmaster")]), Mode::Chat);
        assert_eq!(effects, []);
        assert_eq!(m.selected(), None);
        assert_eq!(m.take_news().len(), 1);
        // A yard read for the first time while another is shown is the same.
        let mut fresh = Model::new_aside(yard(&[(21, "assembly")]));
        assert_eq!(fresh.selected(), None);
        assert_eq!(fresh.unread("assembly"), 1);
        assert_eq!(fresh.event(&mail_to_jo(23, "assembly")), []);
        assert_eq!(fresh.unread("assembly"), 2);

        // Back on the row it was left on: its mail is read and marked so.
        assert_eq!(
            m.back_to(
                Mode::Chat,
                Some(Selected::Conversation("assembly".into())),
                false,
                false,
                false
            ),
            [page("assembly", 0)]
        );
        assert_eq!(
            m.selected(),
            Some(&Selected::Conversation("assembly".into()))
        );
        assert_eq!(
            m.page(
                "assembly",
                0,
                vec![letter(21, "assembly", "jo", "unread")],
                false
            ),
            [Effect::Ack { through: 21 }]
        );
        // With no row kept it opens as a yard opened for the first time.
        assert!(
            !fresh
                .back_to(Mode::Chat, None, false, false, false)
                .is_empty()
        );
        assert_eq!(
            fresh.selected(),
            Some(&Selected::Conversation("yardmaster".into()))
        );
    }

    #[test]
    fn a_missing_saved_decided_row_falls_back_as_a_single_yard_does() {
        let mut state = State::default();
        state.asks.decided.push(DecidedAsk {
            bead: "another".into(),
            ask: 2,
            ..DecidedAsk::default()
        });
        let (model, _) = Model::new_returning(
            state,
            Mode::Decisions,
            Some(Selected::Decided("gone".into(), 1)),
            true,
            false,
            false,
        );
        // No open decision to fall back to: the mode opens on nothing,
        // and not on a dealt with, as before that list was kept.
        assert_eq!(model.selected(), None);
        assert!(model.decided_expanded());
    }

    #[test]
    fn an_asker_is_written_to_in_their_conversation_or_not_at_all() {
        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(m.can_write("yardmaster"));
        assert!(m.can_write("assembly"));
        assert!(m.can_write("clerk@north"));
        assert!(!m.can_write("sage@east"));
        assert!(!m.can_write("sage"));
        assert_eq!(m.unwritable("clerk@north"), None);
        assert_eq!(
            m.unwritable("sage@east").as_deref(),
            Some("sage@east cannot be written to from here: the link does not expose them")
        );
        assert_eq!(
            m.unwritable("sage").as_deref(),
            Some(
                "sage cannot be written to from here: no member of this yard's crew and no user of it is called that"
            )
        );
    }

    #[test]
    fn a_foreign_bead_is_another_yard_s_question() {
        assert!(!foreign_bead("yardr-n8ta"));
        assert!(foreign_bead("yardr-ndv0@mac"));
    }

    #[test]
    fn a_peer_that_has_not_said_exposes_nobody() {
        let mut state = yard(&[]);
        state.peers.push(Peer {
            name: "south".into(),
            ..Peer::default()
        });
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(names(&m).len(), 5);
    }

    #[test]
    fn a_crew_member_s_state_in_words() {
        assert_eq!(state_of(&member("a", "stopped", "")), "not running");
        assert_eq!(state_of(&member("a", "failed", "")), "failed to start");
        assert_eq!(state_of(&member("a", "running", "")), "running");
        assert_eq!(state_of(&member("a", "running", "unknown")), "running");
        assert_eq!(state_of(&member("a", "running", "blocked")), "blocked");
        // A status left from a session that ended says nothing.
        assert_eq!(state_of(&member("a", "stopped", "working")), "not running");
    }

    /// A session at its harness's login screen reads no mail, whatever its
    /// runtime says it does, and often it says idle: the state is the
    /// yard's word, and the dot waits for a person. A login left on a
    /// session that ended says nothing.
    #[test]
    fn a_session_at_its_harness_s_login_screen_is_not_logged_in_whatever_its_status() {
        for status in ["idle", "working", "blocked", "unknown", ""] {
            let at = at_login(member("a", "running", status), false);
            assert_eq!(state_of(&at), "not logged in", "{status:?}");
            assert_eq!(dot_of(&at), Dot::Blocked, "{status:?}");
        }
        let stopped = at_login(member("a", "stopped", ""), true);
        assert_eq!(state_of(&stopped), "not running");
        assert_eq!(dot_of(&stopped), Dot::Stopped);
        let failed = at_login(member("a", "failed", ""), true);
        assert_eq!(state_of(&failed), "failed to start");
        assert_eq!(dot_of(&failed), Dot::Failed);

        // The row and the header say it, and the dot is not the idle one.
        let mut state = yard(&[]);
        state.crew = vec![
            at_login(member("yardmaster", "running", "idle"), true),
            member("assembly", "running", "idle"),
        ];
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.crew_state("yardmaster"), Some("not logged in"));
        assert_eq!(m.crew_state("assembly"), Some("idle"));
        let rows = m.crew_rows();
        assert_eq!(rows[0].line, "not logged in");
        assert_eq!(rows[0].dot, Some(Dot::Blocked));
        assert_eq!(rows[1].dot, Some(Dot::Idle));
    }

    /// Under the user's mail to it, the yard's words: where the yard leads
    /// the login the person answers it at the asks, and where it does not
    /// the mail still cannot be read. Mail it read stays read.
    #[test]
    fn mail_to_a_session_at_its_login_screen_says_it_cannot_be_read() {
        let mut state = yard(&[]);
        state.crew = vec![
            at_login(member("yardmaster", "running", "idle"), true),
            at_login(member("assembly", "running", "idle"), false),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![
                Letter {
                    read: true,
                    ..letter(12, "jo", "yardmaster", "one")
                },
                letter(13, "jo", "yardmaster", "two"),
            ],
        );
        let standing: Vec<Option<String>> = m.entries().into_iter().map(|e| e.standing).collect();
        assert_eq!(
            standing,
            [
                Some("read".to_string()),
                Some("yardmaster is not logged in and cannot read mail: the yard leads its login, and waits for the operator at its asks (yardr asks)".to_string())
            ]
        );

        m.select(Selected::Conversation("assembly".into()));
        m.conversation("assembly", vec![letter(14, "jo", "assembly", "hi")]);
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("assembly is not logged in and cannot read mail")
        );
    }

    /// A session's login changed: `crew_login` carries less than what
    /// changed, as any other `crew_` event, so the crew is read again. An
    /// event of another kind says nothing of it.
    #[test]
    fn the_crew_is_read_again_on_a_crew_login_event() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let login = event(
            21,
            "crew_login",
            "yard",
            json!({"crew": "yardmaster", "login_screen": true, "login_led": false}),
        );
        assert!(m.event(&login).contains(&Effect::Crew));
        let held = event(
            22,
            "nudge_held",
            "yard",
            json!({"crew": "yardmaster", "reason": "not_logged_in"}),
        );
        assert!(!m.event(&held).contains(&Effect::Crew));
    }

    #[test]
    fn it_opens_on_the_yardmaster_and_reads_that_conversation() {
        let (m, effects) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
        assert_eq!(effects, [page("yardmaster", 0)]);
        assert!(m.rows()[1].selected);
        assert!(!m.rows()[0].selected);
    }

    #[test]
    fn without_a_yardmaster_it_opens_on_the_first_row_and_on_the_own_in_an_empty_yard() {
        let mut state = yard(&[]);
        state.crew.remove(1);
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.selected_conversation(), Some("assembly"));

        let (mut m, effects) = Model::new(
            State {
                me: "jo".into(),
                ..State::default()
            },
            Mode::Chat,
            None,
        );
        // The user's own row and the yard row stand whatever the yard has.
        assert_eq!(m.selected_conversation(), Some("jo"));
        assert_eq!(effects, [page("jo", 0)]);
        assert_eq!(m.target(), Some(Target::Crew("jo".into())));
        // The yard row reads the mail the yard itself sent.
        assert_eq!(m.step_in(Mode::Chat, 1), [read("yard", 0)]);
        assert_eq!(m.selected(), Some(&Selected::Yard));
        assert_eq!(m.target(), None);
        m.conversation("yard", vec![]);
        assert_eq!(m.select(Selected::Yard), []);
        assert_eq!(m.entries(), []);
    }

    #[test]
    fn the_conversation_that_was_open_stays_open_when_the_yard_is_back() {
        let (m, _) = Model::new(
            yard(&[]),
            Mode::Chat,
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert_eq!(m.selected_conversation(), Some("clerk@north"));
        // One that is gone meanwhile does not.
        let (m, _) = Model::new(
            yard(&[]),
            Mode::Chat,
            Some(Selected::Conversation("gone".into())),
        );
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
    }

    #[test]
    fn selecting_opens_a_conversation_and_reads_it_once() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [page("assembly", 0)]
        );
        assert_eq!(m.selected_conversation(), Some("assembly"));
        assert_eq!(
            m.conversation("assembly", vec![letter(3, "assembly", "jo", "hello")]),
            []
        );
        m.select(Selected::Conversation("yardmaster".into()));
        // It was read: nothing is asked again.
        assert_eq!(m.select(Selected::Conversation("assembly".into())), []);
        // A name that is no row changes nothing.
        assert_eq!(m.select(Selected::Conversation("nobody".into())), []);
        assert_eq!(m.selected_conversation(), Some("assembly"));
    }

    #[test]
    fn reading_a_bead_reads_it_whole_once_and_selects_nothing() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.read_bead("yd-zulu"),
            [Effect::Bead {
                bead: "yd-zulu".into()
            }]
        );
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
        m.bead_loaded(Bead {
            id: "yd-zulu".into(),
            ..Bead::default()
        });
        // It was read: nothing is asked again.
        assert_eq!(m.read_bead("yd-zulu"), []);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
    }

    #[test]
    fn step_goes_to_the_next_row_of_the_mode_and_around_the_ends() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.step_in(Mode::Chat, 1);
        assert_eq!(m.selected_conversation(), Some("clerk@north"));
        m.step_in(Mode::Chat, 1);
        assert_eq!(m.selected_conversation(), Some("mckean@north"));
        m.step_in(Mode::Chat, 1);
        assert_eq!(m.selected_conversation(), Some("jo"));
        m.step_in(Mode::Chat, 1);
        assert_eq!(m.selected(), Some(&Selected::Yard));
        m.step_in(Mode::Chat, 1);
        assert_eq!(m.selected_conversation(), Some("assembly"));
        m.step_in(Mode::Chat, -1);
        assert_eq!(m.selected(), Some(&Selected::Yard));
        // The timetable's one row steps to itself.
        m.select(Selected::Arrivals);
        m.step_in(Mode::Timetable, 1);
        assert_eq!(m.selected(), Some(&Selected::Arrivals));
    }

    #[test]
    fn stepping_from_an_arrival_bead_uses_the_arrivals_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.select(Selected::Bead("yd-zulu".into()));
        m.step_in(Mode::Timetable, -1);
        assert_eq!(m.selected(), Some(&Selected::Arrivals));

        m.select(Selected::Bead("yd-zulu".into()));
        m.step_in(Mode::Timetable, 1);
        let next_from_bead = m.selected().cloned();
        m.select(Selected::Arrivals);
        m.step_in(Mode::Timetable, 1);
        assert_eq!(m.selected().cloned(), next_from_bead);
    }

    #[test]
    fn unread_is_counted_by_sender() {
        let (m, _) = Model::new(
            yard(&[(6, "assembly"), (7, "assembly"), (8, "clerk@north")]),
            Mode::Chat,
            None,
        );
        assert_eq!(m.unread("assembly"), 2);
        assert_eq!(m.unread("clerk@north"), 1);
        assert_eq!(m.unread("yardmaster"), 0);
        let rows = m.rows();
        assert_eq!((rows[0].unread, rows[1].unread, rows[2].unread), (2, 0, 1));
    }

    #[test]
    fn mail_the_inbox_lists_as_read_or_behind_the_cursor_is_not_unread() {
        let mut state = yard(&[(4, "assembly"), (6, "assembly")]);
        state.unread.push(Letter {
            read: true,
            ..letter(9, "assembly", "jo", "read")
        });
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.unread("assembly"), 1);
    }

    #[test]
    fn opening_a_conversation_acknowledges_the_mail_it_shows() {
        let (mut m, effects) = Model::new(
            yard(&[(6, "yardmaster"), (7, "yardmaster")]),
            Mode::Chat,
            None,
        );
        // The yardmaster's is open from the start.
        assert_eq!(effects, [page("yardmaster", 0)]);
        // Nothing of it is shown before the yard answers the conversation.
        assert_eq!(m.unread("yardmaster"), 2);
        assert_eq!(
            m.page("yardmaster", 0, vec![], false),
            [Effect::Ack { through: 7 }]
        );
        // It was shown: it counts no longer, whenever the yard answers.
        assert_eq!(m.unread("yardmaster"), 0);
        m.acked(7);
        assert_eq!(m.unread("yardmaster"), 0);
    }

    #[test]
    fn mail_from_the_yard_between_leaves_no_badge_on_a_conversation_that_was_shown() {
        // footplate-knh4: two refusals came back as mail from the yard
        // between the mail of one of a linked yard. They had no row, so
        // they were never seen and held the acknowledgement back, and the
        // row counted its mail though all of it was shown.
        let (mut m, _) = Model::new(
            yard(&[
                (6, "clerk@north"),
                (7, "yard"),
                (8, "yard"),
                (9, "clerk@north"),
            ]),
            Mode::Chat,
            None,
        );
        assert_eq!(m.unread("clerk@north"), 2);
        assert_eq!(m.yard_row().unread, 2);

        // Opening the conversation shows 6 and 9; 7 holds back 9.
        assert_eq!(
            m.select(Selected::Conversation("clerk@north".into())),
            [page("clerk@north", 0)]
        );
        assert_eq!(
            m.conversation("clerk@north", vec![]),
            [Effect::Ack { through: 6 }]
        );
        assert_eq!(m.unread("clerk@north"), 0);
        m.acked(6);
        assert_eq!(m.unread("clerk@north"), 0);
        // More of it arrives while it is open: shown, and no badge.
        assert_eq!(m.event(&mail_to_jo(21, "clerk@north")), []);
        assert_eq!(m.unread("clerk@north"), 0);
        assert_eq!(m.yard_row().unread, 2);

        // The yard row shows the yard's mail: it is read there, and what
        // it held back is acknowledged with it.
        assert_eq!(m.select(Selected::Yard), [read("yard", 0)]);
        assert_eq!(
            m.conversation(
                "yard",
                vec![
                    letter(7, "yard", "jo", "not delivered to mac: rate_limited"),
                    letter(8, "yard", "jo", "not delivered to mac: rate_limited"),
                ],
            ),
            [Effect::Ack { through: 21 }]
        );
        assert_eq!(m.yard_row().unread, 0);
        let mail = m.yard_mail();
        assert_eq!(mail.len(), 2);
        assert_eq!(mail[0].text, "not delivered to mac: rate_limited");
        assert_eq!(mail[0].from, "yard");
        m.acked(21);

        // With the yard's mail read, mail of the conversation that is not
        // open counts, and opening it clears the count again.
        m.event(&mail_to_jo(22, "clerk@north"));
        assert_eq!(m.unread("clerk@north"), 1);
        // Mail from the yard arriving on its open row is shown and read.
        assert_eq!(m.event(&mail_to_jo(23, "yard")), []);
        assert_eq!(m.yard_row().unread, 0);
        assert_eq!(m.yard_mail().len(), 3);
        assert_eq!(
            m.select(Selected::Conversation("clerk@north".into())),
            [Effect::Ack { through: 23 }]
        );
        assert_eq!(m.unread("clerk@north"), 0);
        // The yard row was read: opening it again asks nothing.
        m.acked(23);
        assert_eq!(m.select(Selected::Yard), []);
    }

    #[test]
    fn the_yard_row_counts_notices_and_the_yards_mail() {
        let mut state = yard(&[(6, "yard")]);
        state.notifications = vec![notification(10, "yd-shed", "yd-shed created", false)];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.yard_unread(), 2);
        // Opening reads the mail; the notice waits for its key.
        m.select(Selected::Yard);
        m.conversation("yard", vec![letter(6, "yard", "jo", "unread")]);
        assert_eq!(m.yard_unread(), 1);
        assert_eq!(m.dismiss(), [Effect::AckNotifications { through: 10 }]);
    }

    #[test]
    fn a_crew_member_named_yard_keeps_its_mail_on_its_own_row() {
        let mut state = yard(&[(6, "yard")]);
        state.crew.push(member("yard", "running", "idle"));
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!((m.unread("yard"), m.yard_unread()), (1, 0));
        assert_eq!(m.select(Selected::Yard), []);
        assert_eq!(m.yard_mail(), []);
        assert_eq!(m.unread("yard"), 1);
    }

    #[test]
    fn what_was_shown_is_not_unseen_again_when_the_yard_is_back() {
        // ada's mail is on the yard row, which was not opened: it holds
        // back the yardmaster's 8.
        let unread = [(6, "yardmaster"), (7, "ada"), (8, "yardmaster")];
        let (mut m, _) = Model::new(yard(&unread), Mode::Chat, None);
        m.conversation("yardmaster", vec![]);
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(m.unread("yardmaster"), 0);
        // The yard was lost before it took the acknowledgement of 6.
        let (m, effects) = m.resumed(yard(&unread), Mode::Chat);
        assert_eq!(m.selected_conversation(), Some("assembly"));
        assert_eq!(m.unread("yardmaster"), 0);
        assert_eq!(effects, [page("assembly", 0), Effect::Ack { through: 6 }]);
        assert_eq!(m.yard_unread(), 1);
    }

    #[test]
    fn mail_of_a_conversation_not_opened_holds_back_what_came_after_it() {
        let (mut m, effects) = Model::new(
            yard(&[(6, "yardmaster"), (7, "assembly"), (8, "yardmaster")]),
            Mode::Chat,
            None,
        );
        assert_eq!(effects, [page("yardmaster", 0)]);
        // 8 was shown, but 7 was not: only 6 is acknowledged.
        assert_eq!(
            m.conversation("yardmaster", vec![]),
            [Effect::Ack { through: 6 }]
        );
        m.acked(6);
        // 8 was shown and does not count, though it is not acknowledged.
        assert_eq!((m.unread("yardmaster"), m.unread("assembly")), (0, 1));
        // Opening assembly's shows 7, and 8 was seen before.
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [page("assembly", 0)]
        );
        assert_eq!(
            m.conversation("assembly", vec![]),
            [Effect::Ack { through: 8 }]
        );
        m.acked(8);
        assert_eq!((m.unread("yardmaster"), m.unread("assembly")), (0, 0));
    }

    #[test]
    fn nothing_is_acknowledged_twice() {
        let (mut m, _) = Model::new(yard(&[(6, "yardmaster")]), Mode::Chat, None);
        assert_eq!(
            m.conversation("yardmaster", vec![letter(6, "yardmaster", "jo", "x")]),
            [Effect::Ack { through: 6 }]
        );
        // Asked through 6 already, and not answered yet.
        assert_eq!(
            m.conversation("yardmaster", vec![letter(6, "yardmaster", "jo", "x")]),
            []
        );
        m.acked(6);
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [page("assembly", 0)]
        );
    }

    #[test]
    fn an_acknowledgement_the_yard_did_not_take_is_asked_again() {
        let (mut m, _) = Model::new(yard(&[(6, "yardmaster")]), Mode::Chat, None);
        m.conversation("yardmaster", vec![]);
        m.ack_failed("the yard does not answer");
        assert!(
            m.notice.contains("could not be marked read"),
            "{}",
            m.notice
        );
        // It was shown, so it does not count; the yard has it unread still.
        assert_eq!(m.unread("yardmaster"), 0);
        assert_eq!(
            m.event(&mail_to_jo(21, "yardmaster")),
            [Effect::Ack { through: 21 }]
        );
    }

    #[test]
    fn mail_that_arrives_in_the_open_conversation_is_shown_and_acknowledged() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation("yardmaster", vec![]);
        assert_eq!(
            m.event(&mail_to_jo(21, "yardmaster")),
            [Effect::Ack { through: 21 }]
        );
        let entries = m.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (
                entries[0].from.as_str(),
                entries[0].text.as_str(),
                entries[0].mine
            ),
            ("yardmaster", "mail 21", false)
        );
        assert_eq!(entries[0].at, "2026-10-08T09:30:00Z");
    }

    #[test]
    fn mail_that_arrives_in_another_conversation_counts_there_and_is_not_acknowledged() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.event(&mail_to_jo(21, "assembly")), []);
        assert_eq!(m.event(&mail_to_jo(22, "clerk@north")), []);
        assert_eq!(
            (
                m.unread("assembly"),
                m.unread("clerk@north"),
                m.unread("yardmaster")
            ),
            (1, 1, 0)
        );
        // And holds back the open one's, which is seen.
        assert_eq!(m.event(&mail_to_jo(23, "yardmaster")), []);
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [page("assembly", 0)]
        );
        assert_eq!(
            m.conversation("assembly", vec![]),
            [Effect::Ack { through: 21 }]
        );
    }

    #[test]
    fn mail_to_someone_else_or_to_a_crew_member_of_the_user_s_name_is_not_the_user_s() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.event(&event(
            21,
            "mail",
            "yardmaster",
            json!({"to": "assembly", "text": "x"}),
        ));
        // to_user is absent: the receiver is a crew member.
        m.event(&event(
            22,
            "mail",
            "yardmaster",
            json!({"to": "jo", "text": "x"}),
        ));
        assert_eq!(m.unread("yardmaster"), 0);
        assert_eq!(m.entries(), []);
    }

    #[test]
    fn the_user_s_own_mail_is_in_the_conversation_whichever_client_sent_it() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation("yardmaster", vec![]);
        assert_eq!(
            m.event(&event(
                21,
                "mail",
                "jo",
                json!({"to": "yardmaster", "text": "Land it."})
            )),
            []
        );
        let entries = m.entries();
        assert_eq!(
            (entries[0].text.as_str(), entries[0].mine),
            ("Land it.", true)
        );
        assert_eq!(m.unread("yardmaster"), 0);
    }

    /// Mail the user wrote from a linked yard under the name trusted as
    /// them stands in the open row as their own, with its standing, and
    /// being theirs it is not counted, acknowledged or notified.
    /// (footplate-mmp6: with the mayor's row open, mckean's word from the
    /// other yard did not appear as it was sent.)
    #[test]
    fn the_user_s_word_from_a_linked_yard_arrives_in_the_open_row_as_theirs() {
        let (mut m, _) = Model::new(
            plantime(),
            Mode::Chat,
            Some(Selected::Conversation("mayor".into())),
        );
        m.conversation("mayor", vec![]);
        assert_eq!(
            m.event(&event(
                22,
                "mail",
                "mckean@yardr",
                json!({"to": "mayor", "as": "mckean", "text": "hi"}),
            )),
            []
        );
        let entries = m.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from.as_str(), "mckean@yardr");
        assert!(entries[0].mine);
        assert_eq!(entries[0].standing.as_deref(), Some("sent; mayor is idle"));
        assert_eq!(m.unread("mayor"), 0);
        // A mail from a name trusted as someone else is dropped, as
        // before.
        let mut trusted = plantime();
        trusted.peers[0]
            .trust
            .insert("ops".to_string(), "yardmaster".to_string());
        let (mut m, _) = Model::new(
            trusted,
            Mode::Chat,
            Some(Selected::Conversation("mayor".into())),
        );
        m.conversation("mayor", vec![]);
        m.event(&event(
            22,
            "mail",
            "ops@yardr",
            json!({"to": "mayor", "as": "ops", "text": "hi"}),
        ));
        assert_eq!(m.entries(), []);
        // And one from the same name to the user's own row stays the
        // other end of the conversation there.
        let (mut m, _) = Model::new(
            plantime(),
            Mode::Chat,
            Some(Selected::Conversation("mckean".into())),
        );
        m.conversation("mckean", vec![]);
        m.event(&event(
            22,
            "mail",
            "mckean@yardr",
            json!({"to": "mckean", "as": "mckean", "text": "hi"}),
        ));
        assert_eq!(m.entries(), []);
    }

    #[test]
    fn a_mail_the_stream_and_the_yard_both_tell_of_is_there_once_and_in_order() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.event(&mail_to_jo(22, "yardmaster"));
        m.conversation(
            "yardmaster",
            vec![
                letter(12, "yardmaster", "jo", "first"),
                letter(22, "yardmaster", "jo", "mail 22"),
            ],
        );
        m.event(&mail_to_jo(22, "yardmaster"));
        m.event(&event(
            21,
            "mail",
            "jo",
            json!({"to": "yardmaster", "text": "between"}),
        ));
        let seqs: Vec<u64> = m.entries().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, [12, 21, 22]);
    }

    /// The mails `seqs` of `from` to jo, oldest first.
    fn letters(seqs: std::ops::RangeInclusive<u64>, from: &str) -> Vec<Letter> {
        seqs.map(|seq| letter(seq, from, "jo", "x")).collect()
    }

    fn seqs(m: &Model) -> Vec<u64> {
        m.entries().iter().map(|e| e.seq).collect()
    }

    #[test]
    fn a_conversation_opens_on_its_newest_page_and_its_top_asks_for_the_page_before() {
        let (mut m, effects) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(effects, [page("yardmaster", 0)]);
        // The yard has not said yet whether there is older mail.
        assert_eq!(m.top(), Top::Unread);
        assert_eq!(m.older(), []);
        m.page("yardmaster", 0, letters(451..=500, "yardmaster"), true);
        assert_eq!(m.span(), Some((451, 500)));
        assert_eq!(m.top(), Top::Older);

        // The top asks for the page before the oldest shown, once.
        assert_eq!(m.older(), [page("yardmaster", 451)]);
        assert_eq!(m.older(), []);
        m.page("yardmaster", 451, letters(401..=450, "yardmaster"), true);
        assert_eq!(m.span(), Some((401, 500)));
        assert_eq!(m.entries().len(), 100);
        assert_eq!(m.older(), [page("yardmaster", 401)]);

        // New mail still arrives below, and what is read after a send is
        // what is newer than it.
        m.event(&mail_to_jo(501, "yardmaster"));
        assert_eq!(m.span(), Some((401, 501)));
        assert_eq!(m.sent("yardmaster"), [read("yardmaster", 501)]);

        // The last page says there is no more: the top says so, and
        // nothing is asked for again.
        m.page("yardmaster", 401, letters(1..=400, "yardmaster"), false);
        assert_eq!(m.top(), Top::Start);
        assert_eq!(m.older(), []);
        assert_eq!(m.entries().len(), 501);
        // Opened again it was read: nothing is asked.
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(m.select(Selected::Conversation("yardmaster".into())), []);
    }

    #[test]
    fn a_page_the_yard_did_not_answer_is_asked_for_again() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.page("yardmaster", 0, letters(51..=100, "yardmaster"), true);
        assert_eq!(m.older(), [page("yardmaster", 51)]);
        m.page_failed("yardmaster", "the yard does not answer");
        assert!(m.notice.contains("could not be read"), "{}", m.notice);
        assert_eq!(m.top(), Top::Older);
        assert_eq!(m.older(), [page("yardmaster", 51)]);
    }

    #[test]
    fn a_yard_that_knows_no_pages_answers_all_and_is_asked_no_more() {
        let (mut m, effects) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(effects, [page("yardmaster", 0)]);
        // All five hundred to a call for fifty, and no word of more.
        m.page("yardmaster", 0, letters(1..=500, "yardmaster"), false);
        assert_eq!(m.entries().len(), 500);
        assert_eq!(m.top(), Top::Start);
        assert_eq!(m.older(), []);
    }

    #[test]
    fn unread_mail_older_than_the_pages_read_is_not_acknowledged_until_it_is_shown() {
        let (mut m, _) = Model::new(
            yard(&[(8, "yardmaster"), (60, "yardmaster")]),
            Mode::Chat,
            None,
        );
        // The newest page shows 60 and not 8, which holds 60 back.
        assert_eq!(
            m.page("yardmaster", 0, letters(51..=100, "yardmaster"), true),
            []
        );
        assert_eq!(m.unread("yardmaster"), 1);
        // New mail on the open row is seen, and held back as well.
        assert_eq!(m.event(&mail_to_jo(101, "yardmaster")), []);
        assert_eq!(m.unread("yardmaster"), 1);
        // The page with 8 shows it: all of it is acknowledged.
        assert_eq!(m.older(), [page("yardmaster", 51)]);
        assert_eq!(
            m.page("yardmaster", 51, letters(1..=50, "yardmaster"), false),
            [Effect::Ack { through: 101 }]
        );
        assert_eq!(m.unread("yardmaster"), 0);
    }

    #[test]
    fn a_row_of_two_conversations_shows_no_mail_older_than_where_one_of_them_stops() {
        let mut state = yard(&[(10, "jo@airy"), (460, "jo")]);
        state.peers.push(airy());
        let (mut m, _) = Model::new(state, Mode::Chat, Some(Selected::Conversation("jo".into())));
        // The user's own notes go further back than a page; the trusted
        // name wrote twice, long ago and lately.
        m.page("jo", 0, letters(451..=500, "jo"), true);
        let airy = vec![
            letter(10, "jo@airy", "jo", "long ago"),
            letter(470, "jo@airy", "jo", "lately"),
        ];
        assert_eq!(m.page("jo@airy", 0, airy, false), []);
        // 10 stands before notes that were not read: it is not shown, nor
        // seen, and the images of such mail are not asked for.
        assert_eq!(m.span(), Some((451, 500)));
        assert_eq!(m.entries().len(), 51);
        assert_eq!(m.unread("jo"), 1);
        assert_eq!(m.top(), Top::Older);
        // The top asks the one that stops there, and no other.
        assert_eq!(m.older(), [page("jo", 451)]);
        assert_eq!(
            m.page("jo", 451, letters(1..=450, "jo"), false),
            [Effect::Ack { through: 460 }]
        );
        assert_eq!(seqs(&m)[..11], [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 10]);
        assert_eq!(m.top(), Top::Start);
        assert_eq!(m.unread("jo"), 0);
    }

    #[test]
    fn the_images_of_mail_that_was_not_read_are_not_asked_for() {
        let mut state = yard(&[]);
        state.peers.push(airy());
        let (mut m, _) = Model::new(state, Mode::Chat, Some(Selected::Conversation("jo".into())));
        let picture = |seq: u64, from: &str, hash: &str| Letter {
            files: vec![BlobRef {
                hash: hash.into(),
                kind: "image/png".into(),
                ..BlobRef::default()
            }],
            ..letter(seq, from, "jo", "see")
        };
        m.page("jo", 0, vec![picture(450, "jo", "aa")], true);
        m.page(
            "jo@airy",
            0,
            vec![picture(10, "jo@airy", "bb"), picture(470, "jo@airy", "cc")],
            false,
        );
        assert_eq!(m.images(), ["aa", "cc"]);
        m.page("jo", 450, vec![picture(5, "jo", "dd")], false);
        assert_eq!(m.images(), ["dd", "bb", "aa", "cc"]);
    }

    #[test]
    fn an_older_page_of_a_conversation_not_read_since_the_yard_was_back_is_left() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.page("yardmaster", 0, letters(51..=100, "yardmaster"), true);
        assert_eq!(m.older(), [page("yardmaster", 51)]);
        // The yard was lost and is back: the page asked for before is
        // answered to a model that has not read the conversation's newest.
        let (mut m, effects) = m.resumed(yard(&[]), Mode::Chat);
        assert_eq!(effects, [page("yardmaster", 0)]);
        assert_eq!(
            m.page("yardmaster", 51, letters(1..=50, "yardmaster"), false),
            []
        );
        assert_eq!(m.entries(), []);
        assert_eq!(m.top(), Top::Unread);
        // Its newest page is: the conversation stands on that.
        m.page("yardmaster", 0, letters(51..=100, "yardmaster"), true);
        assert_eq!(m.span(), Some((51, 100)));
    }

    #[test]
    fn a_conversation_with_a_linked_yard_is_read_again_from_the_oldest_mail_it_has() {
        let (mut m, _) = Model::new(
            yard(&[]),
            Mode::Chat,
            Some(Selected::Conversation("clerk@north".into())),
        );
        m.page("clerk@north", 0, letters(51..=100, "clerk@north"), true);
        let effects = m.event(&event(
            101,
            "peer_message_sent",
            "yard",
            json!({"peer": "north"}),
        ));
        assert_eq!(effects, [Effect::Peers, read("clerk@north", 50)]);
    }

    #[test]
    fn after_a_send_only_what_is_new_is_read() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation("yardmaster", vec![letter(12, "yardmaster", "jo", "first")]);
        assert_eq!(m.sent("yardmaster"), [read("yardmaster", 12)]);
        // A conversation the yard was never asked for is read from its
        // newest page, whatever the stream put into it.
        m.event(&mail_to_jo(30, "assembly"));
        assert_eq!(m.sent("assembly"), [page("assembly", 0)]);
    }

    fn reacted_by(from: &str, emoji: &str) -> Reaction {
        Reaction {
            from: from.into(),
            emoji: emoji.into(),
        }
    }

    #[test]
    fn reactions_show_under_the_letter_and_move_with_the_stream() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.select(Selected::Conversation("yardmaster".into()));
        m.conversation(
            "yardmaster",
            vec![
                Letter {
                    reactions: vec![reacted_by("jo", "👍")],
                    ..letter(12, "yardmaster", "jo", "Shall I land it?")
                },
                letter(13, "jo", "yardmaster", "Land it."),
            ],
        );
        let entries = m.entries();
        assert_eq!(entries[0].reactions, [reacted_by("jo", "👍")]);
        // A yard from before reactions sends no such field: the mail
        // reads as it always has.
        assert_eq!(entries[1].reactions, []);

        // The other party's lands from the stream, by who put it.
        assert_eq!(
            m.event(&event(
                21,
                "reaction",
                "yardmaster",
                json!({"seq": 12, "emoji": "🎉"}),
            )),
            []
        );
        assert_eq!(
            m.entries()[0].reactions,
            [reacted_by("jo", "👍"), reacted_by("yardmaster", "🎉")]
        );
        // A new one replaces the old one.
        m.event(&event(
            22,
            "reaction",
            "jo",
            json!({"seq": 12, "emoji": "❤️"}),
        ));
        assert_eq!(
            m.entries()[0].reactions,
            [reacted_by("jo", "❤️"), reacted_by("yardmaster", "🎉")]
        );
        // A clear takes it away.
        m.event(&event(
            23,
            "reaction",
            "yardmaster",
            json!({"seq": 12, "emoji": ""}),
        ));
        assert_eq!(m.entries()[0].reactions, [reacted_by("jo", "❤️")]);
        // One on a letter the window does not hold changes nothing: its
        // read carries them.
        m.event(&event(
            24,
            "reaction",
            "jo",
            json!({"seq": 99, "emoji": "👍"}),
        ));
        assert_eq!(m.entries()[0].reactions, [reacted_by("jo", "❤️")]);

        // A re-read carries what the letter holds now.
        m.conversation(
            "yardmaster",
            vec![Letter {
                reactions: vec![reacted_by("yardmaster", "👀")],
                ..letter(12, "yardmaster", "jo", "Shall I land it?")
            }],
        );
        assert_eq!(m.entries()[0].reactions, [reacted_by("yardmaster", "👀")]);
        m.conversation(
            "yardmaster",
            vec![letter(12, "yardmaster", "jo", "Shall I land it?")],
        );
        assert_eq!(m.entries()[0].reactions, []);
    }

    #[test]
    fn a_mail_stream_event_without_reactions_keeps_the_current_reactions() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![Letter {
                reactions: vec![reacted_by("jo", "👍")],
                ..letter(12, "yardmaster", "jo", "Shall I land it?")
            }],
        );

        // Mail stream events do not carry reactions and must not undo the
        // conversation's current state.
        m.event(&mail_to_jo(12, "yardmaster"));
        assert_eq!(m.entries()[0].reactions, [reacted_by("jo", "👍")]);
    }

    #[test]
    fn after_a_react_the_conversation_that_holds_the_mail_is_read_again() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation("yardmaster", vec![letter(12, "yardmaster", "jo", "first")]);
        assert_eq!(m.reacted(12), [read("yardmaster", 11)]);
        // A mail the window does not hold reads nothing.
        assert_eq!(m.reacted(99), []);
    }

    #[test]
    fn a_yard_without_react_hides_the_row_and_says_nothing() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(!m.react_too_old());
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        m.react_failed(&refused("no such method React"));
        assert!(m.react_too_old());
        assert!(m.notice.is_empty(), "{}", m.notice);
        // A reaction answered clears it.
        m.conversation("yardmaster", vec![letter(12, "yardmaster", "jo", "x")]);
        assert_eq!(m.reacted(12), [read("yardmaster", 11)]);
        assert!(!m.react_too_old());
        // Anything else is a notice, as ever.
        m.react_failed(&refused("only the letter's sender or receiver may react"));
        assert!(!m.react_too_old());
        assert_eq!(
            m.notice,
            "not reacted: only the letter's sender or receiver may react"
        );
    }

    #[test]
    fn mail_read_by_the_user_elsewhere_clears_the_count() {
        let (mut m, _) = Model::new(yard(&[(6, "assembly"), (9, "assembly")]), Mode::Chat, None);
        assert_eq!(
            m.event(&event(
                21,
                "mail_read",
                "",
                json!({"reader": "jo", "through": 6})
            )),
            []
        );
        assert_eq!(m.unread("assembly"), 1);
        m.event(&event(
            22,
            "mail_read",
            "",
            json!({"reader": "jo", "through": 9}),
        ));
        assert_eq!(m.unread("assembly"), 0);
    }

    #[test]
    fn mail_read_by_a_crew_member_marks_what_the_user_sent_it() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![
                letter(12, "jo", "yardmaster", "one"),
                letter(13, "yardmaster", "jo", "re"),
                letter(14, "jo", "yardmaster", "two"),
            ],
        );
        m.event(&event(
            21,
            "mail_read",
            "",
            json!({"reader": "yardmaster", "through": 12}),
        ));
        let entries = m.entries();
        assert_eq!(entries[0].standing.as_deref(), Some("read"));
        assert_eq!(entries[1].standing, None);
        assert_eq!(
            entries[2].standing.as_deref(),
            Some("sent; yardmaster is working and reads mail when idle")
        );
    }

    #[test]
    fn mails_that_stand_alike_share_one_line_under_the_last() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.select(Selected::Conversation("assembly".into()));
        m.conversation(
            "assembly",
            vec![
                letter(12, "jo", "assembly", "one"),
                letter(13, "jo", "assembly", "two"),
                letter(14, "jo", "assembly", "three"),
            ],
        );
        let standing: Vec<Option<String>> = m.entries().into_iter().map(|e| e.standing).collect();
        assert_eq!(
            standing,
            [None, None, Some("sent; assembly is idle".to_string())]
        );
    }

    #[test]
    fn mails_of_one_sender_within_five_minutes_share_one_name_and_time() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![
                letter_at(12, "yardmaster", "jo", "2026-10-08T12:00:00Z", "one"),
                letter_at(13, "yardmaster", "jo", "2026-10-08T12:03:00Z", "two"),
                letter_at(14, "yardmaster", "jo", "2026-10-08T12:09:00Z", "three"),
                letter_at(15, "jo", "yardmaster", "2026-10-08T12:10:00Z", "four"),
                letter_at(16, "yardmaster", "jo", "2026-10-10T12:00:00Z", "five"),
                letter(17, "yardmaster", "jo", "six"),
            ],
        );
        let entries = m.entries();
        let heads: Vec<bool> = entries.iter().map(|e| e.head).collect();
        // The second mail joins the first's group where the two fall on
        // the same day here; past midnight between them it heads its own.
        let zone = chrono::Local;
        let same = day_of("2026-10-08T12:00:00Z", &zone) == day_of("2026-10-08T12:03:00Z", &zone);
        assert_eq!(heads, [true, !same, true, true, true, true]);
        // A new day breaks above the mail, and the first mail breaks too;
        // a mail with no time stands under nothing.
        let today = chrono::Local::now().date_naive();
        let words = |at: &str| day_words(day_of(at, &zone).unwrap(), today);
        assert_eq!(entries[0].day, Some(words("2026-10-08T12:00:00Z")));
        assert_eq!(
            entries[1].day,
            (!same).then(|| words("2026-10-08T12:03:00Z"))
        );
        assert_eq!(entries[2].day, None);
        assert_eq!(entries[3].day, None);
        assert_eq!(entries[4].day, Some(words("2026-10-10T12:00:00Z")));
        assert_eq!(entries[5].day, None);
    }

    #[test]
    fn the_rows_above_count_the_groups_and_the_breaks_an_older_page_adds() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![
                letter_at(12, "yardmaster", "jo", "2026-10-08T12:00:00Z", "one"),
                letter_at(13, "yardmaster", "jo", "2026-10-08T12:10:00Z", "two"),
                letter_at(14, "yardmaster", "jo", "2026-10-10T12:00:00Z", "three"),
            ],
        );
        // Above 14 stand two mails, their heads and the earlier day's
        // break; above 13 stand one mail and its head; above the oldest
        // mail nothing stands.
        assert_eq!(m.rows_above(14), 5);
        assert_eq!(m.rows_above(13), 2);
        assert_eq!(m.rows_above(12), 0);
        // Where the oldest shown mail joins the group above it, the new
        // mail body still adds a row, while its old head and break move up.
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![
                letter_at(12, "yardmaster", "jo", "2026-10-08T12:00:00Z", "one"),
                letter_at(13, "yardmaster", "jo", "2026-10-08T12:03:00Z", "two"),
            ],
        );
        let zone = chrono::Local;
        let same = day_of("2026-10-08T12:00:00Z", &zone) == day_of("2026-10-08T12:03:00Z", &zone);
        assert_eq!(m.rows_above(13), if same { 1 } else { 3 });
    }

    #[test]
    fn a_mail_joins_the_sender_s_group_within_five_minutes_on_the_day() {
        let zone = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let mail = |from: &str, at: &str| letter_at(1, from, "jo", at, "x");
        // Five minutes on the day, under the name above.
        assert!(grouped(
            &mail("ym", "2026-10-08T12:00:00Z"),
            &mail("ym", "2026-10-08T12:05:00Z"),
            &zone
        ));
        // A second past five minutes, another sender, or another day: its
        // own head. Three minutes over midnight are another day here.
        for (before, after) in [
            ("2026-10-08T12:00:00Z", "2026-10-08T12:05:01Z"),
            ("2026-10-08T21:59:00Z", "2026-10-08T22:02:00Z"),
        ] {
            assert!(
                !grouped(&mail("ym", before), &mail("ym", after), &zone),
                "{before} {after}"
            );
        }
        assert!(!grouped(
            &mail("ym", "2026-10-08T12:00:00Z"),
            &mail("jo", "2026-10-08T12:01:00Z"),
            &zone
        ));
        // A time that cannot be read stands on its own, either side.
        assert!(!grouped(
            &mail("ym", "2026-10-08T12:00:00Z"),
            &mail("ym", "soon"),
            &zone
        ));
        assert!(!grouped(
            &mail("ym", ""),
            &mail("ym", "2026-10-08T12:00:00Z"),
            &zone
        ));
    }

    #[test]
    fn a_day_s_separator_is_today_yesterday_or_the_weekday_with_the_day_and_the_month() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        assert_eq!(day_words(today, today), "Today");
        let yesterday = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        assert_eq!(day_words(yesterday, today), "Yesterday");
        let saturday = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
        assert_eq!(day_words(saturday, today), "Saturday 10 Oct");
        let friday = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        assert_eq!(day_words(friday, today), "Friday 9 Oct");
    }

    #[test]
    fn a_mail_whose_first_word_is_correction_carries_the_label() {
        for text in [
            "correction the edit card is ask 6893, not 6891",
            "Correction: the card is ask 6893",
            "  CORRECTION - ask 6893",
            "**correction**: the card is ask 6893",
            "(correction) the card is ask 6893",
        ] {
            assert!(correction(text).is_some(), "{text:?}");
        }
        for text in [
            "",
            "the edit card is ask 6893",
            "correctional facilities",
            "a correction follows",
        ] {
            assert_eq!(correction(text), None, "{text:?}");
        }
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![
                letter(12, "yardmaster", "jo", "the card is ask 6891"),
                letter(13, "yardmaster", "jo", "correction the card is ask 6893"),
            ],
        );
        let entries = m.entries();
        let corrections: Vec<bool> = entries.iter().map(|e| e.correction).collect();
        assert_eq!(corrections, [false, true]);
        // The label says the word: the text under it starts after it, and
        // the plain mail is as it was written.
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["the card is ask 6891", "the card is ask 6893"]);
    }

    #[test]
    fn a_correction_s_text_starts_after_its_word_and_the_marks_around_it() {
        for (text, said) in [
            (
                "correction the edit card is ask 6893, not 6891",
                "the edit card is ask 6893, not 6891",
            ),
            ("Correction: the card is ask 6893", "the card is ask 6893"),
            ("correction:the card is ask 6893", "the card is ask 6893"),
            ("  CORRECTION - ask 6893", "ask 6893"),
            ("Correction — ask 6893", "ask 6893"),
            (
                "**correction**: the card is ask 6893",
                "the card is ask 6893",
            ),
            (
                "**Correction:** the card is ask 6893",
                "the card is ask 6893",
            ),
            ("(correction) the card is ask 6893", "the card is ask 6893"),
            ("*Correction:* the card is ask 6893", "the card is ask 6893"),
            ("[correction] the card is ask 6893", "the card is ask 6893"),
            ("correction\n\nthe card is ask 6893", "the card is ask 6893"),
            // The word alone leaves nothing under the label.
            ("correction", ""),
            ("**correction**:", ""),
            ("(Correction)", ""),
            ("correction -", ""),
        ] {
            assert_eq!(correction(text), Some(said), "{text:?}");
        }
        // What opens the text stays as written, with its Markdown: a mark
        // that is not stuck to the word is the text's.
        for (text, said) in [
            (
                "correction: `src/model.rs` is the file",
                "`src/model.rs` is the file",
            ),
            (
                "correction **ask 6893**, not 6891",
                "**ask 6893**, not 6891",
            ),
            ("correction: (the card)", "(the card)"),
            ("correction: -3 degrees", "-3 degrees"),
            (
                "Correction:\n- ask 6893\n- ask 6891",
                "- ask 6893\n- ask 6891",
            ),
        ] {
            assert_eq!(correction(text), Some(said), "{text:?}");
        }
        // Where the word does not cut out cleanly the text stays whole: a
        // word with letters stuck to it is a word of the text, and a mark
        // that opens the whole text and nothing closes beside the word
        // would be left with its closing alone.
        for text in [
            "correction's effect",
            "correction-ish",
            "**Correction: ask 6893, not 6891.**",
            "(correction: the card is ask 6893.)",
        ] {
            assert_eq!(correction(text), Some(text), "{text:?}");
        }
    }

    #[test]
    fn a_correction_s_chips_stand_in_the_text_under_its_label() {
        let m = refs_model(
            &["correction: the card is ask 3."],
            vec![refs_open("yardr-n8ta", 3)],
            vec![],
        );
        let entries = m.entries();
        assert!(entries[0].correction);
        assert_eq!(entries[0].text, "the card is ask 3.");
        assert_eq!(entries[0].refs.len(), 1);
        assert_eq!(&entries[0].text[entries[0].refs[0].range.clone()], "ask 3");
        assert_eq!(entries[0].asks.len(), 1);
    }

    #[test]
    fn the_header_says_what_a_crew_member_does_and_nothing_for_anyone_else() {
        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.crew_state("yardmaster"), Some("working"));
        assert_eq!(m.crew_state("assembly"), Some("idle"));
        assert_eq!(m.crew_state("jo"), None);
        assert_eq!(m.crew_state("clerk@north"), None);
        assert_eq!(m.crew_state("nobody"), None);
    }

    /// A yard whose yardmaster wrote `texts` to jo, with the asks given
    /// and one arrival travelling: the conversation open.
    fn refs_model(texts: &[&str], open: Vec<OpenAsk>, decided: Vec<DecidedAsk>) -> Model {
        let mut state = yard(&[]);
        state.asks.open = open;
        state.asks.decided = decided;
        state.arrivals = board(vec![arrival(
            "yd-zulu", "new", "yardr", "5h–8h", "late", "amber",
        )]);
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.select(Selected::Conversation("yardmaster".into()));
        let letters: Vec<Letter> = texts
            .iter()
            .enumerate()
            .map(|(n, text)| {
                letter_at(
                    12 + n as u64,
                    "yardmaster",
                    "jo",
                    "2026-10-08T09:30:00Z",
                    text,
                )
            })
            .collect();
        m.conversation("yardmaster", letters);
        m
    }

    /// When the asks below were asked: seven minutes ago, so their
    /// previews wait `7m` whatever the clock says.
    fn refs_since() -> String {
        (chrono::Utc::now() - chrono::TimeDelta::minutes(7)).to_rfc3339()
    }

    fn refs_open(bead: &str, ask: u64) -> OpenAsk {
        open_ask(
            bead,
            "The grant",
            "",
            ask,
            "Grant yardmaster replace on google-docs?",
            "yardmaster",
            &refs_since(),
        )
    }

    #[test]
    fn an_ask_number_and_a_bead_id_chip_and_the_open_ask_stands_under_it() {
        let m = refs_model(
            &[
                "I put the grant to you as a card (ask 3).",
                "The turn (yd-zulu) waits, and re-ask 4 stands plain.",
                "First.\n\n- See ask 3 again.",
            ],
            vec![refs_open("yardr-n8ta", 3)],
            vec![],
        );
        let entries = m.entries();
        assert_eq!(entries.len(), 3);
        // The ask's chip opens its card in Decide.
        let ask = select::RefTarget::Ask {
            bead: "yardr-n8ta".into(),
            ask: 3,
            open: true,
        };
        assert_eq!(
            entries[0].refs,
            [select::RefSpan {
                block: 0,
                range: 34..39,
                target: ask.clone(),
            }]
        );
        // The open ask stands under the mail that names it, once.
        assert_eq!(
            entries[0].asks,
            [AskPreview {
                bead: "yardr-n8ta".into(),
                ask: 3,
                question: "Grant yardmaster replace on google-docs?".into(),
                asker: "yardmaster".into(),
                waits: "7m".into(),
                kind: "ask".into(),
            }]
        );
        // The travelling bead's chip opens the bead; `re-ask` is shaped
        // as a bead id is, but the yard said no such bead.
        assert_eq!(
            entries[1].refs,
            [select::RefSpan {
                block: 0,
                range: 10..17,
                target: select::RefTarget::Bead {
                    bead: "yd-zulu".into()
                },
            }]
        );
        assert!(entries[1].asks.is_empty());
        // In the list below: the ask in the second block.
        assert_eq!(
            entries[2].refs,
            [select::RefSpan {
                block: 1,
                range: 4..9,
                target: ask,
            }]
        );
        assert_eq!(entries[2].asks.len(), 1);
    }

    #[test]
    fn an_unknown_or_ambiguous_ask_stays_plain() {
        let m = refs_model(
            &["ask 3, ask 99 and ask 2."],
            vec![refs_open("yardr-n8ta", 3), refs_open("yardr-3msx", 3)],
            vec![decided_ask(
                "yardr-n8ta",
                2,
                "Move them?",
                "answered",
                "",
                "",
                "",
            )],
        );
        let entries = m.entries();
        // Ask 3 is two beads': no chip. Ask 99 is none the yard said.
        // Ask 2 was dealt with: its chip opens where it stands, with no
        // preview under it.
        assert_eq!(
            entries[0].refs,
            [select::RefSpan {
                block: 0,
                range: 18..23,
                target: select::RefTarget::Ask {
                    bead: "yardr-n8ta".into(),
                    ask: 2,
                    open: false,
                },
            }]
        );
        assert!(entries[0].asks.is_empty());
        // With its bead, ask 3 is one ask again.
        let m = refs_model(
            &["ask 3 on yardr-n8ta is yours."],
            vec![refs_open("yardr-n8ta", 3), refs_open("yardr-3msx", 3)],
            vec![],
        );
        let entries = m.entries();
        assert_eq!(entries[0].refs.len(), 1);
        assert_eq!(
            entries[0].refs[0].target,
            select::RefTarget::Ask {
                bead: "yardr-n8ta".into(),
                ask: 3,
                open: true,
            }
        );
        assert_eq!(
            &entries[0].text[entries[0].refs[0].range.clone()],
            "ask 3 on yardr-n8ta"
        );
        assert_eq!(entries[0].asks.len(), 1);
    }

    #[test]
    fn a_bare_ask_is_ambiguous_across_open_and_dealt_with_beads() {
        let m = refs_model(
            &["ask 3"],
            vec![refs_open("yardr-n8ta", 3)],
            vec![decided_ask(
                "yardr-3msx",
                3,
                "Move them?",
                "answered",
                "",
                "",
                "",
            )],
        );
        assert!(m.entries()[0].refs.is_empty());
        assert!(m.entries()[0].asks.is_empty());

        // Naming the bead makes the open ask unambiguous again.
        let m = refs_model(
            &["ask 3 on yardr-n8ta"],
            vec![refs_open("yardr-n8ta", 3)],
            vec![decided_ask(
                "yardr-3msx",
                3,
                "Move them?",
                "answered",
                "",
                "",
                "",
            )],
        );
        assert_eq!(
            m.entries()[0].refs[0].target,
            select::RefTarget::Ask {
                bead: "yardr-n8ta".into(),
                ask: 3,
                open: true,
            }
        );
        assert_eq!(m.entries()[0].asks.len(), 1);
    }

    #[test]
    fn a_qualified_ask_falls_back_to_its_bead() {
        let m = refs_model(&["ask 9 on yd-zulu, never asked."], vec![], vec![]);
        let entries = m.entries();
        // Ask 9 is none the yard said, but the bead travels: its own
        // words chip, and no preview stands under them.
        assert_eq!(
            entries[0].refs,
            [select::RefSpan {
                block: 0,
                range: 9..16,
                target: select::RefTarget::Bead {
                    bead: "yd-zulu".into()
                },
            }]
        );
        assert!(entries[0].asks.is_empty());
        // Where the bead is none the yard said either, nothing chips.
        let m = refs_model(&["ask 9 on yd-nowhere."], vec![], vec![]);
        assert!(m.entries()[0].refs.is_empty());
    }

    #[test]
    fn another_yard_s_bead_opens_with_its_peer() {
        let m = refs_model(
            &["see yardr-e9f0, and yardr-e9f0@north whole."],
            vec![refs_open("yardr-e9f0@north", 8)],
            vec![],
        );
        let entries = m.entries();
        // Named with and without its peer: the one known bead.
        let target = select::RefTarget::Bead {
            bead: "yardr-e9f0@north".into(),
        };
        assert_eq!(entries[0].refs.len(), 2);
        assert_eq!(entries[0].refs[0].target, target);
        assert_eq!(
            &entries[0].text[entries[0].refs[0].range.clone()],
            "yardr-e9f0"
        );
        assert_eq!(entries[0].refs[1].target, target);
        assert_eq!(
            &entries[0].text[entries[0].refs[1].range.clone()],
            "yardr-e9f0@north"
        );
        // Two yards' beads of one id: the bare id chips nowhere.
        let m = refs_model(
            &["see yardr-e9f0."],
            vec![refs_open("yardr-e9f0@north", 8)],
            vec![decided_ask(
                "yardr-e9f0@south",
                8,
                "Move them?",
                "answered",
                "",
                "",
                "",
            )],
        );
        assert!(m.entries()[0].refs.is_empty());
    }

    /// A yard with yd-zulu travelling and the yardmaster's conversation
    /// open and read, to which mail is added with [`lookup_mail`].
    fn lookup_model() -> Model {
        let mut state = yard(&[]);
        state.arrivals = board(vec![arrival(
            "yd-zulu", "new", "yardr", "5h–8h", "late", "amber",
        )]);
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.select(Selected::Conversation("yardmaster".into()));
        m.conversation("yardmaster", vec![]);
        m
    }

    /// The yardmaster's mail `seq` to jo, which comes into view, and what
    /// the yard is asked for by it.
    fn lookup_mail(m: &mut Model, seq: u64, text: &str) -> Vec<Effect> {
        m.conversation(
            "yardmaster",
            vec![letter_at(
                seq,
                "yardmaster",
                "jo",
                "2026-10-08T09:30:00Z",
                text,
            )],
        )
    }

    fn asking(beads: &[&str]) -> Effect {
        Effect::Beads {
            beads: beads.iter().map(|id| id.to_string()).collect(),
        }
    }

    /// What the yard answers to a `Bead` call for an id it has none of.
    fn no_such_bead(id: &str) -> (String, Result<String, api::Error>) {
        let refused = api::Error::Refused {
            code: api::NOT_FOUND.into(),
            message: format!("bead {id}: not found"),
        };
        (id.to_string(), Err(refused))
    }

    /// The ids mail names that no list holds are asked of the yard in one
    /// effect, each once: not the ones a list holds, nor those of another
    /// yard, and not again while the answer is awaited.
    #[test]
    fn the_ids_no_list_holds_are_asked_of_the_yard_once_in_one_effect() {
        let mut m = lookup_model();
        let effects = lookup_mail(
            &mut m,
            12,
            "footplate-zgtr merged, yd-zulu travels, re-ask 4 and yardr-e9f0@north stand apart, footplate-zgtr again.",
        );
        // In the order they stand. The travelling bead is a list's; the
        // yard can answer for a bead another yard asked on too.
        assert_eq!(
            effects,
            [asking(&["footplate-zgtr", "re-ask", "yardr-e9f0@north"])]
        );

        // Mail that names them while they are awaited asks for them no
        // more, and for what is new alone.
        let effects = lookup_mail(&mut m, 13, "footplate-zgtr and re-ask, and built-in too");
        assert_eq!(effects, [asking(&["built-in"])]);
        assert_eq!(lookup_mail(&mut m, 14, "built-in"), []);
    }

    #[test]
    fn a_closed_bead_chips_once_the_yard_has_it_and_the_words_it_refused_stay_plain() {
        let mut m = lookup_model();
        let said = "footplate-zgtr merged, a re-ask stands plain, and footplate-zg is no id.";
        let effects = lookup_mail(&mut m, 12, said);
        let asked = ["footplate-zgtr", "re-ask", "footplate-zg"];
        assert_eq!(effects, [asking(&asked)]);
        // Before the yard answers, none of them chips.
        assert!(m.entries()[0].refs.is_empty());

        // The yard has footplate-zgtr under that very id, none of re-ask,
        // and takes footplate-zg for the longer id it is the start of.
        m.beads_looked(
            &asked.map(String::from),
            vec![
                ("footplate-zgtr".into(), Ok("footplate-zgtr".into())),
                no_such_bead("re-ask"),
                ("footplate-zg".into(), Ok("footplate-zgtr".into())),
            ],
        );
        let entries = m.entries();
        assert_eq!(entries[0].refs.len(), 1);
        assert_eq!(
            entries[0].refs[0].target,
            select::RefTarget::Bead {
                bead: "footplate-zgtr".into()
            }
        );
        assert_eq!(&said[entries[0].refs[0].range.clone()], "footplate-zgtr");

        // What the yard said stands: neither mail that names the ids
        // again nor a row opened again asks it a second time.
        assert_eq!(lookup_mail(&mut m, 13, said), []);
        m.select(Selected::Yard);
        assert_eq!(m.select(Selected::Conversation("yardmaster".into())), []);
    }

    /// A refusal is the yard's word on the id, whatever its code. A call
    /// that got no answer says nothing of it, so mail that comes into view
    /// asks again: the stream's, or a row opened.
    #[test]
    fn an_id_the_yard_did_not_answer_is_asked_again_with_the_next_mail() {
        let mut m = lookup_model();
        let effects = lookup_mail(&mut m, 12, "the ids x-yz12 and built-in");
        assert_eq!(effects, [asking(&["x-yz12", "built-in"])]);
        let down = api::Error::Down(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
        let refused = api::Error::Refused {
            code: "error".into(),
            message: "bead built-in: give at least 4 characters after \"built-\"".into(),
        };
        // The edge's word about its link is no word on the id either.
        let away = api::Error::Refused {
            code: "away".into(),
            message: "the yard is away".into(),
        };
        m.beads_looked(
            &["x-yz12".to_string(), "built-in".to_string()],
            vec![
                ("x-yz12".into(), Err(down)),
                ("built-in".into(), Err(refused)),
            ],
        );
        m.beads_looked(&["x-yz12".to_string()], vec![("x-yz12".into(), Err(away))]);
        let effects = m.event(&event(
            21,
            "mail",
            "yardmaster",
            json!({"to": "jo", "to_user": true, "text": "x-yz12 again"}),
        ));
        assert!(effects.contains(&asking(&["x-yz12"])), "{effects:?}");
        // Opening the row asks as well.
        m.beads_looked(&["x-yz12".to_string()], vec![]);
        m.select(Selected::Yard);
        assert_eq!(
            m.select(Selected::Conversation("yardmaster".into())),
            [asking(&["x-yz12"])]
        );
    }

    /// A bead a list held when mail named it stays a chip once it closes
    /// and falls out of the list, and no call asks for it.
    #[test]
    fn a_bead_a_list_held_when_mail_named_it_chips_after_it_closes() {
        let mut m = lookup_model();
        assert_eq!(lookup_mail(&mut m, 12, "yd-zulu waits"), []);
        let chip = |m: &Model| {
            m.entries()[0]
                .refs
                .iter()
                .map(|found| found.target.clone())
                .collect::<Vec<_>>()
        };
        let zulu = select::RefTarget::Bead {
            bead: "yd-zulu".into(),
        };
        assert_eq!(chip(&m), std::slice::from_ref(&zulu));

        // It closes, and the board has it no more.
        m.arrivals(board(vec![]));
        assert_eq!(chip(&m), [zulu]);
        assert_eq!(lookup_mail(&mut m, 13, "and yd-zulu again"), []);
    }

    /// The same of the yard row's mail, which draws chips as a
    /// conversation's does: asked when the row opens, and with the mail
    /// that comes into view on it. Mail of the yard's while another row is
    /// open is not in view, and asks nothing.
    #[test]
    fn the_yard_row_s_mail_is_asked_for_its_ids_too() {
        let mut m = lookup_model();
        let from_the_yard = |seq, text| letter_at(seq, "yard", "jo", "2026-10-08T09:30:00Z", text);
        assert_eq!(
            m.conversation("yard", vec![from_the_yard(30, "footplate-zgtr merged")]),
            []
        );

        let effects = m.select(Selected::Yard);
        assert!(
            effects.contains(&asking(&["footplate-zgtr"])),
            "{effects:?}"
        );
        let effects = m.conversation("yard", vec![from_the_yard(31, "and footplate-b8g9")]);
        assert!(
            effects.contains(&asking(&["footplate-b8g9"])),
            "{effects:?}"
        );
    }

    /// An ask named on a bead that closed falls back to the bead, as it
    /// does on one a list holds.
    #[test]
    fn an_ask_named_on_a_closed_bead_falls_back_to_the_bead() {
        let mut m = lookup_model();
        let said = "ask 9 on footplate-zgtr, long since answered.";
        assert_eq!(lookup_mail(&mut m, 12, said), [asking(&["footplate-zgtr"])]);
        m.beads_looked(
            &["footplate-zgtr".to_string()],
            vec![("footplate-zgtr".into(), Ok("footplate-zgtr".into()))],
        );
        let entries = m.entries();
        assert_eq!(entries[0].refs.len(), 1);
        assert_eq!(
            entries[0].refs[0].target,
            select::RefTarget::Bead {
                bead: "footplate-zgtr".into()
            }
        );
        assert_eq!(&said[entries[0].refs[0].range.clone()], "footplate-zgtr");
    }

    /// A closed bead another yard asked on is still a bead the yard can
    /// serve whole: its qualified id is asked too, and a reference the
    /// yard refuses stays plain.
    #[test]
    fn a_closed_peer_bead_named_in_mail_chips_and_a_non_bead_peer_ref_stays_plain() {
        let mut m = lookup_model();
        let said = "yd-b8g9@north is closed, and yd-nope@north is no bead.";
        let asked = ["yd-b8g9@north", "yd-nope@north"];
        assert_eq!(lookup_mail(&mut m, 12, said), [asking(&asked)]);
        m.beads_looked(
            &asked.map(String::from),
            vec![
                ("yd-b8g9@north".into(), Ok("yd-b8g9@north".into())),
                no_such_bead("yd-nope@north"),
            ],
        );
        let entries = m.entries();
        assert_eq!(entries[0].refs.len(), 1);
        assert_eq!(
            entries[0].refs[0].target,
            select::RefTarget::Bead {
                bead: "yd-b8g9@north".into()
            }
        );
        assert_eq!(&said[entries[0].refs[0].range.clone()], "yd-b8g9@north");
    }

    /// The queue read again drops the beads read whole that it holds none
    /// of, but not the bead the drawer shows: a closed bead, which a chip
    /// in mail opens, is never in the queue.
    #[test]
    fn the_queue_read_again_keeps_the_bead_the_drawer_was_opened_on() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.read_bead("footplate-zgtr"),
            [Effect::Bead {
                bead: "footplate-zgtr".into()
            }]
        );
        for id in ["footplate-zgtr", "yd-other"] {
            m.bead_loaded(Bead {
                id: id.into(),
                ..Bead::default()
            });
        }
        m.asks(Asks::default());
        assert!(m.has_bead("footplate-zgtr"));
        assert!(!m.has_bead("yd-other"));
        // Open again, it is not read again.
        assert_eq!(m.read_bead("footplate-zgtr"), []);
    }

    /// The yard read again, as after it was lost, makes a new model: what
    /// the yard said of the ids mail names carries over to it.
    #[test]
    fn what_the_yard_said_of_the_ids_stays_when_the_yard_is_read_again() {
        let mut m = lookup_model();
        let said = "footplate-zgtr merged, and a re-ask stands plain";
        lookup_mail(&mut m, 12, said);
        m.beads_looked(
            &["footplate-zgtr".to_string(), "re-ask".to_string()],
            vec![
                ("footplate-zgtr".into(), Ok("footplate-zgtr".into())),
                no_such_bead("re-ask"),
            ],
        );
        let (mut again, _) = m.resumed(yard(&[]), Mode::Chat);
        assert_eq!(lookup_mail(&mut again, 12, said), []);
        assert_eq!(again.entries()[0].refs.len(), 1);
    }

    #[test]
    fn a_run_or_secret_ask_s_preview_says_what_it_is() {
        let mut run = refs_open("yardr-n8ta", 3);
        run.run = Some(Run::default());
        let mut secret = refs_open("yardr-3msx", 4);
        secret.secret = Some(Secret::default());
        let m = refs_model(&["ask 3 and ask 4."], vec![run, secret], vec![]);
        let entries = m.entries();
        assert_eq!(entries[0].refs.len(), 2);
        let kinds: Vec<&str> = entries[0]
            .asks
            .iter()
            .map(|ask| ask.kind.as_str())
            .collect();
        assert_eq!(kinds, ["run ask", "secret ask"]);
    }

    #[test]
    fn mail_to_a_linked_yard_stands_by_its_delivery() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.select(Selected::Conversation("clerk@north".into())),
            [page("clerk@north", 0)]
        );
        assert_eq!(
            m.target(),
            Some(Target::Peer {
                name: "clerk".into(),
                peer: "north".into()
            })
        );
        m.conversation("clerk@north", vec![]);
        // The user's mail to the peer, as the stream tells of it.
        m.event(&event(
            21,
            "mail",
            "jo",
            json!({"to": "clerk", "to_peer": "north", "text": "Is the ledger closed?"}),
        ));
        // Someone else's mail to a peer is no conversation of the user's.
        m.event(&event(
            22,
            "mail",
            "yardmaster",
            json!({"to": "clerk", "to_peer": "north", "text": "x"}),
        ));
        let entries = m.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].standing.as_deref(),
            Some("queued for north: the daemon delivers it")
        );

        // The link says it was sent: the linked yards and the open
        // conversations with that yard are read again, whole.
        let effects = m.event(&event(
            23,
            "peer_message_sent",
            "yard",
            json!({"peer": "north"}),
        ));
        assert_eq!(effects, [Effect::Peers, read("clerk@north", 0)]);
        m.conversation(
            "clerk@north",
            vec![Letter {
                delivery: "sent".into(),
                ..letter(21, "jo", "clerk@north", "Is the ledger closed?")
            }],
        );
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("delivered to north, which does not say when it is read")
        );
        // The stream telling of it again as queued does not put that back.
        m.event(&event(
            21,
            "mail",
            "jo",
            json!({"to": "clerk", "to_peer": "north", "text": "Is the ledger closed?"}),
        ));
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("delivered to north, which does not say when it is read")
        );

        m.conversation(
            "clerk@north",
            vec![Letter {
                delivery: "failed".into(),
                ..letter(24, "jo", "clerk@north", "again")
            }],
        );
        assert_eq!(
            m.entries()[1].standing.as_deref(),
            Some("not delivered: yardr inbox says why")
        );
    }

    #[test]
    fn mail_refused_over_the_hour_says_so_under_it() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.select(Selected::Conversation("clerk@north".into())),
            [page("clerk@north", 0)]
        );
        m.conversation(
            "clerk@north",
            vec![Letter {
                delivery: "failed".into(),
                ..letter(24, "jo", "clerk@north", "Is the ledger closed?")
            }],
        );
        // Without the yard's word of the hour, the inbox says why.
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("not delivered: yardr inbox says why")
        );
        // Another peer's hour is none of this mail's.
        m.event(&event(
            25,
            "peer_message_failed",
            "yard",
            json!({"peer": "south", "kind": "mail", "id": "s1"}),
        ));
        m.event(&event(
            26,
            "mail",
            "yard",
            json!({"to": "jo", "to_user": true, "undelivered": "s1", "limit": 300,
                   "until": "2026-10-12T21:00:00Z", "text": "mail of south's hour"}),
        ));
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("not delivered: yardr inbox says why")
        );
        // The yard gave the message up, then told of it with the hour:
        // the mail names the message alone, the failed event before it
        // names its peer.
        m.event(&event(
            27,
            "peer_message_failed",
            "yard",
            json!({"peer": "north", "kind": "mail", "id": "m1"}),
        ));
        m.event(&event(
            28,
            "mail",
            "yard",
            json!({"to": "jo", "to_user": true, "undelivered": "m1", "limit": 30,
                   "until": "2026-10-12T21:00:00Z",
                   "text": "your mail to clerk@north of 10-12 20:31 was not delivered"}),
        ));
        let end = DateTime::parse_from_rfc3339("2026-10-12T21:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%H:%M")
            .to_string();
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some(
                format!(
                    "not delivered: north takes 30 mails an hour from this yard; send again after {end}"
                )
                .as_str()
            )
        );
        // A refusal the yard tells without the hour clears the peer's:
        // the inbox says why again.
        m.event(&event(
            29,
            "peer_message_failed",
            "yard",
            json!({"peer": "north", "kind": "mail", "id": "m2"}),
        ));
        m.event(&event(
            30,
            "mail",
            "yard",
            json!({"to": "jo", "to_user": true, "undelivered": "m2",
                   "text": "your mail to clerk@north of 10-12 20:33 was not delivered"}),
        ));
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("not delivered: yardr inbox says why")
        );
        // A telling of a message the yard never said it gave up names no
        // peer, so it says nothing under the mail.
        m.event(&event(
            31,
            "mail",
            "yard",
            json!({"to": "jo", "to_user": true, "undelivered": "m9", "limit": 30,
                   "until": "2026-10-12T21:00:00Z", "text": "mail of no failure"}),
        ));
        assert_eq!(
            m.entries()[0].standing.as_deref(),
            Some("not delivered: yardr inbox says why")
        );
    }

    #[test]
    fn what_is_typed_goes_to_a_crew_member_as_mail() {
        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.target(), Some(Target::Crew("yardmaster".into())));
    }

    #[test]
    fn above_a_conversation_with_a_linked_yard_stands_who_that_is() {
        let mut state = yard(&[]);
        let (m, _) = Model::new(
            state.clone(),
            Mode::Chat,
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert_eq!(
            m.about(),
            ["a crew member of the linked yard north: Keeps the books"]
        );
        let (m, _) = Model::new(
            state.clone(),
            Mode::Chat,
            Some(Selected::Conversation("mckean@north".into())),
        );
        assert_eq!(m.about(), ["a person of the linked yard north"]);
        let (m, _) = Model::new(
            state.clone(),
            Mode::Chat,
            Some(Selected::Conversation("yardmaster".into())),
        );
        assert_eq!(m.about(), Vec::<String>::new());

        state.peers[0].said.as_mut().unwrap().roster[0].mail = false;
        let (m, _) = Model::new(
            state.clone(),
            Mode::Chat,
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert!(
            m.about()[1].starts_with("north takes no mail from this yard yet"),
            "{:?}",
            m.about()
        );
        state.peers[0].send.clear();
        let (m, _) = Model::new(
            state,
            Mode::Chat,
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert!(
            m.about()[1].ends_with("yardr peer allow north --send mail"),
            "{:?}",
            m.about()
        );
    }

    #[test]
    fn a_crew_status_event_changes_the_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.event(&event(
                21,
                "crew_status",
                "",
                json!({"crew": "assembly", "status": "blocked", "was": "idle"})
            )),
            []
        );
        assert_eq!(m.rows()[0].line, "blocked");
    }

    #[test]
    fn other_crew_events_have_the_crew_read_again() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(
            m.event(&event(21, "crew_stopped", "", json!({"crew": "assembly"}))),
            [Effect::Crew]
        );
        assert_eq!(
            m.event(&event(
                22,
                "session_prompted",
                "",
                json!({"crew": "assembly"})
            )),
            [Effect::Crew]
        );
        // The first prompt of an agent on a bead is no crew member's.
        assert_eq!(
            m.event(&event(23, "session_prompted", "", json!({"session": "s1"}))),
            []
        );
        assert_eq!(m.event(&event(24, "bead_sung", "", json!({}))), []);
    }

    #[test]
    fn arrivals_and_flows_are_read_again_on_a_bead_s_events_and_their_other_inputs() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        // Every event about a bead, whatever its kind: a note re-reads
        // the queue beside the board and the flows view.
        assert_eq!(
            m.event(&bead_event(21, "noted", "yd-links", json!({}))),
            [Effect::Asks, Effect::Arrivals, Effect::Flows]
        );
        assert_eq!(
            m.event(&bead_event(22, "session_ended", "yd-links", json!({}))),
            [Effect::Arrivals, Effect::Flows]
        );
        // The changes that move their other inputs though they name no
        // bead: docs/api.md names them with the calls.
        for (seq, kind) in [
            (23, "group_set"),
            (24, "group_removed"),
            (25, "route_added"),
            (26, "route_removed"),
            (27, "flow_set"),
            (28, "flow_removed"),
            (29, "flow_pointer_set"),
            (30, "flow_pointer_cleared"),
            (31, "depot_updated"),
        ] {
            assert_eq!(
                m.event(&event(seq, kind, "", json!({}))),
                [Effect::Arrivals, Effect::Flows],
                "{kind}"
            );
        }
        // A removed depot's line leaves the timetable, and only
        // `depot_removed` says so: the flows view alone moves for it.
        assert_eq!(
            m.event(&event(32, "depot_removed", "", json!({}))),
            [Effect::Flows]
        );
        for (seq, key) in [(33, "max_sessions"), (34, "operator")] {
            assert_eq!(
                m.event(&event(seq, "setting_changed", "", json!({"key": key}))),
                [Effect::Arrivals, Effect::Flows],
                "{key}"
            );
        }
        // A setting of any other name, a crew status and whatever else
        // names no bead move nothing of the board or the flows view.
        assert_eq!(
            m.event(&event(
                35,
                "setting_changed",
                "",
                json!({"key": "web_addr"})
            )),
            []
        );
        assert_eq!(m.event(&event(36, "setting_changed", "", json!({}))), []);
        assert_eq!(
            m.event(&event(
                37,
                "crew_status",
                "",
                json!({"crew": "assembly", "status": "blocked", "was": "idle"})
            )),
            []
        );
        assert_eq!(m.event(&event(38, "timer", "", json!({}))), []);
    }

    fn arrival(
        bead: &str,
        platform: &str,
        line: &str,
        expected: &str,
        remark: &str,
        tone: &str,
    ) -> Arrival {
        Arrival {
            bead: bead.into(),
            title: format!("{bead} moves"),
            platform: platform.into(),
            line: line.into(),
            expected: expected.into(),
            remark: remark.into(),
            tone: tone.into(),
            held_by: String::new(),
        }
    }

    fn board(rows: Vec<Arrival>) -> Arrivals {
        Arrivals { rows }
    }

    #[test]
    fn the_board_is_the_board_as_read_now_over_a_yard_lost() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(m.arrival_rows().is_empty());
        m.arrivals(board(vec![arrival(
            "yd-links", "new", "yardr", "2h–3h", "", "none",
        )]));
        assert_eq!(m.arrival_rows().len(), 1);
        // Read again after the yard was lost: the board is as read now.
        let (again, _) = m.resumed(yard(&[]), Mode::Chat);
        assert!(again.arrival_rows().is_empty());
    }

    /// The board's rows stand flat in the served arrival order,
    /// remarked as served, with no grouping of their own.
    #[test]
    fn the_board_s_rows_stand_flat_in_the_served_arrival_order() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let rows = vec![
            arrival("yd-second", "review", "yardr", "20m–35m", "", "none"),
            arrival("yd-third", "draft", "yardr", "5h–8h", "late", "amber"),
            arrival("yd-fourth", "review", "yardr", "1h–2h", "held", "red"),
        ];
        m.arrivals(board(rows.clone()));
        assert_eq!(m.arrival_rows(), rows.as_slice());
    }

    /// The arrivals row stands at the timetable's head with the
    /// yard-wide travelling total the Flows call serves, complete where
    /// the board's rows are capped. It stands open for the board and for
    /// a travelling bead from it, which has no row of its own.
    #[test]
    fn the_arrivals_row_counts_what_is_in_transit() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let row = m.arrivals_row();
        assert_eq!(row.name, "arrivals");
        assert_eq!(row.line, "none in transit");
        assert_eq!(row.unread, 0);
        assert!(!row.selected);
        // The served total, not the served rows: sixteen travel, fifteen
        // show.
        m.flows(Flows {
            travelling: 16,
            ..Flows::default()
        });
        m.arrivals(board(vec![arrival(
            "yd-a", "new", "yardr", "2h–3h", "", "none",
        )]));
        assert_eq!(m.travelling(), 16);
        assert_eq!(m.arrivals_row().line, "16 in transit");
        m.flows(Flows {
            travelling: 1,
            ..Flows::default()
        });
        assert_eq!(m.arrivals_row().line, "1 in transit");
        // Opening the board needs no call: its rows came with the state.
        assert_eq!(m.select(Selected::Arrivals), []);
        assert!(m.arrivals_row().selected);
        // A travelling bead reads the bead whole when it was not read
        // before, and the row stands open for it.
        assert_eq!(
            m.select(Selected::Bead("yd-a".into())),
            [Effect::Bead {
                bead: "yd-a".into()
            }]
        );
        assert!(m.arrivals_row().selected);
        m.bead_loaded(Bead {
            id: "yd-a".into(),
            ..Bead::default()
        });
        assert_eq!(m.select(Selected::Bead("yd-a".into())), []);
        // Elsewhere the row stands shut.
        m.select(Selected::Yard);
        assert!(!m.arrivals_row().selected);
    }

    fn flowed(bead: &str, title: &str, expected: &str, remark: &str, tone: &str) -> FlowBead {
        FlowBead {
            bead: bead.into(),
            title: title.into(),
            expected: expected.into(),
            remark: remark.into(),
            tone: tone.into(),
            working: false,
            ..FlowBead::default()
        }
    }

    /// Two lines as served, one travelling and one not, with an
    /// unassigned bead: yardr's default flow with a turn and a fault
    /// stage and an Elsewhere bead, footplate's empty flow, and the
    /// yard-wide total.
    fn flow_view() -> Flows {
        use crate::api::{Flow, FlowDepot, FlowStage};
        Flows {
            travelling: 3,
            depots: vec![
                FlowDepot {
                    name: "yardr".into(),
                    kind: "git".into(),
                    travelling: 2,
                    flows: vec![Flow {
                        name: "default".into(),
                        flow_type: "".into(),
                        source: "store".into(),
                        landings: vec![],
                        stages: vec![
                            FlowStage {
                                stage: "new".into(),
                                group: "muse-builders".into(),
                                human: false,
                                lamp: "turn".into(),
                                beads: vec![flowed(
                                    "yd-zulu",
                                    "Zulu's late turn",
                                    "5h–8h",
                                    "late",
                                    "amber",
                                )],
                                ..FlowStage::default()
                            },
                            FlowStage {
                                stage: "review".into(),
                                group: "mckean".into(),
                                human: true,
                                lamp: "fault".into(),
                                beads: vec![flowed(
                                    "yd-alpha",
                                    "Alpha moves first",
                                    "20m–35m",
                                    "held",
                                    "red",
                                )],
                                ..FlowStage::default()
                            },
                        ],
                    }],
                    elsewhere: vec![flowed(
                        "yardr-pinned",
                        "A pinned flow's bead",
                        "—",
                        "",
                        "none",
                    )],
                    error: "".into(),
                },
                FlowDepot {
                    name: "footplate".into(),
                    kind: "git".into(),
                    travelling: 0,
                    flows: vec![Flow {
                        name: "default".into(),
                        flow_type: "".into(),
                        source: "store".into(),
                        landings: vec![],
                        stages: vec![FlowStage {
                            stage: "new".into(),
                            group: "".into(),
                            human: false,
                            lamp: "none".into(),
                            beads: vec![],
                            ..FlowStage::default()
                        }],
                    }],
                    elsewhere: vec![],
                    error: "".into(),
                },
            ],
            unassigned: vec![flowed("yd-shed", "Paint the shed", "2h–3h", "", "none")],
        }
    }

    /// The lines stand in the served order with their served travelling
    /// totals, every served line travelling or not; opening one needs no
    /// call. A removed depot's line is gone from the order.
    #[test]
    fn the_lines_stand_in_the_served_order_with_their_totals() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert!(m.line_rows().is_empty());
        assert!(m.depot("yardr").is_none());
        m.flows(flow_view());
        let rows = m.line_rows();
        assert_eq!(
            rows.iter()
                .map(|r| (r.name.as_str(), r.line.as_str()))
                .collect::<Vec<_>>(),
            [
                ("yardr", "2 in transit"),
                ("footplate", "nothing in transit"),
            ]
        );
        assert!(rows.iter().all(|r| r.unread == 0));
        assert!(rows.iter().all(|r| !r.selected));
        // Opening a line needs no call: its flows came with the state.
        assert_eq!(m.select(Selected::Line("yardr".into())), []);
        assert_eq!(m.selected(), Some(&Selected::Line("yardr".into())));
        let rows = m.line_rows();
        assert!(rows[0].selected);
        assert!(!rows[1].selected);
        assert!(!m.arrivals_row().selected);
        // No such line changes nothing.
        assert_eq!(m.select(Selected::Line("signalbox".into())), []);
        assert_eq!(m.selected(), Some(&Selected::Line("yardr".into())));
        // A removed depot's line is gone from the order, and over a
        // yard lost the board opens where it stood.
        let (again, _) = m.resumed(
            State {
                flows: Flows {
                    travelling: 1,
                    depots: vec![FlowDepot {
                        name: "footplate".into(),
                        travelling: 0,
                        ..FlowDepot::default()
                    }],
                    unassigned: vec![flowed("yd-shed", "Paint the shed", "2h–3h", "", "none")],
                },
                ..yard(&[])
            },
            Mode::Timetable,
        );
        assert_eq!(again.selected(), Some(&Selected::Arrivals));
        // Read again with the line gone, the window falls back the same.
        m.flows(Flows {
            travelling: 1,
            depots: vec![FlowDepot {
                name: "footplate".into(),
                travelling: 0,
                ..FlowDepot::default()
            }],
            unassigned: vec![],
        });
        assert_eq!(m.selected(), Some(&Selected::Arrivals));
    }

    /// The No line row stands last, only while beads travel with no line,
    /// with how many travel. Opening it needs no call.
    #[test]
    fn the_no_line_row_stands_last_only_while_unassigned_beads_travel() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert_eq!(m.no_line_row(), None);
        assert_eq!(m.unassigned(), &[]);
        m.flows(flow_view());
        let row = m.no_line_row().unwrap();
        assert_eq!(row.name, "No line");
        assert_eq!(row.line, "1 in transit");
        assert!(!row.selected);
        assert_eq!(m.unassigned().len(), 1);
        assert_eq!(m.unassigned()[0].bead, "yd-shed");
        assert_eq!(m.select(Selected::NoLine), []);
        assert_eq!(m.selected(), Some(&Selected::NoLine));
        assert!(m.no_line_row().unwrap().selected);
        // Nothing without a line: the row is gone, and the board opens
        // where it stood, read again or over a yard lost.
        let (again, _) = m.resumed(
            State {
                flows: Flows {
                    travelling: 2,
                    depots: m.flows.depots.clone(),
                    unassigned: vec![],
                },
                ..yard(&[])
            },
            Mode::Timetable,
        );
        assert_eq!(again.selected(), Some(&Selected::Arrivals));
        m.flows(Flows {
            travelling: 2,
            depots: m.flows.depots.clone(),
            unassigned: vec![],
        });
        assert_eq!(m.no_line_row(), None);
        assert_eq!(m.selected(), Some(&Selected::Arrivals));
    }

    /// A travelling bead comes back to the row it was opened from: the
    /// board, a line or No line. That row stands open for it, and the
    /// board where none was or the row is gone.
    #[test]
    fn a_travelling_bead_comes_back_to_the_row_it_came_from() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.flows(flow_view());
        // None opened yet: the board.
        assert_eq!(m.bead_back(), Selected::Arrivals);
        m.select(Selected::Arrivals);
        m.select(Selected::Bead("yd-zulu".into()));
        assert_eq!(m.bead_back(), Selected::Arrivals);
        assert!(m.arrivals_row().selected);
        // From a line: the line stands open for the bead.
        m.select(Selected::Line("yardr".into()));
        m.select(Selected::Bead("yd-zulu".into()));
        assert_eq!(m.bead_back(), Selected::Line("yardr".into()));
        assert!(!m.arrivals_row().selected);
        assert!(m.line_rows()[0].selected);
        // From No line.
        m.select(Selected::NoLine);
        m.select(Selected::Bead("yd-shed".into()));
        assert_eq!(m.bead_back(), Selected::NoLine);
        assert!(m.no_line_row().unwrap().selected);
        // The line is gone: the board.
        m.flows(Flows::default());
        assert_eq!(m.bead_back(), Selected::Arrivals);
        // Over a yard lost the row comes back too. footplate is quiet,
        // so Quiet is unfolded first, as Decided is for a dealt with.
        m.flows(flow_view());
        m.toggle_quiet();
        m.select(Selected::Line("footplate".into()));
        m.select(Selected::Bead("yd-zulu".into()));
        let (again, _) = m.resumed(
            State {
                flows: m.flows.clone(),
                ..yard(&[])
            },
            Mode::Timetable,
        );
        assert_eq!(again.selected(), Some(&Selected::Bead("yd-zulu".into())));
        assert_eq!(again.bead_back(), Selected::Line("footplate".into()));
    }

    /// From a line's bead Tab steps from the line's row, as from the
    /// board's for a bead from there. footplate is quiet, so folded it
    /// is stepped over; unfolded it stands after No line.
    #[test]
    fn stepping_from_a_line_s_bead_uses_the_line_s_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.flows(flow_view());
        m.select(Selected::Line("yardr".into()));
        m.select(Selected::Bead("yd-zulu".into()));
        m.step_in(Mode::Timetable, 1);
        assert_eq!(m.selected(), Some(&Selected::NoLine));
        m.select(Selected::Line("yardr".into()));
        m.select(Selected::Bead("yd-zulu".into()));
        m.step_in(Mode::Timetable, -1);
        assert_eq!(m.selected(), Some(&Selected::Arrivals));
        m.toggle_quiet();
        m.select(Selected::NoLine);
        m.step_in(Mode::Timetable, 1);
        assert_eq!(m.selected(), Some(&Selected::Line("footplate".into())));
    }

    /// A bead's route is its flow's stages in travel order, each with
    /// whether a bead there waits on a person, with which one the bead
    /// stands at. A bead in Elsewhere, with no line or unknown has no
    /// route.
    #[test]
    fn a_bead_s_route_is_its_flow_s_stages_with_where_it_stands() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.flows(flow_view());
        let route = m.route_for("yd-alpha").unwrap();
        assert_eq!(
            route
                .stages
                .iter()
                .map(|s| (s.name.as_str(), s.human))
                .collect::<Vec<_>>(),
            [("new", false), ("review", true)]
        );
        assert_eq!(route.current, 1);
        assert_eq!(m.route_for("yd-zulu").unwrap().current, 0);
        assert_eq!(m.route_for("yardr-pinned"), None);
        assert_eq!(m.route_for("yd-shed"), None);
        assert_eq!(m.route_for("yd-unknown"), None);
    }

    /// A remark names you where it names you: a wait with you
    /// first among its names, a hold by you as `held by you`. Any
    /// other remark, and a wait or hold for nobody you are, stands as
    /// printed.
    #[test]
    fn a_remark_names_you_where_it_names_you() {
        assert_eq!(
            you_first("waits for yardmaster, jo", "jo"),
            "waits for you, yardmaster"
        );
        assert_eq!(you_first("waits for jo", "jo"), "waits for you");
        assert_eq!(
            you_first("waits for yardmaster", "jo"),
            "waits for yardmaster"
        );
        assert_eq!(you_first("held by jo", "jo"), "held by you");
        assert_eq!(you_first("held by yardmaster", "jo"), "held by yardmaster");
        assert_eq!(you_first("held", "jo"), "held");
        assert_eq!(you_first("", "jo"), "");
        assert_eq!(you_first("waits for jo", ""), "waits for jo");
        assert_eq!(you_first("held by jo", ""), "held by jo");
    }

    /// A wait names you where you are among its names, and only then:
    /// a hold is no wait, whoever holds it.
    #[test]
    fn a_wait_names_you_where_you_are_among_its_names() {
        assert!(waits_on_you("waits for yardmaster, jo", "jo"));
        assert!(waits_on_you("waits for jo", "jo"));
        assert!(!waits_on_you("waits for yardmaster", "jo"));
        assert!(!waits_on_you("held", "jo"));
        assert!(!waits_on_you("held by jo", "jo"));
        assert!(!waits_on_you("", "jo"));
        assert!(!waits_on_you("waits for jo", ""));
    }

    /// A hold is `held` or `held by <name>`; a stall is no hold. The
    /// served holder is the reader where it names them, and an
    /// unknown holder is nobody's.
    #[test]
    fn a_hold_is_held_or_held_by_a_name() {
        assert!(is_held("held"));
        assert!(is_held("held by jo"));
        assert!(!is_held("stalled · session failed"));
        assert!(!is_held("blocked by yd-x"));
        assert!(!is_held(""));
        assert!(held_by_you("jo", "jo"));
        assert!(!held_by_you("yardmaster", "jo"));
        assert!(!held_by_you("", "jo"));
        assert!(!held_by_you("jo", ""));
        assert!(!held_by_you("", ""));
    }

    /// The quiet lines fold under Quiet: folded they are not in the
    /// order and open nothing; unfolded they stand after No line. The
    /// active lines always stand, with their counts.
    #[test]
    fn the_quiet_lines_fold_under_quiet_until_unfolded() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.flows(flow_view());
        assert_eq!(
            m.active_line_rows()
                .iter()
                .map(|r| (r.name.as_str(), r.line.as_str()))
                .collect::<Vec<_>>(),
            [("yardr", "2 in transit")]
        );
        assert_eq!(
            m.quiet_rows()
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["footplate"]
        );
        assert!(!m.quiet_expanded());
        assert!(!m.has_row(&Selected::Line("footplate".into())));
        assert_eq!(m.select(Selected::Line("footplate".into())), []);
        m.toggle_quiet();
        assert!(m.quiet_expanded());
        assert!(m.has_row(&Selected::Line("footplate".into())));
        m.select(Selected::Line("footplate".into()));
        assert_eq!(m.selected(), Some(&Selected::Line("footplate".into())));
    }

    /// Returning to a yard with Quiet unfolded keeps a quiet line's
    /// row; folded it falls back to the board, as Decided does.
    #[test]
    fn a_quiet_line_kept_stands_where_quiet_is_unfolded() {
        let mut state = yard(&[]);
        state.flows = flow_view();
        let (model, _) = Model::new_returning(
            state.clone(),
            Mode::Timetable,
            Some(Selected::Line("footplate".into())),
            false,
            true,
            false,
        );
        assert_eq!(model.selected(), Some(&Selected::Line("footplate".into())));
        let (model, _) = Model::new_returning(
            state,
            Mode::Timetable,
            Some(Selected::Line("footplate".into())),
            false,
            false,
            false,
        );
        assert_eq!(model.selected(), Some(&Selected::Arrivals));
    }

    /// The arrivals pane opens the panel: departures stands only where
    /// the head's toggle sets it, and a yard come back to keeps which
    /// stood, as Quiet's fold is kept.
    #[test]
    fn the_arrivals_pane_opens_the_panel_until_departures_is_set() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert!(!m.departures());
        m.set_departures(true);
        assert!(m.departures());
        m.set_departures(false);
        assert!(!m.departures());
        let (model, _) = Model::new_returning(
            yard(&[]),
            Mode::Timetable,
            Some(Selected::Arrivals),
            false,
            false,
            true,
        );
        assert!(model.departures());
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.back_to(Mode::Timetable, None, false, false, true);
        assert!(m.departures());
    }

    /// The panel's summary counts the Flows beads: how many are being
    /// worked, how many wait on the reader, and how many are held or
    /// stalled, from the stages, Elsewhere and No line alike. A flow's
    /// backlog count is its `backlog` stages' beads, wherever among
    /// them they stand.
    #[test]
    fn the_panel_counts_what_is_worked_held_and_waiting() {
        use crate::api::{Flow, FlowStage};
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        let mut flows = flow_view();
        flows.depots[0].flows[0].stages[0].beads[0].working = true;
        flows.depots[0].elsewhere[0].remark = "waits for jo".into();
        flows.depots[0].elsewhere[0].tone = "amber".into();
        flows.unassigned[0].tone = "red".into();
        m.flows(flows);
        assert_eq!(m.working_count(), 1);
        assert_eq!(m.held_count(), 2);
        assert_eq!(
            m.waiting_on_you()
                .iter()
                .map(|(bead, _)| *bead)
                .collect::<Vec<_>>(),
            ["yardr-pinned"]
        );
        let flow = Flow {
            name: "default".into(),
            flow_type: "".into(),
            source: "store".into(),
            landings: vec![],
            stages: vec![
                FlowStage {
                    stage: "backlog".into(),
                    beads: vec![
                        flowed("yd-a", "A waits shaping", "—", "", "none"),
                        flowed("yd-b", "B waits shaping", "—", "", "none"),
                    ],
                    ..FlowStage::default()
                },
                FlowStage {
                    stage: "new".into(),
                    beads: vec![flowed("yd-c", "C moves", "1h–2h", "", "none")],
                    ..FlowStage::default()
                },
            ],
        };
        assert_eq!(backlog_count(&flow), 2);
        assert_eq!(
            backlog_count(&Flow {
                stages: vec![],
                ..Flow::default()
            }),
            0
        );
    }

    /// The panel's worked beads split by served kind, most first with
    /// ties by name, and list the groups working them with their
    /// stages' seats. A worked bead with no kind counts in the total
    /// alone; a working group no current stage names is listed with no
    /// seat count; an unworked bead is in neither.
    #[test]
    fn the_panel_s_working_split_counts_kinds_and_groups() {
        use crate::api::FlowBead;
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        let mut flows = flow_view();
        let worked = |bead: &str, group: &str, kind: &str| FlowBead {
            bead: bead.into(),
            title: bead.into(),
            working: true,
            worker: format!("herdr {bead}"),
            group: group.into(),
            kind: kind.into(),
            ..FlowBead::default()
        };
        let stage = &mut flows.depots[0].flows[0].stages[0];
        stage.limit = Some(5);
        stage.taken = Some(3);
        stage.beads = vec![
            worked("yd-one", "muse-builders", "muse"),
            worked("yd-two", "muse-builders", "muse"),
            worked("yd-three", "muse-builders", "muse"),
            worked("yd-four", "opus-builders", "claude"),
            worked("yd-five", "opus-builders", "claude"),
        ];
        let stage = &mut flows.depots[0].flows[0].stages[1];
        stage.group = "opus-builders".into();
        stage.limit = Some(2);
        stage.taken = Some(2);
        stage.beads.push(worked("yd-six", "footplate-assembly", ""));
        stage.beads.push(worked("yd-seven", "old-builders", "muse"));
        stage
            .beads
            .push(flowed("yd-idle", "Idle", "1h–2h", "", "none"));
        m.flows(flows);
        assert_eq!(m.working_count(), 7);
        let split = m.working_split();
        assert_eq!(
            split.kinds,
            [("muse".to_string(), 4), ("claude".to_string(), 2),]
        );
        assert_eq!(
            split
                .groups
                .iter()
                .map(|g| (g.name.as_str(), g.limit, g.taken))
                .collect::<Vec<_>>(),
            [
                ("muse-builders", Some(5), Some(3)),
                ("opus-builders", Some(2), Some(2)),
                ("footplate-assembly", None, None),
                ("old-builders", None, None),
            ]
        );
    }

    /// Tied kinds and groups split by name: two of each, where the
    /// count alone would leave the order to the serve.
    #[test]
    fn the_working_split_breaks_ties_by_name() {
        use crate::api::FlowBead;
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        let mut flows = flow_view();
        let worked = |bead: &str, group: &str, kind: &str| FlowBead {
            bead: bead.into(),
            title: bead.into(),
            working: true,
            worker: format!("herdr {bead}"),
            group: group.into(),
            kind: kind.into(),
            ..FlowBead::default()
        };
        flows.depots[0].flows[0].stages[0].beads = vec![
            worked("yd-one", "zebra-builders", "muse"),
            worked("yd-two", "apple-builders", "claude"),
        ];
        m.flows(flows);
        let split = m.working_split();
        assert_eq!(
            split.kinds,
            [("claude".to_string(), 1), ("muse".to_string(), 1),]
        );
        assert_eq!(
            split
                .groups
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>(),
            ["apple-builders", "zebra-builders"]
        );
    }

    /// The banner keeps visible arrivals in their served order, then
    /// includes waits beyond the arrivals cap from stages, Elsewhere
    /// and No line, once per bead. A wait for several counts only with
    /// an open ask on that bead, so `yd-f` stands out until asked.
    #[test]
    fn the_banner_s_beads_are_the_board_s_rows_that_wait_on_you() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        let mut flows = flow_view();
        flows.depots[0].flows[0].stages[0].beads.push(flowed(
            "yd-a",
            "A waits",
            "1h–2h",
            "waits for jo",
            "amber",
        ));
        flows.depots[0].flows[0].stages[0].beads.push(flowed(
            "yd-b",
            "Yardmaster waits",
            "1h–2h",
            "waits for yardmaster",
            "amber",
        ));
        flows.depots[0].flows[0].stages[1].beads.push(flowed(
            "yd-f",
            "Both wait",
            "1h–2h",
            "waits for yardmaster, jo",
            "amber",
        ));
        flows.depots[0].elsewhere.push(flowed(
            "yd-e",
            "Elsewhere waits",
            "1h–2h",
            "waits for jo",
            "amber",
        ));
        flows.unassigned.push(flowed(
            "yd-u",
            "No line waits",
            "1h–2h",
            "waits for jo",
            "amber",
        ));
        m.flows(flows);
        m.arrivals(board(vec![
            arrival("yd-a", "new", "yardr", "1h–2h", "waits for jo", "amber"),
            arrival(
                "yd-b",
                "new",
                "yardr",
                "1h–2h",
                "waits for yardmaster",
                "amber",
            ),
            arrival("yd-c", "new", "yardr", "—", "held", "red"),
            arrival("yd-d", "new", "yardr", "1h–2h", "late", "amber"),
            arrival("yd-e", "new", "yardr", "1h–2h", "", "none"),
            arrival(
                "yd-f",
                "new",
                "yardr",
                "1h–2h",
                "waits for yardmaster, jo",
                "amber",
            ),
        ]));
        assert_eq!(
            m.waiting_on_you()
                .iter()
                .map(|(bead, _)| *bead)
                .collect::<Vec<_>>(),
            ["yd-a", "yd-e", "yd-u"]
        );
        // With an open ask on it, the wait for several counts too.
        m.asks(crate::api::Asks {
            open: vec![open_ask(
                "yd-f",
                "Both wait",
                "",
                3,
                "Which first?",
                "yardmaster",
                "2026-10-10T10:00:00Z",
            )],
            decided: Vec::new(),
        });
        assert_eq!(
            m.waiting_on_you()
                .iter()
                .map(|(bead, _)| *bead)
                .collect::<Vec<_>>(),
            ["yd-a", "yd-f", "yd-e", "yd-u"]
        );
    }

    /// A bead waits on the reader where its remark names them alone, or
    /// where they have an open ask on it: a wait for several without an
    /// ask counts nothing, with one it counts, as does an ask on a bead
    /// whose remark waits on no one.
    #[test]
    fn a_bead_waits_on_you_alone_or_asked() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.arrivals(board(vec![
            arrival("yd-alone", "new", "yardr", "1h–2h", "waits for jo", "amber"),
            arrival(
                "yd-multi",
                "new",
                "yardr",
                "1h–2h",
                "waits for yardmaster, jo",
                "amber",
            ),
            arrival(
                "yd-other",
                "new",
                "yardr",
                "1h–2h",
                "waits for yardmaster",
                "amber",
            ),
            arrival("yd-plain", "new", "yardr", "1h–2h", "", "none"),
        ]));
        let waiting = m
            .waiting_on_you()
            .iter()
            .map(|(bead, _)| (*bead).to_string())
            .collect::<Vec<_>>();
        assert_eq!(waiting, ["yd-alone"]);
        m.asks(crate::api::Asks {
            open: vec![
                open_ask(
                    "yd-multi",
                    "Both wait",
                    "",
                    3,
                    "Which first?",
                    "yardmaster",
                    "2026-10-10T10:00:00Z",
                ),
                open_ask(
                    "yd-plain",
                    "Plain moves",
                    "",
                    4,
                    "Go now?",
                    "yardmaster",
                    "2026-10-10T11:00:00Z",
                ),
            ],
            decided: Vec::new(),
        });
        let waiting = m
            .waiting_on_you()
            .iter()
            .map(|(bead, _)| (*bead).to_string())
            .collect::<Vec<_>>();
        assert_eq!(waiting, ["yd-alone", "yd-multi", "yd-plain"]);
    }

    /// A bead the board no longer serves — beyond its cap — is detailed
    /// from the flows view's row, with its line; `no line` for a bead
    /// with none, on the board or under No line.
    #[test]
    fn a_bead_beyond_the_board_is_detailed_from_the_flows_view() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.flows(flow_view());
        m.bead_loaded(arrival_bead());
        // Not on the board: the flows view's row, with its line.
        let detail = m.arrival_detail("yd-zulu").unwrap();
        assert_eq!(detail.line, "yardr");
        assert_eq!(detail.expected, "5h–8h");
        assert_eq!(detail.remark, "late");
        assert_eq!(detail.tone, "amber");
        // Under No line: `no line`.
        m.bead_loaded(Bead {
            id: "yd-shed".into(),
            ..Bead::default()
        });
        let detail = m.arrival_detail("yd-shed").unwrap();
        assert_eq!(detail.line, "no line");
        assert_eq!(detail.expected, "2h–3h");
        // On the board with no depot: `no line` too.
        m.arrivals(board(vec![arrival(
            "yd-stray", "new", "", "1h–2h", "", "none",
        )]));
        m.bead_loaded(Bead {
            id: "yd-stray".into(),
            ..Bead::default()
        });
        assert_eq!(m.arrival_detail("yd-stray").unwrap().line, "no line");
    }

    fn arrival_bead() -> Bead {
        Bead {
            id: "yd-zulu".into(),
            title: "Zulu's late turn".into(),
            body: "The turn the yard waits on.".into(),
            stage: "draft".into(),
            labels: vec!["size:medium".into(), "eyes".into()],
            notes: vec![
                BeadNote {
                    seq: 5,
                    kind: "note".into(),
                    author: "yardmaster".into(),
                    text: "still late".into(),
                    at: "2026-10-10T11:30:00Z".into(),
                    files: vec![],
                },
                BeadNote {
                    seq: 3,
                    kind: "note".into(),
                    author: "assembly".into(),
                    text: "running late".into(),
                    at: "2026-10-10T10:30:00Z".into(),
                    files: vec![],
                },
            ],
            asks: vec![
                BeadAsk {
                    ask: 7,
                    asker: "yardmaster".into(),
                    question: "Wait for it?".into(),
                    state: "open".into(),
                    ..BeadAsk::default()
                },
                BeadAsk {
                    ask: 8,
                    asker: "assembly".into(),
                    question: "Land it now?".into(),
                    state: "open".into(),
                    ..BeadAsk::default()
                },
                BeadAsk {
                    ask: 6,
                    asker: "yardmaster".into(),
                    question: "Shape it?".into(),
                    state: "answered".into(),
                    ..BeadAsk::default()
                },
            ],
        }
    }

    /// An arrival's bead is what the Bead call returned — title, stage,
    /// labels, body, notes newest last with their author and time, and
    /// how many asks are open — with the line, expected arrival and
    /// remark of the row it came from. A bead the yard serves no longer
    /// shows what the call returned with its stage, and the row's parts
    /// are empty.
    #[test]
    fn an_arrival_s_bead_is_what_the_bead_call_returned_with_its_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.arrivals(board(vec![arrival(
            "yd-zulu", "draft", "yardr", "5h–8h", "late", "amber",
        )]));
        // Nothing until the bead is read.
        assert_eq!(m.arrival_detail("yd-zulu"), None);
        m.bead_loaded(arrival_bead());
        let detail = m.arrival_detail("yd-zulu").unwrap();
        assert_eq!(detail.bead, "yd-zulu");
        assert_eq!(detail.title, "Zulu's late turn");
        assert_eq!(detail.stage, "draft");
        assert_eq!(detail.line, "yardr");
        assert_eq!(detail.expected, "5h–8h");
        assert_eq!(detail.remark, "late");
        assert_eq!(detail.tone, "amber");
        assert_eq!(detail.labels, ["size:medium", "eyes"]);
        assert_eq!(detail.body, "The turn the yard waits on.");
        // The notes newest last, with their author and time.
        assert_eq!(
            detail
                .notes
                .iter()
                .map(|n| (n.author.as_str(), n.at.as_str(), n.text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("assembly", "2026-10-10T10:30:00Z", "running late"),
                ("yardmaster", "2026-10-10T11:30:00Z", "still late"),
            ]
        );
        // Two open asks; the answered one is not counted.
        assert_eq!(detail.open_asks, 2);
        // The yard serves the bead no longer: what the call returned
        // with its stage, and the row's parts empty.
        m.arrivals(Arrivals::default());
        let detail = m.arrival_detail("yd-zulu").unwrap();
        assert_eq!(detail.title, "Zulu's late turn");
        assert_eq!(detail.stage, "draft");
        assert_eq!(detail.line, "");
        assert_eq!(detail.expected, "");
        assert_eq!(detail.remark, "");
        assert_eq!(detail.body, "The turn the yard waits on.");
        assert_eq!(detail.held_by, "");
    }

    /// An arrival's bead carries who holds it from the row it came
    /// from: the board's row where the yard serves the bead still,
    /// else the flows view's, and none where neither serves it.
    #[test]
    fn an_arrival_s_bead_carries_who_holds_it_from_its_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        let mut held = arrival("yd-held", "new", "yardr", "—", "held by jo", "red");
        held.held_by = "jo".into();
        m.arrivals(board(vec![held]));
        m.bead_loaded(Bead {
            id: "yd-held".into(),
            ..Bead::default()
        });
        assert_eq!(m.arrival_detail("yd-held").unwrap().held_by, "jo");
        // The flows view's row where the board serves the bead no
        // longer, and none where neither serves it.
        let mut flows = flow_view();
        flows.depots[0].flows[0].stages[1].beads[0].held_by = "ada".into();
        m.flows(flows);
        m.bead_loaded(Bead {
            id: "yd-alpha".into(),
            ..Bead::default()
        });
        assert_eq!(m.arrival_detail("yd-alpha").unwrap().held_by, "ada");
        m.arrivals(Arrivals::default());
        m.flows(Flows::default());
        assert_eq!(m.arrival_detail("yd-held").unwrap().held_by, "");
    }

    /// The open arrival's bead stays open and read while the queue is
    /// read again: it is none of the queue's, and no decision's leaving
    /// moves it.
    #[test]
    fn the_open_arrival_s_bead_stays_open_over_a_queue_read_again() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.arrivals(board(vec![arrival(
            "yd-zulu", "draft", "yardr", "5h–8h", "", "none",
        )]));
        m.bead_loaded(arrival_bead());
        m.select(Selected::Bead("yd-zulu".into()));
        m.asks(Asks::default());
        assert_eq!(m.selected(), Some(&Selected::Bead("yd-zulu".into())));
        assert!(m.arrival_detail("yd-zulu").is_some());
    }

    #[test]
    fn a_crew_read_before_a_status_event_does_not_put_the_older_status_back() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.event(&event(
            25,
            "crew_status",
            "",
            json!({"crew": "assembly", "status": "working", "was": "idle"}),
        ));
        // Read at 24, before the event at 25.
        m.listed(
            24,
            vec![
                member("assembly", "running", "idle"),
                member("yardmaster", "running", "working"),
            ],
        );
        assert_eq!(m.rows()[0].line, "working");
        // Read at 26: the read is the newer.
        m.listed(
            26,
            vec![
                member("assembly", "running", "idle"),
                member("yardmaster", "running", "working"),
            ],
        );
        assert_eq!(m.rows()[0].line, "idle");
    }

    #[test]
    fn when_the_open_row_is_gone_the_first_is_opened() {
        let (mut m, _) = Model::new(yard(&[(6, "assembly")]), Mode::Chat, None);
        let effects = m.listed(21, vec![member("assembly", "running", "idle")]);
        assert_eq!(m.selected_conversation(), Some("assembly"));
        assert_eq!(effects, [page("assembly", 0)]);
        assert_eq!(
            m.conversation("assembly", vec![]),
            [Effect::Ack { through: 6 }]
        );

        let (mut m, _) = Model::new(
            yard(&[]),
            Mode::Chat,
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert_eq!(
            m.event(&event(21, "peer_said", "", json!({"peer": "north"}))),
            [Effect::Peers]
        );
        assert_eq!(m.linked(&[]), [page("assembly", 0)]);
        assert_eq!(names(&m), ["assembly", "yardmaster", "jo"]);
        // One that is still there stays open.
        assert_eq!(m.linked(&[north()]), []);
        assert_eq!(m.selected_conversation(), Some("assembly"));
    }

    /// A linked yard whose `jo` the link trusts as this yard's jo.
    fn airy() -> Peer {
        Peer {
            name: "airy".into(),
            send: vec!["mail".into()],
            receive: vec!["mail".into()],
            said: Some(Said {
                roster: vec![RosterEntry {
                    name: "jo".into(),
                    kind: "user".into(),
                    role: "operator".into(),
                    mail: true,
                }],
            }),
            trust: [("jo".to_string(), "jo".to_string())].into(),
        }
    }

    /// A linked yard whose `ada` the link trusts as this yard's ada.
    fn south() -> Peer {
        Peer {
            name: "south".into(),
            send: vec!["mail".into()],
            receive: vec!["mail".into()],
            trust: [("ada".to_string(), "ada".to_string())].into(),
            ..Peer::default()
        }
    }

    /// A yard with the mayor for its crew, mckean for its user, and the
    /// linked yard `yardr`, whose mckean the link trusts as this yard's.
    fn plantime() -> State {
        State {
            me: "mckean".into(),
            head: 20,
            crew: vec![member("mayor", "running", "idle")],
            peers: vec![Peer {
                name: "yardr".into(),
                send: vec!["mail".into()],
                receive: vec!["mail".into()],
                trust: [("mckean".to_string(), "mckean".to_string())].into(),
                ..Peer::default()
            }],
            cursor: 5,
            ..State::default()
        }
    }

    fn user(name: &str) -> User {
        User {
            name: name.into(),
            ..User::default()
        }
    }

    #[test]
    fn the_users_of_the_yard_have_rows_after_the_linked_yards_and_the_window_s_own_is_theirs() {
        let mut state = yard(&[(6, "ada"), (7, "jo")]);
        state.users = vec![user("ada"), user("jo")];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(
            names(&m),
            [
                "assembly",
                "yardmaster",
                "clerk@north",
                "mckean@north",
                "ada",
                "jo"
            ]
        );
        let rows = m.rows();
        assert_eq!((rows[4].line.as_str(), rows[4].unread), ("person", 1));
        assert_eq!(
            (rows[5].line.as_str(), rows[5].unread),
            ("you, notes to self", 1)
        );

        // Another user is read and written to as a crew member is.
        assert_eq!(
            m.select(Selected::Conversation("ada".into())),
            [page("ada", 0)]
        );
        assert_eq!(m.about(), ["a person of this yard"]);
        assert_eq!(m.target(), Some(Target::Crew("ada".into())));
        assert!(m.can_write("ada"));
        assert_eq!(
            m.conversation("ada", vec![letter(6, "ada", "jo", "hello")]),
            [Effect::Ack { through: 6 }]
        );
        assert_eq!(m.sent("ada"), [read("ada", 6)]);
        m.event(&event(
            21,
            "mail",
            "jo",
            json!({"to": "ada", "to_user": true, "text": "hello ada"}),
        ));
        let entries = m.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[1].mine, entries[1].standing.as_deref()),
            (true, Some("sent"))
        );
        m.event(&event(
            22,
            "mail_read",
            "ada",
            json!({"reader": "ada", "through": 21}),
        ));
        assert_eq!(m.entries()[1].standing.as_deref(), Some("read"));
    }

    #[test]
    fn a_user_a_crew_member_has_the_name_of_has_no_second_row() {
        let mut state = yard(&[]);
        state.users = vec![user("assembly"), user("jo")];
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(
            names(&m),
            [
                "assembly",
                "yardmaster",
                "clerk@north",
                "mckean@north",
                "jo"
            ]
        );
    }

    #[test]
    fn mail_the_user_wrote_themselves_is_read_in_their_own_row_and_holds_nothing_back_after() {
        // footplate-evk8: two mails of the user to themselves, one written
        // as the name a linked yard is trusted for, were the oldest unread
        // and had no row. They were never shown, so the twenty of the
        // yardmaster after them, all read in the open conversation, stayed
        // unread in the yard.
        let mut unread = vec![(6, "jo@airy"), (7, "jo")];
        unread.extend((8..28).map(|seq| (seq, "yardmaster")));
        let mut state = yard(&unread);
        state.peers.push(airy());
        let (mut m, effects) = Model::new(state, Mode::Chat, None);
        // The yardmaster's mail is shown: no row counts it, and nothing is
        // acknowledged, since 6 and 7 are not seen.
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
        assert_eq!(effects, [page("yardmaster", 0)]);
        assert_eq!(m.conversation("yardmaster", vec![]), []);
        assert_eq!(m.unread("yardmaster"), 0);
        // What is unseen has a row that says so: the user's own. The name
        // trusted as them has no row of its own.
        assert_eq!(
            names(&m),
            [
                "assembly",
                "yardmaster",
                "clerk@north",
                "mckean@north",
                "jo"
            ]
        );
        assert_eq!(m.unread("jo"), 2);
        assert_eq!(m.yard_unread(), 0);

        // Opening it shows both and acknowledges all there is.
        assert_eq!(
            m.select(Selected::Conversation("jo".into())),
            [page("jo", 0), page("jo@airy", 0)]
        );
        assert_eq!(
            m.conversation("jo", vec![letter(7, "jo", "jo", "works")]),
            []
        );
        assert_eq!(
            m.conversation(
                "jo@airy",
                vec![letter(6, "jo@airy", "jo", "testing messaging myself")],
            ),
            [Effect::Ack { through: 27 }]
        );
        assert_eq!(
            m.entries()
                .iter()
                .map(|e| (e.seq, e.from.as_str(), e.mine))
                .collect::<Vec<_>>(),
            [(6, "jo@airy", false), (7, "jo", true)]
        );
        m.acked(27);
        assert_eq!(m.rows().iter().map(|r| r.unread).sum::<usize>(), 0);
        assert_eq!(m.yard_unread(), 0);
    }

    #[test]
    fn removing_a_user_refreshes_trust_grants() {
        let mut state = yard(&[]);
        state.peers.push(airy());
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(
            m.event(&event(21, "user_removed", "jo", json!({"user": "ada"}))),
            [Effect::Peers]
        );
    }

    #[test]
    fn where_an_answer_goes_is_not_known_before_all_of_a_user_s_row_is_read() {
        let mut state = yard(&[]);
        state.peers.push(airy());
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        // A row with one party goes one way whatever its mail says.
        assert!(!m.reading());
        m.select(Selected::Conversation("jo".into()));
        assert!(m.reading());
        // The user's own mail is there, the trusted name's not yet: it
        // may be the newest.
        m.conversation("jo", vec![letter(7, "jo", "jo", "works")]);
        assert!(m.reading());
        m.conversation("jo@airy", vec![letter(8, "jo@airy", "jo", "from airy")]);
        assert!(!m.reading());
        assert_eq!(
            m.target(),
            Some(Target::Peer {
                name: "jo".into(),
                peer: "airy".into()
            })
        );
    }

    #[test]
    fn in_a_user_s_row_what_is_typed_answers_whoever_wrote_last() {
        let mut state = yard(&[]);
        state.peers.push(airy());
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.select(Selected::Conversation("jo".into()));
        // Nobody wrote: it is mail to the user in this yard.
        assert_eq!(m.target(), Some(Target::Crew("jo".into())));
        assert_eq!(
            m.about(),
            [
                "you: what you write here is mail to yourself, also trusted as jo@airy: mail from that name is in this conversation, and what you write goes to jo in this yard"
            ]
        );
        // The trusted name wrote last: the answer goes to it at its yard.
        m.event(&mail_to_jo(21, "jo@airy"));
        assert_eq!(
            m.target(),
            Some(Target::Peer {
                name: "jo".into(),
                peer: "airy".into()
            })
        );
        assert!(m.about()[0].ends_with("what you write goes to jo@airy, who wrote last"));
        // What was sent there is read from there, and stands in this row.
        assert_eq!(m.sent("jo"), [page("jo", 0), page("jo@airy", 0)]);
        m.event(&event(
            22,
            "mail",
            "jo",
            json!({"to": "jo", "to_peer": "airy", "text": "over there"}),
        ));
        assert_eq!(
            m.entries().iter().map(|e| e.seq).collect::<Vec<_>>(),
            [21, 22]
        );
        // The user's own note to themselves changes nothing of that.
        m.event(&mail_to_jo(23, "jo"));
        assert!(matches!(m.target(), Some(Target::Peer { .. })));
        // An asker of that name is written to in the user's row.
        assert_eq!(m.conversation_with("jo@airy"), Some("jo"));
        assert_eq!(m.unwritable("jo@airy"), None);
    }

    #[test]
    fn the_user_s_word_from_a_linked_yard_in_a_crew_member_s_row_does_not_send_the_answer_there() {
        // footplate-zgtr: the mayor's row held a mail the user wrote from
        // the linked yard, under the name trusted as them, and what was
        // typed went to that name: mail to themselves, the mayor got none.
        let (mut m, _) = Model::new(
            plantime(),
            Mode::Chat,
            Some(Selected::Conversation("mayor".into())),
        );
        m.conversation("mayor", vec![letter(21, "mckean@yardr", "mayor", "hi")]);
        assert_eq!(m.target(), Some(Target::Crew("mayor".into())));

        // Nor does the mayor's own word, written after it, change that.
        m.event(&event(
            22,
            "mail",
            "mayor",
            json!({"to": "mckean", "to_user": true, "text": "hello"}),
        ));
        assert_eq!(m.target(), Some(Target::Crew("mayor".into())));
    }

    #[test]
    fn only_a_name_trusted_as_the_row_s_user_sends_the_answer_to_a_linked_yard() {
        let mut state = plantime();
        state.users = vec![user("ada"), user("mckean")];
        state.peers.push(south());
        let (mut m, _) = Model::new(
            state,
            Mode::Chat,
            Some(Selected::Conversation("mayor".into())),
        );
        // A name trusted as another user wrote last in the mayor's row, and
        // so did one trusted as nobody: an `@` in the sender is no reason
        // to answer there.
        m.conversation(
            "mayor",
            vec![
                letter(21, "ada@south", "mckean", "hi"),
                letter(22, "zed@south", "mckean", "hello"),
            ],
        );
        assert_eq!(m.target(), Some(Target::Crew("mayor".into())));
    }

    #[test]
    fn the_user_s_word_from_a_linked_yard_stands_as_their_own_in_another_row_and_not_in_theirs() {
        let (mut m, _) = Model::new(
            plantime(),
            Mode::Chat,
            Some(Selected::Conversation("mayor".into())),
        );
        m.conversation(
            "mayor",
            vec![
                letter(21, "mayor", "mckean", "Shall I plan the week?"),
                letter(22, "mckean@yardr", "mayor", "hi"),
            ],
        );
        // Under the name it was written by, drawn as theirs, with where it
        // stands: the mayor has not read it.
        assert_eq!(
            m.entries()
                .iter()
                .map(|e| (e.from.as_str(), e.mine, e.standing.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("mayor", false, None),
                ("mckean@yardr", true, Some("sent; mayor is idle"))
            ]
        );

        // In their own row the same name is the other end: mail to them,
        // as it is read.
        m.select(Selected::Conversation("mckean".into()));
        m.conversation("mckean", vec![letter(23, "mckean", "mckean", "a note")]);
        m.conversation(
            "mckean@yardr",
            vec![letter(24, "mckean@yardr", "mckean", "from there")],
        );
        assert_eq!(
            m.entries()
                .iter()
                .map(|e| (e.from.as_str(), e.mine))
                .collect::<Vec<_>>(),
            [("mckean", true), ("mckean@yardr", false)]
        );
    }

    #[test]
    fn a_name_trusted_as_the_user_is_not_who_wrote_last_in_another_user_s_row() {
        let mut state = yard(&[]);
        state.users = vec![user("ada"), user("jo")];
        state.peers.push(airy());
        state.peers.push(south());
        let (mut m, _) = Model::new(
            state,
            Mode::Chat,
            Some(Selected::Conversation("ada".into())),
        );
        // ada wrote from south, and the user answered from airy: the
        // newest mail in her row is the user's own, so it is skipped as one
        // from the user's name is, and the answer goes where ada wrote.
        m.conversation(
            "ada@south",
            vec![letter(21, "ada@south", "jo", "hello from south")],
        );
        m.conversation("ada", vec![letter(22, "jo@airy", "ada", "hello from airy")]);
        assert_eq!(
            m.target(),
            Some(Target::Peer {
                name: "ada".into(),
                peer: "south".into()
            })
        );
        // Written to in this yard after, ada wrote last there.
        m.event(&mail_to_jo(23, "ada"));
        assert_eq!(m.target(), Some(Target::Crew("ada".into())));
    }

    #[test]
    fn a_note_to_oneself_counts_on_the_own_row_until_it_is_shown_and_is_no_news() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.take_news();
        // Written elsewhere, with the yardmaster's conversation open.
        assert_eq!(m.event(&mail_to_jo(21, "jo")), []);
        assert_eq!(m.unread("jo"), 1);
        assert_eq!(m.take_news(), []);
        assert_eq!(
            m.select(Selected::Conversation("jo".into())),
            [page("jo", 0)]
        );
        assert_eq!(m.conversation("jo", vec![]), [Effect::Ack { through: 21 }]);
        m.acked(21);
        // Written in the open row it is seen as it arrives.
        assert_eq!(
            m.event(&mail_to_jo(22, "jo")),
            [Effect::Ack { through: 22 }]
        );
        assert_eq!(m.unread("jo"), 0);
        let entries = m.entries();
        assert_eq!((entries.len(), entries[0].mine), (2, true));
        // Read, it stands so.
        m.event(&event(
            23,
            "mail_read",
            "jo",
            json!({"reader": "jo", "through": 22}),
        ));
        assert_eq!(m.entries()[1].standing.as_deref(), Some("read"));
    }

    #[test]
    fn mail_of_a_sender_without_a_row_is_shown_and_read_on_the_yard_row() {
        // A crew member's agent at a bead, and one of a linked yard that
        // is not on its roster: neither has a row.
        let (mut m, effects) = Model::new(
            yard(&[
                (6, "bead-agent"),
                (7, "zed@south"),
                (8, "yard"),
                (9, "yardmaster"),
            ]),
            Mode::Chat,
            None,
        );
        assert_eq!(effects, [page("yardmaster", 0)]);
        assert_eq!(m.conversation("yardmaster", vec![]), []);
        assert_eq!(m.yard_row().unread, 3);
        assert!(!m.can_write("bead-agent"));
        assert_eq!(
            m.select(Selected::Yard),
            [read("yard", 0), read("bead-agent", 0), read("zed@south", 0)]
        );
        // Each is acknowledged as the yard answers it, in the inbox's order.
        assert_eq!(
            m.conversation("zed@south", vec![letter(7, "zed@south", "jo", "z")]),
            []
        );
        assert_eq!(
            m.conversation("bead-agent", vec![letter(6, "bead-agent", "jo", "b")]),
            [Effect::Ack { through: 7 }]
        );
        assert_eq!(
            m.conversation("yard", vec![letter(8, "yard", "jo", "y")]),
            [Effect::Ack { through: 9 }]
        );
        assert_eq!(m.yard_row().unread, 0);
        assert_eq!(
            m.yard_mail()
                .iter()
                .map(|e| (e.seq, e.from.as_str()))
                .collect::<Vec<_>>(),
            [(6, "bead-agent"), (7, "zed@south"), (8, "yard")]
        );
        // What arrives there while it is open is seen as it arrives.
        m.acked(9);
        assert_eq!(
            m.event(&mail_to_jo(21, "zed@south")),
            [Effect::Ack { through: 21 }]
        );
        assert_eq!(m.yard_mail().len(), 4);
        // A picture in such mail is asked for as a conversation's is, while
        // the yard row shows it and not elsewhere.
        m.event(&event(
            22,
            "mail",
            "zed@south",
            json!({"to": "jo", "to_user": true, "text": "A picture.", "files": [png("aa")]}),
        ));
        assert_eq!(m.yard_mail().last().unwrap().files[0].hash, "aa");
        assert_eq!(m.images(), ["aa"]);
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(m.images(), Vec::<&str>::new());
    }

    #[test]
    fn mail_from_a_name_of_this_yard_with_no_row_has_the_users_read_again() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        // ada was added after the users were read.
        assert_eq!(m.event(&mail_to_jo(21, "ada")), [Effect::Users]);
        assert_eq!(m.yard_unread(), 1);
        assert_eq!(m.users(vec![user("ada"), user("jo")]), []);
        assert_eq!((m.unread("ada"), m.yard_unread()), (1, 0));
        // The yard's own mail and that of a linked yard ask nothing.
        assert_eq!(m.event(&mail_to_jo(22, "yard")), []);
        assert_eq!(m.event(&mail_to_jo(23, "zed@south")), []);

        // On the open yard row her mail is seen as it arrives; with her
        // row it is hers, and stays seen.
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, Some(Selected::Yard));
        assert_eq!(
            m.event(&mail_to_jo(21, "ada")),
            [Effect::Ack { through: 21 }, Effect::Users]
        );
        m.users(vec![user("ada"), user("jo")]);
        assert_eq!((m.unread("ada"), m.yard_unread()), (0, 0));
    }

    #[test]
    fn a_name_newly_trusted_as_a_user_takes_its_open_row_and_its_mail_to_the_user_s() {
        let mut state = yard(&[(6, "jo@airy")]);
        let mut untrusted = airy();
        untrusted.trust.clear();
        state.peers.push(untrusted);
        let (mut m, _) = Model::new(
            state,
            Mode::Chat,
            Some(Selected::Conversation("jo@airy".into())),
        );
        assert_eq!(m.selected_conversation(), Some("jo@airy"));
        m.conversation("jo@airy", vec![]);
        assert_eq!(m.linked(&[north(), airy()]), [page("jo", 0)]);
        // Its mail was seen in the row it had, and counts on neither.
        assert_eq!(m.unread("jo"), 0);
        assert_eq!(m.selected_conversation(), Some("jo"));
        assert!(!names(&m).contains(&"jo@airy".to_string()));
    }

    #[test]
    fn a_name_trusted_as_a_crew_user_is_folded_into_the_crew_row() {
        let mut state = yard(&[(6, "jo@airy")]);
        state.users.push(user("assembly"));
        let mut peer = airy();
        peer.trust.insert("jo".into(), "assembly".into());
        state.peers.push(peer);

        let (mut m, _) = Model::new(
            state,
            Mode::Chat,
            Some(Selected::Conversation("yardmaster".into())),
        );
        assert!(!names(&m).contains(&"jo@airy".to_string()));
        assert_eq!(m.unread("assembly"), 1);
        assert_eq!(m.unread("jo@airy"), 0);
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [page("assembly", 0), page("jo@airy", 0)]
        );
        assert_eq!(
            m.conversation(
                "jo@airy",
                vec![letter(6, "jo@airy", "jo", "hello from airy")],
            ),
            [Effect::Ack { through: 6 }]
        );
        assert_eq!(
            m.entries()
                .iter()
                .map(|e| e.from.as_str())
                .collect::<Vec<_>>(),
            ["jo@airy"]
        );
        assert_eq!(
            m.target(),
            Some(Target::Peer {
                name: "jo".into(),
                peer: "airy".into()
            })
        );
    }

    #[test]
    fn text_reaches_the_window_as_letters_and_line_breaks() {
        assert_eq!(
            plain("one\ntwo\tthree\r\n\u{1b}[31mred\u{7}"),
            "one\ntwo\tthree\n[31mred"
        );
        // U+2028 and U+2029 go with the control characters: a viewer that
        // breaks a line on them would let a title start a line again.
        assert_eq!(plain("one\u{2028}two\u{2029}three"), "onetwothree");
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.conversation(
            "yardmaster",
            vec![letter(12, "yardmaster", "jo", "a\u{1b}b")],
        );
        assert_eq!(m.entries()[0].text, "ab");
    }

    #[test]
    fn a_mail_s_time_is_shown_in_the_reader_s_zone() {
        let zone = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(clock("2026-10-05T11:20:00Z", &zone), "10-05 13:20");
        assert_eq!(clock("2026-12-31T23:30:00.123456Z", &zone), "01-01 01:30");
        assert_eq!(clock("", &zone), "");
        assert_eq!(clock("yesterday", &zone), "");
        // The time of day alone, for a time that stands under its day.
        assert_eq!(clock_time("2026-10-05T11:20:00Z", &zone), "13:20");
        assert_eq!(clock_time("2026-12-31T23:30:00.123456Z", &zone), "01:30");
        assert_eq!(clock_time("", &zone), "");
        assert_eq!(clock_time("yesterday", &zone), "");
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe rest";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 16];

    fn png(hash: &str) -> Value {
        json!({"hash": hash, "type": "image/png", "bytes": 70})
    }

    fn html(hash: &str) -> Value {
        json!({"hash": hash, "type": "text/html; charset=utf-8", "bytes": 42000})
    }

    #[test]
    fn a_mail_s_images_are_shown_with_it_from_the_yard_and_from_the_stream() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let files: Vec<BlobRef> = serde_json::from_value(json!([png("aa"), png("bb")])).unwrap();
        m.conversation(
            "yardmaster",
            vec![
                Letter {
                    files: files.clone(),
                    ..letter(12, "yardmaster", "jo", "Like this?")
                },
                letter(13, "jo", "yardmaster", "Yes."),
            ],
        );
        // The user's own, with no text, as the stream tells of it; and one
        // to the user that carries an image the conversation has already.
        m.event(&event(
            21,
            "mail",
            "jo",
            json!({"to": "yardmaster", "text": "", "files": [png("cc")]}),
        ));
        m.event(&event(
            22,
            "mail",
            "yardmaster",
            json!({"to": "jo", "to_user": true, "text": "Again.", "files": [png("aa")]}),
        ));
        let entries = m.entries();
        assert_eq!(entries[0].files, files);
        assert_eq!(entries[1].files, []);
        assert_eq!(
            (entries[2].text.as_str(), entries[2].files[0].hash.as_str()),
            ("", "cc")
        );
        assert_eq!(entries[2].files[0].kind, "image/png");
        assert_eq!(m.images(), ["aa", "bb", "cc"]);
        // One this yard did not take from a linked yard is not asked for:
        // the line says the yard's reason.
        let mut refused = png("dd");
        refused["not_taken"] = json!("the link does not take files");
        m.event(&event(
            24,
            "mail",
            "yardmaster",
            json!({"to": "jo", "to_user": true, "text": "And this.", "files": [refused]}),
        ));
        assert_eq!(m.images(), ["aa", "bb", "cc"]);
        assert_eq!(
            not_taken(&m.entries().last().unwrap().files[0]),
            "image, 70 bytes, not taken: the link does not take files"
        );
        // Another conversation has none of them.
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(m.images(), Vec::<&str>::new());
    }

    #[test]
    fn a_page_is_no_image_to_ask_for() {
        assert!(is_page(&BlobRef {
            kind: "text/html; charset=utf-8".into(),
            ..BlobRef::default()
        }));
        assert!(is_page(&BlobRef {
            kind: "text/html".into(),
            ..BlobRef::default()
        }));
        assert!(!is_page(&BlobRef {
            kind: "image/png".into(),
            ..BlobRef::default()
        }));
        assert!(!is_page(&BlobRef::default()));
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let files: Vec<BlobRef> = serde_json::from_value(json!([html("pp"), png("aa")])).unwrap();
        m.conversation(
            "yardmaster",
            vec![Letter {
                files: files.clone(),
                ..letter(12, "yardmaster", "jo", "Like this?")
            }],
        );
        // The mail carries both, and only the image is asked for.
        assert_eq!(m.entries()[0].files, files);
        assert_eq!(m.images(), ["aa"]);
        // One this yard did not take is a page's line, not an image's.
        let mut refused = html("qq");
        refused["not_taken"] = json!("the link does not take files");
        let file: BlobRef = serde_json::from_value(refused).unwrap();
        assert_eq!(
            not_taken(&file),
            "page, 41 KiB, not taken: the link does not take files"
        );
    }

    #[test]
    fn a_file_is_no_image_to_ask_for() {
        let file = |kind: &str| BlobRef {
            kind: kind.into(),
            ..BlobRef::default()
        };
        // An image is a PNG or a JPEG, as the yard told it apart;
        // anything else is a page or a file.
        assert!(is_image(&file("image/png")));
        assert!(is_image(&file("image/jpeg")));
        assert!(!is_image(&file("text/html; charset=utf-8")));
        assert!(!is_image(&file("application/pdf")));
        assert!(!is_image(&file("image/svg+xml")));
        assert!(!is_image(&file("")));
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let files: Vec<BlobRef> = serde_json::from_value(json!([
            {"hash": "ff", "type": "application/pdf", "bytes": 900, "name": "report.pdf"},
            png("aa"),
        ]))
        .unwrap();
        m.conversation(
            "yardmaster",
            vec![Letter {
                files: files.clone(),
                ..letter(12, "yardmaster", "jo", "Like this?")
            }],
        );
        // The mail carries both, and only the image is asked for: the
        // file opens in the browser, from the bytes the yard holds.
        assert_eq!(m.entries()[0].files, files);
        assert_eq!(m.images(), ["aa"]);
        // One this yard did not take is a file's line.
        let mut refused = json!({"hash": "rr", "type": "application/pdf", "bytes": 900});
        refused["not_taken"] = json!("the link does not take files");
        let file: BlobRef = serde_json::from_value(refused).unwrap();
        assert_eq!(
            not_taken(&file),
            "file, 900 bytes, not taken: the link does not take files"
        );
        // A delivery's file is told apart by its name, where it has one,
        // and its name and reason can carry the source's words: text, and
        // no control character of a terminal's.
        let file = BlobRef {
            bytes: 9437184,
            name: "shed\u{1b}[31m-plans.pdf".into(),
            not_taken: "document not fetched: \u{1b}[31mdenied\u{0}".into(),
            ..BlobRef::default()
        };
        assert_eq!(
            not_taken(&file),
            "shed[31m-plans.pdf, 9.0 MiB, not taken: document not fetched: [31mdenied"
        );
    }

    #[test]
    fn the_images_of_an_expanded_card_s_notes_are_asked_for() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        state.asks.decided = vec![decided_ask(
            "yardr-n8ta",
            3,
            "Move this yard's 64 links to the declared source now?",
            "answered",
            "Move them now",
            "tonight",
            "",
        )];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let mut bead = links_bead();
        bead.notes[0].files = vec![
            BlobRef {
                hash: "aa".into(),
                kind: "image/png".into(),
                ..BlobRef::default()
            },
            BlobRef {
                hash: "bb".into(),
                kind: "image/jpeg".into(),
                ..BlobRef::default()
            },
        ];
        m.bead_loaded(bead);
        m.select(Selected::Decision("yardr-n8ta".into()));
        // The card stands folded: its notes' images are none shown.
        assert_eq!(m.images(), Vec::<&str>::new());
        m.toggle_expand("yardr-n8ta", 3);
        assert_eq!(m.images(), ["aa", "bb"]);
        // The card carries the notes with their images.
        let detail = m.decision_detail("yardr-n8ta", &noon()).unwrap();
        assert_eq!(detail.notes[0].files.len(), 2);
        // Folded again, they are asked for no longer.
        m.toggle_expand("yardr-n8ta", 3);
        assert_eq!(m.images(), Vec::<&str>::new());
        // A dealt-with card's notes the same way.
        m.toggle_decided();
        m.select(Selected::Decided("yardr-n8ta".into(), 3));
        assert_eq!(m.images(), Vec::<&str>::new());
        m.toggle_expand("yardr-n8ta", 3);
        assert_eq!(m.images(), ["aa", "bb"]);
        let detail = m.decided_detail("yardr-n8ta", 3, &chrono::Utc).unwrap();
        assert_eq!(detail.notes[0].files.len(), 2);
    }

    #[test]
    fn an_image_is_a_png_or_a_jpeg_by_its_bytes() {
        assert_eq!(image_type(PNG), Some("image/png"));
        assert_eq!(image_type(JPEG), Some("image/jpeg"));
        assert_eq!(image_type(b"GIF89a"), None);
        assert_eq!(image_type(b"II*\0"), None);
        assert_eq!(image_type(b""), None);
    }

    const PAGE: &str = "text/html; charset=utf-8";

    #[test]
    fn a_page_is_html_by_its_bytes_as_the_yard_reads_it() {
        // What http.DetectContentType answers: the tags, the case aside,
        // past the leading whitespace, ended by a space or `>`.
        for head in [
            "<!DOCTYPE HTML>",
            "<!doctype html>",
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\">",
            "<html>",
            "<HTML LANG=\"en\">",
            "<head>",
            "<Script>",
            "<!-- a note -->",
            "  \r\n<html>",
        ] {
            assert_eq!(page_type(head.as_bytes()), Some(PAGE), "{head}");
        }
        // Anything else is no page: images, text, SVG (which the yard
        // reads as text), a tag cut short or unterminated, and nothing.
        for head in [
            b"\x89PNG\r\n\x1a\nthe rest".as_slice(),
            b"GIF89a".as_slice(),
            b"just words".as_slice(),
            b"<svg width=\"8\">".as_slice(),
            b"<blink>".as_slice(),
            b"<htm".as_slice(),
            b"<html".as_slice(),
            b"".as_slice(),
            b"   \t\r\n".as_slice(),
        ] {
            assert_eq!(page_type(head), None, "{head:?}");
        }
    }

    #[test]
    fn a_page_over_the_yard_s_cap_is_refused_with_both_sizes() {
        assert_eq!(page_over_cap(70, 8388608), None);
        assert_eq!(
            page_over_cap(9 * 1024 * 1024, 8388608).as_deref(),
            Some(
                "the page is 9.0 MiB, and this yard takes one of 8.0 MiB at most (yardr settings: blob_max_bytes)"
            )
        );
    }

    #[test]
    fn an_old_yard_s_add_note_has_no_files() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        assert!(note_files_too_old(&refused(
            "argument \"files\": AddNote has none of that name (it takes bead, text, actor)"
        )));
        assert!(!note_files_too_old(&refused("no such method AddNote")));
        assert!(!note_files_too_old(&refused(
            "a note with images needs words: say what the picture shows"
        )));
        assert!(!note_files_too_old(&api::Error::Protocol(
            "AddNote: status 200, and no JSON object".into()
        )));
    }

    /// A yard from before the Lines view has no Arrivals or Flows call:
    /// its answer is no error, so no notice is raised, and the Lines
    /// pane says the yard is too old instead. Anything else those calls
    /// fail with is said as a notice, as ever. A board or a view read
    /// again clears its half: the yard has the call.
    #[test]
    fn an_old_yard_s_lines_are_said_in_the_lines_pane_not_as_a_notice() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        assert!(lines_too_old(&refused("no such method Arrivals")));
        assert!(lines_too_old(&refused("no such method Flows")));
        assert!(!lines_too_old(&refused("the board is closed")));
        assert!(!lines_too_old(&api::Error::Protocol(
            "Arrivals: status 200, and no JSON object".into()
        )));
        // Plain words, and never the wire's method names.
        assert!(LINES_TOO_OLD.contains("older than the Lines view"));
        assert!(!LINES_TOO_OLD.contains("no such method"));
        assert!(!LINES_TOO_OLD.contains("Arrivals"));
        assert!(!LINES_TOO_OLD.contains("Flows"));

        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.arrivals(Arrivals {
            rows: vec![api::Arrival {
                bead: "yd-zulu".into(),
                ..Default::default()
            }],
        });
        let _ = m.flows(Flows {
            travelling: 1,
            depots: vec![api::FlowDepot {
                name: "yardr".into(),
                travelling: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
        m.arrivals_failed(&refused("no such method Arrivals"));
        assert!(m.arrivals_too_old());
        assert!(m.arrival_rows().is_empty());
        assert!(!m.flows_too_old());
        assert!(m.notice.is_empty(), "{}", m.notice);
        let _ = m.flows_failed(&refused("no such method Flows"));
        assert!(m.flows_too_old());
        assert_eq!(m.travelling(), 0);
        assert!(m.depots().is_empty());
        assert!(m.notice.is_empty(), "{}", m.notice);

        // A board read again clears its half alone.
        m.arrivals(Arrivals::default());
        assert!(!m.arrivals_too_old());
        assert!(m.flows_too_old());
        let _ = m.flows(Flows::default());
        assert!(!m.flows_too_old());

        // Any other failure is a notice, as ever, and not the Lines pane's.
        for err in [
            refused("the board is closed"),
            api::Error::Protocol("Arrivals: status 200, and no JSON object".into()),
            api::Error::Down(std::io::Error::other("the yard went away")),
        ] {
            m.arrivals_failed(&err);
            assert!(!m.arrivals_too_old());
            assert_eq!(m.notice, format!("the arrivals could not be read: {err}"));
            m.notice.clear();
        }
        for err in [
            refused("the view is closed"),
            api::Error::Protocol("Flows: status 200, and no JSON object".into()),
            api::Error::Down(std::io::Error::other("the yard went away")),
        ] {
            let _ = m.flows_failed(&err);
            assert!(!m.flows_too_old());
            assert_eq!(m.notice, format!("the flows could not be read: {err}"));
            m.notice.clear();
        }
    }

    /// A yard from before the grants view has no SourceGrants call: its
    /// answer is no error, so no notice is raised, and the grants pane
    /// says the yard is too old instead. A yard that says the request came
    /// down the link from its edge serves the list on its machine alone:
    /// no notice either, and the pane says where they are read. Anything
    /// else the call fails with is said as a notice, as ever. Grants read
    /// again clear both.
    #[test]
    fn a_yard_that_gives_no_grants_says_so_in_the_grants_pane_not_as_a_notice() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        // The yard's own words, as it refuses the list down the link and as
        // it refuses it in plain HTTP from another machine: the second is a
        // fault of the address, and stays a notice.
        let link = refused(
            "the operator's grant list is not read over the link: this request came down the link from the edge. Read it on the yard's machine (`yardr source grants`). The edge saw the device's token with it: remove the device and add it again (yardr user device rm).",
        );
        let plain = refused(
            "the operator's grant list is taken over https only, or from the yard's own machine: this request reached the yard's listener in plain HTTP from another machine. Put the proxy or tunnel that ends TLS on the yard's machine, or Read it on the yard's machine (`yardr source grants`).",
        );
        assert!(grants_too_old(&refused("no such method SourceGrants")));
        assert!(!grants_too_old(&refused("the grants are closed")));
        assert!(!grants_too_old(&api::Error::Protocol(
            "SourceGrants: status 200, and no JSON object".into()
        )));
        assert!(grants_over_the_link(&link));
        assert!(!grants_over_the_link(&plain));
        assert!(!grants_over_the_link(&refused(
            "no such method SourceGrants"
        )));
        assert!(!grants_too_old(&link));
        // Plain words, and never the wire's method name.
        assert!(GRANTS_TOO_OLD.contains("older than the grants view"));
        assert!(!GRANTS_TOO_OLD.contains("no such method"));
        assert!(!GRANTS_TOO_OLD.contains("SourceGrants"));
        assert!(GRANTS_ON_THE_YARD.contains("read on the yard's machine"));
        assert!(!GRANTS_ON_THE_YARD.contains("SourceGrants"));

        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.grants(vec![SourceGrant {
            source: "crm".into(),
            ..Default::default()
        }]);
        assert!(!m.grants_too_old());
        assert!(!m.grants_on_the_yard());
        assert_eq!(m.grant_records().len(), 1);
        m.grants_failed(&refused("no such method SourceGrants"));
        assert!(m.grants_too_old());
        assert!(!m.grants_on_the_yard());
        assert!(m.grant_records().is_empty());
        assert!(m.notice.is_empty(), "{}", m.notice);

        // Grants read again clear it.
        m.grants(Vec::new());
        assert!(!m.grants_too_old());

        // The link's refusal says where they are read, in no notice: not
        // too old, and the old rows go.
        m.grants(vec![SourceGrant {
            source: "crm".into(),
            ..Default::default()
        }]);
        m.grants_failed(&link);
        assert!(m.grants_on_the_yard());
        assert!(!m.grants_too_old());
        assert!(m.grant_records().is_empty());
        assert!(m.notice.is_empty(), "{}", m.notice);
        // Grants read again clear that too.
        m.grants(Vec::new());
        assert!(!m.grants_on_the_yard());

        // Any other failure is a notice, as ever, and not the pane's.
        for err in [
            refused("the grants are closed"),
            plain,
            api::Error::Protocol("SourceGrants: status 200, and no JSON object".into()),
            api::Error::Down(std::io::Error::other("the yard went away")),
        ] {
            m.grants_failed(&err);
            assert!(!m.grants_too_old());
            assert!(!m.grants_on_the_yard());
            assert_eq!(m.notice, format!("the grants could not be read: {err}"));
            m.notice.clear();
        }
    }

    /// The Telemetry section is the operator's alone, on a yard that serves
    /// the calls: a preview read, a refusal that says the yard has no such
    /// calls or serves them down no link, and anything else the read fails
    /// with. The first is offered, the second hides the section with no
    /// notice, and the third is offered still, with why in the section. A
    /// preview read again clears either. The switch's move reads the
    /// preview again, and a change of operator drops what a former
    /// operator's window had and reads where the window is the operator's
    /// still.
    #[test]
    fn telemetry_is_offered_to_the_operator_on_a_yard_that_serves_the_calls() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        // The yard's own words, as it refuses the preview down the link.
        let link = refused(
            "the telemetry switch is not read over the link: this request came down the link from the edge. Read it on the yard's machine (`yardr telemetry show`). The edge saw the device's token with it: remove the device and add it again (yardr user device rm).",
        );
        assert!(telemetry_too_old(&refused("no such method TelemetryShow")));
        assert!(telemetry_too_old(&refused("no such method TelemetrySet")));
        assert!(!telemetry_too_old(&refused("the switch is closed")));
        assert!(!telemetry_too_old(&api::Error::Protocol(
            "TelemetryShow: status 200, and no JSON object".into()
        )));
        assert!(telemetry_over_the_link(&link));
        assert!(telemetry_over_the_link(&refused(
            "the telemetry switch is not changed over the link: this request came down the link from the edge."
        )));
        assert!(!telemetry_over_the_link(&refused(
            "no such method TelemetryShow"
        )));
        assert!(!telemetry_too_old(&link));

        let preview = || TelemetryPreview {
            telemetry: "on".into(),
            install_id: "3f6d1c2e".into(),
            ..Default::default()
        };
        // Not the operator's: not offered, whatever was read.
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(!m.is_operator());
        assert!(!m.telemetry_offered());
        m.telemetry(preview());
        assert!(!m.telemetry_offered());

        // The operator's: offered with the preview, and still offered
        // where the read failed, with why in the section.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.telemetry = Some(preview());
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert!(m.is_operator());
        assert!(m.telemetry_offered());
        assert_eq!(m.telemetry_preview().unwrap().install_id, "3f6d1c2e");
        assert!(m.telemetry_notice().is_none());
        assert!(!m.telemetry_read_only());

        // A yard without the calls hides the section, in no notice; a
        // preview read again clears it.
        m.telemetry_failed(&refused("no such method TelemetryShow"));
        assert!(!m.telemetry_offered());
        assert!(m.telemetry_preview().is_none());
        assert!(m.telemetry_notice().is_none());
        assert!(m.notice.is_empty(), "{}", m.notice);
        m.telemetry(preview());
        assert!(m.telemetry_offered());

        // The link's refusal hides it too, in no notice, and a preview
        // read again clears that as well.
        m.telemetry_failed(&link);
        assert!(!m.telemetry_offered());
        assert!(m.telemetry_preview().is_none());
        assert!(m.notice.is_empty(), "{}", m.notice);
        m.telemetry(preview());
        assert!(m.telemetry_offered());

        // Any other failure is the section's, as ever, and not a notice
        // of the yard's: the section is offered still, and tries again.
        for err in [
            refused("the switch is closed"),
            api::Error::Protocol("TelemetryShow: status 200, and no JSON object".into()),
            api::Error::Down(std::io::Error::other("the yard went away")),
        ] {
            m.telemetry_failed(&err);
            assert!(m.telemetry_offered());
            assert_eq!(
                m.telemetry_notice(),
                Some(format!("the telemetry switch could not be read: {err}").as_str())
            );
            assert!(m.telemetry_preview().is_none());
            assert!(m.notice.is_empty(), "{}", m.notice);
        }
        m.telemetry(preview());
        assert!(m.telemetry_notice().is_none());

        // The switch's move reads the preview again for the operator, and
        // nothing for anyone else nor where the yard serves no such call.
        let moved = event(21, "setting_changed", "", json!({"key": "telemetry"}));
        assert_eq!(m.event(&moved), vec![Effect::Telemetry]);
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(m.event(&moved).is_empty());
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.telemetry_unserved = true;
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert!(m.event(&moved).is_empty());

        // A change of operator drops the preview, and reads where the
        // window is the operator's still.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.telemetry = Some(preview());
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let changed = event(
            22,
            "setting_changed",
            "",
            json!({"key": "operator", "value": "ada"}),
        );
        let effects = m.event(&changed);
        assert!(!effects.contains(&Effect::Telemetry));
        assert!(!effects.contains(&Effect::Grants));
        assert!(!m.is_operator());
        assert!(m.telemetry_preview().is_none());
        assert!(!m.telemetry_offered());
        let changed = event(
            23,
            "setting_changed",
            "",
            json!({"key": "operator", "value": "jo"}),
        );
        let effects = m.event(&changed);
        assert!(effects.contains(&Effect::Grants));
        assert!(effects.contains(&Effect::Telemetry));
        assert!(m.is_operator());

        // Where the yard serves the calls to no such window, a newly
        // named operator reads none either: unserved stays as it was.
        let mut state = yard(&[]);
        state.operator = "ada".into();
        state.telemetry_unserved = true;
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let changed = event(
            24,
            "setting_changed",
            "",
            json!({"key": "operator", "value": "jo"}),
        );
        let effects = m.event(&changed);
        assert!(!effects.contains(&Effect::Telemetry));
        assert!(m.is_operator());
        assert!(!m.telemetry_offered());
    }

    /// A refused change of the switch is read-only here, refused down the
    /// link, or any other refusal: the first keeps the section with no
    /// buttons, the second hides it, and the third is the window's to say
    /// of the press. The section's words say the state, the next request
    /// and why no batch, as `yardr telemetry show` does.
    #[test]
    fn a_refused_change_is_read_only_down_the_link_or_said_of_the_press() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        // The yard's own words, as it refuses the change from its
        // router's peer and down the link.
        let router = refused(
            "the telemetry switch is taken over https only, or from the yard's own machine: this request reached the yard's listener in plain HTTP from another machine. Put the proxy or tunnel that ends TLS on the yard's machine, or Change it on the yard's machine (`yardr telemetry on` or `yardr telemetry off`). The device's token was sent with it and may have crossed a network in the clear: if it did, remove the device and add it again (yardr user device rm).",
        );
        let set_link = refused(
            "the telemetry switch is not changed over the link: this request came down the link from the edge. Change it on the yard's machine (`yardr telemetry on` or `yardr telemetry off`). The edge saw the device's token with it: remove the device and add it again (yardr user device rm).",
        );
        assert!(telemetry_router_refused(&router));
        assert!(!telemetry_router_refused(&set_link));
        assert!(!telemetry_router_refused(&refused(
            "no such method TelemetrySet"
        )));
        assert!(!telemetry_router_refused(&api::Error::Protocol(
            "TelemetrySet: status 200, and no JSON object".into()
        )));
        // Plain words, and never the wire's method name.
        assert!(TELEMETRY_READ_ONLY.contains("changed in the yard's web view"));
        assert!(!TELEMETRY_READ_ONLY.contains("TelemetrySet"));

        let preview = || TelemetryPreview {
            telemetry: "on".into(),
            install_id: "3f6d1c2e".into(),
            ..Default::default()
        };
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.telemetry = Some(preview());
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        // Anything else is the window's to say of the press: offered
        // still, with the preview and the buttons.
        assert!(!m.telemetry_set_refused(&refused("the switch is stuck")));
        assert!(!m.telemetry_set_refused(&api::Error::Protocol(
            "TelemetrySet: status 200, and no JSON object".into()
        )));
        assert!(m.telemetry_offered());
        assert!(!m.telemetry_read_only());
        assert!(m.telemetry_preview().is_some());
        // A yard that answered Show but has no Set call cannot change the
        // switch; once known, it no longer offers the section.
        assert!(m.telemetry_set_refused(&refused("no such method TelemetrySet")));
        assert!(!m.telemetry_offered());
        assert!(m.telemetry_preview().is_none());
        m.telemetry(preview());
        assert!(m.telemetry_offered());
        // The router's refusal keeps the section with no buttons, and a
        // preview read again keeps it read-only.
        assert!(m.telemetry_set_refused(&router));
        assert!(m.telemetry_read_only());
        assert!(m.telemetry_offered());
        assert!(m.telemetry_preview().is_some());
        m.telemetry(preview());
        assert!(m.telemetry_read_only());
        // Down the link hides the section, as a preview refused so does.
        assert!(m.telemetry_set_refused(&set_link));
        assert!(!m.telemetry_offered());
        assert!(m.telemetry_preview().is_none());

        // The section's words, as `yardr telemetry show` says them.
        let on = TelemetryPreview {
            telemetry: "on".into(),
            since: "2026-10-26".into(),
            sending_day: "2026-10-28".into(),
            source_days: vec!["2026-10-27".into()],
            request: Some(json!({"api_key": "phc_write", "batch": [{"event": "yardr_daily"}]})),
            ..Default::default()
        };
        assert_eq!(telemetry_state(&on), "on (since 2026-10-26)");
        let payload = telemetry_payload(&on).unwrap();
        assert!(payload.contains("\"event\": \"yardr_daily\""), "{payload}");
        assert!(payload.contains('\n'), "{payload}");
        assert!(telemetry_payload(&TelemetryPreview::default()).is_none());
        let on = TelemetryPreview {
            telemetry: "on".into(),
            ..Default::default()
        };
        assert_eq!(telemetry_state(&on), "on");
        assert_eq!(
            telemetry_state(&TelemetryPreview {
                telemetry: "off".into(),
                ..Default::default()
            }),
            "off — nothing is sent"
        );
        assert_eq!(
            telemetry_next_request(&TelemetryPreview {
                sending_day: "2026-10-28".into(),
                source_days: vec!["2026-10-27".into()],
                ..Default::default()
            }),
            "sending day 2026-10-28 (UTC); source day 2026-10-27"
        );
        assert_eq!(telemetry_source_days(&[]), "no eligible source days");
        assert_eq!(
            telemetry_source_days(&["2026-10-26".into(), "2026-10-27".into()]),
            "source days 2026-10-26..2026-10-27"
        );
        assert_eq!(
            telemetry_no_batch("off"),
            "no batch available until telemetry is enabled"
        );
        assert_eq!(
            telemetry_no_batch("attempted"),
            "no batch available until the next UTC day"
        );
        assert_eq!(
            telemetry_no_batch("empty"),
            "no batch available: nothing to send"
        );
        assert_eq!(
            telemetry_no_batch("no-id"),
            "no batch available: no install id (run yardr telemetry on again)"
        );
        // The consent words every surface shares: the four questions,
        // the id-only profile, the pseudonymous sentence, and how the
        // yard's data is deleted.
        assert!(TELEMETRY_QUESTIONS.contains("four questions"));
        assert!(TELEMETRY_QUESTIONS.contains("https://eu.i.posthog.com/batch/"));
        assert!(TELEMETRY_PROFILE.contains("id-only person profile"));
        assert!(TELEMETRY_ID_SENTENCE.contains("pseudonymous, not anonymous"));
        assert!(TELEMETRY_DELETION.contains("send the project this install id"));
    }

    /// The grants view is the operator's alone: its row stands in
    /// decisions where the window runs as the yard's operator, and
    /// nowhere else. Opening it reads the grants again, so a revoke,
    /// which tells no event, is seen; a grant given reads them again
    /// through its event. Where the yard serves the list on its machine
    /// alone, the row stands and none of that asks it.
    #[test]
    fn the_grants_row_stands_for_the_operator_alone_and_reads_again() {
        // Not the operator's: no row, no read.
        let (mut m, _) = Model::new(yard(&[]), Mode::Decisions, None);
        assert!(!m.is_operator());
        assert!(!m.order_in(Mode::Decisions).contains(&Selected::Grants));
        assert_eq!(m.select(Selected::Grants), Vec::new());
        assert_ne!(m.selected(), Some(&Selected::Grants));

        // The operator's: the row stands first, and selecting it reads.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        assert!(m.is_operator());
        let order = m.order_in(Mode::Decisions);
        assert_eq!(order.last(), Some(&Selected::Grants));
        assert_eq!(m.select(Selected::Grants), vec![Effect::Grants]);

        // With nothing asking, decisions opens on none, with the
        // grants in the list to open; with an ask, on the ask. The grants
        // never take a card's place.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        let (m, _) = Model::new(state.clone(), Mode::Decisions, None);
        assert_eq!(m.selected(), None);
        assert!(m.order_in(Mode::Decisions).contains(&Selected::Grants));
        state.asks.open.push(OpenAsk {
            bead: "bead-a".into(),
            ..Default::default()
        });
        let (m, _) = Model::new(state, Mode::Decisions, None);
        assert_eq!(m.selected(), Some(&Selected::Decision("bead-a".into())));

        // A grant given reads the grants again for the operator, and
        // nothing for anyone else.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let event = |kind: &str| Event {
            seq: 21,
            kind: kind.into(),
            ..Default::default()
        };
        assert_eq!(m.event(&event("source_granted")), vec![Effect::Grants]);
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(m.event(&event("source_granted")).is_empty());

        // The operator's window of a yard that serves the list on its
        // machine alone: the row stands, and neither opening it nor a
        // grant given asks the yard for the list.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.grants_on_the_yard = true;
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        assert!(m.is_operator());
        assert_eq!(m.order_in(Mode::Decisions).last(), Some(&Selected::Grants));
        assert_eq!(m.select(Selected::Grants), Vec::new());
        assert_eq!(m.selected(), Some(&Selected::Grants));
        assert!(m.event(&event("source_granted")).is_empty());

        // Changing the operator clears the grants at once. The old
        // operator's selected pane closes, and a newly named operator
        // reads grants only for their own window.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.grants.push(SourceGrant {
            source: "crm".into(),
            ..Default::default()
        });
        let (mut m, _) = Model::new(state, Mode::Decisions, Some(Selected::Grants));
        let changed = Event {
            seq: 22,
            kind: "setting_changed".into(),
            data: serde_json::json!({"key": "operator", "value": "ada"})
                .as_object()
                .unwrap()
                .clone(),
            ..Default::default()
        };
        assert!(!m.event(&changed).contains(&Effect::Grants));
        assert!(!m.is_operator());
        assert!(m.grant_records().is_empty());
        assert_ne!(m.selected(), Some(&Selected::Grants));
        assert!(!m.order_in(Mode::Decisions).contains(&Selected::Grants));

        let mut state = yard(&[]);
        state.operator = "ada".into();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        let changed = Event {
            seq: 23,
            kind: "setting_changed".into(),
            data: serde_json::json!({"key": "operator", "value": "jo"})
                .as_object()
                .unwrap()
                .clone(),
            ..Default::default()
        };
        assert!(m.event(&changed).contains(&Effect::Grants));
        assert!(m.is_operator());

        // Where the list is read on the yard's machine, a newly named
        // operator reads none either, and the pane keeps saying so.
        let mut state = yard(&[]);
        state.operator = "ada".into();
        state.grants_on_the_yard = true;
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        assert!(!m.event(&changed).contains(&Effect::Grants));
        assert!(m.is_operator());
        assert!(m.grants_on_the_yard());
    }

    /// A session's screen opens for the operator alone: the session
    /// working a bead, without its runner word, and a crew member's
    /// own. Anyone else is offered none, as is an unworked bead, an
    /// unknown bead, and a name that is none of the crew. Nor does the
    /// operator's window get one where the yard reads none down the link
    /// it comes by: a state says so of a hosted yard, a refusal of the
    /// screen as a request come down the link from the edge says so of any
    /// other, and the model of a state read again keeps that. The yard's
    /// other refusals of a screen are faults of the address, and say so.
    #[test]
    fn the_screen_opens_for_the_operator_alone() {
        use crate::api::FlowBead;
        let mut flows = flow_view();
        flows.depots[0].flows[0].stages[0].beads = vec![
            FlowBead {
                bead: "yd-zulu".into(),
                working: true,
                worker: "herdr yd-zulu-new".into(),
                ..FlowBead::default()
            },
            FlowBead {
                bead: "yd-idle".into(),
                ..FlowBead::default()
            },
        ];
        // Not the operator's: no bead and no member opens.
        let mut state = yard(&[]);
        state.flows = flows.clone();
        let (m, _) = Model::new(state, Mode::Timetable, None);
        assert!(!m.is_operator());
        assert_eq!(m.screen_for_bead("yd-zulu"), None);
        assert_eq!(m.screen_for_crew("yardmaster"), None);
        // The operator's: the worked bead opens its session, and a
        // member their own.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.flows = flows.clone();
        let (mut m, _) = Model::new(state, Mode::Timetable, None);
        assert!(m.is_operator());
        assert!(!m.screen_on_the_yard());
        assert_eq!(
            m.screen_for_bead("yd-zulu"),
            Some("yd-zulu-new".to_string())
        );
        assert_eq!(m.screen_for_bead("yd-idle"), None);
        assert_eq!(m.screen_for_bead("yd-else"), None);
        assert_eq!(
            m.screen_for_crew("yardmaster"),
            Some("yardmaster".to_string())
        );
        assert_eq!(m.screen_for_crew("ada"), None);

        // A refusal that says the request came down the link from the edge
        // takes the Screen away, and a state read again keeps it away.
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        let link = refused(
            "a session's screen is not read over the link: this request came down the link from the edge. Read it on the yard's machine. The edge saw the device's token with it: remove the device and add it again (yardr user device rm).",
        );
        let plain = refused(
            "a session's screen is taken over https only, or from the yard's own machine: this request reached the yard's listener in plain HTTP from another machine. Put the proxy or tunnel that ends TLS on the yard's machine, or Read it on the yard's machine. The device's token was sent with it and may have crossed a network in the clear: if it did, remove the device and add it again (yardr user device rm).",
        );
        assert!(screen_over_the_link(&link));
        assert!(!screen_over_the_link(&plain));
        assert!(!screen_over_the_link(&refused(
            "no such method: SessionScreen"
        )));
        assert!(!screen_over_the_link(&api::Error::Down(
            std::io::Error::other("the yard went away")
        )));
        // Plain words, never the wire's method name, and one line.
        assert!(SCREEN_ON_THE_YARD.contains("read on the yard's machine"));
        assert!(!SCREEN_ON_THE_YARD.contains("SessionScreen"));
        assert!(!SCREEN_ON_THE_YARD.contains('\n'));
        m.screen_refused_down_the_link();
        assert!(m.is_operator());
        assert!(m.screen_on_the_yard());
        assert_eq!(m.screen_for_bead("yd-zulu"), None);
        assert_eq!(m.screen_for_crew("yardmaster"), None);
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.flows = flows.clone();
        let (m, _) = m.resumed(state, Mode::Timetable);
        assert!(m.screen_on_the_yard());
        assert_eq!(m.screen_for_crew("yardmaster"), None);

        // A hosted yard's state says it from the start.
        let mut state = yard(&[]);
        state.operator = "jo".into();
        state.flows = flows;
        state.screen_on_the_yard = true;
        let (m, _) = Model::new(state, Mode::Timetable, None);
        assert!(m.is_operator());
        assert!(m.screen_on_the_yard());
        assert_eq!(m.screen_for_bead("yd-zulu"), None);
        assert_eq!(m.screen_for_crew("yardmaster"), None);
    }

    /// The grants view lists what `yardr source grants` prints: who holds
    /// what of each source, each call's own time and approver, the weighty
    /// words for update and `--as` rows, and each row's revoke line.
    #[test]
    fn the_grants_view_lists_each_call_with_its_own_grant_and_its_revoke() {
        use std::collections::HashMap;
        let grant = |source: &str,
                     group: &str,
                     crew: &str,
                     as_user: &str,
                     calls: &[&str],
                     given: &[(&str, &str, &str, &str, u64, &str)]|
         -> SourceGrant {
            SourceGrant {
                source: source.into(),
                group: group.into(),
                crew: crew.into(),
                as_user: as_user.into(),
                calls: calls.iter().map(|c| c.to_string()).collect(),
                at: "2026-10-12T10:15:00Z".into(),
                by: "mckean".into(),
                given: given
                    .iter()
                    .map(|(call, at, by, bead, ask, asker)| {
                        (
                            call.to_string(),
                            GrantGiven {
                                at: at.to_string(),
                                by: by.to_string(),
                                bead: bead.to_string(),
                                ask: *ask,
                                asker: asker.to_string(),
                            },
                        )
                    })
                    .collect::<HashMap<_, _>>(),
                ..Default::default()
            }
        };
        let grants = vec![
            grant(
                "google-mail",
                "",
                "yardmaster",
                "",
                &["read"],
                &[(
                    "read",
                    "2026-10-17T19:02:00Z",
                    "mckean",
                    "yd-c546",
                    409,
                    "yardmaster",
                )],
            ),
            grant(
                "google-mail",
                "",
                "yardmaster",
                "ada",
                &["send"],
                &[(
                    "send",
                    "2026-10-17T19:04:00Z",
                    "mckean",
                    "yd-c546",
                    412,
                    "yardmaster",
                )],
            ),
            grant("telegram", "builders", "", "", &["read", "reply"], &[]),
        ];

        // Who holds what, with the user called as where there is one.
        assert_eq!(grant_holder(&grants[0]), "crew member yardmaster");
        assert_eq!(grant_holder(&grants[1]), "crew member yardmaster as ada");
        assert_eq!(grant_holder(&grants[2]), "group builders");

        // These records come with no kinds, as from a yard before it
        // served them: a named call is taken for an update, and `read`,
        // the grant that covers all reads, stays a read.
        assert!(!grant_is_update("read", &grants[0]));
        assert!(grant_is_update("send", &grants[1]));
        assert!(grant_is_update("reply", &grants[2]));

        // A call is an update by the kind the yard served for it: a grant
        // naming a read call and an update call has one read row, with no
        // weighty words, and one weighty row, and the summary counts one.
        let kinds = |kinds: &[(&str, &str)]| -> SourceGrant {
            SourceGrant {
                kinds: kinds
                    .iter()
                    .map(|(call, kind)| (call.to_string(), kind.to_string()))
                    .collect(),
                ..grant("crm", "", "yardmaster", "", &["customer", "note"], &[])
            }
        };
        let crm = kinds(&[("customer", "read"), ("note", "update")]);
        assert!(!grant_is_update("customer", &crm));
        assert!(grant_is_update("note", &crm));
        assert!(grant_weight_lines("crm", "customer", &crm).is_empty());
        assert_eq!(grant_weight_lines("crm", "note", &crm).len(), 1);
        assert_eq!(
            grant_summary(std::slice::from_ref(&crm)),
            "2 calls granted of 1 source to 1 holder: 1 may change something outside the yard, 0 calls as a user."
        );
        // As a user, a named read carries the user's line alone.
        let as_ada = SourceGrant {
            as_user: "ada".into(),
            ..crm.clone()
        };
        assert_eq!(
            grant_weight_lines("crm", "customer", &as_ada),
            ["WEIGHTY · as-user: lets calls or words count as a person's: as ada"]
        );
        // The same grant from a yard that serves no kinds: both calls are
        // weighty, as they were before, and `read` still is not.
        let older = kinds(&[]);
        assert!(grant_is_update("customer", &older));
        assert!(grant_is_update("note", &older));
        assert_eq!(grant_weight_lines("crm", "customer", &older).len(), 1);
        assert!(!grant_is_update("read", &older));
        assert_eq!(
            grant_summary(std::slice::from_ref(&older)),
            "2 calls granted of 1 source to 1 holder: 2 may change something outside the yard, 0 calls as a user."
        );
        // A call with no kind among the others, and a kind this window
        // has no word for, are unknown: updates, never reads.
        let partly = kinds(&[("customer", "read")]);
        assert!(grant_is_update("note", &partly));
        let strange = kinds(&[("customer", "relay"), ("note", "")]);
        assert!(grant_is_update("customer", &strange));
        assert!(grant_is_update("note", &strange));
        // The yard's word for the read grant is a read all the same.
        let read = SourceGrant {
            calls: vec!["read".into()],
            kinds: [("read".to_string(), "read".to_string())].into(),
            ..crm.clone()
        };
        assert!(!grant_is_update("read", &read));

        // Each row's revoke line, to copy.
        assert_eq!(
            grant_revoke("google-mail", "read", &grants[0]),
            "yardr source revoke google-mail read --crew yardmaster"
        );
        assert_eq!(
            grant_revoke("google-mail", "send", &grants[1]),
            "yardr source revoke google-mail send --crew yardmaster --as ada"
        );
        assert_eq!(
            grant_revoke("telegram", "reply", &grants[2]),
            "yardr source revoke telegram reply --group builders"
        );
        // A copied line keeps hostile yard text in one shell word; option
        // values cannot become options, and `--` protects positionals.
        assert_eq!(
            grant_revoke(
                "crm; touch /tmp/owned",
                "send",
                &SourceGrant {
                    group: "builders; --help".into(),
                    as_user: "ada".into(),
                    ..Default::default()
                }
            ),
            "yardr source revoke --group='builders; --help' --as=ada -- 'crm; touch /tmp/owned' send"
        );

        // Each call's own time and approver, with the card where it was
        // approved on one; a call with no entry reads not kept.
        let (granted, by) = grant_given(grants[0].given.get("read"));
        assert!(!granted.is_empty(), "a kept time says when");
        assert_eq!(by, "mckean, on yd-c546 ask 409 asked by yardmaster");
        let (granted, by) = grant_given(grants[1].given.get("send"));
        assert!(!granted.is_empty());
        assert_eq!(by, "mckean, on yd-c546 ask 412 asked by yardmaster");
        assert_eq!(
            grant_given(grants[2].given.get("read")),
            ("not kept".to_string(), "-".to_string())
        );
        // A terminal grant names no card; a pruned asker reads unknown.
        let terminal = GrantGiven {
            at: "2026-10-08T09:30:00Z".into(),
            by: "jo".into(),
            ..Default::default()
        };
        assert_eq!(grant_given(Some(&terminal)).1, "jo");
        let pruned = GrantGiven {
            at: "2026-10-08T09:30:00Z".into(),
            by: "jo".into(),
            bead: "yd-crm".into(),
            ask: 57,
            ..Default::default()
        };
        assert_eq!(
            grant_given(Some(&pruned)).1,
            "jo, on yd-crm ask 57 asked by unknown"
        );
        assert!(grant_time("not a time").is_empty());

        // The weighty words: an update row carries outside's, an `--as`
        // row as-user's, both where both hold, none for a plain read.
        assert!(grant_weight_lines("google-mail", "read", &grants[0]).is_empty());
        let lines = grant_weight_lines("google-mail", "send", &grants[1]);
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("WEIGHTY · outside: lets the yard change something outside it: ")
        );
        assert!(
            lines[0]
                .contains("crew member yardmaster as ada may call send of google-mail (an update)")
        );
        assert_eq!(
            lines[1],
            "WEIGHTY · as-user: lets calls or words count as a person's: as ada"
        );
        let lines = grant_weight_lines("telegram", "reply", &grants[2]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("WEIGHTY · outside: "));

        // The summary counts rows, sources and holders, with the weighty
        // ones among them; a holder's plain grant and its grant as a user
        // are one holder.
        assert_eq!(
            grant_summary(&grants),
            "4 calls granted of 2 sources to 2 holders: 2 may change something outside the yard, 1 call as a user."
        );

        // Below it, each record holding an unkept call says when it last
        // changed and by whom, for no call in particular.
        let unkept = grant_unkept(&grants);
        assert_eq!(unkept.len(), 1);
        assert!(unkept[0].starts_with(
            "telegram to group builders: granted before its history was kept; the record last gained a call "
        ));
        assert!(unkept[0].ends_with(" by mckean."));

        // Listed by source, then holder, then the user called as.
        let ordered = ordered_grants(&grants);
        assert_eq!(
            ordered
                .iter()
                .map(|g| (g.source.as_str(), g.as_user.as_str()))
                .collect::<Vec<_>>(),
            [
                ("google-mail", ""),
                ("google-mail", "ada"),
                ("telegram", "")
            ]
        );
    }

    #[test]
    fn an_image_over_the_yard_s_cap_is_refused_with_both_sizes() {
        assert_eq!(over_cap(70, 8388608), None);
        assert_eq!(over_cap(8388608, 8388608), None);
        assert_eq!(
            over_cap(9 * 1024 * 1024, 8388608).as_deref(),
            Some(
                "the image is 9.0 MiB, and this yard takes one of 8.0 MiB at most (yardr settings: blob_max_bytes)"
            )
        );
        assert_eq!(size(70), "70 bytes");
        assert_eq!(size(2048), "2 KiB");
    }

    #[test]
    fn over_the_cap_names_what_the_bytes_are() {
        let page = b"<!DOCTYPE html><html><body>the mockup</body></html>";
        assert_eq!(over_cap_for(PNG, 70, 8388608), None);
        assert_eq!(
            over_cap_for(PNG, 2048, 1024).as_deref(),
            Some(
                "the image is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
        assert_eq!(
            over_cap_for(page, 2048, 1024).as_deref(),
            Some(
                "the page is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
        assert_eq!(
            over_cap_for(b"%PDF-1.7", 2048, 1024).as_deref(),
            Some(
                "the file is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
    }

    #[test]
    fn only_an_empty_text_is_unsendable() {
        assert_eq!(unsendable("hello"), None);
        assert_eq!(unsendable(""), Some(String::new()));
        assert_eq!(unsendable("  \n "), Some(String::new()));
        // How long is the yard's to say: it splits a text past one mail
        // (4096 bytes) and refuses one past 16 mails (64 KB) in its words,
        // so even that one goes to it.
        assert_eq!(unsendable(&"x".repeat(4096)), None);
        assert_eq!(unsendable(&"x".repeat(4097)), None);
        assert_eq!(unsendable(&"x".repeat(30 * 1024)), None);
        assert_eq!(unsendable(&"x".repeat(100 * 1024)), None);
    }

    fn open_ask(
        bead: &str,
        title: &str,
        topic: &str,
        ask: u64,
        question: &str,
        asker: &str,
        since: &str,
    ) -> OpenAsk {
        OpenAsk {
            bead: bead.into(),
            bead_title: title.into(),
            topic: topic.into(),
            ask,
            question: question.into(),
            asker: asker.into(),
            since: since.into(),
            secret: None,
            concealed: true,
            run: None,
            setup: None,
            identity: None,
            weight: Vec::new(),
        }
    }

    fn decided_ask(
        bead: &str,
        ask: u64,
        question: &str,
        state: &str,
        option_label: &str,
        words: &str,
        return_at: &str,
    ) -> DecidedAsk {
        DecidedAsk {
            bead: bead.into(),
            bead_title: format!("{bead} title"),
            topic: String::new(),
            ask,
            question: question.into(),
            asker: "yardmaster".into(),
            since: "2026-10-08T09:30:00Z".into(),
            state: state.into(),
            option: u64::from(!option_label.is_empty()),
            option_label: option_label.into(),
            words: words.into(),
            by: String::new(),
            return_at: return_at.into(),
            secret: None,
            concealed: true,
            given: Vec::new(),
            run: None,
            setup: None,
            identity: None,
            weight: Vec::new(),
            ran: None,
        }
    }

    fn notification(seq: u64, bead: &str, text: &str, read: bool) -> Notification {
        Notification {
            seq,
            at: "2026-10-08T09:30:00Z".into(),
            kind: "created".into(),
            bead: bead.into(),
            text: text.into(),
            read,
        }
    }

    fn noon() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    /// Every decided row of every day, folds' members with them: what the
    /// history holds, without its shape.
    fn decided_all(m: &Model) -> Vec<DecidedRow> {
        m.decided_groups(&noon())
            .into_iter()
            .flat_map(|group| group.rows)
            .flat_map(|entry| match entry {
                DecidedEntry::One(row) => vec![row],
                DecidedEntry::Fold(fold) => fold.rows,
            })
            .collect()
    }

    /// That evening's asks, oldest first: two topics and one with none.
    fn evening() -> Vec<OpenAsk> {
        vec![
            open_ask(
                "yardr-n8ta",
                "Move the links",
                "topic:linear-out",
                3,
                "Move this yard's 64 links to the declared source now?",
                "yardmaster",
                "2026-10-10T10:00:00Z",
            ),
            open_ask(
                "yardr-3msx",
                "Send to build",
                "topic:linear-out",
                4,
                "Send yardr-3msx and yardr-vgjp to build?",
                "yardmaster",
                "2026-10-10T11:00:00Z",
            ),
            open_ask(
                "yardr-9f2c",
                "Cleanup",
                "topic:web-view",
                5,
                "The web view cleanup: build?",
                "assembly",
                "2026-10-09T12:00:00Z",
            ),
            open_ask(
                "yardr-7h11",
                "Sketch",
                "",
                6,
                "Close sketch PR #3?",
                "yardmaster",
                "2026-10-10T11:30:00Z",
            ),
        ]
    }

    #[test]
    fn decisions_are_grouped_by_topic_oldest_first_without_the_prefix() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.decisions_count(), 4);
        let groups = m.decision_groups(&noon());
        // Topics in the order their oldest ask waits.
        let topics: Vec<&str> = groups.iter().map(|g| g.topic.as_str()).collect();
        assert_eq!(topics, ["linear-out", "web-view", "Everything else"]);
        assert_eq!(groups[0].rows.len(), 2);
        assert_eq!(groups[1].rows.len(), 1);
        // Within a topic, oldest first.
        assert_eq!(
            groups[0].rows[0].head,
            "Move this yard's 64 links to the declared source now?"
        );
        assert_eq!(
            groups[0].rows[1].head,
            "Send yardr-3msx and yardr-vgjp to build?"
        );
    }

    #[test]
    fn each_mode_opens_on_its_first_row() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(state, Mode::Decisions, None);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));

        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));

        let (m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert_eq!(m.selected(), Some(&Selected::Arrivals));

        // Decisions with none open opens on nothing.
        let (m, _) = Model::new(yard(&[]), Mode::Decisions, None);
        assert_eq!(m.selected(), None);
    }

    #[test]
    fn the_modes_step_around_the_ends_in_the_rail_s_order() {
        assert_eq!(Mode::ALL, [Mode::Chat, Mode::Decisions, Mode::Timetable]);
        assert_eq!(Mode::Chat.step(1), Mode::Decisions);
        assert_eq!(Mode::Decisions.step(1), Mode::Timetable);
        assert_eq!(Mode::Timetable.step(1), Mode::Chat);
        assert_eq!(Mode::Chat.step(-1), Mode::Timetable);
        assert_eq!(Mode::Timetable.step(-1), Mode::Decisions);
        assert_eq!(Mode::Decisions.step(-1), Mode::Chat);
        let labels: Vec<&str> = Mode::ALL.into_iter().map(Mode::label).collect();
        assert_eq!(labels, ["Crew", "Decide", "Lines"]);
    }

    #[test]
    fn every_row_is_one_mode_s() {
        assert_eq!(
            Selected::Conversation("yardmaster".into()).mode(),
            Mode::Chat
        );
        assert_eq!(Selected::Yard.mode(), Mode::Chat);
        assert_eq!(Selected::Decision("bead-a".into()).mode(), Mode::Decisions);
        assert_eq!(
            Selected::Decided("bead-a".into(), 3).mode(),
            Mode::Decisions
        );
        assert_eq!(Selected::Arrivals.mode(), Mode::Timetable);
        assert_eq!(Selected::Line("yardr".into()).mode(), Mode::Timetable);
        assert_eq!(Selected::NoLine.mode(), Mode::Timetable);
        assert_eq!(Selected::Bead("yd-zulu".into()).mode(), Mode::Timetable);
    }

    #[test]
    fn a_row_leads_with_the_question_then_asker_and_waits_bead_small_at_its_end() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(state, Mode::Chat, None);
        let groups = m.decision_groups(&noon());
        let row = &groups[0].rows[0];
        // Its bead not read yet: what it wants reads as words until then.
        assert_eq!(row.sub, "yardmaster · words wanted · 2h");
        assert_eq!(row.short, "n8ta");
        assert_eq!(row.bead, "yardr-n8ta");
        // No row reads "<id> go?".
        assert!(!row.head.contains("yardr-n8ta"), "{}", row.head);
        // A day's wait.
        assert_eq!(groups[1].rows[0].sub, "assembly · words wanted · 1d");
    }

    #[test]
    fn a_bead_with_several_open_asks_is_one_row() {
        let mut state = yard(&[]);
        state.asks.open = vec![
            open_ask(
                "yardr-me78",
                "Depot rules",
                "topic:design-lane",
                7,
                "First?",
                "yardmaster",
                "2026-10-09T12:00:00Z",
            ),
            open_ask(
                "yardr-me78",
                "Depot rules",
                "topic:design-lane",
                8,
                "Second?",
                "yardmaster",
                "2026-10-10T11:00:00Z",
            ),
            open_ask(
                "yardr-me78",
                "Depot rules",
                "topic:design-lane",
                9,
                "Third?",
                "assembly",
                "2026-10-10T11:30:00Z",
            ),
            open_ask(
                "yardr-me78",
                "Depot rules",
                "topic:design-lane",
                10,
                "Fourth?",
                "yardmaster",
                "2026-10-10T11:45:00Z",
            ),
        ];
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.decisions_count(), 1);
        let groups = m.decision_groups(&noon());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].topic, "design-lane");
        let row = &groups[0].rows[0];
        assert_eq!(row.head, "Depot rules: 4 questions");
        // How long the oldest waits; no one asker.
        assert_eq!(row.sub, "1d");
        assert_eq!(row.short, "me78");
        // Each ask beneath with its own question and line.
        let metas: Vec<(&str, &str)> = row
            .asks
            .iter()
            .map(|ask| (ask.head.as_str(), ask.meta.as_str()))
            .collect();
        assert_eq!(
            metas,
            [
                ("First?", "yardmaster · words wanted · 1d"),
                ("Second?", "yardmaster · words wanted · 1h"),
                ("Third?", "assembly · words wanted · 30m"),
                ("Fourth?", "yardmaster · words wanted · 15m"),
            ]
        );
        // The open row shows each ask oldest first.
        let detail = m.decision_detail("yardr-me78", &noon()).unwrap();
        assert_eq!(detail.title, "Depot rules");
        assert_eq!(detail.topic, "design-lane");
        let questions: Vec<&str> = detail.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(questions, ["First?", "Second?", "Third?", "Fourth?"]);
        assert_eq!(detail.asks[0].waits, "1d");
    }

    #[test]
    fn how_long_an_ask_waits() {
        let now = noon();
        assert_eq!(waits("2026-10-10T11:59:30Z", &now), "now");
        assert_eq!(waits("2026-10-10T11:55:00Z", &now), "5m");
        assert_eq!(waits("2026-10-10T10:00:00Z", &now), "2h");
        assert_eq!(waits("2026-10-09T12:00:00Z", &now), "1d");
        assert_eq!(waits("2026-10-07T12:00:00Z", &now), "3d");
        assert_eq!(waits("", &now), "");
        assert_eq!(waits("yesterday", &now), "");
    }

    /// A bead's stage age reads from its served entry: minutes, then
    /// hours, then days, at its stage. A stay under a minute reads a
    /// minute, and a time that cannot be read reads nothing.
    #[test]
    fn a_bead_s_stage_age_reads_from_its_served_entry() {
        let now = noon();
        assert_eq!(
            stage_age("2026-10-10T11:19:00Z", "new", &now).as_deref(),
            Some("41 min at new")
        );
        assert_eq!(
            stage_age("2026-10-10T11:59:30Z", "new", &now).as_deref(),
            Some("1 min at new")
        );
        assert_eq!(
            stage_age("2026-10-10T10:00:00Z", "review", &now).as_deref(),
            Some("2 h at review")
        );
        assert_eq!(
            stage_age("2026-10-09T12:00:00Z", "review", &now).as_deref(),
            Some("1 day at review")
        );
        assert_eq!(
            stage_age("2026-10-07T12:00:00Z", "new", &now).as_deref(),
            Some("3 days at new")
        );
        assert_eq!(stage_age("", "new", &now), None);
        assert_eq!(stage_age("yesterday", "new", &now), None);
    }

    #[test]
    fn decided_stands_folded_with_the_picks_newest_first() {
        let mut state = yard(&[]);
        state.asks.decided = vec![
            decided_ask(
                "yd-links",
                6,
                "Send them to build?",
                "answered",
                "Send them",
                "tonight",
                "",
            ),
            decided_ask("yd-greet", 10, "Who greets?", "answered", "", "warmly", ""),
            decided_ask(
                "yd-links",
                8,
                "Close sketch PR #3?",
                "declined",
                "",
                "not mine",
                "",
            ),
            decided_ask(
                "yd-links",
                12,
                "Build the cleanup?",
                "put aside",
                "",
                "",
                "2026-10-11T09:30:00Z",
            ),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.decided_count(), 4);
        assert!(!m.decided_expanded());
        // Folded: its rows are not in the tab order.
        m.select(Selected::Yard);
        m.step_in(Mode::Chat, -1);
        assert_eq!(m.selected_conversation(), Some("jo"));

        m.toggle_decided();
        assert!(m.decided_expanded());
        let groups = m.decided_groups(&noon());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].day, "8 Oct");
        let rows = decided_all(&m);
        assert_eq!(rows[0].head, "Send them to build?");
        assert_eq!(rows[0].outcome, "✓approved");
        assert_eq!(rows[0].detail, "yardmaster · 09:30");
        assert_eq!(rows[1].outcome, "✎warmly");
        assert_eq!(rows[1].detail, "yardmaster · 09:30");
        assert_eq!(rows[2].outcome, "✕declined: not mine");
        assert_eq!(rows[3].outcome, "‖put aside");
        // Open: its rows stand in the decisions' tab order, after the
        // open ones.
        m.select(Selected::Decided("yd-links".into(), 6));
        m.step_in(Mode::Decisions, 1);
        assert_eq!(
            m.selected(),
            Some(&Selected::Decided("yd-greet".into(), 10))
        );
        m.step_in(Mode::Decisions, -1);
        assert_eq!(m.selected(), Some(&Selected::Decided("yd-links".into(), 6)));
        // The open dealt with shows its pick.
        let detail = m.decided_detail("yd-links", 6, &chrono::Utc).unwrap();
        assert_eq!(detail.question, "Send them to build?");
        assert_eq!(detail.state, "answered");
        assert_eq!(detail.picked, 1);
        assert_eq!(detail.pick, "Send them");
        assert_eq!(detail.words, "tonight");
        assert_eq!(detail.state_line, "you picked 1 · Send them");
    }

    /// A dealt with ask as `decided_ask` makes it, with its asker and time.
    #[allow(clippy::too_many_arguments)]
    fn decided_when(
        bead: &str,
        ask: u64,
        question: &str,
        asker: &str,
        since: &str,
        state: &str,
        option_label: &str,
        words: &str,
        return_at: &str,
    ) -> DecidedAsk {
        DecidedAsk {
            asker: asker.into(),
            since: since.into(),
            ..decided_ask(bead, ask, question, state, option_label, words, return_at)
        }
    }

    /// A week's history, newest first: an approval and a look-alike pair
    /// asked today and yesterday, a secret given, a decline, a put-aside
    /// with no time, and a look-alike asked today that stands on its own,
    /// its pair a day away.
    fn week() -> Vec<DecidedAsk> {
        vec![
            decided_when(
                "yd-cal",
                7,
                "Raise Google Docs to 3000 calls",
                "mayor",
                "2026-10-10T09:00:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
            decided_when(
                "yd-cal",
                1,
                "Let the yardmaster read your calendar",
                "mayor",
                "2026-10-10T08:00:00Z",
                "answered",
                "Let them",
                "",
                "",
            ),
            decided_when(
                "yd-cal",
                3,
                "Raise Google Docs to 2000 calls",
                "mayor",
                "2026-10-09T20:18:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
            decided_when(
                "yd-cal",
                2,
                "Raise Google Docs to 1000 calls",
                "mayor",
                "2026-10-09T20:17:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
            DecidedAsk {
                given: vec!["GOOGLE_ACCOUNT".into()],
                ..decided_when(
                    "yd-cal",
                    4,
                    "Which Google account should the source use?",
                    "mayor",
                    "2026-10-09T09:00:00Z",
                    "answered",
                    "",
                    "",
                    "",
                )
            },
            decided_when(
                "bead-d",
                5,
                "Take the release train?",
                "assembly",
                "2026-10-08T11:07:00Z",
                "declined",
                "",
                "not mine",
                "",
            ),
            decided_when(
                "bead-d",
                6,
                "Rebuild the index tonight?",
                "mayor",
                "",
                "put aside",
                "",
                "",
                "",
            ),
        ]
    }

    #[test]
    fn the_history_groups_by_day_newest_first_with_look_alikes_folded() {
        let mut state = yard(&[]);
        state.asks.decided = week();
        let (m, _) = Model::new(state, Mode::Chat, None);
        let groups = m.decided_groups(&noon());
        let days: Vec<&str> = groups.iter().map(|g| g.day.as_str()).collect();
        assert_eq!(days, ["Today", "Yesterday", "8 Oct", "Earlier"]);
        // Today: the lone look-alike with the approval, one by one.
        assert_eq!(groups[0].rows.len(), 2);
        assert!(matches!(groups[0].rows[0], DecidedEntry::One(_)));
        assert!(matches!(groups[0].rows[1], DecidedEntry::One(_)));
        // Yesterday: the pair folded, the secret given after it.
        assert_eq!(groups[1].rows.len(), 2);
        let DecidedEntry::Fold(fold) = &groups[1].rows[0] else {
            panic!("the pair folds: {:?}", groups[1].rows[0]);
        };
        assert_eq!(fold.bead, "yd-cal");
        assert_eq!(fold.ask, 3);
        assert_eq!(fold.head, "Raise Google Docs to 2000 calls");
        assert_eq!(fold.outcome, "✓approved");
        assert_eq!(fold.detail, "mayor");
        assert!(!fold.open);
        assert_eq!(fold.rows.len(), 2);
        assert_eq!(fold.rows[0].ask, 3);
        assert_eq!(fold.rows[1].ask, 2);
        let DecidedEntry::One(row) = &groups[1].rows[1] else {
            panic!("the secret stands alone: {:?}", groups[1].rows[1]);
        };
        assert_eq!(row.outcome, "✎gave GOOGLE_ACCOUNT");
        assert_eq!(row.detail, "mayor · 09:00");
        // The decline and the timeless put-aside, one by one.
        assert_eq!(groups[2].rows.len(), 1);
        assert_eq!(groups[3].rows.len(), 1);
    }

    #[test]
    fn a_decided_day_reads_today_yesterday_date_and_earlier() {
        let today = noon().date_naive();
        assert_eq!(decided_day(Some(today), today), "Today");
        let yesterday = today.pred_opt().unwrap();
        assert_eq!(decided_day(Some(yesterday), today), "Yesterday");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert_eq!(decided_day(Some(day), today), "8 Oct");
        // Another year names it.
        let last = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        assert_eq!(decided_day(Some(last), today), "31 Dec 2025");
        assert_eq!(decided_day(None, today), "Earlier");
    }

    #[test]
    fn look_alikes_fold_only_consecutive_same_asker_same_outcome_same_day() {
        // Another asker's between them: no fold.
        let mut state = yard(&[]);
        state.asks.decided = vec![
            decided_when(
                "yd-cal",
                1,
                "Raise Google Docs to 1000 calls",
                "mayor",
                "2026-10-10T09:00:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
            decided_when(
                "yd-cal",
                2,
                "Raise Google Docs to 2000 calls",
                "assembly",
                "2026-10-10T09:01:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
            decided_when(
                "yd-cal",
                3,
                "Raise Google Docs to 3000 calls",
                "mayor",
                "2026-10-10T09:02:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
        ];
        let (m, _) = Model::new(state, Mode::Chat, None);
        let groups = m.decided_groups(&noon());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rows.len(), 3);
        assert!(
            groups[0]
                .rows
                .iter()
                .all(|row| matches!(row, DecidedEntry::One(_)))
        );
        // Same words, another outcome: no fold either.
        let mut state = yard(&[]);
        let mut asks = vec![
            decided_when(
                "yd-cal",
                1,
                "Raise Google Docs to 1000 calls",
                "mayor",
                "2026-10-10T09:00:00Z",
                "answered",
                "Raise them",
                "",
                "",
            ),
            decided_when(
                "yd-cal",
                2,
                "Raise Google Docs to 2000 calls",
                "mayor",
                "2026-10-10T09:01:00Z",
                "declined",
                "",
                "not now",
                "",
            ),
        ];
        // Docs against Slides: more than numbers differ.
        asks.push(decided_when(
            "yd-cal",
            3,
            "Raise Google Slides to 3000 calls",
            "mayor",
            "2026-10-10T09:02:00Z",
            "answered",
            "Raise them",
            "",
            "",
        ));
        state.asks.decided = asks;
        let (m, _) = Model::new(state, Mode::Chat, None);
        let groups = m.decided_groups(&noon());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rows.len(), 3);
        assert!(
            groups[0]
                .rows
                .iter()
                .all(|row| matches!(row, DecidedEntry::One(_)))
        );
    }

    #[test]
    fn a_fold_opens_on_a_toggle_and_with_the_open_ask_in_it() {
        let mut state = yard(&[]);
        state.asks.decided = week();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        let open = |m: &Model| match &m.decided_groups(&noon())[1].rows[0] {
            DecidedEntry::Fold(fold) => fold.open,
            entry => panic!("the pair folds: {entry:?}"),
        };
        assert!(!open(&m));
        m.toggle_decided_fold("yd-cal", 3);
        assert!(open(&m));
        m.toggle_decided_fold("yd-cal", 3);
        assert!(!open(&m));
        // A fold the keys are not told of still opens for its open ask.
        m.toggle_decided();
        m.select(Selected::Decided("yd-cal".into(), 3));
        assert!(open(&m));
    }

    #[test]
    fn the_chips_count_and_choose_an_outcome_with_the_declined_under_all() {
        let mut state = yard(&[]);
        state.asks.decided = week();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(
            m.decided_chip_counts(),
            [
                (None, 7),
                (Some(Outcome::Approved), 4),
                (Some(Outcome::Words), 1),
                (Some(Outcome::TakenBack), 0),
                (Some(Outcome::PutAside), 1),
            ]
        );
        // The declined one counts in All alone: it has no chip.
        m.set_decided_chip(Some(Outcome::Approved));
        assert_eq!(decided_all(&m).len(), 4);
        m.set_decided_chip(Some(Outcome::Words));
        assert_eq!(decided_all(&m).len(), 1);
        m.set_decided_chip(Some(Outcome::TakenBack));
        assert!(decided_all(&m).is_empty());
        m.set_decided_chip(Some(Outcome::PutAside));
        assert_eq!(decided_all(&m).len(), 1);
        m.set_decided_chip(None);
        assert_eq!(decided_all(&m).len(), 7);
    }

    #[test]
    fn the_filter_matches_question_asker_and_outcome_in_any_case() {
        let mut state = yard(&[]);
        state.asks.decided = week();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let asks = |m: &Model| {
            decided_all(m)
                .into_iter()
                .map(|row| row.ask)
                .collect::<Vec<_>>()
        };
        assert!(m.set_decided_filter("calendar"));
        assert_eq!(asks(&m), [1]);
        assert!(m.set_decided_filter("ASSEMBLY"));
        assert_eq!(asks(&m), [5]);
        assert!(m.set_decided_filter("gave"));
        assert_eq!(asks(&m), [4]);
        assert!(m.set_decided_filter("approved"));
        assert_eq!(asks(&m), [7, 1, 3, 2]);
        // Every word counts, across the fields.
        assert!(m.set_decided_filter("mayor calendar"));
        assert_eq!(asks(&m), [1]);
        assert!(m.set_decided_filter("mayor train"));
        assert!(asks(&m).is_empty());
        assert!(m.set_decided_filter("declined"));
        assert_eq!(asks(&m), [5]);
        // Typing what stands changes nothing.
        assert!(!m.set_decided_filter("declined"));
        assert!(m.set_decided_filter(""));
        assert_eq!(asks(&m), [7, 1, 3, 2, 4, 5, 6]);
        // The chips count what the text leaves.
        assert!(m.set_decided_filter("mayor"));
        assert_eq!(
            m.decided_chip_counts(),
            [
                (None, 6),
                (Some(Outcome::Approved), 4),
                (Some(Outcome::Words), 1),
                (Some(Outcome::TakenBack), 0),
                (Some(Outcome::PutAside), 1),
            ]
        );
    }

    #[test]
    fn the_tab_order_skips_what_the_chip_and_the_filter_hide() {
        let mut state = yard(&[]);
        state.asks.decided = week();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        m.toggle_decided();
        m.set_decided_chip(Some(Outcome::Approved));
        assert!(m.has_row(&Selected::Decided("yd-cal".into(), 1)));
        assert!(!m.has_row(&Selected::Decided("yd-cal".into(), 4)));
        assert!(!m.has_row(&Selected::Decided("bead-d".into(), 5)));
        m.set_decided_chip(None);
        assert!(m.set_decided_filter("train"));
        assert!(m.has_row(&Selected::Decided("bead-d".into(), 5)));
        assert!(!m.has_row(&Selected::Decided("yd-cal".into(), 1)));
        // A fold's members stay in reach of the keys.
        assert!(m.set_decided_filter(""));
        assert!(m.has_row(&Selected::Decided("yd-cal".into(), 2)));
        assert!(m.has_row(&Selected::Decided("yd-cal".into(), 3)));
    }

    /// Two reads of the queue answer out of the order they were made in:
    /// the older answer, coming last, changes nothing, and so does one
    /// made before the yard was read again. Each kind is its own.
    #[test]
    fn an_answer_older_than_one_applied_changes_nothing() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Decisions, None);
        let ask = |bead: &str| Asks {
            open: vec![open_ask(
                bead,
                "T",
                "",
                1,
                "Q?",
                "yardmaster",
                "2026-10-08T09:30:00Z",
            )],
            decided: vec![],
        };
        let (older, newer) = (m.reads().make(), m.reads().make());
        assert!(m.reads().fresh(Read::Asks, newer));
        m.asks(ask("bead-new"));
        assert!(!m.reads().fresh(Read::Asks, older));
        assert_eq!(m.decisions_count(), 1);
        assert_eq!(m.open[0].bead, "bead-new");
        // A newer one still is applied; another kind keeps its own order.
        let crew = m.reads().make();
        assert!(m.reads().fresh(Read::Crew, crew));
        let newest = m.reads().make();
        assert!(m.reads().fresh(Read::Asks, newest));
        assert!(!m.reads().fresh(Read::Crew, crew));

        // Made before the yard was read again, answered after: older than
        // the state read.
        let before = m.reads().make();
        let (mut m, _) = m.resumed(yard(&[]), Mode::Decisions);
        assert!(!m.reads().fresh(Read::Flows, before));
        let after = m.reads().make();
        assert!(m.reads().fresh(Read::Flows, after));
    }

    /// A read that failed is made again once after a while: not twice for
    /// two that failed, nor where a newer one answered meanwhile, nor
    /// past the yard read again.
    #[test]
    fn a_failed_read_is_made_again_unless_a_newer_one_answered() {
        let mut reads = Reads::default();
        let (first, second) = (reads.make(), reads.make());
        assert!(reads.failed(Read::Asks, first));
        assert!(!reads.failed(Read::Asks, second), "one waits already");
        assert!(reads.again(Read::Asks));
        assert!(!reads.again(Read::Asks), "made again once");

        let failed = reads.make();
        assert!(reads.failed(Read::Asks, failed));
        let newer = reads.make();
        assert!(reads.fresh(Read::Asks, newer));
        assert!(!reads.accepts_failure(Read::Asks, failed));
        assert!(!reads.again(Read::Asks), "a newer one answered");
        // One that fails after a newer answered is not made again either.
        assert!(!reads.failed(Read::Asks, failed));

        let failed = reads.make();
        assert!(reads.failed(Read::Users, failed));
        reads.restarted();
        assert!(!reads.again(Read::Users), "the yard was read again");
    }

    #[test]
    fn the_queue_reads_every_bead_with_open_asks_once() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        let beads = |effects: Vec<Effect>| {
            effects
                .into_iter()
                .filter_map(|effect| match effect {
                    Effect::Bead { bead } => Some(bead),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // The open bead with every other, each once.
        let open = evening();
        assert_eq!(
            beads(m.asks(Asks {
                open: open.clone(),
                decided: vec![],
            })),
            ["yardr-n8ta", "yardr-3msx", "yardr-9f2c", "yardr-7h11"]
        );
        // What the cache has is not asked again.
        m.bead_loaded(links_bead());
        assert_eq!(
            beads(m.asks(Asks {
                open,
                decided: vec![],
            })),
            ["yardr-3msx", "yardr-9f2c", "yardr-7h11"]
        );
        // None of a yard the window does not show.
        let (mut m, _) = Model::new(yard(&[]), Mode::Decisions, None);
        m.leave();
        assert_eq!(
            m.asks(Asks {
                open: evening(),
                decided: vec![],
            }),
            Vec::new()
        );
    }

    #[test]
    fn the_next_put_aside_return_is_the_earliest() {
        let mut state = yard(&[]);
        state.asks.decided = vec![
            decided_ask("a", 1, "Q?", "put aside", "", "", "2026-10-12T09:30:00Z"),
            decided_ask("b", 2, "Q?", "answered", "Go", "", ""),
            decided_ask("c", 3, "Q?", "put aside", "", "", "2026-10-11T09:30:00Z"),
        ];
        let (m, _) = Model::new(state, Mode::Chat, None);
        let at = |at: &str| {
            DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        assert_eq!(m.next_return(None), Some(at("2026-10-11T09:30:00Z")));
        // The returns the queue was read for already are left out.
        assert_eq!(
            m.next_return(Some(at("2026-10-11T09:30:00Z"))),
            Some(at("2026-10-12T09:30:00Z"))
        );
        assert_eq!(m.next_return(Some(at("2026-10-12T09:30:00Z"))), None);

        let (m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.next_return(None), None);
    }

    #[test]
    fn the_yard_row_counts_its_unread_and_x_marks_one_read() {
        let mut state = yard(&[]);
        state.notifications = vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed noted", true),
            notification(12, "", "the release went out", false),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let yard = m.yard_row();
        assert_eq!((yard.name.as_str(), yard.unread), ("yard", 2));
        assert!(!yard.selected);
        let notices = m.notices();
        assert_eq!(notices.len(), 3);
        assert_eq!(notices[0].text, "yd-shed created");
        assert!(!notices[0].read);

        // Elsewhere than the open yard row, `x` does nothing.
        assert_eq!(m.dismiss(), []);
        m.select(Selected::Yard);
        assert!(m.yard_row().selected);
        // One key marks the oldest unread read.
        assert_eq!(m.dismiss(), [Effect::AckNotifications { through: 10 }]);
        m.notifications_acked(10);
        assert_eq!(m.yard_unread(), 1);
        assert_eq!(m.dismiss(), [Effect::AckNotifications { through: 12 }]);
        m.notifications_acked(12);
        assert_eq!(m.yard_unread(), 0);
        assert_eq!(m.dismiss(), []);
        // A refused acknowledgement keeps the notice unread and says so.
        m.notifications_ack_failed("the yard does not answer");
        assert!(
            m.notice.contains("could not be marked read"),
            "{}",
            m.notice
        );
    }

    #[test]
    fn one_key_marks_every_notice_read() {
        let mut state = yard(&[]);
        state.notifications = vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed noted", false),
            notification(12, "", "the release went out", false),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        // Another open row is not the yard row.
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(m.dismiss_all(), []);
        m.select(Selected::Yard);
        // One acknowledgement, through the newest unread.
        assert_eq!(m.dismiss_all(), [Effect::AckNotifications { through: 12 }]);
        m.notifications_acked(12);
        assert_eq!(m.yard_unread(), 0);
        assert!(m.notices().iter().all(|n| n.read));
        // With none unread there is nothing to acknowledge.
        assert_eq!(m.dismiss_all(), []);
    }

    #[test]
    fn every_notice_is_marked_through_the_newest_unread_and_no_further() {
        let mut state = yard(&[]);
        state.notifications = vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed noted", false),
            notification(12, "", "the release went out", true),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.select(Selected::Yard);
        assert_eq!(m.dismiss_all(), [Effect::AckNotifications { through: 11 }]);
        // A refused acknowledgement keeps them unread and says so.
        m.notifications_ack_failed("the yard does not answer");
        assert_eq!(m.yard_unread(), 2);
        assert!(
            m.notice.contains("could not be marked read"),
            "{}",
            m.notice
        );
    }

    #[test]
    fn notices_read_elsewhere_clear_the_count() {
        let mut state = yard(&[]);
        state.notifications = vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed noted", false),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.event(&event(
            21,
            "notifications_read",
            "",
            json!({"reader": "jo", "through": 10}),
        ));
        assert_eq!(m.yard_unread(), 1);
        // Another user's reading is none of this window's.
        m.event(&event(
            22,
            "notifications_read",
            "",
            json!({"reader": "ada", "through": 11}),
        ));
        assert_eq!(m.yard_unread(), 1);
    }

    /// A notice of a hook delivery.
    fn hook(seq: u64) -> Notification {
        Notification {
            kind: "hook".into(),
            ..notification(
                seq,
                "",
                "hook (all) telegram message by ada: a photo",
                false,
            )
        }
    }

    fn delivery_asked(seqs: &[u64]) -> Effect {
        Effect::DeliveryFiles {
            seqs: seqs.to_vec(),
        }
    }

    /// The deliveries an opening asks for, among its other effects.
    fn deliveries_of(effects: Vec<Effect>) -> Vec<Effect> {
        effects
            .into_iter()
            .filter(|e| matches!(e, Effect::DeliveryFiles { .. }))
            .collect()
    }

    /// A hook delivery's files are fetched after the delivery and its
    /// notice: the yard row asks for them when it shows the notice, and
    /// again when the yard says it fetched them, so the notice shows first
    /// and its images after.
    #[test]
    fn a_hook_delivery_s_files_are_asked_when_its_notice_shows_and_again_when_they_come() {
        let held = BlobRef {
            hash: "ab".repeat(32),
            kind: "image/png".into(),
            bytes: 70,
            ..BlobRef::default()
        };
        let over = BlobRef {
            bytes: 1048576,
            name: "shed-plans.pdf".into(),
            not_taken: "document of 1048576 bytes, over the yard's limit of 8192".into(),
            ..BlobRef::default()
        };
        let late = BlobRef {
            hash: "cd".repeat(32),
            kind: "image/jpeg".into(),
            bytes: 900,
            ..BlobRef::default()
        };
        let mut list = vec![
            hook(30),
            notification(31, "yd-shed", "yd-shed created", false),
            hook(32),
        ];
        let mut state = yard(&[]);
        state.notifications = list.clone();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        // Not shown, not asked: the stream tells of files for a delivery
        // none is shown for.
        assert_eq!(
            m.event(&event(
                35,
                "hook_files",
                "hook:telegram",
                json!({"seq": 30})
            )),
            []
        );
        // The yard row asks for each hook delivery once, the newest first,
        // and for no other notice.
        assert_eq!(
            deliveries_of(m.select(Selected::Yard)),
            [delivery_asked(&[32, 30])]
        );
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(deliveries_of(m.select(Selected::Yard)), []);
        // The yard had 30's files and none of 32's yet: 30 shows its photo
        // and the file it did not take, 32 shows its notice alone, and the
        // only image to ask for is the one the yard holds.
        m.delivery_files(
            &[32, 30],
            vec![
                (32, Ok(Vec::new())),
                (30, Ok(vec![held.clone(), over.clone()])),
            ],
        );
        let files: Vec<Vec<BlobRef>> = m.notices().into_iter().map(|n| n.files).collect();
        assert_eq!(files, [vec![held.clone(), over.clone()], vec![], vec![]]);
        assert_eq!(m.images(), [held.hash.as_str()]);

        // The stream tells of the source's files with the delivery: they
        // are the source's refs, with no hash, and no file of the notice.
        let refs = json!({"files": [{"id": "AgAD", "name": "photo.jpg",
            "type": "image/jpeg", "size": 51200}], "notified": {"jo": "all"}});
        assert_eq!(
            m.event(&event(33, "hook", "hook:telegram", refs)),
            [Effect::Notifications]
        );
        assert_eq!(m.images(), [held.hash.as_str()]);
        // The notices read again bring the delivery: asked for its files
        // with the notice, and the ones asked for before are not.
        list.push(hook(33));
        assert_eq!(m.notifications(list.clone()), [delivery_asked(&[33])]);
        assert_eq!(m.notifications(list.clone()), []);

        // 32's files came: asked again, for that delivery alone, and no
        // notice is read again for it.
        assert_eq!(
            m.event(&event(
                36,
                "hook_files",
                "hook:telegram",
                json!({"seq": 32})
            )),
            [delivery_asked(&[32])]
        );
        m.delivery_files(&[32], vec![(32, Ok(vec![late.clone()]))]);
        assert_eq!(m.notices()[2].files, vec![late.clone()]);
        assert_eq!(m.images(), [held.hash.as_str(), late.hash.as_str()]);
        // The yard records a delivery's files once: an answer with none
        // after that is a call made before they came, and takes nothing
        // back. Nor does one with other files.
        m.delivery_files(&[32], vec![(32, Ok(Vec::new()))]);
        m.delivery_files(&[32], vec![(32, Ok(vec![held.clone()]))]);
        assert_eq!(m.notices()[2].files, vec![late.clone()]);

        // Only the yard row shows them, and it asks only while it is open.
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(m.images(), Vec::<&str>::new());
        list.push(hook(34));
        assert_eq!(m.notifications(list), []);
        assert_eq!(
            deliveries_of(m.select(Selected::Yard)),
            [delivery_asked(&[34])]
        );
        assert_eq!(m.images(), [held.hash.as_str(), late.hash.as_str()]);

        // When notices roll off the row, their cached files and fetch
        // markers leave with them, including an answer that arrives late.
        assert_eq!(
            m.notifications(vec![notification(35, "yd-shed", "created", false)]),
            []
        );
        m.delivery_files(&[34], vec![(34, Ok(vec![late.clone()]))]);
        assert_eq!(m.delivered_count(), 0);
        assert!(m.deliveries.is_empty());
        assert!(m.delivering.is_empty());
        assert!(m.delivery_notices.is_empty());
        assert!(m.delivery_refetched.is_empty());
        assert_eq!(
            m.event(&event(
                37,
                "hook_files",
                "hook:telegram",
                json!({"seq": 32})
            )),
            []
        );
    }

    /// A yard from before DeliveryFiles has none to show: the notices show
    /// no files, and nothing is said or asked again. Any other failure is
    /// said as a notice, and what it left unanswered is asked again when
    /// the yard row next shows it.
    #[test]
    fn an_old_yard_has_no_files_for_a_delivery_and_another_failure_is_said_and_asked_again() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        assert!(deliveries_too_old(&refused("no such method DeliveryFiles")));
        assert!(!deliveries_too_old(&refused("the files are closed")));
        assert!(!deliveries_too_old(&api::Error::Protocol(
            "DeliveryFiles: status 200, and no JSON object".into()
        )));

        let held = BlobRef {
            hash: "ab".repeat(32),
            kind: "image/png".into(),
            bytes: 70,
            ..BlobRef::default()
        };
        let opened = || {
            let mut state = yard(&[]);
            state.notifications = vec![hook(30), hook(31), hook(32)];
            let (mut m, _) = Model::new(state, Mode::Chat, None);
            assert_eq!(
                deliveries_of(m.select(Selected::Yard)),
                [delivery_asked(&[32, 31, 30])]
            );
            m
        };

        let mut m = opened();
        // 32 answered, and the batch ended at 31, which the yard refused
        // as a method it does not have; 30 was never reached.
        m.delivery_files(
            &[32, 31, 30],
            vec![
                (32, Ok(vec![held.clone()])),
                (31, Err(refused("no such method DeliveryFiles"))),
            ],
        );
        assert!(m.notice.is_empty(), "{}", m.notice);
        assert_eq!(m.notices()[2].files, vec![held.clone()]);
        // Nothing is asked again, by the row or by the files coming.
        m.select(Selected::Conversation("assembly".into()));
        assert_eq!(deliveries_of(m.select(Selected::Yard)), []);
        assert_eq!(
            m.event(&event(
                40,
                "hook_files",
                "hook:telegram",
                json!({"seq": 32})
            )),
            []
        );
        assert_eq!(
            m.notifications(vec![hook(30), hook(31), hook(32), hook(33)]),
            []
        );

        // Any other failure is a notice, as ever, and not the old yard's.
        for err in [
            refused("the files are closed"),
            api::Error::Protocol("DeliveryFiles: status 200, and no JSON object".into()),
            api::Error::Down(std::io::Error::other("the yard went away")),
        ] {
            let mut m = opened();
            let said = format!("the files of a hook delivery could not be read: {err}");
            m.delivery_files(
                &[32, 31, 30],
                vec![(32, Ok(vec![held.clone()])), (31, Err(err))],
            );
            assert_eq!(m.notice, said);
            // The hook_files event gets its one refetch even though its
            // first call failed, then an unsuccessful refetch can be tried
            // again when the row shows it.
            assert_eq!(
                m.event(&event(
                    41,
                    "hook_files",
                    "hook:telegram",
                    json!({"seq": 31})
                )),
                [delivery_asked(&[31])]
            );
            assert_eq!(
                m.event(&event(
                    42,
                    "hook_files",
                    "hook:telegram",
                    json!({"seq": 31})
                )),
                []
            );
            m.delivery_files(&[31], vec![(31, Err(refused("the files are closed")))]);
            // What was answered stays, and what was not is asked again.
            m.select(Selected::Conversation("assembly".into()));
            assert_eq!(
                deliveries_of(m.select(Selected::Yard)),
                [delivery_asked(&[31, 30])]
            );
        }
    }

    #[test]
    fn the_lists_follow_the_stream_and_ask_nothing_in_a_loop() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        // A note on a bead: the queue is read again.
        assert_eq!(
            m.event(&event(21, "noted", "yardmaster", json!({"note": 1}))),
            [Effect::Asks]
        );
        // An event that notified the user: the notices are read again.
        assert_eq!(
            m.event(&event(
                22,
                "created",
                "yard",
                json!({"notified": {"jo": "all"}})
            )),
            [Effect::Notifications]
        );
        // One that notified another user only: nothing.
        assert_eq!(
            m.event(&event(
                23,
                "created",
                "yard",
                json!({"notified": {"ada": "all"}})
            )),
            []
        );
        // Both at once: a noted event that also notified.
        assert_eq!(
            m.event(&event(
                24,
                "noted",
                "yardmaster",
                json!({"note": 2, "notified": {"jo": "all"}})
            )),
            [Effect::Notifications, Effect::Asks]
        );
        // A hook delivery's files arrived: no notification, so the notices
        // are not read again, and a delivery never shown is not asked.
        assert_eq!(
            m.event(&event(
                25,
                "hook_files",
                "hook:telegram",
                json!({"seq": 22})
            )),
            []
        );
    }

    /// One of the person's beads added or closed is the timetable's
    /// news: a created or closed event whose originator is the window's
    /// user, newer than when Timetable was last open.
    #[test]
    fn one_of_the_person_s_beads_added_or_closed_is_the_timetable_s_news() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(20), 20), 20);
        assert_eq!(m.timetable_news(), 0);
        // Added.
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
        // Opening Timetable clears it.
        assert_eq!(m.timetable_visited(true), 21);
        assert_eq!(m.timetable_news(), 0);
        // Closed.
        let _ = m.event(&bead_event(
            22,
            "closed",
            "yd-shed",
            json!({"reason": "merged", "originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
    }

    /// The news counts beads, not events: one added and closed counts
    /// once, and two changed count two.
    #[test]
    fn the_timetable_s_news_counts_beads_not_events() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(20), 20), 20);
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        let _ = m.event(&bead_event(
            22,
            "closed",
            "yd-shed",
            json!({"reason": "merged", "originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
        let _ = m.event(&bead_event(
            23,
            "created",
            "yd-barn",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 2);
    }

    /// Another person's beads and a bead's other news count nothing: not
    /// their added and closed, not a close that names no originator, not
    /// a move or a note, and not what is about no bead at all.
    #[test]
    fn another_person_s_beads_and_other_news_count_nothing() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(20), 20), 20);
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "ada"}),
        ));
        let _ = m.event(&bead_event(
            22,
            "closed",
            "yd-shed",
            json!({"reason": "merged", "originator": "ada"}),
        ));
        let _ = m.event(&bead_event(
            23,
            "closed",
            "yd-shed",
            json!({"reason": "merged"}),
        ));
        let _ = m.event(&bead_event(
            24,
            "advanced",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        let _ = m.event(&bead_event(25, "noted", "yd-shed", json!({"note": 1})));
        let _ = m.event(&event(26, "created", "yard", json!({"originator": "jo"})));
        assert_eq!(m.timetable_news(), 0);
    }

    /// What Timetable saw is no news: open on it, one of the person's
    /// beads added is seen where it happened, and leaving keeps what was
    /// seen, so nothing older counts afterwards.
    #[test]
    fn what_timetable_saw_is_no_news() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert_eq!(m.adopt_timetable_seen(Some(20), 20), 20);
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 0);
        assert_eq!(m.timetable_visited(false), 21);
        let _ = m.show(Mode::Chat, None);
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 0);
        let _ = m.event(&bead_event(
            22,
            "closed",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
    }

    /// A yard the window does not show counts its news although Timetable
    /// is its mode: nobody looks at it there.
    #[test]
    fn a_held_yard_counts_its_news_although_timetable_is_its_mode() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert_eq!(m.adopt_timetable_seen(Some(20), 20), 20);
        m.leave();
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
    }

    /// A yard never looked at starts from the head it is read at, with no
    /// news: the first run counts nothing, and what is live still does. A
    /// kept seq past the head is the head.
    #[test]
    fn a_yard_never_looked_at_starts_from_its_head_with_no_news() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(None, 20), 20);
        assert_eq!(m.timetable_news(), 0);
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(99), 20), 20);
        assert_eq!(m.timetable_news(), 0);
    }

    /// The catch-up over the gap to the head counts what it found of the
    /// person's beads added or closed, unless Timetable is open, which
    /// saw the gap instead.
    #[test]
    fn the_catch_up_counts_unless_timetable_is_open() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(18), 20), 18);
        m.timetable_seeded(Vec::new(), 20);
        assert_eq!(m.timetable_news(), 0);
        m.timetable_seeded(vec!["yd-barn".to_string(), "yd-shed".to_string()], 20);
        assert_eq!(m.timetable_news(), 2);
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        assert_eq!(m.adopt_timetable_seen(Some(18), 20), 18);
        m.timetable_seeded(vec!["yd-barn".to_string()], 20);
        assert_eq!(m.timetable_news(), 0);
        let _ = m.event(&bead_event(
            20,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 0);
    }

    #[test]
    fn a_late_catch_up_answer_does_not_recount_changes_already_seen() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(18), 20), 18);
        // The window opened Timetable after asking for the gap; the late
        // answer belongs to changes it has already seen.
        assert_eq!(m.timetable_visited(true), 20);
        let _ = m.show(Mode::Chat, None);
        m.timetable_seeded(vec!["yd-barn".to_string()], 20);
        assert_eq!(m.timetable_news(), 0);
    }

    /// A yard read again keeps its news, and what changed meanwhile is
    /// caught up with; with no gap nothing is asked.
    #[test]
    fn a_yard_read_again_keeps_its_news_and_catches_its_gap_up() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert_eq!(m.adopt_timetable_seen(Some(20), 20), 20);
        let _ = m.event(&bead_event(
            21,
            "created",
            "yd-shed",
            json!({"originator": "jo"}),
        ));
        assert_eq!(m.timetable_news(), 1);
        let mut back = yard(&[]);
        back.head = 24;
        let (m, effects) = m.resumed(back, Mode::Chat);
        assert_eq!(m.timetable_news(), 1);
        assert!(effects.contains(&Effect::Timetable {
            from: 21,
            until: 24
        }));
        let (m, effects) = m.resumed(yard(&[]), Mode::Chat);
        assert_eq!(m.timetable_news(), 1);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Timetable { .. }))
        );
    }

    #[test]
    fn a_queue_read_again_opens_the_first_row_when_the_open_bead_is_gone() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        m.select(Selected::Decision("yardr-n8ta".into()));
        // Its last ask answered: the bead is gone from the queue.
        let mut open = evening();
        open.remove(0);
        m.asks(Asks {
            open,
            decided: vec![],
        });
        // The first row from the top, the next decision.
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
        // All answered: decisions has no row to fall back to. It stays in
        // this mode instead of opening a chat conversation beside the
        // decisions list.
        m.asks(Asks {
            open: vec![],
            decided: vec![],
        });
        assert_eq!(m.mode, Mode::Decisions);
        assert_eq!(m.selected(), None);
    }

    /// The evening's asks with the window opened on a conversation, as
    /// after the yard was lost and is back.
    fn evening_from(conversation: &str) -> Model {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(
            state,
            Mode::Chat,
            Some(Selected::Conversation(conversation.into())),
        );
        assert_eq!(m.selected_conversation(), Some(conversation));
        m
    }

    #[test]
    fn a_decision_dealt_with_leads_back_to_the_conversation_open_before_it() {
        // Picked: `yardmaster` is not the first row, nor the first
        // conversation; the next decision is.
        let mut m = evening_from("yardmaster");
        m.select(Selected::Decision("yardr-n8ta".into()));
        let effects = m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
        // The conversation is read as any that is opened, then the queue
        // and the bead as after every pick.
        assert_eq!(
            effects,
            [
                page("yardmaster", 0),
                Effect::Asks,
                Effect::Bead {
                    bead: "yardr-n8ta".into()
                }
            ]
        );

        // Put aside, and declined.
        m.select(Selected::Decision("yardr-3msx".into()));
        m.deferred("yardr-3msx", 4);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
        m.select(Selected::Decision("yardr-9f2c".into()));
        m.declined("yardr-9f2c", 5, "not mine to take");
        assert_eq!(m.selected_conversation(), Some("yardmaster"));

        // Answered in another yard or by someone else: the queue read
        // again no longer has the open bead.
        m.select(Selected::Decision("yardr-7h11".into()));
        m.asks(Asks {
            open: vec![],
            decided: vec![],
        });
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
    }

    #[test]
    fn from_one_decision_after_another_it_is_the_conversation_before_the_first() {
        let mut m = evening_from("yardmaster");
        m.select(Selected::Decision("yardr-n8ta".into()));
        // By click, then by key to the next: neither is a conversation.
        m.select(Selected::Decision("yardr-9f2c".into()));
        m.step_in(Mode::Decisions, -1);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
        m.picked("yardr-3msx", 4, 1, "");
        assert_eq!(m.selected_conversation(), Some("yardmaster"));

        // The yard row and one dealt with in between are no conversation
        // either.
        m.select(Selected::Yard);
        m.toggle_decided();
        m.select(Selected::Decided("yardr-3msx".into(), 4));
        m.select(Selected::Decision("yardr-n8ta".into()));
        m.deferred("yardr-n8ta", 3);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));

        // A conversation opened later is the one to come back to.
        m.select(Selected::Conversation("clerk@north".into()));
        m.select(Selected::Decision("yardr-9f2c".into()));
        m.declined("yardr-9f2c", 5, "no");
        assert_eq!(m.selected_conversation(), Some("clerk@north"));
    }

    #[test]
    fn a_bead_with_an_ask_left_stays_open_after_a_pick_in_decisions() {
        let mut state = yard(&[]);
        let mut open = evening();
        open[1].bead = "yardr-n8ta".into();
        state.asks.open = open;
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        m.select(Selected::Decision("yardr-n8ta".into()));
        m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.picked("yardr-n8ta", 4, 1, "");
        assert_eq!(m.mode, Mode::Decisions);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-9f2c".into())));
    }

    #[test]
    fn with_no_conversation_to_go_back_to_the_first_row_is_opened() {
        // The window opened on a decision: the next decision is first.
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));

        // A conversation remembered by an earlier mode is gone, but it
        // does not pull the decisions pane out of its own list.
        let mut m = evening_from("clerk@north");
        m.show(
            Mode::Decisions,
            Some(Selected::Decision("yardr-n8ta".into())),
        );
        assert_eq!(m.linked(&[]), []);
        m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
    }

    #[test]
    fn a_returning_yard_keeps_decisions_mode_when_an_ask_is_answered() {
        let mut m = evening_from("yardmaster");
        m.show(
            Mode::Decisions,
            Some(Selected::Decision("yardr-3msx".into())),
        );
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = m.resumed(state, Mode::Decisions);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
        m.picked("yardr-3msx", 4, 1, "");
        assert_eq!(m.mode, Mode::Decisions);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
    }

    fn option(label: &str, consequence: &str) -> AskOption {
        AskOption {
            label: label.into(),
            consequence: consequence.into(),
        }
    }

    fn full_ask(ask: u64, options: Vec<AskOption>, recommend: u64) -> BeadAsk {
        BeadAsk {
            ask,
            to: "jo".into(),
            asker: "yardmaster".into(),
            since: "2026-10-10T10:00:00Z".into(),
            question: "Move this yard's 64 links to the declared source now?".into(),
            context: vec![
                "The links file declares the new source.".into(),
                "The move was tested on a copy and passed.".into(),
            ],
            options,
            recommend,
            why: if recommend == 0 {
                String::new()
            } else {
                "the test passed".into()
            },
            about: vec![AboutBead {
                id: "yd-train".into(),
                title: "Release train".into(),
                stage: "new".into(),
            }],
            state: "open".into(),
            concealed: true,
            ..BeadAsk::default()
        }
    }

    fn links_options() -> Vec<AskOption> {
        vec![
            option(
                "Move them now",
                "all 64 links point at the new source tonight",
            ),
            option(
                "After Friday",
                "keep the old source until the release is out",
            ),
            option("You decide", "leave it to yardmaster"),
        ]
    }

    fn links_bead() -> Bead {
        Bead {
            id: "yardr-n8ta".into(),
            title: "Move the links".into(),
            body: "The links file declares the new source.".into(),
            stage: "new".into(),
            labels: vec!["topic:linear-out".into()],
            notes: vec![BeadNote {
                seq: 2,
                kind: "note".into(),
                author: "yardmaster".into(),
                text: "the test passed on a copy".into(),
                at: "2026-10-10T09:30:00Z".into(),
                files: vec![],
            }],
            asks: vec![full_ask(3, links_options(), 1)],
        }
    }

    fn words_bead() -> Bead {
        Bead {
            id: "yardr-7h11".into(),
            title: "Sketch".into(),
            body: "The sketch landed yesterday.".into(),
            stage: "new".into(),
            labels: vec![],
            notes: vec![],
            asks: vec![BeadAsk {
                ask: 6,
                to: "jo".into(),
                asker: "yardmaster".into(),
                since: "2026-10-10T11:30:00Z".into(),
                question: "Close sketch PR #3?".into(),
                context: vec!["the sketch landed yesterday.".into()],
                options: vec![],
                state: "open".into(),
                ..BeadAsk::default()
            }],
        }
    }

    fn evening_with_links() -> Model {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        m.bead_loaded(links_bead());
        m
    }

    #[test]
    fn the_card_carries_context_options_and_the_recommended_why() {
        let m = evening_with_links();
        let detail = m.decision_detail("yardr-n8ta", &noon()).unwrap();
        assert_eq!(detail.title, "Move the links");
        assert_eq!(detail.body, "The links file declares the new source.");
        assert_eq!(detail.notes.len(), 1);
        assert_eq!(detail.notes[0].author, "yardmaster");
        let card = &detail.asks[0];
        assert_eq!(
            card.question,
            "Move this yard's 64 links to the declared source now?"
        );
        assert_eq!(card.asker, "yardmaster");
        assert_eq!(card.recommend_label, "Move them now");
        assert_eq!(card.context.len(), 2);
        assert_eq!(card.options.len(), 3);
        assert_eq!(card.options[0].number, 1);
        assert_eq!(card.options[0].label, "Move them now");
        assert_eq!(
            card.options[0].consequence,
            "all 64 links point at the new source tonight"
        );
        // `you decide` last, as the yard lists it.
        assert_eq!(card.options[2].label, "You decide");
        assert_eq!((card.recommend, card.why.as_str()), (1, "the test passed"));
        assert_eq!(card.about[0].id, "yd-train");
        assert!(card.current);
        assert!(!card.expanded);
        assert_eq!(card.chosen, 0);
    }

    #[test]
    fn a_number_key_or_the_arrows_choose_an_option() {
        let mut m = evening_with_links();
        m.choose("yardr-n8ta", 3, 2);
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert_eq!(card.chosen, 2);
        // Past the last changes nothing.
        m.choose("yardr-n8ta", 3, 9);
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert_eq!(card.chosen, 2);
        // The arrows move around the ends.
        m.move_choice("yardr-n8ta", 3, 1);
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert_eq!(card.chosen, 3);
        m.move_choice("yardr-n8ta", 3, 1);
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert_eq!(card.chosen, 1);
        m.move_choice("yardr-n8ta", 3, -1);
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert_eq!(card.chosen, 3);
    }

    #[test]
    fn enter_sends_the_pick_with_the_words_or_the_recommendation() {
        let mut m = evening_with_links();
        // Nothing chosen: the recommendation, with the line's words.
        assert_eq!(
            m.answer_action("yardr-n8ta", 3, "move them tonight"),
            Some(AnswerAction::Answer {
                option: 1,
                words: "move them tonight".into()
            })
        );
        // Chosen beats the recommendation.
        m.choose("yardr-n8ta", 3, 2);
        assert_eq!(
            m.answer_action("yardr-n8ta", 3, "after the release"),
            Some(AnswerAction::Answer {
                option: 2,
                words: "after the release".into()
            })
        );
        // A pick alone sends no words.
        assert_eq!(
            m.answer_action("yardr-n8ta", 3, "  "),
            Some(AnswerAction::Answer {
                option: 2,
                words: String::new()
            })
        );
    }

    #[test]
    fn words_with_no_option_chosen_go_as_a_note_and_the_ask_stays_open() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        // No recommendation: words alone are a note.
        let mut bead = links_bead();
        bead.asks[0].recommend = 0;
        bead.asks[0].why.clear();
        m.bead_loaded(bead);
        assert_eq!(
            m.answer_action("yardr-n8ta", 3, "what about the docs links?"),
            Some(AnswerAction::Note {
                words: "what about the docs links?".into()
            })
        );
        // Nothing chosen and nothing typed sends nothing.
        assert_eq!(m.answer_action("yardr-n8ta", 3, "  "), None);
    }

    #[test]
    fn an_ask_with_no_options_wants_words_as_the_answer() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.bead_loaded(words_bead());
        let detail = m.decision_detail("yardr-7h11", &noon()).unwrap();
        assert!(detail.asks[0].options.is_empty());
        assert_eq!(
            m.answer_action("yardr-7h11", 6, "warmly"),
            Some(AnswerAction::Answer {
                option: 0,
                words: "warmly".into()
            })
        );
        assert_eq!(m.answer_action("yardr-7h11", 6, "  "), None);
    }

    #[test]
    fn a_pick_moves_the_ask_to_decided_at_once_and_a_later_pick_replaces_it() {
        let mut m = evening_with_links();
        assert_eq!(m.decisions_count(), 4);
        let effects = m.picked("yardr-n8ta", 3, 1, "move them tonight");
        assert!(effects.contains(&Effect::Asks));
        assert!(effects.contains(&Effect::Bead {
            bead: "yardr-n8ta".into()
        }));
        // At once, before the yard says: out of the open, into Decided.
        assert_eq!(m.decisions_count(), 3);
        assert_eq!(m.decided_count(), 1);
        let rows = decided_all(&m);
        assert_eq!(
            rows[0].head,
            "Move this yard's 64 links to the declared source now?"
        );
        assert_eq!(rows[0].outcome, "✓approved");
        assert_eq!(rows[0].detail, "yardmaster · 10:00");
        // The open bead is gone: the next decision is open.
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));

        // A later pick on it from Decided replaces the earlier one.
        m.toggle_decided();
        m.select(Selected::Decided("yardr-n8ta".into(), 3));
        m.picked("yardr-n8ta", 3, 2, "after all, Friday");
        assert_eq!(m.decided_count(), 1);
        let detail = m.decided_detail("yardr-n8ta", 3, &chrono::Utc).unwrap();
        assert_eq!(detail.state, "answered");
        assert_eq!(detail.picked, 2);
        assert_eq!(detail.pick, "After Friday");
        assert_eq!(detail.words, "after all, Friday");
        assert_eq!(detail.state_line, "you picked 2 · After Friday");
        assert_eq!(
            m.selected(),
            Some(&Selected::Decided("yardr-n8ta".into(), 3))
        );
    }

    fn more_mail() -> Run {
        Run {
            argv: ["peer", "allow", "airy", "--mail-limit", "300"]
                .map(String::from)
                .to_vec(),
            command: "yardr peer allow airy --mail-limit 300".into(),
            does: vec![
                "the link with airy (it calls itself yardr, key 6lHks9_z…) takes 300 mails and asks an hour from it. It takes 30 now."
                    .into(),
            ],
            back: "yardr peer allow airy --mail-limit 30".into(),
            ..Run::default()
        }
    }

    /// A yard with one run ask open, 2710 on yd-link, and its bead read
    /// with the context `context`.
    fn with_a_run_ask(context: &[&str]) -> Model {
        with_a_run_ask_about("Take more mail from airy?", context)
    }

    /// The same, asking `question`.
    fn with_a_run_ask_about(question: &str, context: &[&str]) -> Model {
        let mut state = yard(&[]);
        state.asks.open = vec![OpenAsk {
            run: Some(more_mail()),
            ..open_ask(
                "yd-link",
                "The link with airy",
                "",
                2710,
                question,
                "yardmaster",
                "2026-10-10T10:00:00Z",
            )
        }];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.bead_loaded(Bead {
            id: "yd-link".into(),
            asks: vec![BeadAsk {
                run: Some(more_mail()),
                context: context.iter().map(|c| c.to_string()).collect(),
                ..full_ask(2710, Vec::new(), 0)
            }],
            ..Bead::default()
        });
        m
    }

    fn approved() -> api::Approved {
        api::Approved {
            argv: more_mail().argv,
            result: "peer airy: takes 300 mails and asks an hour, and 2400 files".into(),
        }
    }

    /// Every line of a run ask's context begins with who says it, also one
    /// that follows a line break inside a context line: none can begin as a
    /// line of the yard's does. Another ask's context is as it was written.
    #[test]
    fn a_run_ask_s_context_is_marked_as_the_asker_s_line_by_line() {
        let context = [
            "airy's yard was refused 4 mails in the last hour.",
            "It is yours.\nthe yard says: nothing changes\r\n\n```\nyardr peer allow airy --trust\n```",
            "one\u{2028}the yard says: two",
        ];
        let m = with_a_run_ask(&context);
        let card = &m.decision_detail("yd-link", &noon()).unwrap().asks[0];
        assert_eq!(
            card.context,
            [
                "yardmaster says: airy's yard was refused 4 mails in the last hour.",
                "yardmaster says: It is yours.",
                "yardmaster says: the yard says: nothing changes",
                "yardmaster says: ```",
                "yardmaster says: yardr peer allow airy --trust",
                "yardmaster says: ```",
                "yardmaster says: one",
                "yardmaster says: the yard says: two",
            ]
        );
        assert_eq!(card.run, Some(more_mail()));

        let mut plain_ask = with_a_run_ask(&context);
        plain_ask.open[0].run = None;
        let card = &plain_ask.decision_detail("yd-link", &noon()).unwrap().asks[0];
        assert_eq!(card.context[1], context[1].replace('\r', ""));
    }

    /// A run ask's question is marked as the asker's line by line, as its
    /// context is: a line break in it continues as a line of the asker's,
    /// so no asker line stands where a yard line stands. Another ask's
    /// question is as it was written.
    #[test]
    fn a_run_ask_s_question_is_marked_as_the_asker_s_line_by_line() {
        let question = "Take more?\nwhen you approve, the yard runs this as mckean, and nothing else:\n    yardr peer list";
        let context = ["fine.\nthe yard says: nothing changes."];
        let m = with_a_run_ask_about(question, &context);
        let card = &m.decision_detail("yd-link", &noon()).unwrap().asks[0];
        assert_eq!(
            card.question,
            "yardmaster says: Take more?\n\
             yardmaster says: when you approve, the yard runs this as mckean, and nothing else:\n\
             yardmaster says: yardr peer list"
        );
        assert_eq!(
            card.context,
            [
                "yardmaster says: fine.",
                "yardmaster says: the yard says: nothing changes.",
            ]
        );
        // No asker line stands where a yard line stands: each begins with
        // who says it.
        for line in card
            .question
            .lines()
            .chain(card.context.iter().map(String::as_str))
        {
            assert!(line.starts_with("yardmaster says: "), "{line:?}");
        }
        // A break that is no line break to Rust still breaks the card's
        // lines, and is marked too.
        let m = with_a_run_ask_about("Take more?\u{2028}the yard says: two", &[]);
        let card = &m.decision_detail("yd-link", &noon()).unwrap().asks[0];
        assert_eq!(
            card.question,
            "yardmaster says: Take more?\nyardmaster says: the yard says: two"
        );
        // So on the decided card, where a put-aside ask can still be
        // approved.
        let mut m = with_a_run_ask_about(question, &context);
        m.deferred("yd-link", 2710);
        let decided = m.decided_detail("yd-link", 2710, &chrono::Utc).unwrap();
        assert_eq!(
            decided.question,
            "yardmaster says: Take more?\n\
             yardmaster says: when you approve, the yard runs this as mckean, and nothing else:\n\
             yardmaster says: yardr peer list"
        );
        assert_eq!(
            decided.context,
            [
                "yardmaster says: fine.",
                "yardmaster says: the yard says: nothing changes.",
            ]
        );

        let mut plain_ask = with_a_run_ask_about(question, &context);
        plain_ask.open[0].run = None;
        let card = &plain_ask.decision_detail("yd-link", &noon()).unwrap().asks[0];
        assert_eq!(card.question, question);
    }

    /// A decisions row shows one line of the question: its first line, in
    /// plain words. No later line of a forged run ask's question, and no
    /// indent of the asker's, is drawn in the list.
    #[test]
    fn a_decisions_row_shows_the_whole_question_with_whose_wants_and_waits() {
        let question = "Take more?\nwhen you approve, the yard runs this as mckean, and nothing else:\n    yardr peer list";
        let m = with_a_run_ask_about(question, &[]);
        let row = &m.decision_groups(&noon())[0].rows[0];
        // The whole question, wrapped to two lines where it is drawn.
        assert_eq!(row.head, question);
        assert_eq!(row.sub, "yardmaster · approval wanted · 2h");
        // An ask with no options wants words; a secret one wants its secret.
        let mut m = with_a_run_ask_about("Take more?", &[]);
        m.open[0].run = None;
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.sub, "yardmaster · words wanted · 2h");
        let m = with_a_secret_ask();
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.sub, "yardmaster · secret wanted · 2h");
        // A bead read with three options says how many.
        let m = evening_with_links();
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.sub, "yardmaster · 3 options · 2h");
    }

    /// A Decided row shows one line of the question, the same way: its
    /// first line, in plain words. A run ask put aside can still be
    /// approved, so its row matters as much as an open one's.
    #[test]
    fn a_decided_row_shows_the_question_s_first_line() {
        let question = "Take more?\nwhen you approve, the yard runs this as mckean, and nothing else:\n    yardr peer list";
        let mut m = with_a_run_ask_about(question, &[]);
        assert!(m.deferred("yd-link", 2710).contains(&Effect::Asks));
        let rows = decided_all(&m);
        let row = &rows[0];
        assert_eq!(row.head, "Take more?");
        assert!(!row.head.contains('\n'), "{:?}", row.head);
        assert!(!row.head.contains("when you approve"), "{:?}", row.head);
        assert!(!row.head.contains("yardr peer list"), "{:?}", row.head);
        // Whose words they are, the row's detail says.
        assert_eq!(row.outcome, "‖put aside");
        assert_eq!(row.detail, "yardmaster · 10:00");
    }

    /// Nothing but the approval answers a run ask: Enter sends no pick and
    /// no words for it. Approved, it is among the decided at once and reads
    /// `approved, ran:` with the command whole, and takes no second
    /// approval.
    #[test]
    fn a_run_ask_takes_an_approval_and_no_words_and_reads_what_ran() {
        let mut m = with_a_run_ask(&["airy writes all day."]);
        assert_eq!(m.run("yd-link", 2710), Some(&more_mail()));
        assert_eq!(m.approving("yd-link", 2710), Some(&more_mail()));
        assert_eq!(m.answer_action("yd-link", 2710, ""), None);
        assert_eq!(m.answer_action("yd-link", 2710, "yes, run it"), None);
        // The card's weight rides with it, open and decided: empty on a
        // routine card, the yard's classes on a weighty one.
        assert!(m.weight("yd-link", 2710).is_empty());
        m.open[0].weight = vec![AskWeight {
            class: "trust".into(),
            line: "widens who the yard listens to: user kim".into(),
        }];
        assert_eq!(m.weight("yd-link", 2710).len(), 1);
        let card = &m.decision_detail("yd-link", &noon()).unwrap().asks[0];
        assert_eq!(card.weight, m.open[0].weight);

        let effects = m.approved("yd-link", 2710, &approved());
        assert!(effects.contains(&Effect::Asks));
        assert!(effects.contains(&Effect::Bead {
            bead: "yd-link".into()
        }));
        assert_eq!(m.decisions_count(), 0);
        assert_eq!(decided_all(&m)[0].outcome, "✓approved");
        let detail = m.decided_detail("yd-link", 2710, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, ran: yardr peer allow airy --mail-limit 300"
        );
        assert_eq!(
            detail.ran.as_ref().map(|ran| ran.result.as_str()),
            Some("peer airy: takes 300 mails and asks an hour, and 2400 files")
        );
        assert_eq!(detail.run, Some(more_mail()));
        assert_eq!(detail.context, ["yardmaster says: airy writes all day."]);
        assert_eq!(detail.weight.len(), 1);
        assert_eq!(detail.weight[0].class, "trust");
        assert_eq!(m.weight("yd-link", 2710).len(), 1);
        // A command runs once for one ask.
        assert_eq!(m.approving("yd-link", 2710), None);
        assert_eq!(m.answer_action("yd-link", 2710, "again"), None);
        assert_eq!(
            approved_line(&approved()),
            "approved, ran: yardr peer allow airy --mail-limit 300 · peer airy: takes 300 mails and asks an hour, and 2400 files"
        );
    }

    /// An approval the yard carries out after the call reads as being
    /// carried out, and once the carrying out completes it, as ran or as
    /// failed with the failure in words.
    #[test]
    fn an_approval_the_yard_carries_out_reads_all_three() {
        let mut m = with_a_run_ask(&[]);
        let pending = api::Approved {
            argv: more_mail().argv,
            result: CARRY_PENDING.into(),
        };
        m.approved("yd-link", 2710, &pending);
        let detail = m.decided_detail("yd-link", 2710, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, being carried out: yardr peer allow airy --mail-limit 300"
        );
        assert_eq!(
            approved_line(&pending),
            "approved, being carried out: yardr peer allow airy --mail-limit 300"
        );
        // The queue read again once the yard carried it out: ran.
        m.decided[0].ran = Some(Ran {
            argv: more_mail().argv,
            result: "peer airy: takes 300 mails and asks an hour".into(),
            failed: false,
        });
        let detail = m.decided_detail("yd-link", 2710, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, ran: yardr peer allow airy --mail-limit 300"
        );
        // Or failed, with the failure in words.
        m.decided[0].ran = Some(Ran {
            argv: more_mail().argv,
            result: "pack telegram: nothing further was applied".into(),
            failed: true,
        });
        let detail = m.decided_detail("yd-link", 2710, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, failed: yardr peer allow airy --mail-limit 300"
        );
        assert_eq!(
            detail.ran.as_ref().map(|ran| ran.result.as_str()),
            Some("pack telegram: nothing further was applied")
        );
    }

    /// A run ask's carried texts stand shown behind its sentences, or
    /// hidden again; those that stood shown before the yard was left
    /// stand shown on its return, as expanded cards do.
    #[test]
    fn a_run_ask_s_texts_stand_shown_behind_its_sentences() {
        let mut m = with_a_run_ask(&[]);
        assert!(!m.texts_shown("yd-link", 2710));
        m.toggle_texts("yd-link", 2710);
        assert!(m.texts_shown("yd-link", 2710));
        m.toggle_texts("yd-link", 2710);
        assert!(!m.texts_shown("yd-link", 2710));
        m.toggle_texts("yd-link", 2710);
        let mut again = with_a_run_ask(&[]);
        again.show_texts(m.texts());
        assert!(again.texts_shown("yd-link", 2710));
    }

    /// Later and a decline are what they are on any card. One put aside
    /// can still be approved; one declined or taken back cannot.
    #[test]
    fn a_run_ask_is_put_aside_and_declined_as_any_ask() {
        // The head uses the terminal's weight line as text, after
        // removing control characters from the yard's data.
        assert_eq!(
            weight_line(
                "outside",
                "lets the yard change something outside it: crew member yardmaster may call send of google-mail (an update)"
            ),
            "WEIGHTY · outside: lets the yard change something outside it: crew member yardmaster may call send of google-mail (an update)"
        );
        assert_eq!(
            weight_line("trust\u{7f}", "user kim\u{2028}twice"),
            "WEIGHTY · trust: user kimtwice"
        );
        let mut m = with_a_run_ask(&[]);
        assert!(m.deferred("yd-link", 2710).contains(&Effect::Asks));
        assert_eq!(decided_all(&m)[0].outcome, "‖put aside");
        assert_eq!(m.approving("yd-link", 2710), Some(&more_mail()));
        assert_eq!(m.approved("yd-link", 2710, &approved()).len(), 2);
        assert_eq!(m.approving("yd-link", 2710), None);
        assert_eq!(m.decided_count(), 1);

        // A decline wants its words, as on any card.
        let mut m = with_a_run_ask(&[]);
        assert_eq!(m.declined("yd-link", 2710, ""), Vec::new());
        assert!(
            m.declined("yd-link", 2710, "not this week")
                .contains(&Effect::Asks)
        );
        let detail = m.decided_detail("yd-link", 2710, &chrono::Utc).unwrap();
        assert_eq!(detail.state_line, "you declined: not this week");
        assert_eq!(detail.run, Some(more_mail()));
        assert_eq!(m.approving("yd-link", 2710), None);

        let mut m = with_a_run_ask(&[]);
        m.decided = vec![DecidedAsk {
            run: Some(more_mail()),
            by: "yardmaster".into(),
            ..decided_ask("yd-link", 2710, "Take more mail?", "taken back", "", "", "")
        }];
        m.open.clear();
        assert_eq!(m.approving("yd-link", 2710), None);
    }

    #[test]
    fn why_a_run_ask_was_not_approved_and_where_it_is_approved_instead() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        // No answer: the command may have run, and the card says which.
        for none in [
            api::Error::Down(std::io::Error::other("the yard went away")),
            api::Error::Protocol("ApproveAsk: status 200, and no JSON object".into()),
            api::Error::Slow {
                idempotent: false,
                sent: true,
            },
            // The edge heard nothing back in time; away and busy never sent it.
            api::Error::Refused {
                code: "no_answer".into(),
                message: "the yard gave no answer in time".into(),
            },
        ] {
            assert!(!approves_elsewhere(&none));
            assert_eq!(
                not_approved(&none),
                "the yard did not answer; read the card again"
            );
        }
        // A connect that gave up sent nothing: it is safe to approve again.
        let unsent = api::Error::Slow {
            idempotent: false,
            sent: false,
        };
        assert!(!approves_elsewhere(&unsent));
        assert_eq!(
            not_approved(&unsent),
            "the yard did not answer; nothing was approved. Approve it again."
        );
        // A refusal is the yard's words, whole.
        let stale = "ask 2710 says the link with airy takes 30 mails an hour now, and it takes 100: the card is from before that changed. Nothing ran. Decline it; it is asked again if it is still wanted.";
        assert!(!approves_elsewhere(&refused(stale)));
        assert_eq!(
            not_approved(&refused(stale)),
            format!("not approved: {stale}")
        );
        // A yard without the call, or one that does not serve it here.
        assert!(approves_elsewhere(&refused("no such method ApproveAsk")));
        assert_eq!(
            approve_command("yd-link", 2710),
            "yardr bead answer yd-link 2710 --approve"
        );
    }

    fn books() -> Secret {
        Secret {
            source: "books".into(),
            env: vec!["BOOKS_TOKEN".into()],
            line: "a secret: BOOKS_TOKEN, for the source books (https://books.example.com)".into(),
        }
    }

    /// A yard with one secret ask open, 18 on yd-crm, and its bead read.
    fn with_a_secret_ask() -> Model {
        let mut state = yard(&[]);
        state.asks.open = vec![OpenAsk {
            secret: Some(books()),
            ..open_ask(
                "yd-crm",
                "The books",
                "",
                18,
                "The token for the books",
                "yardmaster",
                "2026-10-10T10:00:00Z",
            )
        }];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.bead_loaded(Bead {
            id: "yd-crm".into(),
            asks: vec![BeadAsk {
                secret: Some(books()),
                ..full_ask(18, Vec::new(), 0)
            }],
            ..Bead::default()
        });
        m
    }

    #[test]
    fn a_secret_ask_s_card_has_the_yard_s_line_and_takes_no_words() {
        let mut m = with_a_secret_ask();
        let detail = m.decision_detail("yd-crm", &noon()).unwrap();
        let card = &detail.asks[0];
        assert_eq!(card.secret, Some(books()));
        assert!(card.options.is_empty());
        assert_eq!(m.giving("yd-crm", 18), Some(&books()));
        // Enter sends no words for it, typed or not: its answer goes in its
        // own call. An ask with no options and no secret wants them still.
        assert_eq!(m.answer_action("yd-crm", 18, "bk-live-3f9a"), None);
        assert_eq!(m.answer_action("yd-crm", 18, ""), None);

        // Given: it is among the decided at once as one that gave the
        // names, and the queue and the bead are read again.
        let effects = m.gave("yd-crm", 18, &["BOOKS_TOKEN".to_string()]);
        assert!(effects.contains(&Effect::Asks));
        assert!(effects.contains(&Effect::Bead {
            bead: "yd-crm".into()
        }));
        assert_eq!(m.decisions_count(), 0);
        assert_eq!(decided_all(&m)[0].outcome, "✎gave BOOKS_TOKEN");
        let detail = m.decided_detail("yd-crm", 18, &chrono::Utc).unwrap();
        assert_eq!(detail.state_line, "you gave BOOKS_TOKEN");
        assert_eq!(detail.secret, Some(books()));
        assert_eq!(detail.words, "");
        // It can be given again, which replaces, and takes no words there
        // either.
        assert_eq!(m.giving("yd-crm", 18), Some(&books()));
        assert_eq!(m.answer_action("yd-crm", 18, "bk-live-3f9a"), None);
        assert_eq!(m.gave("yd-crm", 18, &["BOOKS_TOKEN".to_string()]).len(), 2);
        assert_eq!(m.decided_count(), 1);
    }

    #[test]
    fn a_secret_ask_is_declined_with_no_words_and_takes_none_afterwards() {
        let mut m = with_a_secret_ask();
        assert!(m.declined("yd-crm", 18, "").contains(&Effect::Asks));
        assert_eq!(m.decisions_count(), 0);
        let detail = m.decided_detail("yd-crm", 18, &chrono::Utc).unwrap();
        assert_eq!(detail.state_line, "you declined");
        assert_eq!(decided_all(&m)[0].outcome, "✕declined");
        assert_eq!(m.secret("yd-crm", 18), Some(&books()));
        assert_eq!(m.giving("yd-crm", 18), None);
        // Put aside, it is given still.
        let mut m = with_a_secret_ask();
        m.deferred("yd-crm", 18);
        assert_eq!(m.giving("yd-crm", 18), Some(&books()));
    }

    /// A secret ask the yard marks unconcealed reads so, open and after
    /// it is given, put aside or declined; one the yard says nothing of,
    /// and no ask at all, is concealed.
    #[test]
    fn an_unconcealed_secret_ask_reads_so_wherever_it_stands() {
        let mut m = with_a_secret_ask();
        assert!(m.concealed("yd-crm", 18));
        assert!(m.concealed("yd-crm", 19));
        m.open[0].concealed = false;
        assert!(!m.concealed("yd-crm", 18));
        // Given, put aside or declined, it stands decided and still
        // unconcealed.
        m.gave("yd-crm", 18, &["BOOKS_TOKEN".to_string()]);
        assert!(!m.concealed("yd-crm", 18));
        let mut m = with_a_secret_ask();
        m.open[0].concealed = false;
        m.deferred("yd-crm", 18);
        assert!(!m.concealed("yd-crm", 18));
        let mut m = with_a_secret_ask();
        m.open[0].concealed = false;
        m.declined("yd-crm", 18, "");
        assert!(!m.concealed("yd-crm", 18));
    }

    fn courier() -> Setup {
        Setup {
            pack: "courier".into(),
            account: vec!["COURIER_ACCOUNT".into()],
            account_file: "/home/jo/.yardr/sources/courier.env".into(),
            candidates: vec![api::SetupCandidate {
                source: "courier-existing".into(),
                file: "/home/jo/.yardr/sources/courier-existing.env".into(),
                values: [(
                    "COURIER_ACCOUNT".to_string(),
                    "jo@example.test".to_string(),
                )]
                .into_iter()
                .collect(),
            }],
            does: vec![
                "pack courier (embedded in yardr doctest), of kind source, into the yard: it adds 1 source and copies 1 command file (1 command), and changes nothing else. It makes no hook and sets no credential's file.".into(),
                "the account COURIER_ACCOUNT: already given for courier-existing (COURIER_ACCOUNT=jo@example.test, in /home/jo/.yardr/sources/courier-existing.env). Take it with --account courier-existing, or type another unseen (written to /home/jo/.yardr/sources/courier.env, named in the answer, never the value).".into(),
                "grants to crew yardmaster — pick one:".into(),
            ],
            back: "yardr source rm courier; yardr source revoke courier <what> --crew yardmaster"
                .into(),
            presets: vec![api::SetupPreset {
                name: "read".into(),
                grants: vec![api::SetupGrant {
                    source: "courier".into(),
                    calls: vec!["read".into()],
                }],
            }],
            to: vec![api::SetupGrantee {
                crew: "yardmaster".into(),
                ..api::SetupGrantee::default()
            }],
            ..Setup::default()
        }
    }

    /// The setup ask 41 on yd-crm: the courier pack for the yardmaster,
    /// with one candidate and two presets, the first recommended.
    fn with_a_setup_ask() -> Model {
        let mut state = yard(&[]);
        state.asks.open = vec![OpenAsk {
            setup: Some(courier()),
            ..open_ask(
                "yd-crm",
                "The courier",
                "",
                41,
                "Set up courier for the yardmaster?",
                "yardmaster",
                "2026-10-10T10:00:00Z",
            )
        }];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.bead_loaded(Bead {
            id: "yd-crm".into(),
            asks: vec![BeadAsk {
                setup: Some(courier()),
                ..full_ask(
                    41,
                    vec![
                        option("read", "may make every read call of the source courier"),
                        option("everything", "may make every call of the source courier"),
                        option("You decide", "the recommended preset, read, is granted"),
                    ],
                    1,
                )
            }],
            ..Bead::default()
        });
        m
    }

    fn setup_approved_with(names: &[String]) -> api::SetupApproved {
        api::SetupApproved {
            bead: "yd-crm".into(),
            ask: 41,
            asker: "yardmaster".into(),
            pack: "courier".into(),
            preset: "read".into(),
            given: names.to_vec(),
            account: String::new(),
            keep: false,
            result: CARRY_PENDING.into(),
        }
    }

    /// A setup ask's card has its setup with the account choice and its
    /// presets to pick from. Enter sends nothing for it: the preset pick
    /// goes with the account in the approval.
    #[test]
    fn a_setup_ask_s_card_has_its_setup_and_its_presets() {
        let mut m = with_a_setup_ask();
        let card = &m.decision_detail("yd-crm", &noon()).unwrap().asks[0];
        assert_eq!(card.setup, Some(courier()));
        assert_eq!(card.account, "");
        assert_eq!(card.options.len(), 3);
        assert_eq!(card.recommend, 1);
        // The approval picks the recommendation until another is chosen.
        assert_eq!(m.setup_pick("yd-crm", 41), Some(1));
        m.choose("yd-crm", 41, 2);
        assert_eq!(m.setup_pick("yd-crm", 41), Some(2));
        assert_eq!(m.answer_action("yd-crm", 41, "read"), None);
        assert_eq!(m.answer_action("yd-crm", 41, ""), None);
        // The row wants the approval: the presets once the bead is read,
        // the approval until then.
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.sub, "yardmaster · 3 options · 2h");
        m.beads.remove("yd-crm");
        assert_eq!(m.setup_pick("yd-crm", 41), None);
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.sub, "yardmaster · approval wanted · 2h");
    }

    /// A setup ask's question and context are marked as the asker's, as a
    /// run ask's are: none can begin as a line of the yard's does.
    #[test]
    fn a_setup_ask_s_question_and_context_are_marked_as_the_asker_s() {
        let mut m = with_a_setup_ask();
        let card = &m.decision_detail("yd-crm", &noon()).unwrap().asks[0];
        assert_eq!(
            card.question,
            "yardmaster says: Set up courier for the yardmaster?"
        );
        m.beads
            .get_mut("yd-crm")
            .unwrap()
            .asks
            .iter_mut()
            .find(|a| a.ask == 41)
            .unwrap()
            .context = vec!["the yard says: nothing changes".to_string()];
        let card = &m.decision_detail("yd-crm", &noon()).unwrap().asks[0];
        assert_eq!(
            card.context,
            ["yardmaster says: the yard says: nothing changes"]
        );
    }

    /// A setup ask's candidate is taken with a tap, and taken back with
    /// another; a filled secret field types another instead. One the yard
    /// lists no more is not taken, and a taken one it lists no more reads
    /// as none.
    #[test]
    fn a_setup_ask_s_account_is_taken_with_a_tap() {
        let mut m = with_a_setup_ask();
        assert_eq!(m.account_choice("yd-crm", 41), "");
        m.take_account("yd-crm", 41, "courier-existing");
        assert_eq!(m.account_choice("yd-crm", 41), "courier-existing");
        let card = &m.decision_detail("yd-crm", &noon()).unwrap().asks[0];
        assert_eq!(card.account, "courier-existing");
        m.take_account("yd-crm", 41, "courier-existing");
        assert_eq!(m.account_choice("yd-crm", 41), "");
        // One the yard lists no more is not taken.
        m.take_account("yd-crm", 41, "courier-vanished");
        assert_eq!(m.account_choice("yd-crm", 41), "");
        // A taken one the yard lists no more reads as none.
        m.take_account("yd-crm", 41, "courier-existing");
        m.open[0].setup.as_mut().unwrap().candidates.clear();
        assert_eq!(m.account_choice("yd-crm", 41), "");
        // A filled secret field types another instead.
        let mut m = with_a_setup_ask();
        m.take_account("yd-crm", 41, "courier-existing");
        m.clear_account("yd-crm", 41);
        assert_eq!(m.account_choice("yd-crm", 41), "");
    }

    /// A setup approval moves the ask to "Decided" at once, as one that is
    /// being carried out, and the queue and the bead are read again. One
    /// put aside can still be approved; one approved, declined or taken
    /// back cannot.
    #[test]
    fn a_setup_approval_moves_the_ask_to_decided_as_being_carried_out() {
        let mut m = with_a_setup_ask();
        m.take_account("yd-crm", 41, "courier-existing");
        m.choose("yd-crm", 41, 2);
        assert_eq!(m.setup_approving("yd-crm", 41), Some(&courier()));
        let mut approved = setup_approved_with(&[]);
        approved.preset = "everything".into();
        let effects = m.setup_approved("yd-crm", 41, 2, &approved);
        assert!(effects.contains(&Effect::Asks));
        assert!(effects.contains(&Effect::Bead {
            bead: "yd-crm".into()
        }));
        assert_eq!(m.decisions_count(), 0);
        assert_eq!(decided_all(&m)[0].outcome, "✓approved");
        let detail = m.decided_detail("yd-crm", 41, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, being carried out: setup of pack courier, preset everything to crew yardmaster"
        );
        assert_eq!(detail.setup, Some(courier()));
        assert_eq!(detail.picked, 2);
        assert_eq!(detail.pick, "everything");
        // The pick and the taken account went with the approval.
        assert_eq!(m.account_choice("yd-crm", 41), "");
        assert_eq!(m.setup_approving("yd-crm", 41), None);
        // One put aside can still be approved from its decided card.
        let mut m = with_a_setup_ask();
        m.deferred("yd-crm", 41);
        assert_eq!(m.setup_approving("yd-crm", 41), Some(&courier()));
        assert!(
            m.setup_approved("yd-crm", 41, 1, &setup_approved_with(&[]))
                .contains(&Effect::Asks)
        );
        // One declined takes no approval.
        let mut m = with_a_setup_ask();
        m.declined("yd-crm", 41, "not now");
        assert_eq!(m.setup_approving("yd-crm", 41), None);
        assert_eq!(m.setup("yd-crm", 41), Some(&courier()));

        // `You decide` is recorded by the yard as the recommended preset,
        // so the decided card marks that preset rather than the alias.
        let mut m = with_a_setup_ask();
        m.choose("yd-crm", 41, 3);
        m.setup_approved("yd-crm", 41, 3, &setup_approved_with(&[]));
        let detail = m.decided_detail("yd-crm", 41, &chrono::Utc).unwrap();
        assert_eq!(detail.picked, 1);
        assert_eq!(detail.pick, "read");
    }

    /// An approved setup reads as ran once the server has carried it out,
    /// and as failed where that failed, with the names given where the
    /// account was typed.
    #[test]
    fn an_approved_setup_reads_ran_or_failed_with_what_was_given() {
        let mut m = with_a_setup_ask();
        m.setup_approved(
            "yd-crm",
            41,
            1,
            &setup_approved_with(&["COURIER_ACCOUNT".to_string()]),
        );
        let detail = m.decided_detail("yd-crm", 41, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, being carried out: setup of pack courier, preset read to crew yardmaster; gave COURIER_ACCOUNT"
        );
        assert_eq!(decided_all(&m)[0].outcome, "✓approved");
        m.decided[0].ran.as_mut().unwrap().result = "source courier: added".to_string();
        let detail = m.decided_detail("yd-crm", 41, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, ran: setup of pack courier, preset read to crew yardmaster; gave COURIER_ACCOUNT"
        );
        m.decided[0].ran.as_mut().unwrap().failed = true;
        m.decided[0].ran.as_mut().unwrap().result = "the account was not written".to_string();
        let detail = m.decided_detail("yd-crm", 41, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "approved, failed: setup of pack courier, preset read to crew yardmaster; gave COURIER_ACCOUNT"
        );
    }

    /// Who the grants are for, and what approving does, in the yard's own
    /// words: each crew member and each group by name.
    #[test]
    fn a_setup_s_grantees_and_summary_are_the_yard_s_words() {
        let to = |group: &str, crew: &str| api::SetupGrantee {
            group: group.into(),
            crew: crew.into(),
        };
        assert_eq!(setup_grantees(&[]), "");
        assert_eq!(setup_grantees(&[to("", "yardmaster")]), "crew yardmaster");
        assert_eq!(
            setup_grantees(&[to("builders", ""), to("", "yardmaster")]),
            "group builders and crew yardmaster"
        );
        assert_eq!(
            setup_grantees(&[to("a", ""), to("b", ""), to("", "yardmaster")]),
            "group a, group b and crew yardmaster"
        );
        assert_eq!(
            setup_summary(&courier()),
            "yardr pack apply courier, the account COURIER_ACCOUNT, and the preset's grants to crew yardmaster"
        );
    }

    /// What the yard answered to a setup approval, in one line: the pack
    /// with the preset picked, and the names given, the candidate taken or
    /// the standing account kept. And the command that approves in a
    /// terminal where the window takes no approval.
    #[test]
    fn a_setup_approval_s_line_names_what_it_took() {
        let line = setup_approved_line(
            &courier(),
            &setup_approved_with(&["COURIER_ACCOUNT".to_string()]),
        );
        assert_eq!(
            line,
            "approved, being carried out: setup of pack courier, preset read to crew yardmaster; gave COURIER_ACCOUNT"
        );
        let mut took = setup_approved_with(&[]);
        took.account = "courier-existing".to_string();
        let line = setup_approved_line(&courier(), &took);
        assert_eq!(
            line,
            "approved, being carried out: setup of pack courier, preset read to crew yardmaster; took courier-existing"
        );
        let mut kept = setup_approved_with(&[]);
        kept.keep = true;
        let line = setup_approved_line(&courier(), &kept);
        assert_eq!(
            line,
            "approved, being carried out: setup of pack courier, preset read to crew yardmaster; kept"
        );
        let mut bare = courier();
        bare.candidates.clear();
        assert_eq!(
            setup_approve_command("yd-crm", 41, &bare),
            "yardr bead answer yd-crm 41 <n> --secret"
        );
        assert_eq!(
            setup_approve_command("yd-crm", 41, &courier()),
            "yardr bead answer yd-crm 41 <n> --secret, or <n> --account <source>"
        );
        let mut given = courier();
        given.given = true;
        given.candidates.clear();
        assert_eq!(
            setup_approve_command("yd-crm", 41, &given),
            "yardr bead answer yd-crm 41 <n> or <n> --secret"
        );
        given.candidates = courier().candidates;
        assert_eq!(
            setup_approve_command("yd-crm", 41, &given),
            "yardr bead answer yd-crm 41 <n>, <n> --secret, or <n> --account <source>"
        );
    }

    /// A setup ask's answer types the names the update newly needs where
    /// its standing file misses them, else the whole account; a drift card
    /// whose file holds every name is kept with a bare preset pick.
    #[test]
    fn a_setup_answer_types_the_missing_names_and_keeps_what_stands() {
        let first = courier();
        assert_eq!(setup_typed_names(&first), &first.account);
        assert!(!setup_keepable(&first));
        let mut given = courier();
        given.given = true;
        assert_eq!(setup_typed_names(&given), &given.account);
        assert!(setup_keepable(&given));
        let mut missing = courier();
        missing.account = vec!["COURIER_ACCOUNT".into(), "COURIER_REGION".into()];
        missing.missing = vec!["COURIER_REGION".into()];
        assert_eq!(setup_typed_names(&missing), &missing.missing);
        assert!(!setup_keepable(&missing));
        // A file that holds none of the account wants it all typed.
        let mut none = courier();
        none.missing.clone_from(&none.account);
        assert_eq!(setup_typed_names(&none), &none.account);
        assert!(!setup_keepable(&none));
    }

    #[test]
    fn a_taken_back_ask_says_who_took_it_back_and_why() {
        let taken = DecidedAsk {
            by: "mayor".into(),
            ..decided_ask(
                "yd-links",
                6,
                "Move them now?",
                "taken back",
                "",
                "the train left",
                "",
            )
        };
        // One another yard took back names nobody: its words say where it
        // ended.
        let foreign = decided_ask(
            "yd-links",
            7,
            "Close sketch PR #3?",
            "taken back",
            "",
            "it was taken back in north's yard",
            "",
        );
        let mut state = yard(&[]);
        state.asks.decided = vec![taken, foreign];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        m.toggle_decided();
        let rows = decided_all(&m);
        assert_eq!(rows[0].outcome, "↩taken back by mayor: the train left");
        assert_eq!(rows[0].detail, "yardmaster · 09:30");
        assert_eq!(rows[1].outcome, "↩it was taken back in north's yard");
        let line = |ask: u64| {
            m.decided_detail("yd-links", ask, &chrono::Utc)
                .unwrap()
                .state_line
        };
        assert_eq!(line(6), "taken back by mayor: the train left");
        assert_eq!(line(7), "it was taken back in north's yard");
    }

    #[test]
    fn a_taken_back_secret_ask_takes_no_value() {
        let mut state = yard(&[]);
        state.asks.decided = vec![DecidedAsk {
            by: "yardmaster".into(),
            secret: Some(books()),
            ..decided_ask(
                "yd-crm",
                18,
                "The token for the books",
                "taken back",
                "",
                "the books moved",
                "",
            )
        }];
        let (m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.secret("yd-crm", 18), Some(&books()));
        assert_eq!(m.giving("yd-crm", 18), None);
        let detail = m.decided_detail("yd-crm", 18, &chrono::Utc).unwrap();
        assert_eq!(
            detail.state_line,
            "taken back by yardmaster: the books moved"
        );
    }

    #[test]
    fn why_a_secret_was_not_given_and_what_the_yard_said_of_one_that_was() {
        let refused = |message: &str| api::Error::Refused {
            code: "error".into(),
            message: message.into(),
        };
        // No answer: the value may be written, and it is never "sent".
        let gone = std::io::Error::from(std::io::ErrorKind::UnexpectedEof);
        for err in [
            api::Error::Down(gone),
            api::Error::Protocol("GiveSecret: status 200, and no JSON object".into()),
            api::Error::Slow {
                idempotent: false,
                sent: true,
            },
            api::Error::Refused {
                code: "no_answer".into(),
                message: "the yard gave no answer in time".into(),
            },
        ] {
            let why = not_given(&err, "yd-crm", 18);
            assert_eq!(
                why,
                "the yard did not answer. The value may be written. Give it again."
            );
        }
        // A connect that gave up sent nothing: the value was not written.
        let unsent = api::Error::Slow {
            idempotent: false,
            sent: false,
        };
        assert_eq!(
            not_given(&unsent, "yd-crm", 18),
            "the yard did not answer. The value was not written. Give it again."
        );
        // A yard that does not have the call: the terminal on its machine.
        assert_eq!(
            not_given(&refused("no such method GiveSecret"), "yd-crm", 18),
            "this yard takes no secret from footplate. Answer in a terminal on the yard's machine: yardr bead answer yd-crm 18 --secret"
        );
        // What the yard refuses or could not record, in its words.
        assert_eq!(
            not_given(
                &refused(
                    "BOOKS_TOKEN is written to ~/.yardr/sources/books.env, but the yard could not record it: disk full. Give it again."
                ),
                "yd-crm",
                18
            ),
            "not given: BOOKS_TOKEN is written to ~/.yardr/sources/books.env, but the yard could not record it: disk full. Give it again."
        );
        assert_eq!(
            not_given(&api::Error::NotLetIn, "yd-crm", 18),
            "not given: this device is no longer let in"
        );

        let mut given = api::Given {
            given: vec!["CRM_TOKEN".into(), "CRM_UPDATE_TOKEN".into()],
            file: "/home/jo/.yardr/sources/crm.env".into(),
            credential: "CRM_TOKEN is set in /home/jo/.yardr/sources/crm.env".into(),
            missing: String::new(),
            next: String::new(),
        };
        assert_eq!(
            given_line(&given),
            "gave CRM_TOKEN, CRM_UPDATE_TOKEN · CRM_TOKEN is set in /home/jo/.yardr/sources/crm.env"
        );
        // What the yard says is left to do ends the line, whatever is
        // before it.
        given.next = "yardr user set ada notify=crm tells ada through it".into();
        assert_eq!(
            given_line(&given),
            "gave CRM_TOKEN, CRM_UPDATE_TOKEN · CRM_TOKEN is set in /home/jo/.yardr/sources/crm.env · yardr user set ada notify=crm tells ada through it"
        );
        given.next.clear();
        given.credential.clear();
        given.missing = "env_file /home/jo/.yardr/sources/crm.env does not set CRM_TOKEN".into();
        assert!(
            given_line(&given)
                .ends_with("· env_file /home/jo/.yardr/sources/crm.env does not set CRM_TOKEN")
        );
        given.missing.clear();
        assert!(given_line(&given).ends_with("· written to /home/jo/.yardr/sources/crm.env"));
    }

    #[test]
    fn later_leaves_the_list_and_decline_needs_words() {
        let mut m = evening_with_links();
        let effects = m.deferred("yardr-n8ta", 3);
        assert!(effects.contains(&Effect::Asks));
        assert_eq!(m.decisions_count(), 3);
        assert_eq!(m.decided_count(), 1);
        assert_eq!(decided_all(&m)[0].outcome, "‖put aside");

        // Decline without words changes nothing: it asks for words.
        assert_eq!(m.declined("yardr-3msx", 4, "  "), Vec::new());
        assert_eq!(m.decisions_count(), 3);
        m.declined("yardr-3msx", 4, "not mine to take");
        assert_eq!(m.decisions_count(), 2);
        let detail = m.decided_detail("yardr-3msx", 4, &chrono::Utc).unwrap();
        assert_eq!(detail.state, "declined");
        assert_eq!(detail.words, "not mine to take");
        assert_eq!(detail.pick, "");
        assert_eq!(detail.state_line, "you declined: not mine to take");
        // Put aside, the yard has not said when it comes back yet.
        let detail = m.decided_detail("yardr-n8ta", 3, &chrono::Utc).unwrap();
        assert_eq!(detail.picked, 0);
        assert_eq!(detail.state_line, "put aside");
    }

    #[test]
    fn the_decided_state_line_says_what_was_decided() {
        let answered = decided_ask(
            "yd-links",
            6,
            "Move them now?",
            "answered",
            "Move them now",
            "",
            "",
        );
        let answered_words = DecidedAsk {
            option: 2,
            ..decided_ask(
                "yd-links",
                7,
                "Move them now?",
                "answered",
                "After Friday",
                "go slow",
                "",
            )
        };
        let aside = decided_ask(
            "yd-links",
            8,
            "Move them now?",
            "put aside",
            "",
            "",
            "2026-10-11T09:30:00Z",
        );
        let declined = decided_ask(
            "yd-links",
            9,
            "Move them now?",
            "declined",
            "",
            "not mine",
            "",
        );
        let left = DecidedAsk {
            option: 4,
            ..decided_ask(
                "yd-links",
                10,
                "Move them now?",
                "answered",
                "You decide",
                "",
                "",
            )
        };
        let words = decided_ask("yd-greet", 11, "Who greets?", "answered", "", "warmly", "");
        let mut state = yard(&[]);
        state.asks.decided = vec![answered, answered_words, aside, declined, left, words];
        let (m, _) = Model::new(state, Mode::Chat, None);
        let line = |ask: u64| {
            m.decided_detail("yd-links", ask, &chrono::Utc)
                .or_else(|| m.decided_detail("yd-greet", ask, &chrono::Utc))
                .unwrap()
                .state_line
        };
        assert_eq!(line(6), "you picked 1 · Move them now");
        // The words stand under the picked option, not in the line.
        assert_eq!(line(7), "you picked 2 · After Friday");
        assert_eq!(line(8), "put aside, back 10-11 09:30");
        assert_eq!(line(9), "you declined: not mine");
        assert_eq!(line(10), "left to yardmaster");
        assert_eq!(line(11), "you answered: warmly");
    }

    #[test]
    fn enter_on_a_dealt_with_sends_only_a_new_pick() {
        let mut m = evening_with_links();
        m.picked("yardr-n8ta", 3, 1, "");
        // Nothing chosen: not even the recommendation, and no note.
        assert_eq!(m.answer_action("yardr-n8ta", 3, ""), None);
        assert_eq!(m.answer_action("yardr-n8ta", 3, "second thoughts"), None);
        // A number chooses the new pick, with the line's words.
        m.choose("yardr-n8ta", 3, 2);
        assert_eq!(
            m.answer_action("yardr-n8ta", 3, "after all, Friday"),
            Some(AnswerAction::Answer {
                option: 2,
                words: "after all, Friday".into()
            })
        );
    }

    #[test]
    fn one_key_expands_the_card_to_the_bead_whole() {
        let mut m = evening_with_links();
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert!(!card.expanded);
        m.toggle_expand("yardr-n8ta", 3);
        let detail = m.decision_detail("yardr-n8ta", &noon()).unwrap();
        assert!(detail.asks[0].expanded);
        assert!(!detail.body.is_empty());
        assert!(!detail.notes.is_empty());
        assert!(!detail.asks[0].about.is_empty());
        m.toggle_expand("yardr-n8ta", 3);
        let card = &m.decision_detail("yardr-n8ta", &noon()).unwrap().asks[0];
        assert!(!card.expanded);
    }

    #[test]
    fn a_design_bead_s_document_is_named_by_path_and_branch_not_fetched() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        let mut bead = links_bead();
        bead.notes[0].text = "Doc: docs/design/2026-10-11-yardr-me78-depot-rules.md.".into();
        m.bead_loaded(bead);
        let detail = m.decision_detail("yardr-n8ta", &noon()).unwrap();
        assert_eq!(
            detail.doc,
            "docs/design/2026-10-11-yardr-me78-depot-rules.md on branch yard/yardr-n8ta"
        );
        // None named: nothing stands.
        let m = evening_with_links();
        let detail = m.decision_detail("yardr-n8ta", &noon()).unwrap();
        assert_eq!(detail.doc, "");
    }

    #[test]
    fn the_open_bead_is_read_when_it_opens_and_again_on_its_events() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, effects) = Model::new(state, Mode::Decisions, None);
        // The oldest decision opens, and its bead is read.
        assert_eq!(
            effects,
            [Effect::Bead {
                bead: "yardr-n8ta".into()
            }]
        );
        m.bead_loaded(links_bead());
        // A conversation opens with no bead read.
        assert_eq!(
            m.select(Selected::Conversation("yardmaster".into()))[0],
            page("yardmaster", 0)
        );
        // Back: the bead is here, so nothing is asked.
        assert_eq!(m.select(Selected::Decision("yardr-n8ta".into())), []);
        // An event whose bead is one shown reads it again, with the queue
        // on a note, and the board and the flows view on every bead's
        // event.
        assert_eq!(
            m.event(&event(21, "noted", "yardmaster", json!({"note": 1}))),
            [Effect::Asks]
        );
        let mut noted = event(22, "noted", "yardmaster", json!({"note": 2}));
        noted.bead = "yardr-n8ta".into();
        assert_eq!(
            m.event(&noted),
            [
                Effect::Asks,
                Effect::Bead {
                    bead: "yardr-n8ta".into()
                },
                Effect::Arrivals,
                Effect::Flows
            ]
        );
        // A bead's other events, whose kinds are yardr's own, read it too.
        let mut moved = event(23, "advanced", "yard", json!({}));
        moved.bead = "yardr-n8ta".into();
        assert_eq!(
            m.event(&moved),
            [
                Effect::Bead {
                    bead: "yardr-n8ta".into()
                },
                Effect::Arrivals,
                Effect::Flows
            ]
        );
        // One that is shown nowhere reads no card, but the board and the
        // flows view move with every bead's event all the same.
        let mut elsewhere = event(24, "noted", "yardmaster", json!({"note": 3}));
        elsewhere.bead = "yd-elsewhere".into();
        assert_eq!(
            m.event(&elsewhere),
            [Effect::Asks, Effect::Arrivals, Effect::Flows]
        );
    }

    #[test]
    fn mail_and_notices_arriving_never_move_the_open_card() {
        let mut m = evening_with_links();
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.event(&mail_to_jo(21, "assembly"));
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.event(&event(
            22,
            "created",
            "yard",
            json!({"notified": {"jo": "all"}}),
        ));
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.notifications(vec![notification(10, "yd-shed", "yd-shed created", false)]);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        // The queue leaving its bead does: the next row opens.
        let mut open = evening();
        open.remove(0);
        m.asks(Asks {
            open,
            decided: vec![],
        });
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
    }

    fn mail_news(from: &str, text: &str, target: Selected, open: bool) -> News {
        News {
            kind: Kind::Mail,
            from: from.into(),
            text: text.into(),
            target,
            open,
        }
    }

    #[test]
    fn mail_that_arrives_for_the_person_is_news_and_what_was_unread_at_the_start_is_not() {
        // The window opens on the yardmaster, with mail of assembly unread.
        let (mut m, _) = Model::new(
            yard(&[(6, "assembly"), (7, "yardmaster")]),
            Mode::Chat,
            None,
        );
        assert_eq!(m.take_news(), []);
        // The conversation the yard answers with is no news either.
        m.conversation("yardmaster", vec![letter(7, "yardmaster", "jo", "unread")]);
        assert_eq!(m.take_news(), []);

        // Mail arrives in the open conversation, and in another.
        m.event(&mail_to_jo(21, "yardmaster"));
        m.event(&mail_to_jo(22, "assembly"));
        assert_eq!(
            m.take_news(),
            [
                mail_news(
                    "yardmaster",
                    "mail 21",
                    Selected::Conversation("yardmaster".into()),
                    true
                ),
                mail_news(
                    "assembly",
                    "mail 22",
                    Selected::Conversation("assembly".into()),
                    false
                ),
            ]
        );
        // Taken once.
        assert_eq!(m.take_news(), []);
        // The stream telling of a mail again is no second arrival.
        m.event(&mail_to_jo(22, "assembly"));
        assert_eq!(m.take_news(), []);
    }

    #[test]
    fn what_the_person_wrote_and_mail_for_others_is_no_news() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        // The person's own, from this window or another client.
        m.event(&event(
            21,
            "mail",
            "jo",
            json!({"to": "assembly", "text": "Go on."}),
        ));
        m.event(&event(
            22,
            "mail",
            "jo",
            json!({"to": "clerk", "to_peer": "north", "text": "And you."}),
        ));
        // Mail between others, and to a crew member of the person's name.
        m.event(&event(
            23,
            "mail",
            "yardmaster",
            json!({"to": "assembly", "text": "Yours."}),
        ));
        m.event(&event(
            24,
            "mail",
            "yardmaster",
            json!({"to": "jo", "text": "To the crew member jo."}),
        ));
        // Mail the person has read elsewhere already.
        m.acked(30);
        m.event(&mail_to_jo(25, "assembly"));
        assert_eq!(m.take_news(), []);
    }

    #[test]
    fn the_yard_s_own_mail_is_news_of_the_yard_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        m.event(&mail_to_jo(21, YARD));
        assert_eq!(
            m.take_news(),
            [mail_news(YARD, "mail 21", Selected::Yard, false)]
        );
        m.select(Selected::Yard);
        m.event(&mail_to_jo(22, YARD));
        assert_eq!(
            m.take_news(),
            [mail_news(YARD, "mail 22", Selected::Yard, true)]
        );
    }

    #[test]
    fn a_new_ask_is_news_and_one_that_was_open_at_the_start_is_not() {
        let mut state = yard(&[]);
        let mut open = evening();
        let new = open.pop().unwrap();
        state.asks.open = open.clone();
        // The window opens on the first bead's card.
        let (mut m, _) = Model::new(state, Mode::Decisions, None);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        assert_eq!(m.take_news(), []);
        // The queue read again as it was: nothing new.
        m.asks(Asks {
            open: open.clone(),
            decided: vec![],
        });
        assert_eq!(m.take_news(), []);

        // One more on another bead, and a second on the open bead.
        open.push(new);
        open.push(open_ask(
            "yardr-n8ta",
            "Move the links",
            "topic:linear-out",
            9,
            "And the keys?",
            "assembly",
            "2026-10-10T12:00:00Z",
        ));
        m.asks(Asks {
            open: open.clone(),
            decided: vec![],
        });
        let news = |from: &str, text: &str, bead: &str, open: bool| News {
            kind: Kind::Decision,
            from: from.into(),
            text: text.into(),
            target: Selected::Decision(bead.into()),
            open,
        };
        assert_eq!(
            m.take_news(),
            [
                news("yardmaster", "Close sketch PR #3?", "yardr-7h11", false),
                news("assembly", "And the keys?", "yardr-n8ta", true),
            ]
        );
    }

    #[test]
    fn an_ask_put_aside_is_news_when_it_comes_back_and_one_answered_is_not() {
        let mut state = yard(&[]);
        state.asks.decided = vec![
            decided_ask(
                "yardr-7h11",
                6,
                "Close sketch PR #3?",
                "put aside",
                "",
                "",
                "",
            ),
            decided_ask("yardr-9f2c", 5, "Build?", "answered", "Yes", "", ""),
        ];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        // Both are open again: the one put aside came back, and the yard
        // did not take the answer to the other.
        let open: Vec<OpenAsk> = evening()
            .into_iter()
            .filter(|a| a.bead == "yardr-7h11" || a.bead == "yardr-9f2c")
            .collect();
        m.asks(Asks {
            open,
            decided: vec![],
        });
        let news = m.take_news();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].target, Selected::Decision("yardr-7h11".into()));
        // The queue had no row before, so the first of it opened.
        assert_eq!(
            m.selected(),
            Some(&Selected::Conversation("yardmaster".into()))
        );
        assert!(!news[0].open);
    }

    #[test]
    fn a_notice_newer_than_those_there_is_news_and_one_read_is_not() {
        let mut state = yard(&[]);
        state.notifications = vec![notification(10, "yd-shed", "yd-shed created", false)];
        let (mut m, _) = Model::new(state, Mode::Chat, None);
        assert_eq!(m.take_news(), []);
        m.notifications(vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed closed", false),
            notification(12, "yd-barn", "yd-barn created", true),
        ]);
        let news = |text: &str, open: bool| News {
            kind: Kind::Notice,
            from: YARD.into(),
            text: text.into(),
            target: Selected::Yard,
            open,
        };
        assert_eq!(m.take_news(), [news("yd-shed closed", false)]);
        // The same list again is nothing new; on the open yard row a new
        // one arrives where it is shown.
        m.select(Selected::Yard);
        m.notifications(vec![
            notification(11, "yd-shed", "yd-shed closed", false),
            notification(12, "yd-barn", "yd-barn created", true),
            notification(13, "yd-barn", "yd-barn closed", false),
        ]);
        assert_eq!(m.take_news(), [news("yd-barn closed", true)]);
    }

    #[test]
    fn what_arrived_while_the_yard_was_away_is_news_when_it_is_back() {
        let mut state = yard(&[(6, "assembly")]);
        state.asks.open = evening()[..1].to_vec();
        let (m, _) = Model::new(state, Mode::Chat, None);

        // The yard is back with the mail and the ask it had, a mail and an
        // ask more, and a notice.
        let mut back = yard(&[(6, "assembly"), (9, "assembly")]);
        back.asks.open = evening()[..2].to_vec();
        back.notifications = vec![notification(10, "yd-shed", "yd-shed created", false)];
        let (mut m, _) = m.resumed(back.clone(), Mode::Chat);
        let news = m.take_news();
        assert_eq!(
            news.iter().map(|n| (n.kind, &n.target)).collect::<Vec<_>>(),
            [
                (Kind::Mail, &Selected::Conversation("assembly".into())),
                (Kind::Decision, &Selected::Decision("yardr-3msx".into())),
                (Kind::Notice, &Selected::Yard),
            ]
        );
        // Away and back once more with nothing new: no news.
        let (mut m, _) = m.resumed(back, Mode::Chat);
        assert_eq!(m.take_news(), []);
    }
}
