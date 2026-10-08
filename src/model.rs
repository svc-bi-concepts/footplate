//! What the window shows, without the window: who there is to talk with,
//! which conversation is open, what is unread, and what the yard has to be
//! asked next. It follows `yardr chat` (internal/chat of the yardr
//! repository): the state is read once, and events are applied from then on.
//!
//! Nothing here calls the yard. A change that needs a call answers it as an
//! [`Effect`], and the window makes it and hands the answer back.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, TimeZone};

use crate::api::{BlobRef, CrewMember, Event, Letter, Peer};

/// The kind of message a link must carry out for mail to be written.
const PEER_MAIL: &str = "mail";

/// A mail holds this many bytes.
pub const MAIL_BYTES: usize = 4096;

/// What the yard says of mail with an image for a linked yard, and the
/// window before it asks.
pub const NO_IMAGES_OVER_THE_LINK: &str = "images do not cross the link yet";

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
    selected: Option<String>,
    /// The conversations by whom they are with, each in order of seq, and
    /// those the yard was asked for whole.
    convo: HashMap<String, Vec<Letter>>,
    loaded: HashSet<String>,
    /// The user's unread mail by seq. `cursor` is how far the yard has it
    /// acknowledged, `asked` how far the window has asked for.
    unread: BTreeMap<u64, Waiting>,
    cursor: u64,
    asked: u64,
    /// The seq of the last `crew_status` applied to each crew member: a
    /// read of the crew that is older does not put an older status back.
    status_at: HashMap<String, u64>,
    /// What went wrong last, in words; empty when nothing.
    pub notice: String,
}

impl Model {
    /// The model of a state just read. `selected` is the conversation that
    /// was open before the yard was lost, if any: it stays open when its
    /// row is still there. Else the yardmaster's is, as in `yardr chat`,
    /// else the first.
    pub fn new(state: State, selected: Option<&str>) -> (Model, Vec<Effect>) {
        let mut m = Model {
            me: state.me,
            parties: parties(&state.peers),
            crew: state.crew,
            cursor: state.cursor,
            asked: state.cursor,
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
        let names = m.names();
        m.selected = selected
            .filter(|s| names.iter().any(|n| n == s))
            .or(names
                .iter()
                .map(String::as_str)
                .find(|n| *n == "yardmaster"))
            .or(names.first().map(String::as_str))
            .map(str::to_string);
        m.opened();
        let effects = m.open();
        (m, effects)
    }

    pub fn me(&self) -> &str {
        &self.me
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The names of the list, in order: the crew, then the addresses of
    /// those the linked yards expose.
    fn names(&self) -> Vec<String> {
        self.crew
            .iter()
            .map(|r| r.name.clone())
            .chain(self.parties.iter().map(Party::address))
            .collect()
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
        let names = self.names();
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

    /// The list on the left.
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
                selected: self.selected.as_deref() == Some(name.as_str()),
                name,
                line,
            })
            .collect()
    }

    /// What stands above the open conversation: nothing for a crew member,
    /// and for one of a linked yard who that is there, then whatever keeps
    /// the mail from arriving.
    pub fn about(&self) -> Vec<String> {
        let Some(p) = self.selected().and_then(|s| self.party(s)) else {
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
        let Some(letters) = self.selected().and_then(|s| self.convo.get(s)) else {
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

    /// The hash of every image the open conversation shows, each once.
    pub fn images(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let letters = self.selected().and_then(|s| self.convo.get(s));
        for f in letters.into_iter().flatten().flat_map(|l| &l.files) {
            if !out.contains(&f.hash.as_str()) {
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
        let name = self.selected()?;
        Some(match self.party(name) {
            Some(p) => Target::Peer {
                name: p.name.clone(),
                peer: p.peer.clone(),
            },
            None => Target::Crew(name.to_string()),
        })
    }

    /// Opens the conversation with `name`, a row of the list.
    pub fn select(&mut self, name: &str) -> Vec<Effect> {
        if !self.names().iter().any(|n| n == name) {
            return Vec::new();
        }
        self.selected = Some(name.to_string());
        self.notice.clear();
        self.opened();
        self.open()
    }

    /// Opens the row `by` after the open one, around the ends.
    pub fn step(&mut self, by: isize) -> Vec<Effect> {
        let names = self.names();
        if names.is_empty() {
            return Vec::new();
        }
        let n = names.len() as isize;
        let at = self
            .selected()
            .and_then(|s| names.iter().position(|name| name == s))
            .unwrap_or(0) as isize;
        let next = (at + by).rem_euclid(n) as usize;
        self.select(&names[next])
    }

    /// What opening a conversation does here: the unread mail in it is seen.
    fn opened(&mut self) {
        let Some(sel) = &self.selected else { return };
        for w in self.unread.values_mut().filter(|w| &w.from == sel) {
            w.seen = true;
        }
    }

    /// Reads the open conversation if it was not read before, and
    /// acknowledges what opening it showed.
    fn open(&mut self) -> Vec<Effect> {
        let Some(sel) = self.selected.clone() else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        if !self.loaded.contains(&sel) {
            effects.push(self.read(&sel));
        }
        effects.extend(self.ack());
        effects
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

    /// Applies one event of the stream.
    pub fn event(&mut self, e: &Event) -> Vec<Effect> {
        match e.kind.as_str() {
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
        }
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
                let seen = self.selected() == Some(e.actor.as_str());
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
    /// newer than the read and stays. When the open conversation's row is
    /// gone the first is opened.
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

    fn reselect(&mut self) -> Vec<Effect> {
        let names = self.names();
        if self.selected.as_ref().is_some_and(|s| names.contains(s)) {
            return Vec::new();
        }
        self.selected = names.first().cloned();
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
    use crate::api::{RosterEntry, Said};
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
        assert_eq!(m.selected(), Some("yardmaster"));
        assert_eq!(effects, [read("yardmaster", 0)]);
        assert!(m.rows()[1].selected);
        assert!(!m.rows()[0].selected);
    }

    #[test]
    fn without_a_yardmaster_it_opens_on_the_first_row_and_on_none_in_an_empty_yard() {
        let mut state = yard(&[]);
        state.crew.remove(1);
        let (m, _) = Model::new(state, None);
        assert_eq!(m.selected(), Some("assembly"));

        let (mut m, effects) = Model::new(
            State {
                me: "jo".into(),
                ..State::default()
            },
            None,
        );
        assert_eq!(m.selected(), None);
        assert_eq!(effects, []);
        assert_eq!(m.target(), None);
        assert_eq!(m.step(1), []);
        assert_eq!(m.entries(), []);
    }

    #[test]
    fn the_conversation_that_was_open_stays_open_when_the_yard_is_back() {
        let (m, _) = Model::new(yard(&[]), Some("clerk@north"));
        assert_eq!(m.selected(), Some("clerk@north"));
        // One that is gone meanwhile does not.
        let (m, _) = Model::new(yard(&[]), Some("gone"));
        assert_eq!(m.selected(), Some("yardmaster"));
    }

    #[test]
    fn selecting_opens_a_conversation_and_reads_it_once() {
        let (mut m, _) = Model::new(yard(&[]), None);
        assert_eq!(m.select("assembly"), [read("assembly", 0)]);
        assert_eq!(m.selected(), Some("assembly"));
        assert_eq!(
            m.conversation("assembly", vec![letter(3, "assembly", "jo", "hello")]),
            []
        );
        m.select("yardmaster");
        // It was read: nothing is asked again.
        assert_eq!(m.select("assembly"), []);
        // A name that is no row changes nothing.
        assert_eq!(m.select("nobody"), []);
        assert_eq!(m.selected(), Some("assembly"));
    }

    #[test]
    fn step_goes_to_the_next_row_and_around_the_ends() {
        let (mut m, _) = Model::new(yard(&[]), None);
        m.step(1);
        assert_eq!(m.selected(), Some("clerk@north"));
        m.step(1);
        m.step(1);
        assert_eq!(m.selected(), Some("assembly"));
        m.step(-1);
        assert_eq!(m.selected(), Some("mckean@north"));
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
            m.select("assembly"),
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
        assert_eq!(m.select("assembly"), [read("assembly", 0)]);
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
            m.select("assembly"),
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
        m.select("assembly");
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
        assert_eq!(m.select("clerk@north"), [read("clerk@north", 0)]);
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
        let (m, _) = Model::new(state.clone(), Some("clerk@north"));
        assert_eq!(
            m.about(),
            ["a crew member of the linked yard north: Keeps the books"]
        );
        let (m, _) = Model::new(state.clone(), Some("mckean@north"));
        assert_eq!(m.about(), ["a person of the linked yard north"]);
        let (m, _) = Model::new(state.clone(), Some("yardmaster"));
        assert_eq!(m.about(), Vec::<String>::new());

        state.peers[0].said.as_mut().unwrap().roster[0].mail = false;
        let (m, _) = Model::new(state.clone(), Some("clerk@north"));
        assert!(
            m.about()[1].starts_with("north takes no mail from this yard yet"),
            "{:?}",
            m.about()
        );
        state.peers[0].send.clear();
        let (m, _) = Model::new(state, Some("clerk@north"));
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
        assert_eq!(m.selected(), Some("assembly"));
        assert_eq!(effects, [read("assembly", 0), Effect::Ack { through: 6 }]);

        let (mut m, _) = Model::new(yard(&[]), Some("clerk@north"));
        assert_eq!(
            m.event(&event(21, "peer_said", "", json!({"peer": "north"}))),
            [Effect::Peers]
        );
        assert_eq!(m.linked(&[]), [read("assembly", 0)]);
        assert_eq!(names(&m), ["assembly", "yardmaster"]);
        // One that is still there stays open.
        assert_eq!(m.linked(&[north()]), []);
        assert_eq!(m.selected(), Some("assembly"));
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
        // Another conversation has none of them.
        m.select("assembly");
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
}
