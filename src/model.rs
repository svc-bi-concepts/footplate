//! What the window shows, without the window: who there is to talk with,
//! which conversation is open, what is unread, and what the yard has to be
//! asked next. It follows `yardr chat` (internal/chat of the yardr
//! repository): the state is read once, and events are applied from then on.
//!
//! Nothing here calls the yard. A change that needs a call answers it as an
//! [`Effect`], and the window makes it and hands the answer back.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::api::{
    self, AboutBead, Arrival, Arrivals, Asks, Bead, BeadAsk, BeadNote, BlobRef, CrewMember,
    DecidedAsk, Event, FlowBead, FlowDepot, Flows, Letter, Me, Notification, OpenAsk, Peer, Ran,
    Run, Secret, User,
};

/// The kind of message a link must carry out for mail to be written.
const PEER_MAIL: &str = "mail";

/// A mail holds this many bytes.
pub const MAIL_BYTES: usize = 4096;

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
    /// remarked, beside the platforms in the order the beads travel.
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

/// What a crew member's session does, in the words of `yardr chat`.
pub fn state_of(member: &CrewMember) -> &'static str {
    match (member.state.as_str(), member.status.as_str()) {
        ("failed", _) => "failed to start",
        ("running", "working") => "working",
        ("running", "idle") => "idle",
        ("running", "blocked") => "blocked",
        ("running", _) => "running",
        _ => "not running",
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
    /// The user's own.
    pub mine: bool,
    /// Where a mail the user sent stands. Mails that stand alike share one
    /// line, under the last of them.
    pub standing: Option<String>,
}

/// Where what is typed goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// `Mail` to a crew member or a user of this yard.
    Crew(String),
    /// `MailPeer` to one of a linked yard.
    Peer { name: String, peer: String },
}

/// What the left list has open: a bead with open asks, one dealt with, a
/// conversation, what the yard reports, the arrivals board, a line's
/// flows, the beads with no line, or one travelling bead whole.
#[derive(Debug, Clone, PartialEq)]
pub enum Selected {
    /// A bead with open asks, by its id. A bead with several is one row.
    Decision(String),
    /// One dealt with, by bead and ask note seq.
    Decided(String, u64),
    /// A conversation, by crew name, user name or `<name>@<peer>`.
    Conversation(String),
    Yard,
    /// The arrivals board: the served rows under their platform.
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
    /// row are chat's, the beads with open asks and the dealt with are
    /// decisions', the timetable's rows and a travelling bead timetable's.
    pub fn mode(&self) -> Mode {
        match self {
            Selected::Conversation(_) | Selected::Yard => Mode::Chat,
            Selected::Decision(_) | Selected::Decided(_, _) => Mode::Decisions,
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

/// One bead's row among the decisions: the question's first line in plain
/// words, or the bead's title with how many it asks, with every ask
/// expanded beneath it.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRow {
    /// The bead's id, whole; `short` is what the row ends with.
    pub bead: String,
    /// The question's first line, or `<title>: <n> questions`.
    pub head: String,
    /// Every open ask expanded, in ask order.
    pub asks: Vec<DecisionRowAsk>,
    /// `<asker> · <waits>`, or `<waits>` when several ask.
    pub sub: String,
    /// The bead's id small, at the row's end: past its last `-`.
    pub short: String,
    pub selected: bool,
}

/// One ask of a bead's open row, as its expanded row shows it: the
/// question's first line with its numbered options from the Bead read, or
/// what its card shows where it has none.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRowAsk {
    /// The ask note's seq: what an answer names.
    pub ask: u64,
    /// The question's first line in plain words.
    pub head: String,
    /// The numbered options' words, from the Bead read: empty until read.
    pub options: Vec<(u64, String)>,
    /// What a secret ask asks for, in the yard's words; none on any other.
    pub secret: Option<String>,
    /// The command a run ask proposes, whole; none on any other.
    pub run: Option<String>,
}

/// One topic's beads, in the order their oldest ask waits.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicGroup {
    /// Without the `topic:` prefix, or "Everything else".
    pub topic: String,
    pub rows: Vec<DecisionRow>,
}

/// One dealt with, with its pick.
#[derive(Debug, Clone, PartialEq)]
pub struct DecidedRow {
    pub bead: String,
    pub ask: u64,
    /// The question's first line in plain words.
    pub head: String,
    /// `<asker> · <pick|declined|put aside|taken back by ...>`.
    pub sub: String,
    pub short: String,
    pub selected: bool,
}

/// One platform's arrivals, with the served rows that stand at it, in
/// the served order.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrivalGroup {
    pub platform: String,
    pub rows: Vec<Arrival>,
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
    /// that shows marks, where another has options or a line for words.
    pub secret: Option<Secret>,
    /// The command a run ask proposes: its card has the yard's block and
    /// the control that approves, and its question and context are marked
    /// as the asker's.
    pub run: Option<Run>,
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
    /// `Bead` with `bead`; answered to [`Model::bead_loaded`] or
    /// [`Model::bead_failed`].
    Bead { bead: String },
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
    /// The platforms in the order the beads travel, as served beside
    /// the rows: the board groups under them. Empty from a yard that
    /// serves no list, and then the board groups in first-appearance
    /// order.
    arrival_platforms: Vec<String>,
    /// The flows view as read: the lines with their flows, stages, beads
    /// and lamps, the unassigned rows, and the travelling totals.
    flows: Flows,
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
    /// One bead whole, by id, as the cards show it: read when a decision
    /// row opens, and again on every event whose bead is one shown.
    beads: HashMap<String, Bead>,
    /// The option chosen with a number key or the arrows, by bead and ask.
    chosen: HashMap<(String, u64), u64>,
    /// The asks whose card stands expanded, by bead and ask.
    expanded: HashSet<(String, u64)>,
    /// The run asks whose carried texts stand shown, by bead and ask: the
    /// tap behind the yard's sentences.
    texts: HashSet<(String, u64)>,
    /// Which ask of a bead with several the keys act on, by bead.
    current: HashMap<String, u64>,
    /// The seq of the last `crew_status` applied to each crew member: a
    /// read of the yard that is older does not put an older status back.
    status_at: HashMap<String, u64>,
    /// What arrived and the window has not taken yet, oldest first.
    news: Vec<News>,
    /// The window shows another yard: no row of this one is open, and none
    /// is opened by what arrives, so nothing of it is seen or marked read
    /// until the window is back ([`Model::back_to`]).
    aside: bool,
    /// What went wrong last, in words; empty when nothing.
    pub notice: String,
}

impl Model {
    /// The model of a state just read. `selected` is what was open before
    /// the yard was lost, if any: it stays open when its row is still
    /// there. Else the mode's own default is.
    pub fn new(state: State, mode: Mode, selected: Option<Selected>) -> (Model, Vec<Effect>) {
        Self::new_returning(state, mode, selected, false)
    }

    /// The model of a state read when returning to a yard. The folded
    /// "Decided" list must be open before `selected` is checked, since its
    /// rows do not exist in the order while it is folded.
    pub fn new_returning(
        state: State,
        mode: Mode,
        selected: Option<Selected>,
        decided_expanded: bool,
    ) -> (Model, Vec<Effect>) {
        let mut m = Self::of(state, decided_expanded);
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
            ..Self::of(state, false)
        }
    }

    /// What a state just read has, with no row open yet.
    fn of(state: State, decided_expanded: bool) -> Model {
        let mut m = Model {
            me: state.me,
            parties: parties(&state.peers),
            trust: trusted(&state.peers),
            crew: state.crew,
            arrivals: state.arrivals.rows,
            arrival_platforms: state.arrivals.platforms,
            flows: state.flows,
            cursor: state.cursor,
            asked: state.cursor,
            notifications: state.notifications,
            open: state.asks.open,
            decided: state.asks.decided,
            decided_expanded,
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
                // that list was kept: to an open decision, or none.
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
    ) -> Vec<Effect> {
        self.aside = false;
        self.mode = mode;
        self.decided_expanded |= decided_expanded;
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
    /// when "Decided" stands open in decisions, the arrivals row, the
    /// lines in the served order and No line last in timetable, while
    /// unassigned beads travel. A travelling bead is open from the board,
    /// a flow, Elsewhere or No line, and has no row here.
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
                    for ask in &self.decided {
                        out.push(Selected::Decided(ask.bead.clone(), ask.ask));
                    }
                }
            }
            Mode::Timetable => {
                out.push(Selected::Arrivals);
                out.extend(
                    self.flows
                        .depots
                        .iter()
                        .map(|depot| Selected::Line(depot.name.clone())),
                );
                if !self.flows.unassigned.is_empty() {
                    out.push(Selected::NoLine);
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

    /// The conversations of the list on the left.
    pub fn rows(&self) -> Vec<Row> {
        let crew = self
            .crew
            .iter()
            .map(|r| (r.name.clone(), state_of(r).to_string()));
        let people = self.people().map(|u| {
            let line = if *u == self.me {
                "you, notes to self"
            } else {
                "person"
            };
            (u.clone(), line.to_string())
        });
        let folded: Vec<String> = self
            .parties
            .iter()
            .map(Party::address)
            .filter(|a| self.folded(a).is_some())
            .collect();
        let linked = self.parties.iter().map(|p| {
            let is = if p.person { "person" } else { "crew member" };
            let line = if p.role.is_empty() {
                is.to_string()
            } else {
                format!("{is}: {}", plain(&p.role))
            };
            (p.address(), line)
        });
        crew.chain(linked.filter(|(address, _)| !folded.contains(address)))
            .chain(people)
            .map(|(name, line)| Row {
                unread: self.unread(&name),
                selected: self.selected_conversation() == Some(name.as_str()),
                name,
                line,
            })
            .collect()
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
            let row = DecisionRow {
                bead: bead.clone(),
                head: decision_head(first, &asks),
                asks: asks
                    .iter()
                    .map(|ask| {
                        let full =
                            cached.and_then(|b| b.asks.iter().find(|full| full.ask == ask.ask));
                        DecisionRowAsk {
                            ask: ask.ask,
                            head: row_head(&ask.question),
                            options: full.map_or(Vec::new(), |full| {
                                card_options(full)
                                    .into_iter()
                                    .map(|option| (option.number, option.label))
                                    .collect()
                            }),
                            secret: ask.secret.as_ref().map(|secret| secret.line.clone()),
                            run: ask.run.as_ref().map(|run| plain(&run.line())),
                        }
                    })
                    .collect(),
                sub: decision_sub(first, &asks, now),
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

    /// The arrivals board as read: the rows as served, ordered and
    /// remarked.
    pub fn arrival_rows(&self) -> &[Arrival] {
        &self.arrivals
    }

    /// The arrivals board as read again.
    pub fn arrivals(&mut self, board: Arrivals) {
        self.arrivals = board.rows;
        self.arrival_platforms = board.platforms;
    }

    /// The served rows under their platform, the platforms in the served
    /// travel order, each with its rows in the served order. A row whose
    /// platform the served list does not name stands under its own group
    /// at the end, in first-appearance order, so the board still draws
    /// against a yard that serves no list. footplate invents no stage
    /// order of its own: the groups stand as served.
    pub fn arrival_groups(&self) -> Vec<ArrivalGroup> {
        let mut groups: Vec<ArrivalGroup> = Vec::new();
        for row in self.arrival_rows() {
            match groups.iter_mut().find(|g| g.platform == row.platform) {
                Some(g) => g.rows.push(row.clone()),
                None => groups.push(ArrivalGroup {
                    platform: row.platform.clone(),
                    rows: vec![row.clone()],
                }),
            }
        }
        // In the served order; what it does not name stands last. The
        // sort is stable, so those keep first-appearance order.
        groups.sort_by_key(|g| {
            self.arrival_platforms
                .iter()
                .position(|p| p == &g.platform)
                .unwrap_or(usize::MAX)
        });
        groups
    }

    /// The yard-wide travelling total the Flows call serves: the open
    /// beads outside the backlog, complete, not capped.
    pub fn travelling(&self) -> usize {
        self.flows.travelling
    }

    /// The flows view as read again. A line or No line that is gone
    /// falls back to the board; a travelling bead is never gone.
    pub fn flows(&mut self, board: Flows) -> Vec<Effect> {
        self.flows = board;
        self.reselect()
    }

    /// One line's flows as served, when the yard serves the line.
    pub fn depot(&self, name: &str) -> Option<&FlowDepot> {
        self.flows.depots.iter().find(|d| d.name == name)
    }

    /// The beads travelling with no line, newest change first, as served.
    pub fn unassigned(&self) -> &[FlowBead] {
        &self.flows.unassigned
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
            .map(|depot| {
                let back = Selected::Line(depot.name.clone());
                Row {
                    name: depot.name.clone(),
                    line: line_transit_words(depot.travelling),
                    unread: 0,
                    selected: self.selected.as_ref() == Some(&back)
                        || matches!(self.selected.as_ref(), Some(Selected::Bead(_)))
                            && self.bead_back() == back,
                }
            })
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
        })
    }

    /// A travelling bead's row of the flows view with the depot it stands
    /// in, `None` for one travelling with no line. The stages first, in
    /// the served order, then the depot's Elsewhere, then the unassigned.
    fn flow_bead(&self, bead: &str) -> Option<(&FlowBead, Option<&str>)> {
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

    /// One travelling bead whole, as the details pane shows it: what the
    /// Bead call returned, with the line, expected arrival and remark of
    /// the row it came from. The board's row where the yard serves the
    /// bead still, else the flows view's; `no line` for a bead with none.
    /// None until the bead is read.
    pub fn arrival_detail(&self, bead: &str) -> Option<ArrivalDetail> {
        let b = self.beads.get(bead)?;
        let row = self.arrivals.iter().find(|r| r.bead == bead);
        let flowed = self.flow_bead(bead);
        let (line, expected, remark, tone) = match (row, flowed) {
            (Some(r), _) if !r.line.is_empty() => (
                r.line.clone(),
                r.expected.clone(),
                r.remark.clone(),
                r.tone.clone(),
            ),
            (Some(r), _) => (
                "no line".to_string(),
                r.expected.clone(),
                r.remark.clone(),
                r.tone.clone(),
            ),
            (None, Some((found, Some(depot)))) => (
                depot.to_string(),
                found.expected.clone(),
                found.remark.clone(),
                found.tone.clone(),
            ),
            (None, Some((found, None))) => (
                "no line".to_string(),
                found.expected.clone(),
                found.remark.clone(),
                found.tone.clone(),
            ),
            (None, None) => (String::new(), String::new(), String::new(), String::new()),
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
    pub fn decided_rows(&self) -> Vec<DecidedRow> {
        self.decided
            .iter()
            .map(|ask| DecidedRow {
                bead: ask.bead.clone(),
                ask: ask.ask,
                head: row_head(&ask.question),
                sub: decided_sub(ask),
                short: short_bead(&ask.bead),
                selected: self.selected.as_ref()
                    == Some(&Selected::Decided(ask.bead.clone(), ask.ask)),
            })
            .collect()
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
                        question: card_question(&ask.question, &ask.asker, ask.run.is_some()),
                        asker: ask.asker.clone(),
                        waits: waits(&ask.since, now),
                        recommend_label: full.map(recommend_label).unwrap_or_default(),
                        context: full.map_or(Vec::new(), |f| {
                            card_context(&f.context, &ask.asker, ask.run.is_some())
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
            question: card_question(&a.question, &a.asker, a.run.is_some()),
            asker: a.asker.clone(),
            state: a.state.clone(),
            state_line: decided_state(a, zone),
            picked: a.option,
            pick: a.option_label.clone(),
            words: plain(&a.words),
            return_at: a.return_at.clone(),
            recommend_label: full.map(recommend_label).unwrap_or_default(),
            context: full.map_or(Vec::new(), |f| {
                card_context(&f.context, &a.asker, a.run.is_some())
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
                    given: Vec::new(),
                    run: open.run,
                    ran: None,
                },
            );
        } else {
            return Vec::new();
        }
        self.chosen.remove(&(bead.to_string(), ask));
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
                    given: names.to_vec(),
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
                    run: open.run,
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
                given: Vec::new(),
                run: open.run,
                ran: None,
            },
        );
        self.chosen.remove(&(bead.to_string(), ask));
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
                given: Vec::new(),
                run: open.run,
                ran: None,
            },
        );
        self.chosen.remove(&(bead.to_string(), ask));
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
            })
            .collect()
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
        let mut out: Vec<Entry> = Vec::with_capacity(letters.len());
        for l in letters {
            let mine = l.from == self.me;
            let standing = mine.then(|| self.standing(l));
            // The line is under the last of those that stand alike.
            if let Some(before) = out.last_mut()
                && before.mine
                && before.standing == standing
            {
                before.standing = None;
            }
            out.push(Entry {
                seq: l.seq,
                at: l.at.clone(),
                from: l.from.clone(),
                text: plain(&l.text),
                files: l.files.clone(),
                mine,
                standing,
            });
        }
        out
    }

    /// The hash of every image shown, each once: the open conversation's,
    /// then the open row's expanded cards' notes'. One this yard did not
    /// take from a linked yard is none it has.
    pub fn images(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let letters = match self.selected.as_ref() {
            Some(sel @ Selected::Conversation(_)) => self.letters(sel),
            _ => Vec::new(),
        };
        for f in letters.into_iter().flat_map(|l| &l.files) {
            push_image(&mut out, f);
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
        match self.member(name).map(state_of) {
            Some("idle") => format!("sent; {name} is idle"),
            Some("working") => format!("sent; {name} is working and reads mail when idle"),
            Some("blocked") => format!("{name} waits at a prompt in its pane and cannot read mail"),
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
    /// the newest mail that is not the window's own came from there, else
    /// the user in this yard.
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
            .find(|l| l.from != self.me);
        Some(match last.and_then(|l| l.from.split_once('@')) {
            Some((name, peer)) => Target::Peer {
                name: name.to_string(),
                peer: peer.to_string(),
            },
            None => Target::Crew(name.to_string()),
        })
    }

    /// Whether the yard has the row `sel` to open.
    pub fn has_row(&self, sel: &Selected) -> bool {
        matches!(sel, Selected::Bead(_)) || self.order().contains(sel)
    }

    /// Opens a row of a list: a bead, one dealt with, a conversation,
    /// the yard row, a timetable row, or one travelling bead from the
    /// board, a flow, Elsewhere or No line, which has no row of the list.
    /// A row that is not there changes nothing.
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
    /// bead read the bead whole when it was not read before. The board,
    /// a line and No line need no call: their rows came with the state,
    /// and events re-read them.
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
                effects
            }
            // The yard row has no top to reach among its notices: the mail
            // it shows is read whole.
            Some(sel @ Selected::Yard) => {
                let mut effects: Vec<Effect> = self
                    .sources(&sel)
                    .into_iter()
                    .filter(|from| !self.loaded.contains(from))
                    .map(|name| Effect::Read { name, after: 0 })
                    .collect();
                effects.extend(self.ack());
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

    /// Mail was sent to `to`. It also comes from the stream; reading it
    /// shows it when the stream is lost at this moment, and costs what is
    /// new.
    pub fn sent(&mut self, to: &str) -> Vec<Effect> {
        self.sources(&Selected::Conversation(to.to_string()))
            .iter()
            .map(|with| self.read(with))
            .collect()
    }

    /// Puts `l` into the conversation with `name`, in order. A mail it has
    /// already (the stream and the yard both tell of each) only gains what
    /// `l` knows more: that it was read, or where it stands.
    fn add(&mut self, name: &str, l: Letter) {
        let c = self.convo.entry(name.to_string()).or_default();
        match c.binary_search_by_key(&l.seq, |have| have.seq) {
            Ok(i) => {
                c[i].read |= l.read;
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
            self.add(name, l);
        }
        self.see();
        self.ack()
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
        if e.notified(&self.me) {
            effects.push(Effect::Notifications);
        }
        effects.extend(match e.kind.as_str() {
            "mail" => self.mail(e),
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
            "notifications_read" => {
                let (reader, through) = (e.text("reader"), e.number("through"));
                if reader == self.me {
                    self.notifications_acked(through);
                }
                Vec::new()
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
            // changed, or a crew member's first prompt arrived: the event
            // carries less than what changed.
            kind if kind.starts_with("crew_") => vec![Effect::Crew],
            kind if kind.starts_with("session_prompt") && !e.text("crew").is_empty() => {
                vec![Effect::Crew]
            }
            // Removing a user also removes any link trust that named them;
            // unlike a direct grant change, this is not a `peer_` event.
            "user_removed" => vec![Effect::Peers],
            kind if kind.starts_with("peer_") => self.link(e),
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
                self.add(&l.to.clone(), l);
            }
            return Vec::new();
        }
        // Mail the user wrote to themselves is both: their own, and unread
        // in their inbox until their row shows it.
        let mine = e.actor == self.me;
        let for_me = to == self.me && e.flag("to_user");
        if !mine && !for_me {
            return Vec::new();
        }
        if for_me {
            l.read = e.seq <= self.cursor;
        }
        let read = l.read;
        self.add(if mine { to } else { &e.actor }, l);
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

    /// The notices as read again. The yard row stands whatever they are.
    pub fn notifications(&mut self, list: Vec<Notification>) {
        let before = std::mem::replace(&mut self.notifications, list);
        self.notice_news(&before);
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
        // what the details show.
        if let Some(Selected::Bead(bead)) = self.selected.as_ref() {
            keep.insert(bead.clone());
        }
        self.beads.retain(|bead, _| keep.contains(bead));
        self.chosen.retain(|(bead, _), _| keep.contains(bead));
        self.expanded.retain(|(bead, _)| keep.contains(bead));
        self.current.retain(|bead, _| keep.contains(bead));
        let effects = self.reselect();
        // Which row is open is known only now.
        let selected = self.selected.clone();
        for n in &mut self.news[news..] {
            n.open = selected.as_ref() == Some(&n.target);
        }
        if !effects.is_empty() {
            return effects;
        }
        // The open bead stayed: it is newly in the queue, or newly without
        // a read bead. It is read when it is not yet.
        match self.selected.as_ref() {
            Some(Selected::Decision(_) | Selected::Decided(_, _) | Selected::Bead(_)) => {
                self.open()
            }
            _ => Vec::new(),
        }
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
fn short_bead(bead: &str) -> String {
    bead.rsplit('-').next().unwrap_or(bead).to_string()
}

/// Whether the bead is another yard's question: `<id>@<peer>`, as the
/// queue names it. The yard refuses a plain note on it: notes stay local,
/// and words for the asker go as mail.
pub fn foreign_bead(bead: &str) -> bool {
    bead.contains('@')
}

/// A bead's row head: the question's first line in plain words, or the
/// bead's title with how many it asks.
fn decision_head(first: &OpenAsk, asks: &[&OpenAsk]) -> String {
    if asks.len() == 1 {
        return row_head(&first.question);
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

/// A bead's row sub: whose it is and how long it waits, or how long the
/// oldest of several waits.
fn decision_sub<Tz: TimeZone>(first: &OpenAsk, asks: &[&OpenAsk], now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let waits = waits(&first.since, now);
    if asks.len() == 1 {
        return format!("{} · {waits}", first.asker);
    }
    waits
}

/// A dealt with row sub: whose it was and its pick.
fn decided_sub(ask: &DecidedAsk) -> String {
    let mut sub = ask.asker.clone();
    match ask.state.as_str() {
        "answered" if !ask.option_label.is_empty() => {
            sub = format!("{sub} · {}", plain(&ask.option_label));
            if !ask.words.is_empty() {
                sub = format!("{sub} · {}", plain(&ask.words));
            }
        }
        "answered" if !ask.given.is_empty() => {
            sub = format!("{sub} · gave {}", ask.given.join(", "));
        }
        "answered" if ask.ran.is_some() => {
            sub = format!("{sub} · approved");
        }
        "answered" => {
            sub = format!("{sub} · {}", plain(&ask.words));
        }
        "declined" => {
            sub = format!("{sub} · declined");
            if !ask.words.is_empty() {
                sub = format!("{sub} · {}", plain(&ask.words));
            }
        }
        "taken back" => {
            sub = format!("{sub} · {}", taken_back(ask));
        }
        _ => {
            sub = format!("{sub} · put aside");
        }
    }
    sub
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
        "answered" if !ask.given.is_empty() => format!("you gave {}", ask.given.join(", ")),
        "declined" if ask.secret.is_some() => "you declined".to_string(),
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
/// be sent or not sent; a yard that does not have the call sends its person
/// to a terminal. The words hold what the yard said and never a value.
pub fn not_given(err: &api::Error, bead: &str, ask: u64) -> String {
    match err {
        api::Error::Down(_) | api::Error::Protocol(_) => {
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

/// Whether the yard has no `ApproveAsk` for this window: one from before
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

/// Why a run ask was not approved, for the card. A call that ended without
/// an answer may have run the command all the same, so it is never said to
/// be approved or not: the card, read again, says which. A refusal is the
/// yard's words, whole.
pub fn not_approved(err: &api::Error) -> String {
    match err {
        api::Error::Down(_) | api::Error::Protocol(_) => {
            "the yard did not answer; read the card again".to_string()
        }
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

/// Why what is typed is not sent as it is, if it is not: a mail is 1 to
/// 4096 bytes.
pub fn unsendable(text: &str) -> Option<String> {
    let bytes = text.trim().len();
    match bytes {
        0 => Some(String::new()),
        n if n > MAIL_BYTES => Some(format!(
            "a mail holds {MAIL_BYTES} bytes, and this is {n}: send it in parts"
        )),
        _ => None,
    }
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

/// The line for a file of mail from a linked yard that this yard did not
/// take, with the yard's reason.
pub fn not_taken(file: &BlobRef) -> String {
    let what = match (is_page(file), is_image(file)) {
        (true, _) => "page",
        (false, true) => "image",
        (false, false) => "file",
    };
    format!(
        "{what}, {}, not taken: {}",
        size(file.bytes),
        file.not_taken
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
        assert!(!fresh.back_to(Mode::Chat, None, false).is_empty());
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
        }
    }

    fn board(rows: Vec<Arrival>, platforms: &[&str]) -> Arrivals {
        Arrivals {
            rows,
            platforms: platforms.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn the_board_is_the_board_as_read_now_over_a_yard_lost() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        assert!(m.arrival_rows().is_empty());
        m.arrivals(board(
            vec![arrival("yd-links", "new", "yardr", "2h–3h", "", "none")],
            &[],
        ));
        assert_eq!(m.arrival_rows().len(), 1);
        // Read again after the yard was lost: the board is as read now.
        let (again, _) = m.resumed(yard(&[]), Mode::Chat);
        assert!(again.arrival_rows().is_empty());
    }

    /// The board groups the served rows by platform, the platforms in
    /// the served travel order, each with its rows in the served order.
    /// A group of one is still a group, and nothing is remarked here:
    /// the rows are as served.
    #[test]
    fn the_board_groups_the_served_rows_by_platform_in_the_served_order() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let rows = vec![
            arrival("yd-second", "review", "yardr", "20m–35m", "", "none"),
            arrival("yd-third", "draft", "yardr", "5h–8h", "late", "amber"),
            arrival("yd-fourth", "review", "yardr", "1h–2h", "held", "red"),
        ];
        // The served list names draft first, though review appears first
        // in the served rows: the groups stand as served.
        m.arrivals(board(rows.clone(), &["draft", "review"]));
        let groups = m.arrival_groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].platform, "draft");
        assert_eq!(groups[1].platform, "review");
        // Each with its rows in the served order, remarked as served.
        assert_eq!(groups[0].rows, vec![rows[1].clone()]);
        assert_eq!(groups[1].rows, vec![rows[0].clone(), rows[2].clone()]);
        assert_eq!(m.arrival_rows(), rows.as_slice());
    }

    /// Without a served platforms list the board groups as it always
    /// has: the platforms in the order they first appear in the served
    /// order, each with its rows in the served order.
    #[test]
    fn without_a_platforms_list_the_board_groups_in_first_appearance_order() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let rows = vec![
            arrival("yd-second", "review", "yardr", "20m–35m", "", "none"),
            arrival("yd-third", "draft", "yardr", "5h–8h", "late", "amber"),
            arrival("yd-fourth", "review", "yardr", "1h–2h", "held", "red"),
        ];
        m.arrivals(board(rows.clone(), &[]));
        let groups = m.arrival_groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].platform, "review");
        assert_eq!(groups[1].platform, "draft");
        assert_eq!(groups[0].rows, vec![rows[0].clone(), rows[2].clone()]);
        assert_eq!(groups[1].rows, vec![rows[1].clone()]);
    }

    /// A row whose platform the served list does not name stands under
    /// its own group at the end, in first-appearance order with the
    /// other unnamed ones.
    #[test]
    fn a_platform_the_list_does_not_name_stands_last_in_appearance_order() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Chat, None);
        let rows = vec![
            arrival("yd-first", "shape", "yardr", "20m–35m", "", "none"),
            arrival("yd-second", "review", "yardr", "20m–35m", "", "none"),
            arrival("yd-third", "draft", "yardr", "5h–8h", "late", "amber"),
            arrival("yd-fourth", "build", "yardr", "1h–2h", "held", "red"),
            arrival("yd-fifth", "review", "yardr", "1h–2h", "", "none"),
        ];
        m.arrivals(board(rows.clone(), &["draft", "review"]));
        let groups = m.arrival_groups();
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].platform, "draft");
        assert_eq!(groups[1].platform, "review");
        // Shape and build are not on the list: their own groups at the
        // end, in the order they first appear.
        assert_eq!(groups[2].platform, "shape");
        assert_eq!(groups[3].platform, "build");
        assert_eq!(groups[0].rows, vec![rows[2].clone()]);
        assert_eq!(groups[1].rows, vec![rows[1].clone(), rows[4].clone()]);
        assert_eq!(groups[2].rows, vec![rows[0].clone()]);
        assert_eq!(groups[3].rows, vec![rows[3].clone()]);
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
        m.arrivals(board(
            vec![arrival("yd-a", "new", "yardr", "2h–3h", "", "none")],
            &[],
        ));
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
                        stages: vec![FlowStage {
                            stage: "new".into(),
                            group: "".into(),
                            human: false,
                            lamp: "none".into(),
                            beads: vec![],
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
        // Over a yard lost the row comes back too.
        m.flows(flow_view());
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
    /// board's for a bead from there.
    #[test]
    fn stepping_from_a_line_s_bead_uses_the_line_s_row() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.flows(flow_view());
        m.select(Selected::Line("yardr".into()));
        m.select(Selected::Bead("yd-zulu".into()));
        m.step_in(Mode::Timetable, 1);
        assert_eq!(m.selected(), Some(&Selected::Line("footplate".into())));
        m.select(Selected::Line("yardr".into()));
        m.select(Selected::Bead("yd-zulu".into()));
        m.step_in(Mode::Timetable, -1);
        assert_eq!(m.selected(), Some(&Selected::Arrivals));
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
        m.arrivals(board(
            vec![arrival("yd-stray", "new", "", "1h–2h", "", "none")],
            &[],
        ));
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
        m.arrivals(board(
            vec![arrival(
                "yd-zulu", "draft", "yardr", "5h–8h", "late", "amber",
            )],
            &[],
        ));
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
    }

    /// The open arrival's bead stays open and read while the queue is
    /// read again: it is none of the queue's, and no decision's leaving
    /// moves it.
    #[test]
    fn the_open_arrival_s_bead_stays_open_over_a_queue_read_again() {
        let (mut m, _) = Model::new(yard(&[]), Mode::Timetable, None);
        m.arrivals(board(
            vec![arrival("yd-zulu", "draft", "yardr", "5h–8h", "", "none")],
            &[],
        ));
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
    fn a_mail_is_one_to_4096_bytes() {
        assert_eq!(unsendable("hello"), None);
        assert_eq!(unsendable("  \n "), Some(String::new()));
        assert_eq!(unsendable(&"x".repeat(4096)), None);
        let long = unsendable(&"é".repeat(2049)).unwrap();
        assert!(long.contains("this is 4098"), "{long}");
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
            run: None,
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
            given: Vec::new(),
            run: None,
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
        assert_eq!(row.sub, "yardmaster · 2h");
        assert_eq!(row.short, "n8ta");
        assert_eq!(row.bead, "yardr-n8ta");
        // No row reads "<id> go?".
        assert!(!row.head.contains("yardr-n8ta"), "{}", row.head);
        // A day's wait.
        assert_eq!(groups[1].rows[0].sub, "assembly · 1d");
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
        let rows = m.decided_rows();
        assert_eq!(rows[0].head, "Send them to build?");
        assert_eq!(rows[0].sub, "yardmaster · Send them · tonight");
        assert_eq!(rows[1].sub, "yardmaster · warmly");
        assert_eq!(rows[2].sub, "yardmaster · declined · not mine");
        assert_eq!(rows[3].sub, "yardmaster · put aside");
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
        let rows = m.decided_rows();
        assert_eq!(
            rows[0].head,
            "Move this yard's 64 links to the declared source now?"
        );
        assert_eq!(
            rows[0].sub,
            "yardmaster · Move them now · move them tonight"
        );
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
    fn a_decisions_row_shows_the_question_s_first_line() {
        let question = "Take more?\nwhen you approve, the yard runs this as mckean, and nothing else:\n    yardr peer list";
        let m = with_a_run_ask_about(question, &[]);
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.head, "Take more?");
        assert!(!row.head.contains('\n'), "{:?}", row.head);
        assert!(!row.head.contains("when you approve"), "{:?}", row.head);
        assert!(!row.head.contains("yardr peer list"), "{:?}", row.head);
        // Whose words they are, the row's sub says.
        assert_eq!(row.sub, "yardmaster · 2h");
        // A break that is no line break to Rust still ends the row's line.
        let m = with_a_run_ask_about("Take more?\u{2028}the yard says: two", &[]);
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.head, "Take more?");
        // So for another ask: the first line, without the asker's indent.
        let mut m = with_a_run_ask_about("  Take more?\n    yardr peer list", &[]);
        m.open[0].run = None;
        let row = &m.decision_groups(&noon())[0].rows[0];
        assert_eq!(row.head, "Take more?");
    }

    /// A Decided row shows one line of the question, the same way: its
    /// first line, in plain words. A run ask put aside can still be
    /// approved, so its row matters as much as an open one's.
    #[test]
    fn a_decided_row_shows_the_question_s_first_line() {
        let question = "Take more?\nwhen you approve, the yard runs this as mckean, and nothing else:\n    yardr peer list";
        let mut m = with_a_run_ask_about(question, &[]);
        assert!(m.deferred("yd-link", 2710).contains(&Effect::Asks));
        let row = &m.decided_rows()[0];
        assert_eq!(row.head, "Take more?");
        assert!(!row.head.contains('\n'), "{:?}", row.head);
        assert!(!row.head.contains("when you approve"), "{:?}", row.head);
        assert!(!row.head.contains("yardr peer list"), "{:?}", row.head);
        // Whose words they are, the row's sub says.
        assert_eq!(row.sub, "yardmaster · put aside");
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

        let effects = m.approved("yd-link", 2710, &approved());
        assert!(effects.contains(&Effect::Asks));
        assert!(effects.contains(&Effect::Bead {
            bead: "yd-link".into()
        }));
        assert_eq!(m.decisions_count(), 0);
        assert_eq!(m.decided_rows()[0].sub, "yardmaster · approved");
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
        let mut m = with_a_run_ask(&[]);
        assert!(m.deferred("yd-link", 2710).contains(&Effect::Asks));
        assert_eq!(m.decided_rows()[0].sub, "yardmaster · put aside");
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
        ] {
            assert!(!approves_elsewhere(&none));
            assert_eq!(
                not_approved(&none),
                "the yard did not answer; read the card again"
            );
        }
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
        assert_eq!(m.decided_rows()[0].sub, "yardmaster · gave BOOKS_TOKEN");
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
        assert_eq!(m.decided_rows()[0].sub, "yardmaster · declined");
        assert_eq!(m.secret("yd-crm", 18), Some(&books()));
        assert_eq!(m.giving("yd-crm", 18), None);
        // Put aside, it is given still.
        let mut m = with_a_secret_ask();
        m.deferred("yd-crm", 18);
        assert_eq!(m.giving("yd-crm", 18), Some(&books()));
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
        let rows = m.decided_rows();
        assert_eq!(
            rows[0].sub,
            "yardmaster · taken back by mayor: the train left"
        );
        assert_eq!(
            rows[1].sub,
            "yardmaster · it was taken back in north's yard"
        );
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
        ] {
            let why = not_given(&err, "yd-crm", 18);
            assert_eq!(
                why,
                "the yard did not answer. The value may be written. Give it again."
            );
        }
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
        assert_eq!(m.decided_rows()[0].sub, "yardmaster · put aside");

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
