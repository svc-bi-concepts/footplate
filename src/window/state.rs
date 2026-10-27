//! What the window keeps beside the model: where a yard stood when
//! it was left, what waits in the yards it follows, and what the
//! switch and the settings are asking or doing.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui::{Pixels, Task};

use super::count_words;
use crate::api::{self, Client};
use crate::fetch::Kept;
use crate::model::{Mode, Model, Outcome, Selected};
use crate::session;
use crate::settings;
use crate::setup;

/// A pairing's code as it was typed where the yard's address is known
/// already: what a pairing's QR code holds, `<address>#<code>`, may be
/// given for it, and is its code. Where the address is the edge's the QR
/// code holds `<address>#<code>#<yard>`, and the code is its middle.
pub(crate) fn code_of(typed: &str) -> String {
    let typed = typed.trim();
    match typed.split('#').collect::<Vec<_>>().as_slice() {
        [_, code, _] => code.trim().to_string(),
        _ => typed
            .rsplit_once('#')
            .map_or(typed, |(_, code)| code)
            .trim()
            .to_string(),
    }
}

/// Which of the account's yards a pairing is for, where what was typed
/// for its code holds the edge QR code's third segment,
/// `<address>#<code>#<yard>`. None where no third stands: a bare code,
/// or a QR code of a yard reached direct.
pub(crate) fn yard_of(typed: &str) -> Option<String> {
    match typed.trim().split('#').collect::<Vec<_>>().as_slice() {
        [_, _, yard] => match yard.trim() {
            "" => None,
            yard => Some(yard.to_string()),
        },
        _ => None,
    }
}

/// A mail of the open conversation to keep in place while older mail is
/// put in above it: its place among the children of what scrolls, and how
/// far its top stood off the top of the view.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Anchor {
    pub(crate) item: usize,
    pub(crate) off: Pixels,
}

/// A piece of a letter the pointer can stand on: its head, its body, or
/// the bar of reactions floating over it. The head and the body stand
/// touching, with no strip between them that is neither, so the letter
/// is one hover target; each still tells the window of the pointer on
/// its own: see [`Footplate::hover_react`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Piece {
    Head,
    Body,
    Bar,
}

/// Where a yard stood when it was left for another: each mode's open row
/// and its list's scroll, and the scroll of what the open row showed. Of
/// a conversation and the yard row that is how far above its newest the
/// view stood, since a yard opened again has the newest of a
/// conversation first and the older above it; of a card, how far below
/// its top, with the cards that stood expanded: folded again, a card is
/// shorter than where it stood.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Place {
    /// The open row of each mode, by mode: what each mode reopens on.
    pub(crate) rows: [Option<Selected>; 3],
    /// The list's scroll in each mode, by mode: how far below its top.
    pub(crate) lists: [Pixels; 3],
    pub(crate) off: Pixels,
    pub(crate) expanded: HashSet<(String, u64)>,
    pub(crate) texts: HashSet<(String, u64)>,
    pub(crate) decided_expanded: bool,
    pub(crate) quiet_expanded: bool,
    /// Which arrivals view stood: the departures board where true, the
    /// panel where false, as the head's toggle sets it.
    pub(crate) departures: bool,
    /// The history's chip and the folded rows standing open, as the
    /// folded history itself is kept.
    pub(crate) decided_chip: Option<Outcome>,
    pub(crate) decided_folds: HashSet<(String, u64)>,
}

impl Place {
    /// The row the mode was left on, if any.
    pub(crate) fn row(&self, mode: Mode) -> Option<Selected> {
        self.rows[mode.index()].clone()
    }

    /// How far below its top the mode's list stood.
    pub(crate) fn list(&self, mode: Mode) -> Pixels {
        self.lists[mode.index()]
    }
}

/// Why a yard that could not be opened is not reached, in a word or two:
/// its address or its token's file is refused, and nothing was asked.
pub(crate) const NOT_OPENED: &str = "cannot be opened";

/// Whether a yard that is away is asked again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Away {
    /// Every few seconds.
    Trying,
    /// Not until another is chosen: the one that was could not be opened.
    Stopped,
    /// Not until the device is paired again: the yard does not let it in.
    Out,
    /// Not until signed in again: the edge does not let the session in.
    SignedOut,
}

/// What is typed in the settings for a yard: one to add, a sign-in for
/// a hosted one, a code for one that is listed, or another name for one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Adding {
    /// A yard of this machine: its name and its socket.
    Near,
    /// A yard at an address: its name, its address and a pairing's code.
    Far,
    /// A pairing's code for the yard of this name, to pair its device
    /// again.
    Again(String),
    /// A hosted yard: its name, the edge's origin, which of the
    /// account's yards there, and the Kratos account's email and password
    /// to sign in with. Signing in writes the session file and moves to
    /// pairing the device.
    Hosted,
    /// The Kratos account's email and password for the hosted yard of
    /// this name, whose session is signed in again.
    SignInAgain(String),
    /// Another name for the yard of this name, which keeps its reach,
    /// its files and what was written for it.
    Rename(String),
}

/// What a pairing failure says, with its next step. Refusals already say
/// why the yard declined the code; transport and unreadable-answer errors
/// need a way to recover too.
pub(crate) fn pairing_failure(error: api::Error) -> String {
    match error {
        api::Error::Refused { code, message } if code == "wrong_code" => format!(
            "Pairing failed: {message} Check the code on the ready page before trying again; if the pairing ended, ask the operator to open another one."
        ),
        api::Error::Refused { message, .. } => format!("Pairing failed: {message}"),
        api::Error::Down(why) => format!(
            "Pairing failed because the yard does not answer ({why}). Check the origin and your connection, then try the code again. If it has been used, ask the operator to open another pairing."
        ),
        api::Error::Protocol(why) => format!(
            "Pairing failed because the yard's answer could not be read: {why}. Check the origin and ask the operator to open another pairing."
        ),
        api::Error::NotLetIn => {
            "Pairing failed: this device is no longer let in. Ask the operator to open another pairing for this device.".to_string()
        }
        api::Error::NotSignedIn => {
            "Pairing failed: this session is no longer signed in. Sign in again, then pair this device.".to_string()
        }
        // The pairing's own 401 is read before this (no pairing is open);
        // one a gateway answered says nothing of the code.
        api::Error::Gateway => {
            "Pairing failed: a gateway turned the request away (401). Try the code again; if the yard says it is used, ask the operator to open another pairing.".to_string()
        }
        api::Error::NoDevice => {
            "Pairing failed: no yard answers at this origin. Check the origin and ask its operator to open a pairing.".to_string()
        }
        // A pairing reads no stream; said as nothing answering, were it.
        api::Error::Silent => {
            "Pairing failed because the yard does not answer. Check the origin and your connection, then try the code again.".to_string()
        }
        api::Error::Slow { idempotent: false } => "The yard did not answer in time; pairing may or may not have happened. Check whether this device is paired before trying again.".to_string(),
        // A pairing is not idempotent, but keep the plain recovery if a
        // caller supplies the other timeout kind.
        api::Error::Slow { idempotent: true } => {
            "Pairing failed because the yard is slow to answer. Try again shortly.".to_string()
        }
    }
}

/// What the switch is doing: nothing else of it starts meanwhile. Asking
/// is down's question being read, which is too quick to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SwitchBusy {
    Starting,
    Stopping,
    Asking,
    Login,
}

/// What "Agents and you" is doing: nothing else of it starts meanwhile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AgentsBusy {
    Reading,
    Saving,
}

/// The About card, while it stands over the window: what it says of the
/// yardr footplate keeps. None until that yardr has been asked its version,
/// which is done when the card opens.
#[derive(Debug)]
pub(crate) struct About {
    pub(crate) kept: Option<Kept>,
}

/// What the status row at the list's head is for the open yard.
pub(crate) enum SwitchFor {
    /// A yard of this machine: its home, or none for this run's, which
    /// takes no `--home` flag.
    Near(Option<PathBuf>),
    /// A yard at an address: its processes are elsewhere, and there is no
    /// switch.
    Far,
    /// A yard of this machine whose socket names no yard home: the switch
    /// says so, and starts nothing.
    NoHome,
}

/// What switching "start at login" came to: what `up --login` did, or
/// what the daemon's uninstall removed.
pub(crate) enum LoginDid {
    Checked(Result<setup::Up, String>),
    Unchecked(Result<setup::Uninstalled, String>),
}

/// What a reading of the switch came to: the yardr to run from here on,
/// and what `status` said.
pub(crate) enum SwitchRead {
    /// yardr is ready: the program, the PATH widened by doctor, what
    /// status said, and what there is to know of the yardr where a newer
    /// one could not be had.
    Ready {
        program: PathBuf,
        path: std::ffi::OsString,
        status: Result<setup::Status, setup::StatusError>,
        stale: Option<String>,
    },
    /// yardr could not be had, in the fetch's words: nothing of the
    /// switch runs.
    NoYardr(String),
}

/// What a yard has waiting: the switcher shows it for a yard that is not
/// open, as it stands now, since every yard is followed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Counts {
    pub(crate) mail: usize,
    pub(crate) decisions: usize,
    pub(crate) notices: usize,
    pub(crate) changed: usize,
}

/// What makes a yard entry the same yard while a call is away from the
/// window. Its display name alone may be reused after the entry is removed.
#[derive(Clone, Debug, Eq)]
pub(crate) enum YardIdentity {
    Listed(settings::Yard),
    Environment(String, Option<PathBuf>),
}

impl PartialEq for YardIdentity {
    /// The same yard is the same reach: a change of the web view's
    /// address or the run base alone restores no draft less.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (YardIdentity::Listed(a), YardIdentity::Listed(b)) => a.same_yard(b),
            (YardIdentity::Environment(a, at), YardIdentity::Environment(b, by)) => {
                a == b && at == by
            }
            _ => false,
        }
    }
}

/// What the open yard's rail may count: its model while it answers, and
/// nothing while it is away, even where its last rows stay on screen.
pub(crate) fn rail_counts(model: Option<&Model>, away: bool) -> Option<Counts> {
    if away { None } else { model.map(Counts::of) }
}

impl Counts {
    pub(crate) fn of(model: &Model) -> Counts {
        Counts {
            mail: model.rows().iter().map(|row| row.unread).sum(),
            decisions: model.decisions_count(),
            notices: model.yard_unread(),
            changed: model.timetable_news(),
        }
    }

    /// How many things of the yard wait for the person: the mail that is
    /// not read and the cards to decide. A notice of the yard asks nothing
    /// of them, and neither does a bead that changed.
    pub(crate) fn waiting(&self) -> usize {
        self.mail + self.decisions
    }

    /// In words.
    pub(crate) fn line(&self) -> String {
        let parts: Vec<String> = [
            count_words(self.mail, "mail", "mail"),
            count_words(self.decisions, "decision", "decisions"),
            count_words(self.notices, "notice", "notices"),
        ]
        .into_iter()
        .flatten()
        .collect();
        match parts.is_empty() {
            true => "nothing waiting".to_string(),
            false => parts.join(", "),
        }
    }

    /// What the switcher's row carries of these counts: the waiting
    /// number with the tooltip's words. The tooltip numbers each kind,
    /// decisions first; a notice asks nothing, so it does not add to the
    /// waiting number.
    pub(crate) fn switcher(&self) -> (String, String) {
        let words = [
            count_words(self.decisions, "decision", "decisions"),
            count_words(self.mail, "mail", "mail"),
            count_words(self.notices, "notice", "notices"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let words = match words.is_empty() {
            true => "nothing waiting".to_string(),
            false => words,
        };
        (self.waiting().to_string(), words)
    }

    /// What the rail's tile of `mode` carries of these counts: the
    /// badge's label with the tooltip's words, or none where the tile
    /// carries nothing. Chat carries the unread mail, decisions the open
    /// asks, the timetable the person's beads added or closed since they
    /// last had it open; nothing is carried where nothing waits, and the
    /// notices are no glyph's. Above nine the badge says 9+, and the
    /// tooltip still says the number.
    pub(crate) fn rail(&self, mode: Mode) -> Option<(String, String)> {
        let (n, one, more) = match mode {
            Mode::Chat => (self.mail, "mail", "mail"),
            Mode::Decisions => (self.decisions, "decision", "decisions"),
            Mode::Timetable => (self.changed, "bead changed", "beads changed"),
        };
        let label = match n > 9 {
            true => "9+".to_string(),
            false => n.to_string(),
        };
        count_words(n, one, more).map(|words| (label, words))
    }
}

/// How many things wait for the person over every yard that is followed,
/// which the dock's icon says (dock.rs): each yard's as the switcher counts
/// them, summed. A yard has none to count until it was reached once; one
/// that was and is not reached now counts what it last had, which waits
/// there still.
pub(crate) fn waiting(yards: impl IntoIterator<Item = Option<Counts>>) -> usize {
    yards.into_iter().flatten().map(|c| c.waiting()).sum()
}

/// A yard as the window follows it, and all that is of it and not of the
/// window: the open yard's is in the window's own fields of these names,
/// and that of each other yard of the settings is held here, going on with
/// its yard, until the window shows it.
pub(crate) struct Held {
    /// Which following this is, counted from the window's start: what the
    /// follower's words and the answers to its calls are of, wherever the
    /// yard is by the time they come.
    pub(crate) session: u64,
    /// The yard of the settings it was opened as. One listed otherwise by
    /// now is another yard, and is opened anew.
    pub(crate) entry: Option<settings::Yard>,
    pub(crate) client: Option<Client>,
    pub(crate) following: Option<Task<()>>,
    pub(crate) following_stop: Option<session::FollowControl>,
    /// No row of it is open while it is held ([`Model::leave`]).
    pub(crate) model: Option<Model>,
    pub(crate) down: Option<String>,
    pub(crate) away: Away,
    pub(crate) said: Option<api::Me>,
    pub(crate) approving: Option<(String, u64)>,
    pub(crate) approve_elsewhere: HashSet<(String, u64)>,
    pub(crate) refused: HashMap<(String, u64), String>,
    pub(crate) ask_generation: u64,
    pub(crate) returns_read: Option<chrono::DateTime<chrono::Utc>>,
}

impl Drop for Held {
    /// A yard that is held no longer is followed no longer.
    fn drop(&mut self) {
        if let Some(stop) = self.following_stop.take() {
            stop.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a pairing's QR code holds is its code, and where the address
    /// is the edge's, which of the account's yards beside it.
    #[test]
    fn what_a_pairing_s_qr_code_holds_is_a_code_and_maybe_a_yard() {
        assert_eq!(code_of(" K7QM-2XPD "), "K7QM-2XPD");
        assert_eq!(code_of("https://mac.example.net#K7QM-2XPD"), "K7QM-2XPD");
        assert_eq!(code_of("https://my.yardr.dev#K7QM-2XPD#airy"), "K7QM-2XPD");
        assert_eq!(yard_of("K7QM-2XPD"), None);
        assert_eq!(yard_of("https://mac.example.net#K7QM-2XPD"), None);
        assert_eq!(
            yard_of(" https://my.yardr.dev#K7QM-2XPD#airy "),
            Some("airy".to_string())
        );
        assert_eq!(yard_of("https://my.yardr.dev#K7QM-2XPD#"), None);
    }
}
