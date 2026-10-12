//! What the window shows, without the window: who there is to talk with,
//! which conversation is open, what is unread, and what the yard has to be
//! asked next. It follows `yardr chat` (internal/chat of the yardr
//! repository): the state is read once, and events are applied from then on.
//!
//! Nothing here calls the yard. A change that needs a call answers it as an
//! [`Effect`], and the window makes it and hands the answer back.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, TimeZone};

use crate::api::{
    AboutBead, Asks, Bead, BeadAsk, BlobRef, CrewMember, DecidedAsk, Event, Letter, Me,
    Notification, OpenAsk, Peer,
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

/// What a window for the user `me` starts from, read at the seq `head`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub me: String,
    pub head: u64,
    pub crew: Vec<CrewMember>,
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
    /// A crew member's name, or `<name>@<peer>`.
    pub name: String,
    /// Under the name: a crew member's state, or what one of a linked yard
    /// is there and their role line.
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
    /// The images it carries, shown under its text.
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
    /// `Mail` to a crew member of this yard.
    Crew(String),
    /// `MailPeer` to one of a linked yard.
    Peer { name: String, peer: String },
}

/// What the left list has open: a bead with open asks, one dealt with, a
/// conversation, or what the yard reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Selected {
    /// A bead with open asks, by its id. A bead with several is one row.
    Decision(String),
    /// One dealt with, by bead and ask note seq.
    Decided(String, u64),
    /// A conversation, by crew name or `<name>@<peer>`.
    Conversation(String),
    Yard,
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

/// One bead's row among the decisions: the question in plain words, or the
/// bead's title with how many it asks.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRow {
    /// The bead's id, whole; `short` is what the row ends with.
    pub bead: String,
    /// The question, or `<title>: <n> questions`.
    pub head: String,
    /// `<asker> · <waits>`, or `<waits>` when several ask.
    pub sub: String,
    /// The bead's id small, at the row's end: past its last `-`.
    pub short: String,
    pub selected: bool,
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
    /// The question in plain words.
    pub head: String,
    /// `<asker> · <pick|declined|put aside>`.
    pub sub: String,
    pub short: String,
    pub selected: bool,
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

/// One note of the bead's body, as the expanded card shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct CardNote {
    pub author: String,
    pub text: String,
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
    /// `answered`, `put aside` or `declined`.
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
}

/// A call the yard has to be asked, with what the model does on its answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// `Conversation` with `name` after a seq; answered to
    /// [`Model::conversation`].
    Read { name: String, after: u64 },
    /// `AckUserInbox` through a seq; answered to [`Model::acked`] or
    /// [`Model::ack_failed`].
    Ack { through: u64 },
    /// `Head`, then `Crew`; answered to [`Model::listed`].
    Crew,
    /// `Peers`; answered to [`Model::linked`].
    Peers,
    /// `UserNotifications`; answered to [`Model::notifications`].
    Notifications,
    /// `AckUserNotifications` through a seq; answered to
    /// [`Model::notifications_acked`] or [`Model::notifications_ack_failed`].
    AckNotifications { through: u64 },
    /// `UserAsks`; answered to [`Model::asks`].
    Asks,
    /// `Bead` with `bead`; answered to [`Model::bead_loaded`] or
    /// [`Model::bead_failed`].
    Bead { bead: String },
}

/// One unread mail to the user: who sent it, and whether the window has
/// shown it.
#[derive(Debug, Clone, PartialEq)]
struct Waiting {
    from: String,
    seen: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    me: String,
    crew: Vec<CrewMember>,
    parties: Vec<Party>,
    selected: Option<Selected>,
    /// The conversation that was open last: the one opened again when the
    /// open row is gone, so that a decision dealt with leads back to where
    /// it was opened from.
    back: Option<String>,
    /// The conversations by whom they are with, each in order of seq, and
    /// those the yard was asked for whole.
    convo: HashMap<String, Vec<Letter>>,
    loaded: HashSet<String>,
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
    /// Which ask of a bead with several the keys act on, by bead.
    current: HashMap<String, u64>,
    /// The seq of the last `crew_status` applied to each crew member: a
    /// read of the yard that is older does not put an older status back.
    status_at: HashMap<String, u64>,
    /// What went wrong last, in words; empty when nothing.
    pub notice: String,
}

impl Model {
    /// The model of a state just read. `selected` is what was open before
    /// the yard was lost, if any: it stays open when its row is still
    /// there. Else the yardmaster's conversation is, as in `yardr chat`,
    /// else the first row from the top.
    pub fn new(state: State, selected: Option<Selected>) -> (Model, Vec<Effect>) {
        let mut m = Model {
            me: state.me,
            parties: parties(&state.peers),
            crew: state.crew,
            cursor: state.cursor,
            asked: state.cursor,
            notifications: state.notifications,
            open: state.asks.open,
            decided: state.asks.decided,
            ..Model::default()
        };
        for l in state.unread.iter().filter(|l| !l.read && l.seq > m.cursor) {
            m.unread.insert(
                l.seq,
                Waiting {
                    from: l.from.clone(),
                    seen: false,
                },
            );
        }
        let order = m.order();
        m.selected = selected
            .filter(|s| order.contains(s))
            .or_else(|| {
                order
                    .iter()
                    .find(|s| matches!(s, Selected::Decision(_)))
                    .cloned()
            })
            .or_else(|| {
                order.iter().find_map(|s| match s {
                    Selected::Conversation(name) if name == "yardmaster" => Some(s.clone()),
                    _ => None,
                })
            })
            .or_else(|| order.into_iter().next());
        m.opened();
        let effects = m.open();
        (m, effects)
    }

    /// The model of a state read again after the yard was lost: what was
    /// open stays open as in [`Model::new`], and the conversation to come
    /// back to from a decision stays that one.
    pub fn resumed(&self, state: State) -> (Model, Vec<Effect>) {
        let (mut m, effects) = Model::new(state, self.selected.clone());
        if m.back.is_none() {
            m.back = self.back.clone();
        }
        (m, effects)
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

    /// The names of the conversations, in order: the crew, then the
    /// addresses of those the linked yards expose.
    fn conversation_names(&self) -> Vec<String> {
        self.crew
            .iter()
            .map(|r| r.name.clone())
            .chain(self.parties.iter().map(Party::address))
            .collect()
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

    /// Every row of the left list from the top: the beads with open asks,
    /// the dealt with when "Decided" stands open, the conversations, the
    /// yard row.
    fn order(&self) -> Vec<Selected> {
        let mut out: Vec<Selected> = Vec::new();
        for (bead, _) in self.decision_beads() {
            out.push(Selected::Decision(bead));
        }
        if self.decided_expanded {
            for ask in &self.decided {
                out.push(Selected::Decided(ask.bead.clone(), ask.ask));
            }
        }
        out.extend(
            self.conversation_names()
                .into_iter()
                .map(Selected::Conversation),
        );
        out.push(Selected::Yard);
        out
    }

    fn member(&self, name: &str) -> Option<&CrewMember> {
        self.crew.iter().find(|r| r.name == name)
    }

    /// The one of a linked yard with the address, when a peer exposes it.
    pub fn party(&self, address: &str) -> Option<&Party> {
        self.parties.iter().find(|p| p.address() == address)
    }

    /// How many mails from `name` the user has not seen.
    pub fn unread(&self, name: &str) -> usize {
        self.unread.values().filter(|w| w.from == name).count()
    }

    /// The unread mail of senders who have no row (another user, one of a
    /// linked yard that is not on its roster): how many, and from whom, in
    /// name order. `yardr inbox` reads it.
    pub fn others(&self) -> (usize, Vec<String>) {
        let names = self.conversation_names();
        let mut from: Vec<String> = Vec::new();
        let mut count = 0;
        for w in self.unread.values().filter(|w| !names.contains(&w.from)) {
            count += 1;
            if !from.contains(&w.from) {
                from.push(w.from.clone());
            }
        }
        from.sort();
        (count, from)
    }

    /// The conversations of the list on the left.
    pub fn rows(&self) -> Vec<Row> {
        let crew = self
            .crew
            .iter()
            .map(|r| (r.name.clone(), state_of(r).to_string()));
        let linked = self.parties.iter().map(|p| {
            let is = if p.person { "person" } else { "crew member" };
            let line = if p.role.is_empty() {
                is.to_string()
            } else {
                format!("{is}: {}", plain(&p.role))
            };
            (p.address(), line)
        });
        crew.chain(linked)
            .map(|(name, line)| Row {
                unread: self.unread(&name),
                selected: self.selected_conversation() == Some(name.as_str()),
                name,
                line,
            })
            .collect()
    }

    /// How many beads have open asks: the `decisions (n)`.
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
            let row = DecisionRow {
                bead: bead.clone(),
                head: decision_head(first, &asks),
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

    /// The dealt with, newest first, with their picks.
    pub fn decided_rows(&self) -> Vec<DecidedRow> {
        self.decided
            .iter()
            .map(|ask| DecidedRow {
                bead: ask.bead.clone(),
                ask: ask.ask,
                head: plain(&ask.question),
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
                        text: plain(&n.text),
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
                        question: plain(&ask.question),
                        asker: ask.asker.clone(),
                        waits: waits(&ask.since, now),
                        recommend_label: full.map(recommend_label).unwrap_or_default(),
                        context: full
                            .map_or(Vec::new(), |f| f.context.iter().map(|c| plain(c)).collect()),
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
                        text: plain(&n.text),
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
            question: plain(&a.question),
            asker: a.asker.clone(),
            state: a.state.clone(),
            state_line: decided_state(a, zone),
            picked: a.option,
            pick: a.option_label.clone(),
            words: plain(&a.words),
            return_at: a.return_at.clone(),
            recommend_label: full.map(recommend_label).unwrap_or_default(),
            context: full.map_or(Vec::new(), |f| f.context.iter().map(|c| plain(c)).collect()),
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
    /// nothing, as does an ask the bead does not show.
    pub fn choose(&mut self, bead: &str, ask: u64, option: u64) {
        if option == 0 {
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
        if last == 0 {
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
    /// Enter alone sends nothing, not even the recommendation.
    pub fn answer_action(&self, bead: &str, ask: u64, words: &str) -> Option<AnswerAction> {
        let full = self.bead_ask(bead, ask)?;
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
                    return_at: String::new(),
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
                return_at: String::new(),
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

    /// The ask is declined with words: not a pick and not "you decide".
    pub fn declined(&mut self, bead: &str, ask: u64, words: &str) -> Vec<Effect> {
        let words = words.trim().to_string();
        if words.is_empty() {
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
                return_at: String::new(),
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

    /// How many notifications the user has not read.
    pub fn yard_unread(&self) -> usize {
        self.notifications.iter().filter(|n| !n.read).count()
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
    /// and for one of a linked yard who that is there, then whatever keeps
    /// the mail from arriving.
    pub fn about(&self) -> Vec<String> {
        let Some(p) = self.selected_conversation().and_then(|s| self.party(s)) else {
            return Vec::new();
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

    /// The open conversation, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        let Some(letters) = self.selected_conversation().and_then(|s| self.convo.get(s)) else {
            return Vec::new();
        };
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

    /// The hash of every image the open conversation shows, each once. One
    /// this yard did not take from a linked yard is none it has.
    pub fn images(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let letters = self.selected_conversation().and_then(|s| self.convo.get(s));
        for f in letters.into_iter().flatten().flat_map(|l| &l.files) {
            if f.not_taken.is_empty() && !out.contains(&f.hash.as_str()) {
                out.push(&f.hash);
            }
        }
        out
    }

    /// Where a mail the user sent stands: read or not for a crew member,
    /// and for one of a linked yard its delivery, since that yard does not
    /// say what was read.
    fn standing(&self, l: &Letter) -> String {
        if let Some((_, peer)) = l.to.split_once('@') {
            return match l.delivery.as_str() {
                "queued" => format!("queued for {peer}: the daemon delivers it"),
                "failed" => "not delivered: yardr inbox says why".to_string(),
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

    /// Where what is typed goes, when a conversation is open.
    pub fn target(&self) -> Option<Target> {
        let name = self.selected_conversation()?;
        Some(match self.party(name) {
            Some(p) => Target::Peer {
                name: p.name.clone(),
                peer: p.peer.clone(),
            },
            None => Target::Crew(name.to_string()),
        })
    }

    /// Opens a row of the list: a bead, one dealt with, a conversation, the
    /// yard row. A row that is not there changes nothing.
    pub fn select(&mut self, sel: Selected) -> Vec<Effect> {
        if !self.order().contains(&sel) {
            return Vec::new();
        }
        self.selected = Some(sel);
        self.notice.clear();
        self.opened();
        self.open()
    }

    /// Opens the row `by` after the open one, around the ends.
    pub fn step(&mut self, by: isize) -> Vec<Effect> {
        let order = self.order();
        if order.is_empty() {
            return Vec::new();
        }
        let n = order.len() as isize;
        let at = self
            .selected
            .as_ref()
            .and_then(|s| order.iter().position(|o| o == s))
            .unwrap_or(0) as isize;
        let next = (at + by).rem_euclid(n) as usize;
        self.select(order[next].clone())
    }

    /// What opening a conversation does here: the unread mail in it is
    /// seen, and it is the one to come back to.
    fn opened(&mut self) {
        let Some(Selected::Conversation(sel)) = &self.selected else {
            return;
        };
        for w in self.unread.values_mut().filter(|w| &w.from == sel) {
            w.seen = true;
        }
        self.back = Some(sel.clone());
    }

    /// Reads the open conversation if it was not read before, and
    /// acknowledges what opening it showed. A bead and one dealt with read
    /// the bead whole when it was not read before; the yard row needs no
    /// call to open.
    fn open(&mut self) -> Vec<Effect> {
        match self.selected.clone() {
            Some(Selected::Conversation(sel)) => {
                let mut effects = Vec::new();
                if !self.loaded.contains(&sel) {
                    effects.push(self.read(&sel));
                }
                effects.extend(self.ack());
                effects
            }
            Some(Selected::Decision(bead)) | Some(Selected::Decided(bead, _)) => {
                if self.beads.contains_key(&bead) {
                    Vec::new()
                } else {
                    vec![Effect::Bead { bead }]
                }
            }
            _ => Vec::new(),
        }
    }

    /// The call for the conversation with `name` after the last mail the
    /// window has of it.
    fn read(&self, name: &str) -> Effect {
        let after = match self.convo.get(name) {
            Some(c) if self.loaded.contains(name) => c.last().map_or(0, |l| l.seq),
            _ => 0,
        };
        Effect::Read {
            name: name.to_string(),
            after,
        }
    }

    /// Mail was sent to `to`. It also comes from the stream; reading it
    /// shows it when the stream is lost at this moment, and costs what is
    /// new.
    pub fn sent(&mut self, to: &str) -> Vec<Effect> {
        vec![self.read(to)]
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
        self.ack()
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
                } else if let Some(c) = self.convo.get_mut(reader) {
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
        effects
    }

    /// A mail event: one the user sent, or one to the user. Mail the user
    /// wrote to another yard is the conversation with `<name>@<peer>`,
    /// queued as the event tells of it; mail from there has that address as
    /// its sender already.
    fn mail(&mut self, e: &Event) -> Vec<Effect> {
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
        if e.actor == self.me {
            self.add(to, l);
        } else if to == self.me && e.flag("to_user") {
            l.read = e.seq <= self.cursor;
            let read = l.read;
            self.add(&e.actor, l);
            if !read && !self.unread.contains_key(&e.seq) {
                // In the open conversation it is seen as it arrives.
                let seen = self.selected_conversation() == Some(e.actor.as_str());
                self.unread.insert(
                    e.seq,
                    Waiting {
                        from: e.actor.clone(),
                        seen,
                    },
                );
                return self.ack();
            }
        }
        Vec::new()
    }

    /// An event of a link. The roster a peer says changes without an event
    /// of its own, so every one is the occasion to read the linked yards
    /// again, and one that tells how a message to a peer ended to read the
    /// conversations with that yard again, whole: where the user's mail
    /// stands is what changed.
    fn link(&mut self, e: &Event) -> Vec<Effect> {
        let mut effects = vec![Effect::Peers];
        let peer = e.text("peer");
        if matches!(e.kind.as_str(), "peer_message_sent" | "peer_message_failed")
            && !peer.is_empty()
        {
            let mut names: Vec<&String> = self
                .loaded
                .iter()
                .filter(|n| n.split_once('@').is_some_and(|(_, of)| of == peer))
                .collect();
            names.sort();
            effects.extend(names.into_iter().map(|name| Effect::Read {
                name: name.clone(),
                after: 0,
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

    /// The linked yards as read again.
    pub fn linked(&mut self, peers: &[Peer]) -> Vec<Effect> {
        self.parties = parties(peers);
        self.reselect()
    }

    /// The notices as read again. The yard row stands whatever they are.
    pub fn notifications(&mut self, list: Vec<Notification>) {
        self.notifications = list;
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

    /// The user's notifications through `through` are read: by this window,
    /// or by anything else that acknowledged them (`yardr inbox --ack`).
    pub fn notifications_acked(&mut self, through: u64) {
        for n in self.notifications.iter_mut().filter(|n| n.seq <= through) {
            n.read = true;
        }
    }

    /// The yard did not take the acknowledgement: the notice stays unread,
    /// and the next `x` asks again.
    pub fn notifications_ack_failed(&mut self, why: &str) {
        self.notice = format!("your notices could not be marked read: {why}");
    }

    /// The queue as read again. When the open bead or the open dealt with
    /// is gone the conversation open last is opened, else the first row
    /// from the top. A bead no longer in
    /// the queue leaves what the cards kept of it; the open bead is read
    /// when it is not yet.
    pub fn asks(&mut self, asks: Asks) -> Vec<Effect> {
        self.open = asks.open;
        self.decided = asks.decided;
        let mut keep = HashSet::new();
        for bead in self
            .open
            .iter()
            .map(|a| &a.bead)
            .chain(self.decided.iter().map(|a| &a.bead))
        {
            keep.insert(bead.clone());
        }
        self.beads.retain(|bead, _| keep.contains(bead));
        self.chosen.retain(|(bead, _), _| keep.contains(bead));
        self.expanded.retain(|(bead, _)| keep.contains(bead));
        self.current.retain(|bead, _| keep.contains(bead));
        let effects = self.reselect();
        if !effects.is_empty() {
            return effects;
        }
        // The open bead stayed: it is newly in the queue, or newly without
        // a read bead. It is read when it is not yet.
        match self.selected.as_ref() {
            Some(Selected::Decision(_) | Selected::Decided(_, _)) => self.open(),
            _ => Vec::new(),
        }
    }

    /// When a put-aside ask comes back next, if one waits: the window reads
    /// the queue again then, since that return arrives with no event.
    pub fn next_return(&self) -> Option<String> {
        self.decided
            .iter()
            .filter(|a| a.state == "put aside" && !a.return_at.is_empty())
            .map(|a| a.return_at.clone())
            .min()
    }

    /// When the open row is gone, opens the conversation that was open
    /// last: a decision picked, put aside, declined or answered elsewhere
    /// leads back to where it was opened from. With none before it, or
    /// when that one is gone too, the first row from the top is opened.
    fn reselect(&mut self) -> Vec<Effect> {
        let order = self.order();
        if self.selected.as_ref().is_some_and(|s| order.contains(s)) {
            return Vec::new();
        }
        self.selected = self
            .back
            .clone()
            .map(Selected::Conversation)
            .filter(|s| order.contains(s))
            .or_else(|| order.into_iter().next());
        self.opened();
        self.open()
    }
}

/// Text as it reaches the window: letters and line breaks, and no other
/// control character. What another yard or a crew member wrote is shown as
/// text and nothing else.
pub fn plain(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
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

/// A bead's row head: the question in plain words, or the bead's title
/// with how many it asks.
fn decision_head(first: &OpenAsk, asks: &[&OpenAsk]) -> String {
    if asks.len() == 1 {
        return plain(&first.question);
    }
    let title = if first.bead_title.is_empty() {
        first.bead.clone()
    } else {
        first.bead_title.clone()
    };
    format!("{}: {} questions", plain(&title), asks.len())
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
        "answered" => {
            sub = format!("{sub} · {}", plain(&ask.words));
        }
        "declined" => {
            sub = format!("{sub} · declined");
            if !ask.words.is_empty() {
                sub = format!("{sub} · {}", plain(&ask.words));
            }
        }
        _ => {
            sub = format!("{sub} · put aside");
        }
    }
    sub
}

/// What one dealt with did with it, in plain words: `you picked 1 · Move
/// them now`, `put aside, back 10-11 09:30`, `you declined: <words>`,
/// `left to <asker>`. A pick carries no time: the yard serves none of its
/// own for an answer, and the ask's `since` is when it arrived, not when
/// it was decided.
fn decided_state<Tz: TimeZone>(ask: &DecidedAsk, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match ask.state.as_str() {
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
        _ => format!("{} · {}", ask.asker, plain(&ask.state)),
    }
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

/// The type of an image the yard takes, read from its bytes as the yard
/// reads it: `image/png` or `image/jpeg`, and nothing for anything else.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// Why an image of `bytes` bytes is not sent in a yard whose cap on one is
/// `cap`, if it is not.
pub fn over_cap(bytes: usize, cap: u64) -> Option<String> {
    (bytes as u64 > cap).then(|| {
        format!(
            "the image is {}, and this yard takes one of {} at most (yardr settings: blob_max_bytes)",
            size(bytes as u64),
            size(cap)
        )
    })
}

/// The line for an image of mail from a linked yard that this yard did not
/// take, with the yard's reason.
pub fn not_taken(file: &BlobRef) -> String {
    format!("image, {}, not taken: {}", size(file.bytes), file.not_taken)
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

    #[test]
    fn the_rows_are_the_crew_then_those_the_linked_yards_expose() {
        let (m, _) = Model::new(yard(&[]), None);
        assert_eq!(
            names(&m),
            ["assembly", "yardmaster", "clerk@north", "mckean@north"]
        );
        let rows = m.rows();
        assert_eq!(rows[0].line, "idle");
        assert_eq!(rows[1].line, "working");
        assert_eq!(rows[2].line, "crew member: Keeps the books");
        assert_eq!(rows[3].line, "person");
    }

    #[test]
    fn a_peer_that_has_not_said_exposes_nobody() {
        let mut state = yard(&[]);
        state.peers.push(Peer {
            name: "south".into(),
            ..Peer::default()
        });
        let (m, _) = Model::new(state, None);
        assert_eq!(names(&m).len(), 4);
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
        let (m, effects) = Model::new(yard(&[]), None);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
        assert_eq!(effects, [read("yardmaster", 0)]);
        assert!(m.rows()[1].selected);
        assert!(!m.rows()[0].selected);
    }

    #[test]
    fn without_a_yardmaster_it_opens_on_the_first_row_and_on_yard_in_an_empty_yard() {
        let mut state = yard(&[]);
        state.crew.remove(1);
        let (m, _) = Model::new(state, None);
        assert_eq!(m.selected_conversation(), Some("assembly"));

        let (mut m, effects) = Model::new(
            State {
                me: "jo".into(),
                ..State::default()
            },
            None,
        );
        // The yard row stands whatever the yard has.
        assert_eq!(m.selected(), Some(&Selected::Yard));
        assert_eq!(effects, []);
        assert_eq!(m.target(), None);
        assert_eq!(m.step(1), []);
        assert_eq!(m.selected(), Some(&Selected::Yard));
        assert_eq!(m.entries(), []);
    }

    #[test]
    fn the_conversation_that_was_open_stays_open_when_the_yard_is_back() {
        let (m, _) = Model::new(
            yard(&[]),
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert_eq!(m.selected_conversation(), Some("clerk@north"));
        // One that is gone meanwhile does not.
        let (m, _) = Model::new(yard(&[]), Some(Selected::Conversation("gone".into())));
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
    }

    #[test]
    fn selecting_opens_a_conversation_and_reads_it_once() {
        let (mut m, _) = Model::new(yard(&[]), None);
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [read("assembly", 0)]
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
    fn step_goes_to_the_next_row_and_around_the_ends() {
        let (mut m, _) = Model::new(yard(&[]), None);
        m.step(1);
        assert_eq!(m.selected_conversation(), Some("clerk@north"));
        m.step(1);
        assert_eq!(m.selected_conversation(), Some("mckean@north"));
        m.step(1);
        assert_eq!(m.selected(), Some(&Selected::Yard));
        m.step(1);
        assert_eq!(m.selected_conversation(), Some("assembly"));
        m.step(-1);
        assert_eq!(m.selected(), Some(&Selected::Yard));
    }

    #[test]
    fn unread_is_counted_by_sender() {
        let (m, _) = Model::new(
            yard(&[(6, "assembly"), (7, "assembly"), (8, "clerk@north")]),
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
        let (m, _) = Model::new(state, None);
        assert_eq!(m.unread("assembly"), 1);
    }

    #[test]
    fn opening_a_conversation_acknowledges_the_mail_it_shows() {
        let (mut m, effects) = Model::new(yard(&[(6, "yardmaster"), (7, "yardmaster")]), None);
        // The yardmaster's is open from the start.
        assert_eq!(effects, [read("yardmaster", 0), Effect::Ack { through: 7 }]);
        // It counts until the yard says it is read.
        assert_eq!(m.unread("yardmaster"), 2);
        m.acked(7);
        assert_eq!(m.unread("yardmaster"), 0);
    }

    #[test]
    fn mail_of_a_conversation_not_opened_holds_back_what_came_after_it() {
        let (mut m, effects) = Model::new(
            yard(&[(6, "yardmaster"), (7, "assembly"), (8, "yardmaster")]),
            None,
        );
        // 8 was shown, but 7 was not: only 6 is acknowledged.
        assert_eq!(effects, [read("yardmaster", 0), Effect::Ack { through: 6 }]);
        m.acked(6);
        assert_eq!((m.unread("yardmaster"), m.unread("assembly")), (1, 1));
        // Opening assembly's shows 7, and 8 was seen before.
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [read("assembly", 0), Effect::Ack { through: 8 }]
        );
        m.acked(8);
        assert_eq!((m.unread("yardmaster"), m.unread("assembly")), (0, 0));
    }

    #[test]
    fn nothing_is_acknowledged_twice() {
        let (mut m, _) = Model::new(yard(&[(6, "yardmaster")]), None);
        // Asked through 6 already, and not answered yet.
        assert_eq!(
            m.conversation("yardmaster", vec![letter(6, "yardmaster", "jo", "x")]),
            []
        );
        m.acked(6);
        assert_eq!(
            m.select(Selected::Conversation("assembly".into())),
            [read("assembly", 0)]
        );
    }

    #[test]
    fn an_acknowledgement_the_yard_did_not_take_is_asked_again() {
        let (mut m, _) = Model::new(yard(&[(6, "yardmaster")]), None);
        m.ack_failed("the yard does not answer");
        assert!(
            m.notice.contains("could not be marked read"),
            "{}",
            m.notice
        );
        assert_eq!(m.unread("yardmaster"), 1);
        assert_eq!(
            m.event(&mail_to_jo(21, "yardmaster")),
            [Effect::Ack { through: 21 }]
        );
    }

    #[test]
    fn mail_that_arrives_in_the_open_conversation_is_shown_and_acknowledged() {
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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
            [read("assembly", 0), Effect::Ack { through: 21 }]
        );
    }

    #[test]
    fn mail_to_someone_else_or_to_a_crew_member_of_the_user_s_name_is_not_the_user_s() {
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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

    #[test]
    fn after_a_send_only_what_is_new_is_read() {
        let (mut m, _) = Model::new(yard(&[]), None);
        m.conversation("yardmaster", vec![letter(12, "yardmaster", "jo", "first")]);
        assert_eq!(m.sent("yardmaster"), [read("yardmaster", 12)]);
        // A conversation never read whole is read whole, whatever the
        // stream put into it.
        m.event(&mail_to_jo(30, "assembly"));
        assert_eq!(m.sent("assembly"), [read("assembly", 0)]);
    }

    #[test]
    fn mail_read_by_the_user_elsewhere_clears_the_count() {
        let (mut m, _) = Model::new(yard(&[(6, "assembly"), (9, "assembly")]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
        assert_eq!(
            m.select(Selected::Conversation("clerk@north".into())),
            [read("clerk@north", 0)]
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
    fn what_is_typed_goes_to_a_crew_member_as_mail() {
        let (m, _) = Model::new(yard(&[]), None);
        assert_eq!(m.target(), Some(Target::Crew("yardmaster".into())));
    }

    #[test]
    fn above_a_conversation_with_a_linked_yard_stands_who_that_is() {
        let mut state = yard(&[]);
        let (m, _) = Model::new(
            state.clone(),
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert_eq!(
            m.about(),
            ["a crew member of the linked yard north: Keeps the books"]
        );
        let (m, _) = Model::new(
            state.clone(),
            Some(Selected::Conversation("mckean@north".into())),
        );
        assert_eq!(m.about(), ["a person of the linked yard north"]);
        let (m, _) = Model::new(
            state.clone(),
            Some(Selected::Conversation("yardmaster".into())),
        );
        assert_eq!(m.about(), Vec::<String>::new());

        state.peers[0].said.as_mut().unwrap().roster[0].mail = false;
        let (m, _) = Model::new(
            state.clone(),
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert!(
            m.about()[1].starts_with("north takes no mail from this yard yet"),
            "{:?}",
            m.about()
        );
        state.peers[0].send.clear();
        let (m, _) = Model::new(state, Some(Selected::Conversation("clerk@north".into())));
        assert!(
            m.about()[1].ends_with("yardr peer allow north --send mail"),
            "{:?}",
            m.about()
        );
    }

    #[test]
    fn a_crew_status_event_changes_the_row() {
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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
    fn a_crew_read_before_a_status_event_does_not_put_the_older_status_back() {
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(yard(&[(6, "assembly")]), None);
        let effects = m.listed(21, vec![member("assembly", "running", "idle")]);
        assert_eq!(m.selected_conversation(), Some("assembly"));
        assert_eq!(effects, [read("assembly", 0), Effect::Ack { through: 6 }]);

        let (mut m, _) = Model::new(
            yard(&[]),
            Some(Selected::Conversation("clerk@north".into())),
        );
        assert_eq!(
            m.event(&event(21, "peer_said", "", json!({"peer": "north"}))),
            [Effect::Peers]
        );
        assert_eq!(m.linked(&[]), [read("assembly", 0)]);
        assert_eq!(names(&m), ["assembly", "yardmaster"]);
        // One that is still there stays open.
        assert_eq!(m.linked(&[north()]), []);
        assert_eq!(m.selected_conversation(), Some("assembly"));
    }

    #[test]
    fn unread_mail_of_senders_without_a_row_is_counted_apart() {
        let (m, _) = Model::new(
            yard(&[(6, "ada"), (7, "zed@south"), (8, "ada"), (9, "assembly")]),
            None,
        );
        assert_eq!(
            m.others(),
            (3, vec!["ada".to_string(), "zed@south".to_string()])
        );
    }

    #[test]
    fn text_reaches_the_window_as_letters_and_line_breaks() {
        assert_eq!(
            plain("one\ntwo\tthree\r\n\u{1b}[31mred\u{7}"),
            "one\ntwo three\n[31mred"
        );
        let (mut m, _) = Model::new(yard(&[]), None);
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

    #[test]
    fn a_mail_s_images_are_shown_with_it_from_the_yard_and_from_the_stream() {
        let (mut m, _) = Model::new(yard(&[]), None);
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
    fn an_image_is_a_png_or_a_jpeg_by_its_bytes() {
        assert_eq!(image_type(PNG), Some("image/png"));
        assert_eq!(image_type(JPEG), Some("image/jpeg"));
        assert_eq!(image_type(b"GIF89a"), None);
        assert_eq!(image_type(b"II*\0"), None);
        assert_eq!(image_type(b""), None);
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
            return_at: return_at.into(),
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
        let (m, _) = Model::new(state, None);
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
    fn first_open_decision_is_selected_before_the_yardmaster_conversation() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(state, None);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));

        let (m, _) = Model::new(yard(&[]), None);
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
    }

    #[test]
    fn a_row_leads_with_the_question_then_asker_and_waits_bead_small_at_its_end() {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(state, None);
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
        let (m, _) = Model::new(state, None);
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
        let (mut m, _) = Model::new(state, None);
        assert_eq!(m.decided_count(), 4);
        assert!(!m.decided_expanded());
        // Folded: its rows are not in the tab order.
        m.select(Selected::Yard);
        m.step(-1);
        assert_eq!(m.selected_conversation(), Some("mckean@north"));

        m.toggle_decided();
        assert!(m.decided_expanded());
        let rows = m.decided_rows();
        assert_eq!(rows[0].head, "Send them to build?");
        assert_eq!(rows[0].sub, "yardmaster · Send them · tonight");
        assert_eq!(rows[1].sub, "yardmaster · warmly");
        assert_eq!(rows[2].sub, "yardmaster · declined · not mine");
        assert_eq!(rows[3].sub, "yardmaster · put aside");
        // Open: its rows stand between the decisions and the conversations.
        m.select(Selected::Yard);
        m.step(-1);
        assert_eq!(m.selected_conversation(), Some("mckean@north"));
        m.select(Selected::Conversation("assembly".into()));
        m.step(-1);
        assert_eq!(
            m.selected(),
            Some(&Selected::Decided("yd-links".into(), 12))
        );
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
        let (m, _) = Model::new(state, None);
        assert_eq!(m.next_return().as_deref(), Some("2026-10-11T09:30:00Z"));

        let (m, _) = Model::new(yard(&[]), None);
        assert_eq!(m.next_return(), None);
    }

    #[test]
    fn the_yard_row_counts_its_unread_and_x_marks_one_read() {
        let mut state = yard(&[]);
        state.notifications = vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed noted", true),
            notification(12, "", "the release went out", false),
        ];
        let (mut m, _) = Model::new(state, None);
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
    fn notices_read_elsewhere_clear_the_count() {
        let mut state = yard(&[]);
        state.notifications = vec![
            notification(10, "yd-shed", "yd-shed created", false),
            notification(11, "yd-shed", "yd-shed noted", false),
        ];
        let (mut m, _) = Model::new(state, None);
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
        let (mut m, _) = Model::new(yard(&[]), None);
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
        let (mut m, _) = Model::new(state, None);
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
        // All answered: the yardmaster's conversation is not first; the
        // first row is.
        m.asks(Asks {
            open: vec![],
            decided: vec![],
        });
        assert_eq!(m.selected_conversation(), Some("assembly"));
    }

    /// The evening's asks with the window opened on a conversation, as
    /// after the yard was lost and is back.
    fn evening_from(conversation: &str) -> Model {
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (m, _) = Model::new(state, Some(Selected::Conversation(conversation.into())));
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
                read("yardmaster", 0),
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
        m.step(-1);
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
    fn a_bead_with_an_ask_left_stays_open_after_a_pick() {
        let mut state = yard(&[]);
        let mut open = evening();
        open[1].bead = "yardr-n8ta".into();
        state.asks.open = open;
        let (mut m, _) = Model::new(state, Some(Selected::Conversation("yardmaster".into())));
        m.select(Selected::Decision("yardr-n8ta".into()));
        m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.picked("yardr-n8ta", 4, 1, "");
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
    }

    #[test]
    fn with_no_conversation_to_go_back_to_the_first_row_is_opened() {
        // The window opened on a decision: the next decision is first.
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = Model::new(state, None);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-n8ta".into())));
        m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));

        // The conversation it was opened from is gone meanwhile.
        let mut m = evening_from("clerk@north");
        m.select(Selected::Decision("yardr-n8ta".into()));
        assert_eq!(m.linked(&[]), []);
        m.picked("yardr-n8ta", 3, 1, "");
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
    }

    #[test]
    fn the_conversation_to_come_back_to_is_kept_when_the_yard_is_back() {
        let mut m = evening_from("yardmaster");
        m.select(Selected::Decision("yardr-3msx".into()));
        let mut state = yard(&[]);
        state.asks.open = evening();
        let (mut m, _) = m.resumed(state);
        assert_eq!(m.selected(), Some(&Selected::Decision("yardr-3msx".into())));
        m.picked("yardr-3msx", 4, 1, "");
        assert_eq!(m.selected_conversation(), Some("yardmaster"));
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
        let (mut m, _) = Model::new(state, None);
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
        let (mut m, _) = Model::new(state, None);
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
        let (mut m, _) = Model::new(state, None);
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
        let (m, _) = Model::new(state, None);
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
        let (mut m, _) = Model::new(state, None);
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
        let (mut m, effects) = Model::new(state, None);
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
            read("yardmaster", 0)
        );
        // Back: the bead is here, so nothing is asked.
        assert_eq!(m.select(Selected::Decision("yardr-n8ta".into())), []);
        // An event whose bead is one shown reads it again, with the queue
        // on a note.
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
                }
            ]
        );
        // A bead's other events, whose kinds are yardr's own, read it too.
        let mut moved = event(23, "advanced", "yard", json!({}));
        moved.bead = "yardr-n8ta".into();
        assert_eq!(
            m.event(&moved),
            [Effect::Bead {
                bead: "yardr-n8ta".into()
            }]
        );
        // One that is shown nowhere reads nothing.
        let mut elsewhere = event(24, "noted", "yardmaster", json!({"note": 3}));
        elsewhere.bead = "yd-elsewhere".into();
        assert_eq!(m.event(&elsewhere), [Effect::Asks]);
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
}
