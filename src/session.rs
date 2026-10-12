//! Following a yard: who the window is for, the state it starts from, and
//! the stream from then on, on a thread of its own. When the yard is not
//! there, or goes away, it says so and tries again every few seconds.

use std::fmt;
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
    /// It is not there, or not for this person: why, in words.
    Down(String),
}

/// Why there is nothing to show.
#[derive(Debug)]
pub enum Trouble {
    Yard(api::Error),
    /// The name the window runs as is no user of the yard, and the yard
    /// names no operator that is one.
    NoUser(String),
}

impl From<api::Error> for Trouble {
    fn from(err: api::Error) -> Self {
        Trouble::Yard(err)
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
            Trouble::Yard(err) => {
                write!(f, "The yard is there, but footplate cannot read it: {err}")
            }
            Trouble::NoUser(name) => {
                write!(
                    f,
                    "{name} is no user of this yard, and the yard names no operator. The operator adds one: yardr user add {name}"
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
    let peers = client.peers()?;
    let user = who(client, name)?;
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
        peers,
        cursor: user.cursor,
        unread,
        notification_cursor: user.notification_cursor,
        notifications,
        asks,
    })
}

/// Follows the yard until `tell` answers false (the window is gone): the
/// state, then every event after it. When the stream ends the state is read
/// again, so nothing in between is lost; when that fails the window is told
/// why, and the yard is tried again after `retry`.
pub fn follow(client: &Client, name: &str, retry: Duration, mut tell: impl FnMut(Update) -> bool) {
    loop {
        let trouble = match load(client, name) {
            Ok(state) => {
                let head = state.head;
                if !tell(Update::Up(Box::new(state))) {
                    return;
                }
                let mut heard = false;
                if let Ok(events) = client.events(head) {
                    // An event that cannot be read ends the stream as its
                    // end does: the state is read again.
                    for event in events.map_while(Result::ok) {
                        heard = true;
                        if !tell(Update::Event(event)) {
                            return;
                        }
                    }
                }
                // A stream that ended: a server that replaces itself is
                // back in a moment, and the state is read again. One that
                // gave nothing before it ended is not asked again in a
                // loop without a pause.
                thread::sleep(if heard { REPLACING_PAUSE } else { retry });
                continue;
            }
            Err(trouble) => trouble.to_string(),
        };
        if !tell(Update::Down(trouble)) {
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

    #[test]
    fn no_yard_is_said_in_words_and_tried_again_until_it_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yard.sock");
        let (tx, rx) = mpsc::channel();
        let follower = thread::spawn({
            let client = Client::new(&socket);
            move || {
                follow(&client, "jo", Duration::from_millis(20), |update| {
                    tx.send(update).is_ok()
                })
            }
        });
        let wait = Duration::from_secs(10);

        // The socket is missing.
        let Update::Down(why) = rx.recv_timeout(wait).unwrap() else {
            panic!("the yard is not there")
        };
        assert!(
            why.starts_with("No yard: nothing answers at the yard's socket"),
            "{why}"
        );
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Update::Down(_)));

        // The yard starts: the state, then the stream after its head.
        let yard = Yard::start_at(&socket);
        a_yard_of_jo(&yard);
        yard.emit(r#"{"seq": 20, "kind": "mail", "actor": "jo", "data": {"to": "yardmaster", "text": "old"}}"#);
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "to_user": true, "text": "new"}}"#);
        let state = loop {
            match rx.recv_timeout(wait).unwrap() {
                // A call may have come between two answers being set.
                Update::Down(_) => continue,
                Update::Up(state) => break state,
                Update::Event(e) => panic!("an event before the state: {e:?}"),
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
                Update::Down(why) => break why,
                // The state may be read once more as the stream ends.
                Update::Up(_) | Update::Event(_) => continue,
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
