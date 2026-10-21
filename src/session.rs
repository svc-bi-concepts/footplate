//! Following a yard: who the window is for, the state it starts from, and
//! the stream from then on, on a thread of its own. When the yard is not
//! there, or goes away, it says so and tries again every few seconds.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::api::{self, Arrivals, Client, Event, Flows};
use crate::model::{self, State};

/// How long until the yard is tried again.
pub const RETRY: Duration = Duration::from_secs(3);

/// A stream that ends is opened again at once when the server only replaces
/// itself with a newer build; this is the moment between the two.
const REPLACING_PAUSE: Duration = Duration::from_millis(100);

/// How a follower watches a stream that says nothing. The yard says
/// nothing on a quiet stream, and nor does a stream that a proxy or a
/// tunnel holds open after the yard behind it went away, while the yard
/// moves on. After `quiet` with nothing the yard is asked its head, on a
/// connection of its own. A head beyond the last event read gives that
/// event `late` more to come, since it may be on its way; after that the
/// stream is stuck, and the yard is read again as when a stream ends.
#[derive(Debug, Clone, Copy)]
pub struct Watch {
    pub quiet: Duration,
    pub late: Duration,
}

/// A stuck stream is replaced within about a minute and a few seconds.
pub const WATCH: Watch = Watch {
    quiet: Duration::from_secs(60),
    late: Duration::from_secs(5),
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
}

#[derive(Default)]
struct Following {
    stopped: bool,
    stream: Option<api::EventStop>,
}

/// Stops a yard follower, including an event stream blocked waiting for the
/// next event. The window keeps one for each yard it follows, which is every
/// yard of its settings.
#[derive(Clone, Default)]
pub struct FollowControl(Arc<Mutex<Following>>);

impl FollowControl {
    pub fn stop(&self) {
        let stream = {
            let mut following = self.0.lock().unwrap();
            following.stopped = true;
            following.stream.take()
        };
        if let Some(stream) = stream {
            stream.stop();
        }
    }

    fn stopped(&self) -> bool {
        self.0.lock().unwrap().stopped
    }

    fn follow_stream(&self, stream: api::EventStop) {
        let stop = {
            let mut following = self.0.lock().unwrap();
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
        self.0.lock().unwrap().stream = None;
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
    /// The yard at an address answered 401: the device was removed, or its
    /// token is not that yard's. Trying again changes nothing.
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
            Trouble::Yard(_) => "cannot be read",
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
    // grants row; a yard from before the call has no grants to serve.
    let operator = client.settings().map(|s| s.operator).unwrap_or_default();
    let is_operator = !operator.is_empty() && operator == user.name;
    let (grants, grants_notice, grants_too_old) = match is_operator {
        false => (Vec::new(), None, false),
        true => match client.source_grants() {
            Ok(grants) => (grants, None, false),
            Err(err) if model::grants_too_old(&err) => (Vec::new(), None, true),
            Err(err) => (
                Vec::new(),
                Some(format!("the grants could not be read: {err}")),
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
        yard,
    })
}

/// Follows the yard until `tell` answers false (the window is gone): the
/// state, then every event after it. When the stream ends the state is read
/// again, so nothing in between is lost; when that fails the window is told
/// why, and the yard is tried again after `retry`. A yard that does not let
/// the device in, or the edge that does not let the session in, is not
/// tried again: the window is told, and this returns.
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
    mut tell: impl FnMut(Update) -> bool,
) {
    loop {
        if control.stopped() {
            return;
        }
        let trouble = match load(client, name) {
            Ok(state) => {
                let head = state.head;
                if !tell(Update::Up(Box::new(state))) {
                    return;
                }
                if control.stopped() {
                    return;
                }
                // Whether the stream brought anything, or was found stuck
                // while the yard answered beside it.
                let mut heard = false;
                if let Ok(mut events) = client.events(head, watch.quiet) {
                    control.follow_stream(events.stop_handle());
                    // The last seq read, and whether the yard's head was
                    // beyond it when the stream last said nothing.
                    let (mut read, mut behind) = (head, false);
                    // An event that cannot be read ends the stream as its
                    // end does: the state is read again. So does a stuck
                    // one, and one whose yard does not say its head.
                    while !control.stopped() {
                        match events.next() {
                            Some(Ok(event)) => {
                                heard = true;
                                read = read.max(event.seq);
                                if !tell(Update::Event(event)) {
                                    control.stream_ended();
                                    return;
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
                                    if events.set_quiet(watch.late).is_err() {
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
                            Some(Err(_)) | None => break,
                        }
                    }
                    control.stream_ended();
                }
                if control.stopped() {
                    return;
                }
                // A stream that ended: a server that replaces itself is
                // back in a moment, and the state is read again, as it is
                // at once of a yard that answered beside a stuck stream.
                // One that gave nothing before it ended is not asked again
                // in a loop without a pause.
                thread::sleep(if heard { REPLACING_PAUSE } else { retry });
                continue;
            }
            Err(Trouble::NotLetIn) => {
                tell(Update::Out(Trouble::NotLetIn.to_string()));
                return;
            }
            Err(Trouble::NotSignedIn) => {
                tell(Update::SignedOut(Trouble::NotSignedIn.to_string()));
                return;
            }
            Err(trouble) => trouble.with(client),
        };
        if !tell(Update::Down(trouble.to_string(), trouble.short())) {
            return;
        }
        if control.stopped() {
            return;
        }
        thread::sleep(retry);
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
    /// empty with its pane saying so, and an unreadable one with why.
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

        // An unreadable one: empty, with why, and not too old.
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
    }

    /// Anywhere but the operator's, the window reads no grants: not for
    /// another user, not where the yard names no operator, and not where
    /// its settings could not be read. The window still opens.
    #[test]
    fn load_reads_no_grants_where_the_window_is_not_the_operator_s() {
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
    }

    /// A yard at an address: the window is for the user its token is of,
    /// nothing answering is said as it is there, and a device that is
    /// removed is told once and never asked after again.
    #[test]
    fn a_device_that_is_removed_is_told_so_and_the_yard_is_not_asked_again() {
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
            }
        };
        assert!(why.starts_with("This device is no longer let in"), "{why}");
        assert!(why.contains("yardr user device pair"), "{why}");
        // The thread has ended with the window still there, and many of its
        // 20 milliseconds later nothing more was asked.
        follower.join().unwrap();
        let asked = yard.requests();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(yard.requests(), asked);
        assert!(rx.try_recv().is_err());
    }

    /// Through the edge: a session that is revoked is told so and never
    /// asked after again, with signing in again as what to do, while the
    /// device pairing stands as it was.
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

        // The session is revoked at the edge: what it relayed is cut, and
        // the next request is answered with the edge's own 401.
        edge.revoke(SESSION);
        let why = loop {
            match rx.recv_timeout(wait).unwrap() {
                Update::SignedOut(why) => break why,
                Update::Out(why) => panic!("the device is still paired: {why}"),
                Update::Down(why, _) => panic!("it is there, and says no: {why}"),
                Update::Up(_) | Update::Event(_) => continue,
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

    #[test]
    fn stopping_a_follower_closes_its_idle_stream() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
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
                    |update| updates.send(update).is_ok(),
                );
                finished.send(()).unwrap();
            }
        });
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            Update::Up(_)
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while yard.streams_asked().is_empty() && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(!yard.streams_asked().is_empty(), "the event stream opened");

        cancel.stop();

        done.recv_timeout(Duration::from_secs(1))
            .expect("stopping the follower ends its blocked read");
        follower.join().unwrap();
    }

    /// A follower of `yard` for jo, trying again after 20 milliseconds and
    /// watching its stream as `watch` says, once its state is told and its
    /// stream opened: what it tells from then on.
    fn following(yard: &Yard, watch: Watch) -> mpsc::Receiver<Update> {
        let (tx, rx) = mpsc::channel();
        let client = yard.client();
        thread::spawn(move || {
            let retry = Duration::from_millis(20);
            follow_with_control(&client, "jo", retry, watch, FollowControl::default(), |u| {
                tx.send(u).is_ok()
            })
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

    /// A stream that stays open and brings nothing while the yard moves
    /// on is replaced: the yard is read again, as when a stream ends, and
    /// what happened meanwhile is in what it reads.
    #[test]
    fn a_stream_that_says_nothing_while_the_yard_moves_on_is_replaced() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let watch = Watch {
            quiet: Duration::from_millis(100),
            late: Duration::from_millis(100),
        };
        let rx = following(&yard, watch);
        yard.mute();
        yard.answer("Head", r#"{"result": 21}"#);
        yard.emit(r#"{"seq": 21, "kind": "noted", "actor": "yardmaster", "bead": "yd-shed", "data": {"note": 7}}"#);
        let Update::Up(state) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the yard read again, and no event before it")
        };
        assert_eq!(state.head, 21);
        // The stream after it is opened from the head read again.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while yard.streams_asked().len() < 2 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(yard.streams_asked(), vec![20, 21]);
    }

    /// A stream that says nothing while the yard says nothing either is
    /// read on: the yard is asked its head, and the stream stays.
    #[test]
    fn a_quiet_stream_of_a_quiet_yard_is_read_on() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let watch = Watch {
            quiet: Duration::from_millis(50),
            late: Duration::from_millis(50),
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
    /// more to come: one that comes in it leaves the stream as it is.
    #[test]
    fn an_event_on_its_way_is_given_a_moment_to_come() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let watch = Watch {
            quiet: Duration::from_millis(100),
            late: Duration::from_secs(5),
        };
        let rx = following(&yard, watch);
        yard.answer("Head", r#"{"result": 21}"#);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while yard.asked("Head").len() < 2 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "text": "new"}}"#);
        let Update::Event(event) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("the event on its way")
        };
        assert_eq!(event.seq, 21);
        // Read level with the head again: the stream stays past several
        // more of its silences.
        let asked = yard.asked("Head").len();
        while yard.asked("Head").len() < asked + 3 && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(yard.streams_asked(), vec![20]);
        assert!(rx.try_recv().is_err());
    }

    /// The yard's last device removed: it answers 404 then, to a token as
    /// to none. That is said as what it is, and asked again, since an
    /// address that is no yard's answers the same.
    #[test]
    fn a_yard_with_no_device_left_is_said_so() {
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
        assert_eq!(Trouble::NoUser("jo".into()).short(), "no user here");
        assert_eq!(
            Trouble::Yard(api::Error::Protocol("x".into())).short(),
            "cannot be read"
        );
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
        drop(rx);
        follower.join().unwrap();
    }

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
