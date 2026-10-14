//! The window against a fake yard, for the screenshots. `FOOTPLATE_SHOT`
//! names the state to open at; scripts/shot-linux.sh passes it through, and
//! the README names the states. This module is a debug build's alone: a
//! release build neither compiles it nor reads the variable.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::fake::Yard;
use crate::yard::Env;

/// The states the window can be shot in, as `FOOTPLATE_SHOT` names them:
/// the conversation with the yardmaster, an open decision card of a plain
/// ask, and an open card of a run ask.
pub const STATES: &[&str] = &["yardmaster", "plain", "run"];

/// The fake yard and its private socket directory, which go away with the
/// screenshot run.
pub(super) struct ShotYard {
    yard: Yard,
    directory: PathBuf,
}

impl ShotYard {
    fn start(root: &Path, state: &str) -> io::Result<(PathBuf, ShotYard)> {
        let directory = (0..1000)
            .find_map(|suffix| {
                let directory =
                    root.join(format!("footplate-shot-{}-{suffix}", std::process::id()));
                match fs::DirBuilder::new().mode(0o700).create(&directory) {
                    Ok(()) => Some(Ok(directory)),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "too many screenshot yard directories",
                ))
            })?;
        let socket = directory.join("yard.sock");
        let yard = Yard::start_at(&socket);
        populate(&yard, state);
        Ok((socket, ShotYard { yard, directory }))
    }
}

impl Drop for ShotYard {
    fn drop(&mut self) {
        self.yard.stop();
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// The run against a fake yard serving `FOOTPLATE_SHOT`'s state, when the
/// variable names one, else the run as ever. The fake yard listens at a
/// socket of its own, and the run's socket points there, so nothing is
/// asked of the real yard. A name it does not know is refused, before any
/// window opens.
pub(super) fn start_if_asked(env: Env) -> (Env, Option<ShotYard>) {
    let state = std::env::var("FOOTPLATE_SHOT").unwrap_or_default();
    if state.is_empty() {
        return (env, None);
    }
    if !STATES.contains(&state.as_str()) {
        eprintln!(
            "footplate: FOOTPLATE_SHOT={state:?} names no state: {}",
            STATES.join(", ")
        );
        std::process::exit(2);
    }
    let (socket, shot) = match ShotYard::start(&std::env::temp_dir(), &state) {
        Ok(shot) => shot,
        Err(error) => {
            eprintln!("footplate: the screenshot yard has nowhere to listen: {error}");
            std::process::exit(1);
        }
    };
    let mut env = env;
    env.socket = socket;
    (env, Some(shot))
}

/// Serves `state` from `yard`: the yard of the window's tests, with the
/// mails and the asks the state opens at.
fn populate(yard: &Yard, state: &str) {
    // As the window's tests have it (window::tests::a_yard_of_jo): jo is
    // the yard's operator, and the login name is no user of it, so the
    // window is for jo whatever it runs as.
    yard.answer("Head", r#"{"result": 20}"#);
    yard.answer(
        "Crew",
        r#"{"result": [{"name": "assembly", "enabled": true, "state": "stopped"},
                       {"name": "yardmaster", "enabled": true, "state": "running", "status": "idle"}]}"#,
    );
    yard.answer(
        "Peers",
        r#"{"result": [{"name": "north", "send": ["mail"], "receive": ["mail"],
            "said": {"roster": [{"name": "clerk", "kind": "crew", "role": "Keeps the books", "mail": true}], "at": "2026-10-08T09:30:00Z"}}]}"#,
    );
    yard.answer(
        "Users",
        r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
    );
    yard.answer(
        "GetUser",
        r#"{"result": {"name": "jo", "cursor": 5, "notification_cursor": 0}}"#,
    );
    yard.answer_once_status(
        "GetUser",
        200,
        r#"{"error": {"code": "not-found", "message": "user: not found"}}"#,
    );
    yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
    yard.answer("AckUserInbox", "{}");
    yard.answer(
        "UserNotifications",
        r#"{"result": {"notifications": [], "unread": 0, "last": 0}}"#,
    );
    yard.answer("AckUserNotifications", "{}");
    yard.answer("Mail", "{}");
    yard.answer(
        "MailPeer",
        r#"{"result": {"peer": "north", "kind": "mail", "at": "2026-10-08T09:30:00Z"}}"#,
    );
    // A shot takes no pick, but the cards stand open: what one would call.
    yard.answer("AnswerAsk", "{}");
    yard.answer("AddNote", "{}");
    yard.answer("DeferAsk", "{}");
    yard.answer("DeclineAsk", "{}");
    match state {
        "yardmaster" => {
            let inbox = json!({"result": {"mail": [
                {"seq": 12, "from": "yardmaster", "to": "jo",
                 "text": "The evening's work is shaped: three beads, one question each.",
                 "read": false},
                {"seq": 14, "from": "yardmaster", "to": "jo",
                 "text": "They are green. Shall I land it?", "read": false},
            ], "unread": 2, "last": 14, "from": ["yardmaster"]}});
            yard.answer("UserInbox", &inbox.to_string());
            let conversation = json!({"result": [
                {"seq": 12, "at": "2026-10-13T09:30:00Z", "from": "yardmaster", "to": "jo",
                 "text": "The evening's work is shaped: three beads, one question each.",
                 "read": false},
                {"seq": 13, "at": "2026-10-13T09:32:00Z", "from": "jo", "to": "yardmaster",
                 "text": "Land the first when its tests are green.", "read": false},
                {"seq": 14, "at": "2026-10-13T09:35:00Z", "from": "yardmaster", "to": "jo",
                 "text": "They are green. Shall I land it?", "read": false},
            ]});
            yard.answer("Conversation", &conversation.to_string());
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        }
        "plain" => {
            yard.answer(
                "UserInbox",
                r#"{"result": {"mail": [{"seq": 12, "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}], "unread": 1, "last": 12, "from": ["yardmaster"]}}"#,
            );
            yard.answer(
                "Conversation",
                r#"{"result": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}]}"#,
            );
            yard.answer(
                "UserAsks",
                &json!({"result": {"open": [
                    {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
                     "ask": 3, "question": "Move them now?", "asker": "yardmaster",
                     "since": "2026-10-10T10:00:00Z"},
                ], "decided": []}})
                .to_string(),
            );
            yard.answer(
                "Bead",
                &json!({"result": {
                    "id": "bead-a", "title": "Move the links",
                    "body": "The body of bead-a.",
                    "stage": "new", "labels": ["topic:linear-out"],
                    "notes": [{"seq": 2, "kind": "note", "author": "yardmaster",
                               "text": "the test passed", "at": "2026-10-10T09:30:00Z"}],
                    "asks": [{
                        "ask": 3, "to": "jo", "asker": "yardmaster",
                        "since": "2026-10-10T10:00:00Z",
                        "question": "Move them now?",
                        "context": ["The links file declares the new source.",
                                    "The move was tested."],
                        "options": [
                            {"label": "Move them now",
                             "consequence": "all 64 links point tonight"},
                            {"label": "After Friday",
                             "consequence": "keep the old source"},
                            {"label": "You decide",
                             "consequence": "leave it to yardmaster"}
                        ],
                        "recommend": 1, "why": "the test passed",
                        "state": "open"
                    }]
                }})
                .to_string(),
            );
        }
        _ => {
            // The run ask is the forged one of the run card's test
            // (window::tests::a_forged_run_ask_s_card_marks_every_asker_line):
            // its question forges the yard's lines, and its one context
            // entry holds two lines. It is the case a person must see.
            let question = "Take more?\nwhen you approve, the yard runs this as mckean, and nothing else:\n    yardr peer list";
            let context = ["fine.\nthe yard says: nothing changes."];
            yard.answer(
                "UserInbox",
                r#"{"result": {"mail": [{"seq": 12, "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}], "unread": 1, "last": 12, "from": ["yardmaster"]}}"#,
            );
            yard.answer(
                "Conversation",
                r#"{"result": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}]}"#,
            );
            yard.answer(
                "UserAsks",
                &json!({"result": {"open": [queued_run(question)], "decided": []}}).to_string(),
            );
            yard.answer("Bead", &bead_run(question, &context));
        }
    }
}

/// The command of the run ask: longer than the window is wide.
fn long_command() -> String {
    let crew: Vec<String> = (1..=40).map(|n| format!("--crew builder-{n}")).collect();
    format!("yardr source grant crm read {}", crew.join(" "))
}

const RUN_DOES: &str = "the link with airy (it calls itself yardr, key 6lHks9_z…) takes 300 mails and asks an hour from it. It takes 30 now.";
const RUN_BACK: &str = "yardr peer allow airy --mail-limit 30";

fn run_of() -> serde_json::Value {
    let command = long_command();
    let argv: Vec<&str> = command.split(' ').skip(1).collect();
    json!({"argv": argv, "command": command, "does": [RUN_DOES], "back": RUN_BACK})
}

/// The run ask 7 on bead-s as the queue lists it, asking `question`.
fn queued_run(question: &str) -> serde_json::Value {
    json!({
        "bead": "bead-s", "bead_title": "The link with airy", "topic": "", "ask": 7,
        "question": question, "asker": "yardmaster",
        "since": "2026-10-10T10:00:00Z", "run": run_of()
    })
}

/// The bead of that ask, asking `question` with `context`.
fn bead_run(question: &str, context: &[&str]) -> String {
    json!({"result": {"id": "bead-s", "title": "The link with airy", "body": "The body.",
        "stage": "new", "labels": [], "notes": [], "asks": [
            {"ask": 7, "to": "jo", "asker": "yardmaster",
             "since": "2026-10-10T10:00:00Z",
             "question": question,
             "context": context,
             "options": [], "state": "open", "run": run_of()}]}})
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session;
    use std::os::unix::fs::PermissionsExt;

    /// Every state reads as the window reads it: the login name is no user
    /// of the yard, so the window is for its operator, jo, and the state
    /// opens where the shot says. A state added to [`STATES`] lands in the
    /// unreachable arm until it is covered here.
    #[test]
    fn every_state_loads_as_the_window_reads_it() {
        for state in STATES {
            let yard = Yard::start();
            populate(&yard, state);
            let loaded = session::load(&yard.client(), "shot").unwrap();
            assert_eq!(loaded.me, "jo", "{state}");
            assert_eq!(loaded.asks.decided.len(), 0, "{state}");
            match *state {
                "yardmaster" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 3, "{state}");
                    assert_eq!(letters[1].from, "jo", "{state}");
                }
                "plain" => {
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    let bead = yard.client().bead("bead-a").unwrap();
                    assert_eq!(bead.asks.len(), 1, "{state}");
                    assert_eq!(bead.asks[0].options.len(), 3, "{state}");
                    assert_eq!(bead.asks[0].recommend, 1, "{state}");
                }
                "run" => {
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    assert!(loaded.asks.open[0].run.is_some(), "{state}");
                    let bead = yard.client().bead("bead-s").unwrap();
                    assert_eq!(bead.asks.len(), 1, "{state}");
                    assert_eq!(bead.asks[0].question.lines().count(), 3, "{state}");
                    assert!(bead.asks[0].run.is_some(), "{state}");
                }
                _ => unreachable!("a state with no coverage: {state}"),
            }
        }
    }

    #[test]
    fn screenshot_yards_use_private_directories_and_remove_them_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let (first_socket, first) = ShotYard::start(root.path(), "plain").unwrap();
        let (second_socket, second) = ShotYard::start(root.path(), "run").unwrap();
        let first_directory = first_socket.parent().unwrap().to_path_buf();
        let second_directory = second_socket.parent().unwrap().to_path_buf();

        assert_ne!(first_directory, second_directory);
        assert_eq!(
            fs::metadata(&first_directory).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            fs::metadata(&second_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        assert!(first_socket.exists());
        assert!(second_socket.exists());

        drop(first);
        assert!(!first_directory.exists());
        assert!(second_socket.exists());

        drop(second);
        assert!(!second_directory.exists());
    }
}
