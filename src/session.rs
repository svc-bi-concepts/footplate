//! Following a yard: who the window is for, the state it starts from, and
//! the stream from then on, on a thread of its own. When the yard is not
//! there, or goes away, it says so and tries again every few seconds.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::api::{self, Client, Event};
use crate::model::State;

/// How long until the yard is tried again.
pub const RETRY: Duration = Duration::from_secs(3);

/// A stream that ends is opened again at once when the server only replaces
/// itself with a newer build; this is the moment between the two.
const REPLACING_PAUSE: Duration = Duration::from_millis(100);

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
}

#[derive(Default)]
struct Following {
    stopped: bool,
    stream: Option<api::EventStop>,
}

/// Stops a yard follower, including an event stream blocked waiting for the
/// next event. One is kept by the window for the currently open yard.
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
    /// Nothing answers at the yard's address.
    Unreached(std::io::Error),
}

impl From<api::Error> for Trouble {
    fn from(err: api::Error) -> Self {
        match err {
            api::Error::NotLetIn => Trouble::NotLetIn,
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
        yard,
    })
}

/// Follows the yard until `tell` answers false (the window is gone): the
/// state, then every event after it. When the stream ends the state is read
/// again, so nothing in between is lost; when that fails the window is told
/// why, and the yard is tried again after `retry`. A yard that does not let
/// the device in is not tried again: the window is told, and this returns.
/// The handle stops the current stream from another thread, so switching
/// yards does not leave an idle old yard's connection alive indefinitely.
pub fn follow_with_control(
    client: &Client,
    name: &str,
    retry: Duration,
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
                let mut heard = false;
                if let Ok(mut events) = client.events(head) {
                    control.follow_stream(events.stop_handle());
                    // An event that cannot be read ends the stream as its
                    // end does: the state is read again.
                    while !control.stopped() {
                        match events.next() {
                            Some(Ok(event)) => {
                                heard = true;
                                if !tell(Update::Event(event)) {
                                    control.stream_ended();
                                    return;
                                }
                            }
                            Some(Err(_)) | None => break,
                        }
                    }
                    control.stream_ended();
                }
                if control.stopped() {
                    return;
                }
                // A stream that ended: a server that replaces itself is
                // back in a moment, and the state is read again. One that
                // gave nothing before it ended is not asked again in a
                // loop without a pause.
                thread::sleep(if heard { REPLACING_PAUSE } else { retry });
                continue;
            }
            Err(Trouble::NotLetIn) => {
                tell(Update::Out(Trouble::NotLetIn.to_string()));
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
                follow_with_control(&client, "jo", Duration::from_secs(10), stop, |update| {
                    updates.send(update).is_ok()
                });
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
                Update::Out(why) => panic!("the socket lets everyone in: {why}"),
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
                Update::Out(why) => panic!("the socket lets everyone in: {why}"),
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
