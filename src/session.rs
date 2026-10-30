//! Following a yard: who the window is for, the state it starts from, and
//! the stream from then on, on a thread of its own. When the yard is not
//! there, or goes away, it says so and tries again, waiting longer between
//! tries.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
#[cfg(test)]
use std::thread;
use std::time::Duration;

use crate::api::{self, Arrivals, Client, Event, Flows};
use crate::model::{self, State};

/// How long until the yard is tried again after a failed read: the first
/// wait, doubled for each failure in a row up to [`BACKOFF_CAP`].
pub const RETRY: Duration = Duration::from_secs(3);

/// The longest a failed state read waits before the yard is tried again.
/// On a slow yard every call is work, so retrying at the fixed retry adds
/// load exactly when the yard is slowest; waiting longer between tries
/// asks a yard that stays away about once a minute, not twenty times.
pub const BACKOFF_CAP: Duration = Duration::from_secs(60);

/// How long until the yard is tried again after `failed` state reads in a
/// row failed: the retry, doubled for each failure past the first, up to
/// [`BACKOFF_CAP`]. A read that answers resets the count, so a yard that
/// flaps is tried at the retry again.
fn backoff(retry: Duration, failed: u32) -> Duration {
    retry
        .saturating_mul(1 << failed.saturating_sub(1).min(30))
        .min(BACKOFF_CAP)
}

/// A stream that ends is opened again at once when the server only replaces
/// itself with a newer build; this is the moment between the two.
const REPLACING_PAUSE: Duration = Duration::from_millis(100);

/// How a follower watches a stream that says nothing. The yard says
/// nothing on a quiet stream, and nor does a stream that a proxy or a
/// tunnel holds open after the yard behind it went away, while the yard
/// moves on. After `quiet` with nothing the yard is asked its head, on a
/// connection of its own. A head beyond the last event read gives that
/// event `quiet` more to come, since it may be on its way: a stream that
/// is late but not broken is waited for, not replaced at once, since on
/// a slow yard replacing it only adds calls. After that the stream is
/// stuck, and is replaced as when a stream ends.
#[derive(Debug, Clone, Copy)]
pub struct Watch {
    pub quiet: Duration,
}

/// A stuck stream is replaced within about two minutes and a few seconds.
pub const WATCH: Watch = Watch {
    quiet: Duration::from_secs(60),
};

/// What the thread tells the window.
#[derive(Debug)]
pub enum Update {
    /// The yard is there, and this is what it has.
    Up(Box<State>),
    /// Something happened in it.
    Event(Event),
    /// It is not there, or not for this person: why, in words, and in a
    /// word or two for where there is no room for them ([`Trouble::short`]).
    Down(String, &'static str),
    /// It is there and does not let this device in: why, in words. The
    /// yard is not asked again; the thread has ended. In a word or two it
    /// is [`PAIR_AGAIN`].
    Out(String),
    /// It is there and does not let this session in: why, in words. The
    /// yard is not asked again; the thread has ended. In a word or two it
    /// is [`SIGN_IN_AGAIN`].
    SignedOut(String),
    /// The edge no longer lets the account into this yard of another
    /// account's: its owner removed the membership. Never asked again; the
    /// window forgets the yard.
    Removed,
}

#[derive(Default)]
struct Following {
    stopped: bool,
    stream: Option<api::EventStop>,
}

#[derive(Default)]
struct FollowState {
    following: Mutex<Following>,
    changed: Condvar,
}

/// Stops a yard follower, waking it from a retry wait or an event stream
/// blocked waiting for the next event. The window keeps one for each yard
/// it follows, which is every yard of its settings.
#[derive(Clone, Default)]
pub struct FollowControl(Arc<FollowState>);

impl FollowControl {
    pub fn stop(&self) {
        let stream = {
            let mut following = self.0.following.lock().unwrap();
            following.stopped = true;
            following.stream.take()
        };
        self.0.changed.notify_all();
        if let Some(stream) = stream {
            stream.stop();
        }
    }

    fn stopped(&self) -> bool {
        self.0.following.lock().unwrap().stopped
    }

    /// Waits for `duration`, or until the window no longer follows this yard.
    fn wait(&self, duration: Duration) -> bool {
        let following = self.0.following.lock().unwrap();
        if following.stopped {
            return true;
        }
        self.0
            .changed
            .wait_timeout_while(following, duration, |following| !following.stopped)
            .unwrap()
            .0
            .stopped
    }

    fn follow_stream(&self, stream: api::EventStop) {
        let stop = {
            let mut following = self.0.following.lock().unwrap();
            if following.stopped {
                Some(stream)
            } else {
                following.stream = Some(stream);
                None
            }
        };
        if let Some(stream) = stop {
            stream.stop();
        }
    }

    fn stream_ended(&self) {
        self.0.following.lock().unwrap().stream = None;
    }
}

/// Why a yard is not reached, in a word or two: it is stopped, or away
/// from the network.
pub const NOTHING_ANSWERS: &str = "nothing answers";

/// The same of a yard that no longer lets this device in.
pub const PAIR_AGAIN: &str = "pair again";

/// The same of a hosted yard that no longer lets this session in.
pub const SIGN_IN_AGAIN: &str = "sign in again";

/// Why there is nothing to show.
#[derive(Debug)]
pub enum Trouble {
    Yard(api::Error),
    /// The name the window runs as is no user of the yard, and the yard
    /// names no operator that is one.
    NoUser(String),
    /// The yard at an address refused the device with its own 401: the
    /// device was removed, or its token is not that yard's. Trying again
    /// changes nothing. A 401 that is not the yard's check is no such
    /// thing, and is [`api::Error::Gateway`] in `Yard`.
    NotLetIn,
    /// The edge answered 401 carrying `X-Edge-Refusal: session`: the
    /// session was revoked, or is not that edge's. Trying again changes
    /// nothing; the device pairing is untouched.
    NotSignedIn,
    /// Nothing answers at the yard's address.
    Unreached(std::io::Error),
}

impl From<api::Error> for Trouble {
    fn from(err: api::Error) -> Self {
        match err {
            api::Error::NotLetIn => Trouble::NotLetIn,
            api::Error::NotSignedIn => Trouble::NotSignedIn,
            err => Trouble::Yard(err),
        }
    }
}

impl Trouble {
    /// The trouble as it is with the yard behind `client`: one at an
    /// address has no socket, and no yardr command of this machine starts
    /// it.
    fn with(self, client: &Client) -> Trouble {
        match self {
            Trouble::Yard(api::Error::Down(err)) if client.is_remote() => Trouble::Unreached(err),
            trouble => trouble,
        }
    }

    /// The trouble in a word or two, for a row that names the yard and has
    /// no room for the telling: what the person does about it where that
    /// is one thing, else what is wrong.
    pub fn short(&self) -> &'static str {
        match self {
            Trouble::Yard(api::Error::Down(_)) | Trouble::Unreached(_) => NOTHING_ANSWERS,
            Trouble::NotLetIn | Trouble::Yard(api::Error::NoDevice) => PAIR_AGAIN,
            Trouble::NotSignedIn => SIGN_IN_AGAIN,
            Trouble::NoUser(_) => "no user here",
            Trouble::Yard(api::Error::Slow { .. }) => "slow to answer",
            Trouble::Yard(api::Error::Refused { code, .. })
                if code == "away" || code == "no_answer" =>
            {
                "slow to answer"
            }
            Trouble::Yard(api::Error::Refused { code, .. }) if code == "busy" => "busy",
            Trouble::Yard(api::Error::Gateway) => "gateway refused",
            Trouble::Yard(_) => "cannot be read",
        }
    }

    /// What the follower tells the window for this trouble: on a yard of
    /// another account's, a refused device is told to pair this device
    /// again from the yard's card, in the member's own session — never to
    /// ask the operator for a code, which a member can never be given (the
    /// operator never holds a member's code). Every other trouble, on any
    /// yard, is told as it is said.
    fn told(&self, is_member: bool) -> String {
        match (self, is_member) {
            (Trouble::NotLetIn, true) => "This device is no longer let in: the yard refuses its token, or your access was removed. Pair this device again from the yard's card, in your own session.".into(),
            (Trouble::Yard(api::Error::NoDevice), true) => "Nothing of a yard is under /user/ at this address: the yard has no device any more, or the address is not a yard's listener. Pair this device again from the yard's card, in your own session.".into(),
            _ => self.to_string(),
        }
    }
}

impl fmt::Display for Trouble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trouble::Yard(api::Error::Down(err)) => {
                write!(
                    f,
                    "No yard: nothing answers at the yard's socket ({err}). Any yardr command that reads the yard starts its server: yardr status"
                )
            }
            Trouble::Yard(api::Error::NoDevice) => f.write_str(
                "Nothing of a yard is under /user/ at this address: the yard has no device any more, or the address is not a yard's listener. Pair this device again, with a code the yard's operator opens: yardr user device pair <user> <device>",
            ),
            Trouble::Yard(api::Error::Slow { .. }) => f.write_str(
                "The yard is slow to answer: it did not answer in time. The window tries again shortly.",
            ),
            // Not the yard's check of the device, which is `NotLetIn`: the
            // device stays paired, and what turned the request away may
            // not turn the next one.
            Trouble::Yard(api::Error::Gateway) => f.write_str(
                "A gateway turned the window's request away (401), which does not mean this device is out. The window tries again shortly.",
            ),
            // The edge's link, not its yard: away or slow is a slow yard,
            // busy is busy. Each is tried again, and said in the edge's
            // words, never as what footplate cannot read.
            Trouble::Yard(api::Error::Refused { code, .. }) if code == "away" => f.write_str(
                "The yard is away. The window tries again shortly.",
            ),
            Trouble::Yard(api::Error::Refused { code, .. }) if code == "no_answer" => f.write_str(
                "The yard gave no answer in time. The window tries again shortly.",
            ),
            Trouble::Yard(api::Error::Refused { code, .. }) if code == "busy" => {
                f.write_str("The yard is busy. The window tries again shortly.")
            }
            Trouble::Yard(err) => {
                write!(f, "The yard is there, but footplate cannot read it: {err}")
            }
            Trouble::NoUser(name) => {
                write!(
                    f,
                    "{name} is no user of this yard, and the yard names no operator. The operator adds one: yardr user add {name}"
                )
            }
            Trouble::NotLetIn => f.write_str(
                "This device is no longer let in: the yard refuses its token. Its operator removed the device, or the token is not this yard's. Pair it again, with a code the operator opens: yardr user device pair <user> <device>",
            ),
            Trouble::NotSignedIn => f.write_str(
                "This session is no longer signed in: the edge refuses its token. It was revoked, or is not this edge's. Sign in to this hosted yard again.",
            ),
            Trouble::Unreached(err) => {
                write!(
                    f,
                    "No yard: nothing answers at the yard's address ({err}). Its listener, and what stands in front of it, are the yard's operator's to start."
                )
            }
        }
    }
}

/// The name the window runs as before the yard is asked, taken as
/// `yardr chat` takes it: `$YARDR_ACTOR`, else the login name.
pub fn login_name() -> String {
    ["YARDR_ACTOR", "USER", "LOGNAME"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|name| !name.is_empty())
        .unwrap_or_default()
}

/// Who the window is for: `name` when it is a user of the yard, else the
/// yard's operator. Nobody types it.
pub fn who(client: &Client, name: &str) -> Result<api::User, Trouble> {
    if !name.is_empty() {
        match client.get_user(name) {
            Ok(user) => return Ok(user),
            Err(err) if err.not_found() => {}
            Err(err) => return Err(err.into()),
        }
    }
    let operator = client.settings()?.operator;
    if operator.is_empty() || operator == name {
        return Err(Trouble::NoUser(name.to_string()));
    }
    match client.get_user(&operator) {
        Ok(user) => Ok(user),
        Err(err) if err.not_found() => Err(Trouble::NoUser(name.to_string())),
        Err(err) => Err(err.into()),
    }
}

/// Reads what a window starts from, in the order `yardr chat` reads it:
/// the head first, so that what happens meanwhile comes again from the
/// stream.
pub fn load(client: &Client, name: &str) -> Result<State, Trouble> {
    let head = client.head()?;
    let crew = client.crew()?;
    let users = client.users()?;
    let peers = client.peers()?;
    // At an address the token says who the window is for, and no name of
    // this machine is asked after.
    let yard = match client.is_remote() {
        true => Some(client.me()?),
        false => None,
    };
    let user = match &yard {
        Some(me) => client.get_user(&me.user)?,
        None => who(client, name)?,
    };
    // The count first, then that many: a user's inbox lists the unread
    // before the read, and the read are not wanted here.
    let mut inbox = client.user_inbox(&user.name, 1)?;
    if inbox.unread > 1 {
        inbox = client.user_inbox(&user.name, inbox.unread)?;
    }
    let unread = inbox.mail.into_iter().filter(|l| !l.read).collect();
    let notifications = client.user_notifications(&user.name, 0)?.notifications;
    let asks = client.user_asks(&user.name)?;
    // One call's failure must not hold the whole yard away: an
    // unreadable board reads as empty, with why for the window to say,
    // and an unreadable flows view with it. A yard from before the
    // calls has no Lines to serve: that reads as empty too, with the
    // Lines pane saying so instead of a notice.
    let (arrivals, arrivals_notice, arrivals_too_old) = match client.arrivals() {
        Ok(board) => (board, None, false),
        Err(err) if model::lines_too_old(&err) => (Arrivals::default(), None, true),
        Err(err) => (
            Arrivals::default(),
            Some(format!("the arrivals could not be read: {err}")),
            false,
        ),
    };
    let (flows, flows_notice, flows_too_old) = match client.flows() {
        Ok(board) => (board, None, false),
        Err(err) if model::lines_too_old(&err) => (Flows::default(), None, true),
        Err(err) => (
            Flows::default(),
            Some(format!("the flows could not be read: {err}")),
            false,
        ),
    };
    // Who the grants view is offered to: the yard's operator, and no one
    // else. Its settings unreadable, the window still opens, with no
    // grants row; a yard from before the call has no grants to serve. The
    // yard serves the list to no device that comes down the link from its
    // edge, so a hosted yard is not asked for it, and one that refuses it
    // so after all is no error either: the grants are read on its machine.
    let operator = client.settings().map(|s| s.operator).unwrap_or_default();
    let is_operator = !operator.is_empty() && operator == user.name;
    // Who the Telemetry section is offered to: the yard's operator, and
    // no one else, on a yard that serves the calls to such a window. The
    // yard serves neither call to a device that comes down the link from
    // its edge, so a hosted yard is not asked for the preview; a yard
    // from before the calls, and one that refuses the preview so after
    // all, offer no section either.
    let (telemetry, telemetry_notice, telemetry_unserved) = match is_operator && !client.hosted() {
        false => (None, None, client.hosted()),
        true => match client.telemetry_show() {
            Ok(preview) => (Some(preview), None, false),
            Err(err) if model::telemetry_too_old(&err) => (None, None, true),
            Err(err) if model::telemetry_over_the_link(&err) => (None, None, true),
            Err(err) => (
                None,
                Some(format!("the telemetry switch could not be read: {err}")),
                false,
            ),
        },
    };
    let mut grants_on_the_yard = client.hosted();
    let (grants, grants_notice, grants_too_old) = match is_operator && !grants_on_the_yard {
        false => (Vec::new(), None, false),
        true => match client.source_grants() {
            Ok(grants) => (grants, None, false),
            Err(err) if model::grants_too_old(&err) => (Vec::new(), None, true),
            Err(err) if model::grants_over_the_link(&err) => {
                grants_on_the_yard = true;
                (Vec::new(), None, false)
            }
            Err(err) => (
                Vec::new(),
                Some(format!("the grants could not be read: {err}")),
                false,
            ),
        },
    };
    // Who the teammates view is offered to: the yard's operator, and no
    // one else. Unlike the grant list, the teammates come down the link
    // from the edge to a paired device of the operator's, as an approval
    // does, so a hosted yard is asked for them. A yard from before the
    // call has no teammates to serve.
    let (teammates, teammates_notice, teammates_too_old) = match is_operator {
        false => (Vec::new(), None, false),
        true => match client.teammates() {
            Ok(teammates) => (teammates, None, false),
            Err(err) if model::teammates_too_old(&err) => (Vec::new(), None, true),
            Err(err) => (
                Vec::new(),
                Some(format!("the teammates could not be read: {err}")),
                false,
            ),
        },
    };
    // Who the Pair a phone pane is offered to: the yard's operator, on
    // the yard's own machine. The yard serves the pairing calls on its
    // socket alone, never to a device, so a yard at an address is not
    // asked for its pairing: no pairing call is made for it. A yard from
    // before the calls has no pairing to serve.
    let near = !client.is_remote();
    let (pairing, pairing_notice, pairing_too_old) = match is_operator && near {
        false => (None, None, false),
        true => match client.device_pair_state() {
            Ok(pairing) => (pairing, None, false),
            Err(err) if model::pairing_too_old(&err) => (None, None, true),
            Err(err) => (
                None,
                Some(format!("the pairing could not be read: {err}")),
                false,
            ),
        },
    };
    Ok(State {
        me: user.name.clone(),
        head,
        crew,
        users,
        peers,
        cursor: user.cursor,
        unread,
        notification_cursor: user.notification_cursor,
        notifications,
        asks,
        arrivals,
        arrivals_notice,
        arrivals_too_old,
        flows,
        flows_notice,
        flows_too_old,
        operator,
        grants,
        grants_notice,
        grants_too_old,
        teammates,
        teammates_notice,
        teammates_too_old,
        near,
        pairing,
        pairing_notice,
        pairing_too_old,
        grants_on_the_yard,
        // The yard reads a session's screen to no device that comes down
        // the link from its edge either. Nothing is asked for it here: the
        // way the client reaches the yard is all there is to go by, and a
        // refusal later teaches the window the rest.
        screen_on_the_yard: client.hosted(),
        telemetry,
        telemetry_notice,
        telemetry_unserved,
        yard,
    })
}

/// What came of following one stream.
struct Streamed {
    /// The last seq read.
    read: u64,
    /// Whether the stream brought anything, or was found stuck while the
    /// yard answered beside it.
    heard: bool,
    /// Whether the stream opened again after `read` takes up where this one
    /// stopped. An event that cannot be read, or a stream that cannot be
    /// opened, would meet the same again, and the state read again is past
    /// both.
    resumes: bool,
}

/// Follows the stream after the seq `after` until it ends, telling each
/// event; `None` when the window is gone.
fn stream(
    client: &Client,
    after: u64,
    watch: Watch,
    control: &FollowControl,
    tell: &mut impl FnMut(Update) -> bool,
) -> Option<Streamed> {
    let Ok(mut events) = client.events(after, watch.quiet) else {
        return Some(Streamed {
            read: after,
            heard: false,
            resumes: false,
        });
    };
    control.follow_stream(events.stop_handle());
    // The last seq read, and whether the yard's head was beyond it when the
    // stream last said nothing.
    let (mut read, mut behind) = (after, false);
    let (mut heard, mut resumes) = (false, true);
    // An event that cannot be read ends the stream: the yard would send it
    // again after `read`, so it is the state read again that gets past it.
    // A stuck stream, and one whose yard does not say its head, end as any
    // other does.
    while !control.stopped() {
        match events.next() {
            Some(Ok(event)) => {
                heard = true;
                read = read.max(event.seq);
                // The event its grace was waited for has come: a head beyond
                // it again is a new lateness, given a moment of its own.
                behind = false;
                if !tell(Update::Event(event)) {
                    control.stream_ended();
                    return None;
                }
            }
            Some(Err(api::Error::Silent)) => match client.head() {
                Ok(at) if at <= read => {
                    if behind && events.set_quiet(watch.quiet).is_err() {
                        break;
                    }
                    behind = false;
                }
                Ok(_) if !behind => {
                    if events.set_quiet(watch.quiet).is_err() {
                        break;
                    }
                    behind = true;
                }
                Ok(_) => {
                    heard = true;
                    break;
                }
                Err(_) => break,
            },
            Some(Err(api::Error::Protocol(_))) => {
                resumes = false;
                break;
            }
            Some(Err(_)) | None => break,
        }
    }
    control.stream_ended();
    Some(Streamed {
        read,
        heard,
        resumes,
    })
}

/// Follows the yard until `tell` answers false (the window is gone): the
/// state, then every event after it. When the stream ends, or is found
/// stuck, it is opened again after the last seq read, as the yard's API
/// says, so nothing in between is lost and nothing is read again; the state
/// is read again only when that cannot be done: the stream cannot be opened,
/// or brings an event that cannot be read, or the yard does not say its
/// head, or its head is behind what was read (a yard rewound). A state read
/// again that is the one last told, with nothing heard since, is not told
/// again, so that a quiet yard whose stream cannot be kept changes nothing
/// in the window. When the state cannot be read the window is told why, and
/// the yard is tried again, waiting longer between tries: `retry`, doubled
/// for each failure in a row, up to a minute. A yard that does not let the
/// device in, or the edge that does not let the session in, is not tried
/// again: the window is told, and this returns. Where `is_member` says the
/// yard is another account's, the edge answering it holds no such yard is
/// the owner having removed the membership: told once as removed, never
/// tried again, and any other refusal is tried as any yard's is.
/// The handle stops the current stream from another thread, so switching
/// yards does not leave an idle old yard's connection alive indefinitely.
/// A stream that says nothing while the yard moves on is watched as
/// `watch` says, and ends as one that ended.
pub fn follow_with_control(
    client: &Client,
    name: &str,
    retry: Duration,
    watch: Watch,
    control: FollowControl,
    is_member: bool,
    mut tell: impl FnMut(Update) -> bool,
) {
    // The state last told as the window's, until the yard is away from it.
    let mut told: Option<State> = None;
    // How many state reads in a row failed: the wait before the next try
    // doubles with it, so a yard that stays away is asked less and less.
    let mut failed: u32 = 0;
    loop {
        if control.stopped() {
            return;
        }
        let trouble = match load(client, name) {
            Ok(state) => {
                failed = 0;
                let mut read = state.head;
                if told.as_ref() != Some(&state) {
                    told = Some(state.clone());
                    if !tell(Update::Up(Box::new(state))) {
                        return;
                    }
                }
                loop {
                    if control.stopped() {
                        return;
                    }
                    let Some(streamed) = stream(client, read, watch, &control, &mut tell) else {
                        return;
                    };
                    // The events told moved the window on from the state it
                    // was told: a state read again is told, whatever it is.
                    if streamed.read > read {
                        told = None;
                    }
                    read = streamed.read;
                    if control.stopped() {
                        return;
                    }
                    // A stream that ended: a server that replaces itself is
                    // back in a moment. One that gave nothing before it
                    // ended is not asked again in a loop without a pause.
                    let pause = if streamed.heard {
                        REPLACING_PAUSE
                    } else {
                        retry
                    };
                    if control.wait(pause) {
                        return;
                    }
                    // A yard that is at or past what was read has all that
                    // came after it to send again.
                    if !streamed.resumes || !client.head().is_ok_and(|at| at >= read) {
                        break;
                    }
                }
                continue;
            }
            Err(Trouble::NotLetIn) => {
                tell(Update::Out(Trouble::NotLetIn.told(is_member)));
                return;
            }
            Err(Trouble::NotSignedIn) => {
                tell(Update::SignedOut(Trouble::NotSignedIn.to_string()));
                return;
            }
            Err(Trouble::Yard(api::Error::Refused { code, message }))
                if is_member && code == "no_yard" && message == api::HOLDS_NO_YARD =>
            {
                tell(Update::Removed);
                return;
            }
            Err(trouble) => trouble.with(client),
        };
        // Down is something the window shows: the state read when the yard
        // answers again is told, whatever it is.
        told = None;
        if !tell(Update::Down(trouble.told(is_member), trouble.short())) {
            return;
        }
        if control.stopped() {
            return;
        }
        failed = failed.saturating_add(1);
        if control.wait(backoff(retry, failed)) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Yard;
    use serde_json::json;
    use std::sync::mpsc;

    fn a_yard_of_jo(yard: &Yard) {
        yard.answer("Head", r#"{"result": 20}"#);
        yard.answer("Crew", r#"{"result": [{"name": "yardmaster", "enabled": true, "state": "running", "status": "idle"}]}"#);
        yard.answer("Peers", r#"{"result": []}"#);
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
        );
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "jo", "cursor": 5, "notification_cursor": 0}}"#,
        );
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [], "unread": 0, "last": 0, "from": []}}"#,
        );
        yard.answer(
            "UserNotifications",
            r#"{"result": {"notifications": [], "unread": 0, "last": 0}}"#,
        );
        yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        yard.answer("Arrivals", r#"{"result": []}"#);
        yard.answer(
            "Flows",
            r#"{"result": {"travelling": 0, "depots": [], "unassigned": []}}"#,
        );
        yard.answer("SourceGrants", r#"{"result": []}"#);
        yard.answer("Teammates", r#"{"result": []}"#);
        yard.answer(
            "DevicePairState",
            r#"{"error": {"code": "not-found", "message": "an open device pairing: not found"}}"#,
        );
    }

    const NOT_FOUND: &str = r#"{"error": {"code": "not-found", "message": "user: not found"}}"#;

    #[test]
    fn the_name_is_the_one_the_window_runs_as_when_the_yard_has_that_user() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        assert_eq!(who(&yard.client(), "jo").unwrap().name, "jo");
        assert_eq!(yard.asked("GetUser"), vec![json!({"name": "jo"})]);
        assert_eq!(yard.asked("Settings"), Vec::<serde_json::Value>::new());
    }

    #[test]
    fn else_it_is_the_yard_s_operator() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer_once_status("GetUser", 200, NOT_FOUND);
        let user = who(&yard.client(), "mac-login").unwrap();
        assert_eq!((user.name.as_str(), user.cursor), ("jo", 5));
        assert_eq!(
            yard.asked("GetUser"),
            vec![json!({"name": "mac-login"}), json!({"name": "jo"})]
        );
    }

    #[test]
    fn a_yard_without_an_operator_has_nobody_to_show_it_to() {
        let yard = Yard::start();
        yard.answer("GetUser", NOT_FOUND);
        yard.answer("Settings", r#"{"result": {}}"#);
        let trouble = who(&yard.client(), "mac-login").unwrap_err();
        assert!(
            matches!(&trouble, Trouble::NoUser(name) if name == "mac-login"),
            "{trouble:?}"
        );
        assert!(
            trouble.to_string().ends_with("yardr user add mac-login"),
            "{trouble}"
        );
        // An operator that is no user either.
        yard.answer("Settings", r#"{"result": {"operator": "gone"}}"#);
        assert!(matches!(
            who(&yard.client(), "mac-login"),
            Err(Trouble::NoUser(_))
        ));
    }

    #[test]
    fn load_reads_the_unread_mail_all_of_it() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let mail: Vec<_> = (6..=8).map(|seq| json!({"seq": seq, "from": "yardmaster", "to": "jo", "text": "x", "read": false})).collect();
        yard.answer(
            "UserInbox",
            &json!({"result": {"mail": mail, "unread": 3, "last": 8, "from": ["yardmaster"]}})
                .to_string(),
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!((state.me.as_str(), state.head, state.cursor), ("jo", 20, 5));
        assert_eq!(state.crew[0].name, "yardmaster");
        assert_eq!(state.users[0].name, "jo");
        assert_eq!(state.unread.len(), 3);
        // One to learn how many are unread, then that many.
        assert_eq!(
            yard.asked("UserInbox"),
            vec![
                json!({"name": "jo", "limit": 1}),
                json!({"name": "jo", "limit": 3})
            ]
        );
    }

    #[test]
    fn load_reads_the_notifications_and_the_asks() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "jo", "cursor": 5, "notification_cursor": 9}}"#,
        );
        yard.answer(
            "UserNotifications",
            r#"{"result": {"notifications": [
                {"seq": 10, "kind": "created", "bead": "yd-shed", "text": "yd-shed created", "read": false}
            ], "unread": 1, "last": 10}}"#,
        );
        yard.answer(
            "UserAsks",
            r#"{"result": {"open": [
                {"bead": "yd-links", "bead_title": "Move the links", "topic": "topic:linear-out",
                 "ask": 3, "question": "Move them now?", "asker": "yardmaster",
                 "since": "2026-10-08T09:30:00Z"}
            ], "decided": []}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.notification_cursor, 9);
        assert_eq!(state.notifications.len(), 1);
        assert_eq!(state.notifications[0].bead, "yd-shed");
        assert_eq!(state.asks.open.len(), 1);
        assert_eq!(state.asks.open[0].question, "Move them now?");
        assert_eq!(
            yard.asked("UserNotifications"),
            vec![json!({"name": "jo", "limit": 0})]
        );
        assert_eq!(yard.asked("UserAsks"), vec![json!({"name": "jo"})]);
    }

    /// An arrivals board the yard serves unreadably does not fail the
    /// state read: the board is empty, with why for the window to say.
    #[test]
    fn load_reads_on_past_an_unreadable_arrivals_board() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("Arrivals", r#"{"result": [7]}"#);
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.arrivals.rows.is_empty());
        assert!(
            state
                .arrivals_notice
                .as_deref()
                .unwrap()
                .starts_with("the arrivals could not be read: "),
            "{:?}",
            state.arrivals_notice
        );
        // A board that reads carries no notice.
        yard.answer("Arrivals", r#"{"result": []}"#);
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.arrivals_notice, None);
    }

    /// A flows view the yard serves unreadably does not fail the state
    /// read either: the view is empty, with why for the window to say.
    #[test]
    fn load_reads_on_past_an_unreadable_flows_view() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("Flows", r#"{"result": {"travelling": [7]}}"#);
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.flows.travelling, 0);
        assert!(state.flows.depots.is_empty());
        assert!(state.flows.unassigned.is_empty());
        assert!(
            state
                .flows_notice
                .as_deref()
                .unwrap()
                .starts_with("the flows could not be read: "),
            "{:?}",
            state.flows_notice
        );
        // A view that reads carries no notice.
        yard.answer(
            "Flows",
            r#"{"result": {"travelling": 0, "depots": [], "unassigned": []}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.flows_notice, None);
    }

    /// A yard from before the Lines view has no Arrivals or Flows call:
    /// both read as empty, with neither raising a notice, and the Lines
    /// pane says the yard is too old instead. Either half alone reads
    /// the same, and a half that reads is not too old.
    #[test]
    fn load_reads_on_past_a_yard_too_old_for_lines() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let old = |method: &str| {
            format!(r#"{{"error": {{"code": "error", "message": "no such method {method}"}}}}"#)
        };
        // Neither call: empty, with no notice, and both halves too old.
        yard.answer("Arrivals", &old("Arrivals"));
        yard.answer("Flows", &old("Flows"));
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.arrivals.rows.is_empty());
        assert_eq!(state.arrivals_notice, None);
        assert!(state.arrivals_too_old);
        assert_eq!(state.flows, Flows::default());
        assert_eq!(state.flows_notice, None);
        assert!(state.flows_too_old);
        // Either half alone: its half too old, the other as it reads.
        yard.answer("Arrivals", r#"{"result": []}"#);
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.arrivals_notice, None);
        assert!(!state.arrivals_too_old);
        assert_eq!(state.flows_notice, None);
        assert!(state.flows_too_old);
        yard.answer("Arrivals", &old("Arrivals"));
        yard.answer(
            "Flows",
            r#"{"result": {"travelling": 0, "depots": [], "unassigned": []}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.arrivals_too_old);
        assert!(!state.flows_too_old);
        // Both halves read: neither too old.
        yard.answer("Arrivals", r#"{"result": []}"#);
        let state = load(&yard.client(), "jo").unwrap();
        assert!(!state.arrivals_too_old);
        assert!(!state.flows_too_old);
    }

    /// The grants are read where the window is the operator's, with the
    /// yard's operator as read: a yard from before the call reads as
    /// empty with its pane saying so, one that refuses the list as a
    /// request come down the link from its edge reads as empty with the
    /// pane saying where they are read, and an unreadable one with why.
    /// Anywhere else no grants call is made.
    #[test]
    fn load_reads_the_grants_for_the_operator_alone() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // The operator's: the grants are read.
        yard.answer(
            "SourceGrants",
            r#"{"result": [{"source": "crm", "crew": "yardmaster",
                 "calls": ["read"], "at": "2026-10-08T09:30:00Z", "by": "jo"}]}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.me, "jo");
        assert_eq!(state.operator, "jo");
        assert_eq!(state.grants.len(), 1);
        assert_eq!(state.grants[0].source, "crm");
        assert_eq!(state.grants_notice, None);
        assert!(!state.grants_too_old);
        assert!(!state.grants_on_the_yard);
        assert_eq!(yard.asked("SourceGrants"), vec![json!({})]);

        // A yard from before the call: empty, with no notice, and too old.
        yard.answer(
            "SourceGrants",
            r#"{"error": {"code": "error", "message": "no such method SourceGrants"}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.grants.is_empty());
        assert_eq!(state.grants_notice, None);
        assert!(state.grants_too_old);
        assert!(!state.grants_on_the_yard);

        // A yard that says the request came down the link from its edge:
        // empty, with no notice, not too old, and read on its machine.
        yard.answer(
            "SourceGrants",
            &json!({"error": {"code": "error", "message": "the operator's grant list is not read over the link: this request came down the link from the edge. Read it on the yard's machine (`yardr source grants`). The edge saw the device's token with it: remove the device and add it again (yardr user device rm)."}}).to_string(),
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.grants.is_empty());
        assert_eq!(state.grants_notice, None);
        assert!(!state.grants_too_old);
        assert!(state.grants_on_the_yard);

        // An unreadable one: empty, with why, and neither.
        yard.answer_nothing("SourceGrants");
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.grants.is_empty());
        assert!(
            state
                .grants_notice
                .as_deref()
                .unwrap()
                .starts_with("the grants could not be read: ")
        );
        assert!(!state.grants_too_old);
        assert!(!state.grants_on_the_yard);
    }

    /// The operator's window reads the teammates: each row's user and
    /// devices. A yard from before the call reads as empty with no
    /// notice, and too old instead, and an unreadable one with why.
    /// Anywhere else no teammates call is made.
    #[test]
    fn load_reads_the_teammates_for_the_operator_alone() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // The operator's: the teammates are read.
        yard.answer(
            "Teammates",
            r#"{"result": [
                {"name": "hal", "role": "member", "email": "hal@example.net",
                 "invited_by": "jo", "invited_at": "2026-10-08T09:30:00Z",
                 "devices": []},
                {"name": "sam", "role": "member", "email": "sam@example.net",
                 "invited_by": "jo", "invited_at": "2026-10-08T09:30:00Z",
                 "devices": [{"name": "tablet", "last_used": "2026-10-08T09:30:00Z"}]}
            ]}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.me, "jo");
        assert_eq!(state.operator, "jo");
        assert_eq!(state.teammates.len(), 2);
        assert_eq!(state.teammates[0].name, "hal");
        assert!(state.teammates[0].devices.is_empty());
        assert_eq!(state.teammates[1].name, "sam");
        assert_eq!(state.teammates[1].devices[0].name, "tablet");
        assert_eq!(state.teammates_notice, None);
        assert!(!state.teammates_too_old);
        assert_eq!(yard.asked("Teammates"), vec![json!({})]);

        // A yard from before the call: empty, with no notice, and too old.
        yard.answer(
            "Teammates",
            r#"{"error": {"code": "error", "message": "no such method Teammates"}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.teammates.is_empty());
        assert_eq!(state.teammates_notice, None);
        assert!(state.teammates_too_old);

        // An unreadable one: empty, with why, and not too old.
        yard.answer_nothing("Teammates");
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.teammates.is_empty());
        assert!(
            state
                .teammates_notice
                .as_deref()
                .unwrap()
                .starts_with("the teammates could not be read: ")
        );
        assert!(!state.teammates_too_old);
    }

    /// Anywhere but the operator's, the window reads no teammates: not
    /// for another user, not where the yard names no operator, and not
    /// where its settings could not be read. But the operator's window
    /// of a hosted yard is asked: the teammates come down the link from
    /// its edge, as an approval does. The window still opens.
    #[test]
    fn load_reads_no_teammates_where_the_window_is_not_the_operator_s() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // Another user: the operator's call is never made.
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "ada", "cursor": 9, "notification_cursor": 4}}"#,
        );
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0},
                           {"name": "ada", "cursor": 9, "notification_cursor": 4}]}"#,
        );
        let state = load(&yard.client(), "ada").unwrap();
        assert_eq!(state.me, "ada");
        assert_eq!(state.operator, "jo");
        assert!(state.teammates.is_empty());
        assert_eq!(state.teammates_notice, None);
        assert!(!state.teammates_too_old);
        assert!(yard.asked("Teammates").is_empty());

        // No operator: nobody's window is the operator's.
        yard.answer("Settings", r#"{"result": {}}"#);
        let state = load(&yard.client(), "ada").unwrap();
        assert_eq!(state.me, "ada");
        assert!(state.operator.is_empty());
        assert!(state.teammates.is_empty());
        assert!(yard.asked("Teammates").is_empty());

        // Unreadable settings: the window still opens, with no teammates.
        yard.answer_nothing("Settings");
        let state = load(&yard.client(), "ada").unwrap();
        assert!(state.operator.is_empty());
        assert!(state.teammates.is_empty());

        // The operator's window of a hosted yard: asked down the link,
        // and answered, where the grant list is not.
        use crate::fake::{Router, SESSION, TOKEN};
        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        yard.answer(
            "Teammates",
            r#"{"result": [{"name": "sam", "role": "member", "email": "sam@example.net",
                 "invited_by": "jo", "invited_at": "2026-10-08T09:30:00Z",
                 "devices": [{"name": "tablet"}]}]}"#,
        );
        let edge = Router::start(&yard, SESSION);
        let hosted = load(&edge.client_with(TOKEN, SESSION), "jo").unwrap();
        assert_eq!(hosted.me, "jo");
        assert_eq!(hosted.operator, "jo");
        assert_eq!(hosted.teammates.len(), 1);
        assert_eq!(hosted.teammates[0].name, "sam");
        assert_eq!(hosted.teammates_notice, None);
        assert!(!hosted.teammates_too_old);
        assert_eq!(yard.asked("Teammates"), vec![json!({})]);
    }

    /// The preview is read where the window is the operator's: a yard
    /// from before the calls reads as unserved with no notice, one that
    /// refuses the preview as a request come down the link from its edge
    /// reads as unserved too, and an unreadable one with why. Anywhere
    /// else no call is made: not for another user, and not for the
    /// operator's window of a hosted yard, which the yard serves neither
    /// call down the link, where the same yard asked at its address with
    /// no edge before it is asked, and answers. The window still opens.
    #[test]
    fn load_reads_the_telemetry_preview_for_the_operator_on_a_yard_that_serves_it() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // The operator's: the preview is read.
        yard.answer(
            "TelemetryShow",
            r#"{"result": {"install_id": "3f6d1c2e-9a4b-4e7a-b5c1-2d8f0a6e4b90", "telemetry": "on", "since": "2026-10-26", "sending_day": "2026-10-28", "hosted": false, "request": {"batch": []}}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert_eq!(state.me, "jo");
        assert_eq!(state.operator, "jo");
        assert_eq!(
            state.telemetry.as_ref().unwrap().install_id,
            "3f6d1c2e-9a4b-4e7a-b5c1-2d8f0a6e4b90"
        );
        assert_eq!(state.telemetry_notice, None);
        assert!(!state.telemetry_unserved);
        assert_eq!(yard.asked("TelemetryShow"), vec![json!({})]);

        // A yard from before the calls: unserved, with no notice.
        yard.answer(
            "TelemetryShow",
            r#"{"error": {"code": "error", "message": "no such method TelemetryShow"}}"#,
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.telemetry.is_none());
        assert_eq!(state.telemetry_notice, None);
        assert!(state.telemetry_unserved);

        // A yard that says the request came down the link from its edge:
        // unserved too, with no notice.
        yard.answer(
            "TelemetryShow",
            &json!({"error": {"code": "error", "message": "the telemetry switch is not read over the link: this request came down the link from the edge. Read it on the yard's machine (`yardr telemetry show`). The edge saw the device's token with it: remove the device and add it again (yardr user device rm)."}}).to_string(),
        );
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.telemetry.is_none());
        assert_eq!(state.telemetry_notice, None);
        assert!(state.telemetry_unserved);

        // An unreadable one: empty, with why, and served still.
        yard.answer("TelemetryShow", r#"{"result": {"telemetry": 7}}"#);
        let state = load(&yard.client(), "jo").unwrap();
        assert!(state.telemetry.is_none());
        assert!(
            state
                .telemetry_notice
                .as_deref()
                .unwrap()
                .starts_with("the telemetry switch could not be read: ")
        );
        assert!(!state.telemetry_unserved);

        // Another user: the operator's call is never made.
        yard.answer(
            "TelemetryShow",
            r#"{"result": {"telemetry": "off", "sending_day": "2026-10-28", "hosted": false, "request": null, "no_batch": "off"}}"#,
        );
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "ada", "cursor": 9, "notification_cursor": 4}}"#,
        );
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0},
                           {"name": "ada", "cursor": 9, "notification_cursor": 4}]}"#,
        );
        let asked = yard.asked("TelemetryShow").len();
        let state = load(&yard.client(), "ada").unwrap();
        assert_eq!(state.me, "ada");
        assert_eq!(state.operator, "jo");
        assert!(state.telemetry.is_none());
        assert_eq!(state.telemetry_notice, None);
        assert!(!state.telemetry_unserved);
        assert_eq!(yard.asked("TelemetryShow").len(), asked);

        // The operator's window of a hosted yard: no call, no notice, and
        // unserved. The same yard at its address, with no edge before it,
        // is asked, and answers.
        use crate::fake::{Router, SESSION, TOKEN};
        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        yard.answer(
            "TelemetryShow",
            r#"{"result": {"telemetry": "off", "sending_day": "2026-10-28", "hosted": false, "request": null, "no_batch": "off"}}"#,
        );
        let direct = load(&yard.client(), "jo").unwrap();
        assert_eq!(direct.me, "jo");
        assert!(!direct.telemetry_unserved);
        assert_eq!(yard.asked("TelemetryShow"), vec![json!({})]);
        let edge = Router::start(&yard, SESSION);
        let hosted = load(&edge.client_with(TOKEN, SESSION), "jo").unwrap();
        assert_eq!(hosted.me, "jo");
        assert_eq!(hosted.operator, "jo");
        assert!(hosted.telemetry.is_none());
        assert_eq!(hosted.telemetry_notice, None);
        assert!(hosted.telemetry_unserved);
        assert_eq!(yard.asked("TelemetryShow"), vec![json!({})]);
    }

    /// Anywhere but the operator's, the window reads no grants: not for
    /// another user, not where the yard names no operator, and not where
    /// its settings could not be read. Nor does the operator's window of a
    /// hosted yard: the yard serves the list to no request that comes down
    /// the link from its edge, so it is not asked, where the same yard
    /// asked at its address with no edge before it is. That yard reads no
    /// session's screen down that link either, which the state says too. The
    /// window still opens.
    #[test]
    fn load_reads_no_grants_where_the_window_is_not_given_them() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // Another user: the operator's call is never made.
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "ada", "cursor": 9, "notification_cursor": 4}}"#,
        );
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0},
                           {"name": "ada", "cursor": 9, "notification_cursor": 4}]}"#,
        );
        let state = load(&yard.client(), "ada").unwrap();
        assert_eq!(state.me, "ada");
        assert_eq!(state.operator, "jo");
        assert!(state.grants.is_empty());
        assert_eq!(state.grants_notice, None);
        assert!(!state.grants_too_old);
        assert!(yard.asked("SourceGrants").is_empty());

        // No operator: nobody's window is the operator's.
        yard.answer("Settings", r#"{"result": {}}"#);
        let state = load(&yard.client(), "ada").unwrap();
        assert_eq!(state.me, "ada");
        assert!(state.operator.is_empty());
        assert!(state.grants.is_empty());
        assert!(yard.asked("SourceGrants").is_empty());

        // Unreadable settings: the window still opens, with no grants.
        yard.answer_nothing("Settings");
        let state = load(&yard.client(), "ada").unwrap();
        assert!(state.operator.is_empty());
        assert!(state.grants.is_empty());

        // The operator's window of a hosted yard: no call, no notice, and
        // the pane says where they are read. The same yard at its address,
        // with no edge before it, is asked, and answers.
        use crate::fake::{Router, SESSION, TOKEN};
        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        let direct = load(&yard.client(), "jo").unwrap();
        assert_eq!(direct.me, "jo");
        assert_eq!(direct.operator, "jo");
        assert!(!direct.grants_on_the_yard);
        assert!(!direct.screen_on_the_yard);
        assert_eq!(yard.asked("SourceGrants"), vec![json!({})]);
        let edge = Router::start(&yard, SESSION);
        let hosted = load(&edge.client_with(TOKEN, SESSION), "jo").unwrap();
        assert_eq!(hosted.me, "jo");
        assert_eq!(hosted.operator, "jo");
        assert!(hosted.grants.is_empty());
        assert_eq!(hosted.grants_notice, None);
        assert!(!hosted.grants_too_old);
        assert!(hosted.grants_on_the_yard);
        assert!(hosted.screen_on_the_yard);
        assert_eq!(yard.asked("SourceGrants"), vec![json!({})]);

        // Another user's window of it says the same, and asks no more: the
        // yard is reached by the edge whoever is at the window, and an
        // operator named later is not asked either.
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "ada", "cursor": 9, "notification_cursor": 4}}"#,
        );
        let hosted = load(&edge.client_with(TOKEN, SESSION), "ada").unwrap();
        assert_eq!(hosted.me, "ada");
        assert_eq!(hosted.operator, "jo");
        assert!(hosted.grants_on_the_yard);
        assert!(hosted.screen_on_the_yard);
        assert_eq!(yard.asked("SourceGrants"), vec![json!({})]);
    }

    /// A yard at an address: the window is for the user its token is of,
    /// nothing answering is said as it is there, and a device that is
    /// removed is told once and never asked after again.
    #[test]
    fn a_device_that_is_removed_is_told_so_and_the_yard_is_not_asked_again() {
        use crate::fake::{Router, SESSION, TOKEN};

        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = yard.client();
            // The login name of this machine is nobody there.
            move || {
                follow_with_control(
                    &client,
                    "mac-login",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    false,
                    |update| tx.send(update).is_ok(),
                )
            }
        });
        let wait = Duration::from_secs(10);
        let state = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Up(state) => break state,
                // A startup read can fail transiently; the follower says
                // Down and retries before it has a state to show.
                Update::Down(..) => continue,
                Update::Event(event) => panic!("an event before the state: {event:?}"),
                Update::Out(why) | Update::SignedOut(why) => {
                    panic!("the edge and yard let the device in: {why}")
                }
                Update::Removed => panic!("no member yard here"),
            }
        };
        assert_eq!(state.me, "jo");
        assert_eq!(state.yard.as_ref().unwrap().said_by, "laptop");
        assert_eq!(yard.asked("GetUser"), vec![json!({"name": "jo"})]);
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Update::Event(_)));

        // The operator removes the device, one of two: its stream ends, and
        // the next request is answered 401.
        yard.add_device("yardr-dev-phone");
        yard.remove_device(crate::fake::TOKEN);
        let why = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Out(why) => break why,
                Update::SignedOut(why) => panic!("no edge stands here: {why}"),
                Update::Down(why, _) => panic!("it is there, and says no: {why}"),
                Update::Up(_) | Update::Event(_) => continue,
                Update::Removed => panic!("no member yard here"),
            }
        };
        assert_eq!(
            why,
            "This device is no longer let in: the yard refuses its token. Its operator removed the device, or the token is not this yard's. Pair it again, with a code the operator opens: yardr user device pair <user> <device>"
        );
        // The thread has ended with the window still there, and many of its
        // 20 milliseconds later nothing more was asked.
        follower.join().unwrap();
        let asked = yard.requests();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(yard.requests(), asked);
        assert!(rx.try_recv().is_err());

        // A member yard refuses the same device without a code the member
        // cannot get from its operator.
        let member_yard = Yard::start_tcp();
        a_yard_of_jo(&member_yard);
        member_yard.add_device("yardr-dev-phone");
        member_yard.remove_device(TOKEN);
        let edge = Router::start(&member_yard, SESSION);
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = edge.client_with_yard(TOKEN, SESSION, "alice/lab");
            move || {
                follow_with_control(
                    &client,
                    "mac-login",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    true,
                    |update| tx.send(update).is_ok(),
                )
            }
        });
        let Update::Out(why) = rx.recv_timeout(Duration::from_secs(10)).unwrap() else {
            panic!("a refused member device is out, not down")
        };
        follower.join().unwrap();
        assert_eq!(
            why,
            "This device is no longer let in: the yard refuses its token, or your access was removed. Pair this device again from the yard's card, in your own session."
        );
        assert!(!why.contains("operator"), "{why}");
        assert!(!why.contains("yardr user device pair"), "{why}");
    }

    /// Through the edge: a session that is revoked is told so and never
    /// asked after again, with signing in again as what to do, while the
    /// device pairing stands as it was. A 401 that is neither the edge's
    /// nor the yard's is told as trouble that passes, before that, and the
    /// yard is asked again until it answers.
    #[test]
    fn a_session_that_is_revoked_is_told_so_and_the_yard_is_not_asked_again() {
        use crate::fake::{Router, SESSION};

        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        let edge = Router::start(&yard, SESSION);
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = edge.client_with(crate::fake::TOKEN, SESSION);
            move || {
                follow_with_control(
                    &client,
                    "mac-login",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    false,
                    |update| tx.send(update).is_ok(),
                )
            }
        });
        let wait = Duration::from_secs(10);
        let Update::Up(state) = rx.recv_timeout(wait).unwrap() else {
            panic!("the yard is there")
        };
        assert_eq!(state.me, "jo");
        // `Up` is told before the follower opens its event stream. Wait
        // until the fake yard has accepted that stream before emitting, so
        // this checks a live relay instead of racing its setup.
        let deadline = std::time::Instant::now() + wait;
        while yard.streams_asked().is_empty() && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(yard.streams_asked(), vec![20]);
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
        let Update::Event(event) = rx.recv_timeout(wait).unwrap() else {
            panic!("the live stream carries the event")
        };
        assert_eq!(event.seq, 21);

        // A gateway turns the next state read away with a 401 of its own,
        // as the yard's web view does what the edge sent it by mistake
        // (yardr-hosted-c0sb). Neither the device nor the session is out:
        // the window is told the yard cannot be read, the thread goes on
        // asking, and the state comes back once the gateway lets it by. A
        // device that is out ends the thread (the test above).
        edge.turn_away("/user/call/Head");
        yard.end_streams();
        let (why, short) = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Down(why, short) => break (why, short),
                Update::Out(why) => panic!("a gateway's 401 is not the device refused: {why}"),
                Update::SignedOut(why) => panic!("the session is good: {why}"),
                Update::Up(_) | Update::Event(_) => continue,
                Update::Removed => panic!("no member yard here"),
            }
        };
        assert!(
            why.starts_with("A gateway turned the window's request away (401)"),
            "{why}"
        );
        assert!(why.contains("tries again"), "{why}");
        assert!(!why.contains("no longer let in"), "{why}");
        assert_eq!(short, "gateway refused");
        edge.let_through();
        loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Up(state) => {
                    assert_eq!(state.me, "jo");
                    break;
                }
                Update::Down(..) | Update::Event(_) => continue,
                Update::Out(why) | Update::SignedOut(why) => panic!("it passes again: {why}"),
                Update::Removed => panic!("no member yard here"),
            }
        }

        // The session is revoked at the edge: what it relayed is cut, and
        // the next request is answered with the edge's own 401.
        edge.revoke(SESSION);
        let why = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::SignedOut(why) => break why,
                Update::Out(why) => panic!("the device is still paired: {why}"),
                Update::Down(why, _) => panic!("it is there, and says no: {why}"),
                Update::Up(_) | Update::Event(_) => continue,
                Update::Removed => panic!("no member yard here"),
            }
        };
        assert!(
            why.starts_with("This session is no longer signed in"),
            "{why}"
        );
        assert!(why.contains("Sign in to this hosted yard again"), "{why}");
        // The thread has ended with the window still there, and many of its
        // 20 milliseconds later nothing more was asked, of the edge or of
        // the yard.
        follower.join().unwrap();
        let (proxied, asked) = (edge.proxied(), yard.requests());
        thread::sleep(Duration::from_millis(200));
        assert_eq!(edge.proxied(), proxied);
        assert_eq!(yard.requests(), asked);
        assert!(rx.try_recv().is_err());
    }

    /// Through the edge, a member yard the edge answers holds no longer is
    /// told once as removed and never asked again; any other refusal of
    /// such a yard, and the same sentence of an own yard, are told as
    /// trouble that passes, and the yard is asked again.
    #[test]
    fn a_member_yard_the_edge_holds_no_longer_is_removed_and_nothing_else_is() {
        use crate::fake::{Router, SESSION, TOKEN};

        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        let edge = Router::start(&yard, SESSION);
        let wait = Duration::from_secs(10);

        // A member yard, working.
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = edge.client_with_yard(TOKEN, SESSION, "alice/lab");
            move || {
                follow_with_control(
                    &client,
                    "mac-login",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    true,
                    |update| tx.send(update).is_ok(),
                )
            }
        });
        let Update::Up(state) = rx.recv_timeout(wait).unwrap() else {
            panic!("the yard is there")
        };
        assert_eq!(state.me, "jo");

        // Any other 403 of a member yard is trouble that passes: the yard
        // is said down and asked again, never removed.
        edge.no_yard("the yard is building");
        yard.end_streams();
        let (why, _) = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Down(why, short) => break (why, short),
                Update::Up(_) | Update::Event(_) => continue,
                Update::Out(why) | Update::SignedOut(why) => {
                    panic!("it is there, and says no: {why}")
                }
                Update::Removed => panic!("a building yard is not a removal"),
            }
        };
        assert!(why.contains("the yard is building"), "{why}");
        // Still asked: another Down comes.
        loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Down(..) => break,
                Update::Up(_) | Update::Event(_) => continue,
                update => panic!("still asked, not removed: {update:?}"),
            }
        }

        // The holds-no-yard answer is the removal: told once, never asked
        // again.
        edge.no_yard(crate::api::HOLDS_NO_YARD);
        loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Removed => break,
                Update::Down(..) | Update::Up(_) | Update::Event(_) => continue,
                Update::Out(why) | Update::SignedOut(why) => {
                    panic!("it is there, and says no: {why}")
                }
            }
        }
        follower.join().unwrap();
        let (proxied, asked) = (edge.proxied(), yard.requests());
        thread::sleep(Duration::from_millis(200));
        assert_eq!(edge.proxied(), proxied);
        assert_eq!(yard.requests(), asked);
        assert!(rx.try_recv().is_err());

        // The same sentence of an own yard is trouble that passes, not a
        // removal.
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = edge.client_with(TOKEN, SESSION);
            move || {
                follow_with_control(
                    &client,
                    "mac-login",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    false,
                    |update| tx.send(update).is_ok(),
                )
            }
        });
        let (why, _) = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Down(why, short) => break (why, short),
                Update::Up(_) | Update::Event(_) => continue,
                Update::Out(why) | Update::SignedOut(why) => {
                    panic!("it is there, and says no: {why}")
                }
                Update::Removed => panic!("an own yard is not removed"),
            }
        };
        assert!(why.contains(crate::api::HOLDS_NO_YARD), "{why}");
        drop(rx);
        follower.join().unwrap();
    }

    /// A follower that is stopped while its read waits on a quiet stream
    /// ends at once, whatever the yard does about the stop. These yards keep
    /// the stream open when their client hangs up, as a proxy that takes
    /// the FIN for a half-close does, so that the yard's closing cannot be
    /// what wakes the read: on Windows, shutting a socket down does not.
    #[test]
    fn stopping_a_follower_closes_its_idle_stream() {
        for yard in [Yard::start(), Yard::start_tcp()] {
            a_yard_of_jo(&yard);
            yard.hold_streams();
            let way = yard.client().place();
            let stop = FollowControl::default();
            let cancel = stop.clone();
            let (updates, received) = mpsc::channel();
            let (finished, done) = mpsc::channel();
            let follower = thread::spawn({
                let client = yard.client();
                move || {
                    follow_with_control(
                        &client,
                        "jo",
                        Duration::from_secs(10),
                        WATCH,
                        stop,
                        false,
                        |update| updates.send(update).is_ok(),
                    );
                    finished.send(()).unwrap();
                }
            });
            assert!(
                matches!(
                    received.recv_timeout(Duration::from_secs(2)).unwrap(),
                    Update::Up(_)
                ),
                "{way}"
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while yard.streams_asked().is_empty() && std::time::Instant::now() < deadline {
                thread::yield_now();
            }
            assert!(!yard.streams_asked().is_empty(), "{way}: the stream opened");
            // An event read and told: the follower goes on to wait for the
            // next. Stopped before it waits, its read would fail at once on
            // any system, and the test would show nothing.
            yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
            assert!(
                matches!(
                    received.recv_timeout(Duration::from_secs(2)).unwrap(),
                    Update::Event(_)
                ),
                "{way}"
            );
            // Nothing says that the read is waiting. A little time makes it
            // so; a stop that came earlier still ends the follower, and the
            // test is then no proof, but never a failure.
            thread::sleep(Duration::from_millis(100));

            cancel.stop();

            done.recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("{way}: stopping the follower ends its blocked read"));
            follower.join().unwrap();
        }
    }

    /// A follower of `yard` for jo, trying again after 20 milliseconds and
    /// watching its stream as `watch` says, once its state is told and its
    /// stream opened: what it tells from then on.
    fn following(yard: &Yard, watch: Watch) -> mpsc::Receiver<Update> {
        let (tx, rx) = mpsc::channel();
        let client = yard.client();
        thread::spawn(move || {
            let retry = Duration::from_millis(20);
            follow_with_control(
                &client,
                "jo",
                retry,
                watch,
                FollowControl::default(),
                false,
                |u| tx.send(u).is_ok(),
            )
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            Update::Up(_)
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.streams_asked().is_empty() && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(yard.streams_asked(), vec![20]);
        rx
    }

    /// A follower whose next step waits for the test to let it go. This lets
    /// a test set up the yard before a quiet stream can ask for its head.
    fn following_stepwise(yard: &Yard, watch: Watch) -> (mpsc::Receiver<Update>, mpsc::Sender<()>) {
        let (tx, rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let client = yard.client();
        thread::spawn(move || {
            let retry = Duration::from_millis(20);
            follow_with_control(
                &client,
                "jo",
                retry,
                watch,
                FollowControl::default(),
                false,
                |update| tx.send(update).is_ok() && continue_rx.recv().is_ok(),
            )
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            Update::Up(_)
        ));
        continue_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.streams_asked().is_empty() && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(yard.streams_asked(), vec![20]);
        (rx, continue_tx)
    }

    /// Waits until the yard has been asked for `n` streams.
    fn streams_opened(yard: &Yard, n: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.streams_asked().len() < n && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            yard.streams_asked().len() >= n,
            "{:?}",
            yard.streams_asked()
        );
    }

    /// Moves the yard's head to `head` and waits until the follower, finding
    /// its stream silent, has asked for it. The follower is held in that
    /// ask, and reads no stream in it: no quiet runs there, whatever the
    /// speed of this machine, until [`Yard::release_calls`] lets it go on.
    /// The call is held before the head moves, so that an ask that came
    /// first was answered with the old head and found the stream level.
    fn hold_the_follower(yard: &Yard, head: u64) {
        yard.hold_calls("Head");
        yard.answer("Head", &json!({ "result": head }).to_string());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.calls_held("Head") == 0 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(
            yard.calls_held("Head"),
            1,
            "the follower asked for the head"
        );
    }

    /// Waits until the yard has been asked for its head `n` times.
    fn heads_asked(yard: &Yard, n: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.asked("Head").len() < n && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        let asked = yard.asked("Head").len();
        assert!(asked >= n, "the head was asked for {asked} times, not {n}");
    }

    /// Emits `event` and waits until the yard's stream has written it: a
    /// follower that reads has it at once.
    fn emit_to_the_wire(yard: &Yard, event: &str) {
        let sent = yard.events_sent();
        yard.emit(event);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.events_sent() == sent && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(yard.events_sent() > sent, "the stream wrote the event");
    }

    /// A stream that stays open and brings nothing while the yard moves
    /// on is replaced: the stream is opened again after the last seq read,
    /// and what happened meanwhile comes down it, with the state left as
    /// it was.
    #[test]
    fn a_stream_that_says_nothing_while_the_yard_moves_on_is_replaced() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let watch = Watch {
            quiet: Duration::from_millis(100),
        };
        let rx = following(&yard, watch);
        yard.mute();
        yard.answer("Head", r#"{"result": 21}"#);
        yard.emit(r#"{"seq": 21, "kind": "noted", "actor": "yardmaster", "bead": "yd-shed", "data": {"note": 7}}"#);
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the event the stuck stream held back, and no state read again")
        };
        assert_eq!(event.seq, 21);
        assert_eq!(yard.streams_asked(), vec![20, 20]);
        assert_eq!(yard.asked("UserInbox").len(), 1);
    }

    /// A stream that ends is opened again after the last seq read, and the
    /// state is not read again: nothing is told but what the stream brings,
    /// so no view of the window is rebuilt for a stream that was cut and
    /// came back with nothing new, as an edge cuts one.
    #[test]
    fn a_stream_that_ends_is_opened_again_after_the_last_seq_and_tells_nothing() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let rx = following(&yard, WATCH);

        // Cut with nothing read: opened again after the head it began from.
        yard.end_streams();
        streams_opened(&yard, 2);
        assert_eq!(yard.streams_asked(), vec![20, 20]);

        // An event down the new stream; cut again with the yard's head at
        // it: opened again after that event.
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the event down the stream that came back")
        };
        assert_eq!(event.seq, 21);
        yard.answer("Head", r#"{"result": 21}"#);
        yard.end_streams();
        streams_opened(&yard, 3);
        assert_eq!(yard.streams_asked(), vec![20, 20, 21]);

        // Down that one, another event: the stream is live. And nothing was
        // told in between, nor was the state read again.
        yard.emit(r#"{"seq": 22, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "newer"}}"#);
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the event down the stream that came back again")
        };
        assert_eq!(event.seq, 22);
        assert!(rx.try_recv().is_err());
        assert_eq!(yard.asked("UserInbox").len(), 1);
    }

    /// A yard whose head is behind what was read is not the one that was
    /// followed (it was rewound, or restored from a backup): the state is
    /// read again and told, and the stream is opened after its head.
    #[test]
    fn a_stream_that_ends_on_a_yard_behind_what_was_read_reads_the_state_again() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let rx = following(&yard, WATCH);
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Update::Event(_)
        ));
        // Its head still says 20, as the state it was told did.
        yard.end_streams();
        let Update::Up(state) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the state read again")
        };
        assert_eq!(state.head, 20);
        streams_opened(&yard, 2);
        assert_eq!(yard.streams_asked(), vec![20, 20]);
    }

    /// An event that cannot be read ends the stream, and the stream opened
    /// again after the last seq read would meet it again: the state is read
    /// again, which is past it, and the stream is opened after that head.
    #[test]
    fn an_event_that_cannot_be_read_has_the_state_read_again_past_it() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let rx = following(&yard, WATCH);
        yard.answer("Head", r#"{"result": 21}"#);
        yard.emit(r#"{"seq": 21, "kind": 7}"#);
        let Update::Up(state) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the state read again")
        };
        assert_eq!(state.head, 21);
        streams_opened(&yard, 2);
        assert_eq!(yard.streams_asked(), vec![20, 21]);
    }

    /// A stream that cannot be opened has the state read again, which is
    /// told only when it is not the state the window has: a quiet yard
    /// whose stream is refused is asked over and over and changes nothing
    /// in the window, and one that moved is told as ever.
    #[test]
    fn a_state_read_again_that_is_the_one_told_is_not_told_again() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // The stream is held twice over: the second is the retry of the
        // first. So it is not opened, and the state is read again.
        yard.stall_once("/events");
        yard.stall_once("/events");
        let watch = Watch {
            quiet: Duration::from_millis(100),
        };
        let rx = following(&yard, watch);
        assert_eq!(yard.asked("UserInbox").len(), 2);
        assert!(rx.try_recv().is_err());

        // The yard moves while its stream cannot be had: told.
        yard.stall_once("/events");
        yard.stall_once("/events");
        yard.answer("Head", r#"{"result": 21}"#);
        yard.end_streams();
        let Update::Up(state) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the state read again, which is not the one told")
        };
        assert_eq!(state.head, 21);
    }

    /// A stream that says nothing while the yard says nothing either is
    /// read on: the yard is asked its head, and the stream stays.
    #[test]
    fn a_quiet_stream_of_a_quiet_yard_is_read_on() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let watch = Watch {
            quiet: Duration::from_millis(50),
        };
        let rx = following(&yard, watch);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.asked("Head").len() < 4 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(yard.asked("Head").len() >= 4);
        assert_eq!(yard.streams_asked(), vec![20]);
        assert!(rx.try_recv().is_err());
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the stream read on brings the event")
        };
        assert_eq!(event.seq, 21);
        assert_eq!(yard.streams_asked(), vec![20]);
    }

    /// A head beyond what the stream brought gives the event a moment
    /// more to come: one that comes in it leaves the stream as it is. A
    /// stream that is late but not broken is waited for up to the quiet
    /// limit: an event that arrives after a quiet of lateness still comes
    /// down the stream it was waited for on, with the state left as it was.
    /// A second late event, with no level head between, is given the same
    /// moment: reading the waited-for event ends its grace.
    /// The test holds the follower in its ask for the head while it puts the
    /// event on the wire, so that how late the event is is not up to how
    /// fast this machine is. The moment itself is shown from the side a slow
    /// machine cannot move: a stream not given its event is asked about
    /// again, and then replaced, no sooner than most of a quiet after the
    /// head was found ahead.
    #[test]
    fn an_event_on_its_way_is_given_a_moment_to_come() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let watch = Watch {
            quiet: Duration::from_millis(300),
        };
        let (rx, continue_following) = following_stepwise(&yard, watch);
        hold_the_follower(&yard, 21);
        emit_to_the_wire(
            &yard,
            r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#,
        );
        yard.release_calls("Head");
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the event on its way")
        };
        assert_eq!(event.seq, 21);
        continue_following.send(()).unwrap();
        // Read level with the head again: the stream stays past several
        // more of its silences.
        heads_asked(&yard, yard.asked("Head").len() + 3);
        assert_eq!(yard.streams_asked(), vec![20]);
        assert!(rx.try_recv().is_err());

        // Late: the head moves again, and the event comes after the ask for
        // it, as the quiet runs out. The stream is waited for, not replaced,
        // and the state is not read again.
        hold_the_follower(&yard, 22);
        emit_to_the_wire(
            &yard,
            r#"{"seq": 22, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "newer"}}"#,
        );
        yard.release_calls("Head");
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the late event, down the stream it was waited for on")
        };
        assert_eq!(event.seq, 22);
        assert_eq!(yard.streams_asked(), vec![20]);
        assert_eq!(yard.asked("UserInbox").len(), 1);

        // A second late event, with no level silence between: reading the
        // event its grace was waited for ends that grace, so this one is
        // given a moment of its own, and the stream stays.
        yard.hold_calls("Head");
        yard.answer("Head", &json!({ "result": 23 }).to_string());
        continue_following.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.calls_held("Head") == 0 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(
            yard.calls_held("Head"),
            1,
            "the follower asked for the head"
        );
        emit_to_the_wire(
            &yard,
            r#"{"seq": 23, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "newest"}}"#,
        );
        yard.release_calls("Head");
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the second late event, down the stream it was waited for on")
        };
        assert_eq!(event.seq, 23);
        continue_following.send(()).unwrap();
        assert_eq!(yard.streams_asked(), vec![20]);
        assert_eq!(yard.asked("UserInbox").len(), 1);

        // No event comes. Level with the head once more, the follower waits
        // for none; then it finds the head ahead and is given its moment, and
        // asks again, to replace the stream, only when that is over: no
        // sooner than most of a quiet after it was let go, and a slow
        // machine only makes it later.
        heads_asked(&yard, yard.asked("Head").len() + 1);
        hold_the_follower(&yard, 24);
        let asked = yard.asked("Head").len();
        let ahead = std::time::Instant::now();
        yard.release_calls("Head");
        heads_asked(&yard, asked + 1);
        assert!(
            ahead.elapsed() >= watch.quiet * 2 / 3,
            "asked again after {:?}",
            ahead.elapsed()
        );
        streams_opened(&yard, 2);
        assert_eq!(yard.streams_asked(), vec![20, 23]);
        assert_eq!(yard.asked("UserInbox").len(), 1);
    }

    /// The yard's last device removed: it answers 404 then, to a token as
    /// to none. That is said as what it is, and asked again, since an
    /// address that is no yard's answers the same.
    #[test]
    fn a_yard_with_no_device_left_is_said_so() {
        use crate::fake::{Router, SESSION, TOKEN};

        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        let client = yard.client();
        yard.remove_device(crate::fake::TOKEN);
        let trouble = load(&client, "jo").unwrap_err();
        assert!(
            matches!(trouble, Trouble::Yard(api::Error::NoDevice)),
            "{trouble:?}"
        );
        let why = trouble.to_string();
        assert!(
            why.starts_with("Nothing of a yard is under /user/"),
            "{why}"
        );
        assert!(why.contains("yardr user device pair"), "{why}");
        assert_eq!(trouble.short(), PAIR_AGAIN);

        // Both kinds of yard reach the same 404, but a member gets the
        // pairing route in its own session while an own yard keeps today's
        // operator instructions.
        let tell_first = |client, is_member| {
            let (tx, rx) = mpsc::channel();
            let follower = thread::spawn(move || {
                follow_with_control(
                    &client,
                    "mac-login",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    is_member,
                    |update| {
                        let _ = tx.send(update);
                        false
                    },
                )
            });
            let update = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            follower.join().unwrap();
            update
        };

        let Update::Down(own_why, short) = tell_first(yard.client(), false) else {
            panic!("an own yard with no device is down")
        };
        assert_eq!(
            own_why,
            "Nothing of a yard is under /user/ at this address: the yard has no device any more, or the address is not a yard's listener. Pair this device again, with a code the yard's operator opens: yardr user device pair <user> <device>"
        );
        assert_eq!(short, PAIR_AGAIN);

        let edge = Router::start(&yard, SESSION);
        let Update::Down(member_why, short) =
            tell_first(edge.client_with_yard(TOKEN, SESSION, "alice/lab"), true)
        else {
            panic!("a member yard with no device is down")
        };
        assert_eq!(
            member_why,
            "Nothing of a yard is under /user/ at this address: the yard has no device any more, or the address is not a yard's listener. Pair this device again from the yard's card, in your own session."
        );
        assert_eq!(short, PAIR_AGAIN);
        assert!(!member_why.contains("operator"), "{member_why}");
        assert!(
            !member_why.contains("yardr user device pair"),
            "{member_why}"
        );
    }

    /// A yard that is slow to answer is said so, in plain words, by its
    /// socket and at its address alike: nothing is to be started, so the
    /// trouble stays the yard's on either.
    #[test]
    fn a_slow_yard_is_said_so() {
        for (idempotent, sent) in [(true, true), (false, true), (true, false), (false, false)] {
            let trouble = Trouble::Yard(api::Error::Slow { idempotent, sent });
            assert_eq!(
                trouble.to_string(),
                "The yard is slow to answer: it did not answer in time. The window tries again shortly."
            );
            assert!(!trouble.to_string().contains("os error"));
            let yard = Yard::start_tcp();
            assert!(matches!(
                trouble.with(&yard.client()),
                Trouble::Yard(api::Error::Slow { .. })
            ));
        }
    }

    /// The short why says what the person does where that is one thing,
    /// and never more than a row has room for.
    #[test]
    fn a_trouble_is_said_in_a_word_or_two() {
        let gone = || std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert_eq!(
            Trouble::Yard(api::Error::Down(gone())).short(),
            "nothing answers"
        );
        assert_eq!(Trouble::Unreached(gone()).short(), "nothing answers");
        assert_eq!(Trouble::NotLetIn.short(), "pair again");
        assert_eq!(Trouble::NotSignedIn.short(), "sign in again");
        assert_eq!(
            Trouble::Yard(api::Error::Gateway).short(),
            "gateway refused"
        );
        assert_eq!(Trouble::NoUser("jo".into()).short(), "no user here");
        assert_eq!(
            Trouble::Yard(api::Error::Protocol("x".into())).short(),
            "cannot be read"
        );
        for (idempotent, sent) in [(true, true), (false, true), (true, false), (false, false)] {
            assert_eq!(
                Trouble::Yard(api::Error::Slow { idempotent, sent }).short(),
                "slow to answer"
            );
        }
        // The edge's link: away or slow is slow, busy is busy.
        for (code, short) in [
            ("away", "slow to answer"),
            ("no_answer", "slow to answer"),
            ("busy", "busy"),
        ] {
            assert_eq!(
                Trouble::Yard(api::Error::Refused {
                    code: code.into(),
                    message: "the edge's words".into(),
                })
                .short(),
                short,
                "{code}"
            );
        }
    }

    /// The edge's away, slow and busy are said in the edge's words, and
    /// tried again: away or slow as a slow yard is, busy as busy. Never
    /// as what footplate cannot read.
    #[test]
    fn an_edge_that_is_away_slow_or_busy_is_said_in_its_words() {
        for (code, message, why) in [
            (
                "away",
                "the yard is away",
                "The yard is away. The window tries again shortly.",
            ),
            (
                "no_answer",
                "the yard gave no answer in time",
                "The yard gave no answer in time. The window tries again shortly.",
            ),
            (
                "busy",
                "the yard's link holds too many open calls and streams",
                "The yard is busy. The window tries again shortly.",
            ),
        ] {
            let trouble = Trouble::Yard(api::Error::Refused {
                code: code.into(),
                message: message.into(),
            });
            assert_eq!(trouble.to_string(), why, "{code}");
            assert!(!trouble.to_string().contains("cannot read"), "{code}");
        }
    }

    /// A failed state read waits the retry, doubled for each failure in a
    /// row, up to a minute: a yard that flaps is tried at the retry again,
    /// one that stays away about once a minute.
    #[test]
    fn a_failed_state_read_waits_longer_between_tries_up_to_a_minute() {
        let retry = Duration::from_secs(3);
        assert_eq!(backoff(retry, 1), Duration::from_secs(3));
        assert_eq!(backoff(retry, 2), Duration::from_secs(6));
        assert_eq!(backoff(retry, 3), Duration::from_secs(12));
        assert_eq!(backoff(retry, 5), Duration::from_secs(48));
        assert_eq!(backoff(retry, 6), Duration::from_secs(60));
        assert_eq!(backoff(retry, 60), Duration::from_secs(60));
        assert_eq!(backoff(retry, u32::MAX), Duration::from_secs(60));
        let quick = Duration::from_millis(20);
        assert_eq!(backoff(quick, 1), Duration::from_millis(20));
        assert_eq!(backoff(quick, 4), Duration::from_millis(160));
    }

    #[test]
    fn nothing_answering_at_an_address_is_said_as_it_is_there_and_tried_again() {
        let yard = Yard::start_tcp();
        let client = yard.client();
        yard.stop();
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn(move || {
            follow_with_control(
                &client,
                "jo",
                Duration::from_millis(20),
                WATCH,
                FollowControl::default(),
                false,
                |update| tx.send(update).is_ok(),
            )
        });
        let wait = Duration::from_secs(10);
        for _ in 0..2 {
            let Update::Down(why, short) = rx.recv_timeout(wait).unwrap() else {
                panic!("the yard is not there")
            };
            assert!(
                why.starts_with("No yard: nothing answers at the yard's address"),
                "{why}"
            );
            assert_eq!(short, NOTHING_ANSWERS);
            assert!(!why.contains("socket"), "{why}");
        }
        // And waiting longer between tries: in half a second a yard that
        // answers nothing is asked a handful of times, not every retry.
        // The waits double from the retry, so five reads fall in 500 of
        // those milliseconds; six still pass, and a thread that is slow to
        // wake only asks less.
        let start = std::time::Instant::now();
        let mut downs = 2;
        while start.elapsed() < Duration::from_millis(500) {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Update::Down(..)) => downs += 1,
                Ok(update) => panic!("the yard is not there: {update:?}"),
                Err(_) => {}
            }
        }
        assert!(downs <= 6, "{downs}");
        drop(rx);
        follower.join().unwrap();

        // Replacing this yard while its next retry is backed off wakes the
        // follower at once instead of keeping its thread around for five
        // seconds.
        let yard = Yard::start_tcp();
        let client = yard.client();
        yard.stop();
        let control = FollowControl::default();
        let follower_control = control.clone();
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn(move || {
            follow_with_control(
                &client,
                "jo",
                Duration::from_secs(5),
                WATCH,
                follower_control,
                false,
                |update| tx.send(update).is_ok(),
            )
        });
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Update::Down(..)));
        let stopped = std::time::Instant::now();
        control.stop();
        follower.join().unwrap();
        assert!(stopped.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn no_yard_is_said_in_words_and_tried_again_until_it_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yard.sock");
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = Client::new(&socket);
            move || {
                follow_with_control(
                    &client,
                    "jo",
                    Duration::from_millis(20),
                    WATCH,
                    FollowControl::default(),
                    false,
                    |update| tx.send(update).is_ok(),
                )
            }
        });
        let wait = Duration::from_secs(10);

        // The socket is missing.
        let Update::Down(why, short) = rx.recv_timeout(wait).unwrap() else {
            panic!("the yard is not there")
        };
        assert!(
            why.starts_with("No yard: nothing answers at the yard's socket"),
            "{why}"
        );
        assert_eq!(short, NOTHING_ANSWERS);
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Update::Down(..)));

        // The yard starts: the state, then the stream after its head.
        let yard = Yard::start_at(&socket);
        a_yard_of_jo(&yard);
        yard.emit(r#"{"seq": 20, "kind": "mail", "actor": "jo", "data": {"to": "yardmaster", "text": "old"}}"#);
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "to_user": true, "text": "new"}}"#);
        let state = loop {
            match rx.recv_timeout(wait).unwrap() {
                // A call may have come between two answers being set.
                Update::Down(..) => continue,
                Update::Up(state) => break state,
                Update::Event(e) => panic!("an event before the state: {e:?}"),
                Update::Out(why) | Update::SignedOut(why) => {
                    panic!("the socket lets everyone in: {why}")
                }
                Update::Removed => panic!("no member yard here"),
            }
        };
        assert_eq!((state.me.as_str(), state.head), ("jo", 20));
        let Update::Event(event) = rx.recv_timeout(wait).unwrap() else {
            panic!("the event after the head")
        };
        assert_eq!((event.seq, event.text("text")), (21, "new"));

        // The server goes down: the window is told, and the yard tried again.
        yard.stop();
        let why = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::Down(why, _) => break why,
                // The state may be read once more as the stream ends.
                Update::Up(_) | Update::Event(_) => continue,
                Update::Out(why) | Update::SignedOut(why) => {
                    panic!("the socket lets everyone in: {why}")
                }
                Update::Removed => panic!("no member yard here"),
            }
        };
        assert!(why.starts_with("No yard"), "{why}");

        // And it is back.
        let yard = Yard::start_at(&socket);
        a_yard_of_jo(&yard);
        loop {
            if let Update::Up(state) = rx.recv_timeout(wait).unwrap() {
                assert_eq!(state.me, "jo");
                break;
            }
        }

        // The window closes: the thread ends.
        drop(rx);
        yard.emit(r#"{"seq": 22, "kind": "mail"}"#);
        follower.join().unwrap();
    }
}
