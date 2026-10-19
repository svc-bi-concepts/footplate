//! The window against a fake yard, for the screenshots. `FOOTPLATE_SHOT`
//! names the state to open at; scripts/shot-linux.sh passes it through, and
//! the README names the states. This module is a debug build's alone: a
//! release build neither compiles it nor reads the variable.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local};
use serde_json::json;

use crate::fake::{Router, Yard};
use crate::yard::Env;

/// The states the window can be shot in, as `FOOTPLATE_SHOT` names them:
/// the conversation with the yardmaster, chat with the sidebar's groups
/// full: crew working and stopped, a linked yard's people, this yard's
/// people, and unread notices, a long wrapped thread with an
/// image, a long conversation over three days, grouped by sender, with
/// short links and a correction, an open decision card of a plain
/// ask, that card with one of jo's beads changed, so Lines carries its
/// badge, an open card of a run ask,
/// a yard that is stopped, a yard at an
/// address, the settings on each of their sections, a running
/// yard whose server is then stopped, shot reconnecting with a draft in
/// the composer, an open card whose note carries a page, an image and a file, a
/// mail with a page and an image, that mail's image open full size over
/// the window, the first screen's way to a yard at an address, with a
/// pairing's http address refused under its fields, the
/// conversation with one of a
/// linked yard whose mail was refused over the hour, the first screen
/// with a way's fields open after the yard answered, with the line that
/// esc opens it, step 2 of setting up a yard here, posed with a
/// canned doctor of two providers and their model offers, the first
/// screen with the hosted way's fields open and filled, and the away
/// screen of a hosted yard that is signed out, the away screen of a
/// yard at an address that no longer lets the device in, with a way's
/// fields open, the arrivals panel open
/// on the arrivals row with its lamps, headcodes and quiet lines, the
/// departures board open under the toggle, one
/// arrival's bead open from the panel, one
/// line's flow open on its row, the Lines pane against a yard too old
/// to serve it, saying so in plain words, an open card of a secret ask posed
/// with a value typed, hidden and shown, that card with the yard's
/// refusal of its answer, the switcher's list open over four yards, one
/// of them away, the yardmaster's conversation naming an ask and two
/// bead ids, the cheatsheet open over an open card, and the
/// yardmaster's conversation with the parties' reactions under two
/// mails, with the row of reactions to tap under one of them.
/// The stopped yard answers nothing, as the refused one answers nowhere:
/// both open on a yard that is down, as the posed setup does. The yard at
/// an address is opened from the settings the shot script writes; the
/// signed-out yard from settings the state writes itself, against a
/// throwaway config, with a session the edge no longer knows; the away
/// yard from settings the state writes itself, against a throwaway
/// config, with a device the yard no longer knows; the
/// switcher's four yards from settings the state writes itself, with
/// adventuretime open, this machine second, plantime live at its listener,
/// and droplet away; the
/// settings open over a running yard, each state on its section;
/// the page's card stands expanded at
/// its end, with the web view taken as answering, so the row carries Open
/// and run. The mail's conversation opens on the mail, with the web view
/// taken as answering, so the row carries Open; a click on its image
/// opens it over the window, and the image state serves the same mail
/// with it open. The
/// undelivered conversation opens on the row the shot names,
/// with the hour's line under the mail. The answered yard serves the
/// yardmaster's conversation, as the running ones do; the window opens
/// with the way's fields standing, so the yard answers under them. The
/// script answers the switch's yardr for every state, so the status row
/// draws the same on every shot.
pub const STATES: &[&str] = &[
    "yardmaster",
    "crew",
    "thread",
    "column",
    "plain",
    "lines",
    "run",
    "stopped",
    "far",
    "settings",
    "settings-appearance",
    "settings-yards",
    "settings-agents",
    "settings-notifications",
    "settings-connection",
    "reconnecting",
    "page",
    "mail",
    "image",
    "refusal",
    "undelivered",
    "answered",
    "setup",
    "hosted",
    "signedout",
    "away",
    "arrivals",
    "departures",
    "arrival",
    "flow",
    "old-lines",
    "secret",
    "secret-shown",
    "secret-refused",
    "switcher",
    "refs",
    "decisions",
    "decisions-filtered",
    "keys",
    "reactions",
    "reactions-hover",
];

/// The image in the `page`, `mail` and `image` states, and its hash: the
/// yard serves the bytes, and the note or the mail names them.
const IMAGE: &[u8] = include_bytes!("fixtures/shot-page.png");
const IMAGE_HASH: &str = "ca2421063b23c6ff5739c2000b22125bea109764fff6e643eccea0c00f6fe6c3";

/// The overlay the `image` state opens with: the hash it shows, and its
/// bytes for the line while they are not there yet.
pub(crate) const IMAGE_OPEN: (&str, u64) = (IMAGE_HASH, IMAGE.len() as u64);

/// The pages in the `page` and `mail` states: the note's, which Open
/// takes to the browser, and a mail's, which Open takes there too, with
/// no run. Named, never served: the yard is asked for neither.
const PAGE_HASH: &str = "abababababababababababababababababababababababababababababababab";
const MAIL_PAGE_HASH: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

/// The file in the `page` state: the note's third row, under its name,
/// which Open takes to the browser. Named, never served, as the pages.
const FILE_HASH: &str = "efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef";

/// The fake yard and its private socket directory, which go away with the
/// screenshot run.
pub(super) struct ShotYard {
    yard: std::sync::Arc<Yard>,
    directory: PathBuf,
    /// The hosted yard and the edge in front of it, for `signedout` alone:
    /// the window opens the hosted yard through the edge, whose session
    /// is none the edge knows.
    edge: Option<(Yard, Router)>,
    /// The switcher's other live yards, for `switcher` alone: adventuretime
    /// and plantime at their listeners, held from the start beside this
    /// machine.
    extra: Vec<std::sync::Arc<Yard>>,
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
        Ok((
            socket,
            ShotYard {
                yard: std::sync::Arc::new(yard),
                directory,
                edge: None,
                extra: Vec::new(),
            },
        ))
    }
}

impl Drop for ShotYard {
    fn drop(&mut self) {
        self.yard.stop();
        for extra in &self.extra {
            extra.stop();
        }
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
    start_for_state(env, &state)
}

/// The run for `state`: the fake yard serves it, and the run's socket
/// points there. For `stopped`, `refusal`, `setup` and `hosted` no yard
/// listens: the socket names a directory that holds none, so the window
/// opens on a yard that is down, and the switch says the stopped one is
/// stopped. The refusal shot is the first screen, with the pairing's
/// fields open on it; the setup shot is the first run posed at step 2;
/// the hosted shot is the first screen with the hosted way's fields open
/// on it. The signedout shot opens the hosted yard south through an edge
/// that no longer knows its session, so the window opens on the away
/// screen of a yard that is signed out. The away shot opens the yard
/// far at its address with a device the yard no longer knows, so the
/// window opens on the away screen that offers to pair again, with a
/// way's fields open.
fn start_for_state(env: Env, state: &str) -> (Env, Option<ShotYard>) {
    if state == "stopped" || state == "refusal" || state == "setup" || state == "hosted" {
        let mut env = env;
        env.socket = std::env::temp_dir().join(format!(
            "footplate-shot-{}-{state}/yard.sock",
            std::process::id()
        ));
        return (env, None);
    }
    let (socket, mut shot) = match ShotYard::start(&std::env::temp_dir(), state) {
        Ok(shot) => shot,
        Err(error) => {
            eprintln!("footplate: the screenshot yard has nowhere to listen: {error}");
            std::process::exit(1);
        }
    };
    if state == "signedout" {
        // The window opens south through the edge, as the sign-in-again
        // test has it: the session stands in its file, and the edge knew
        // it until it was revoked there.
        let hosted = Yard::start_tcp();
        populate(&hosted, state);
        let edge = Router::start(&hosted, crate::fake::SESSION);
        edge.revoke(crate::fake::SESSION);
        write_signedout_settings(edge.address());
        shot.edge = Some((hosted, edge));
    }
    if state == "away" {
        // The window opens far at its address, as the pair-again test
        // has it: the token stands in its file, and the yard knew the
        // device until its operator removed it there. The yard is held
        // beside the shot's, so its listener stands for the shot.
        let far = Yard::start_tcp();
        populate(&far, state);
        far.add_device("footplate-shot-phone");
        far.remove_device(crate::fake::TOKEN);
        write_away_settings(far.address());
        shot.extra = vec![std::sync::Arc::new(far)];
    }
    if state == "switcher" {
        // The switcher's other live yards, held from the start: adventuretime
        // with two mails and one decision, plantime with twelve mails, so
        // the counts stand one and two figures in their column.
        let adventuretime = Yard::start_tcp();
        switcher_adventuretime(&adventuretime);
        let plantime = Yard::start_tcp();
        switcher_plantime(&plantime);
        write_switcher_settings(adventuretime.address(), plantime.address());
        shot.extra = vec![
            std::sync::Arc::new(adventuretime),
            std::sync::Arc::new(plantime),
        ];
    }
    if state == "reconnecting" {
        // The yard answers, the window reads it, and then its server is
        // stopped: the shot is the reconnecting state. The stop waits for
        // the window's read of the open conversation, and a breath after
        // it for the reply to land, so the shot is the conversation as
        // last seen; past half a minute it stops anyway, and the shot is
        // whatever that is. The event before the stop only marks the
        // stream as heard, so the window notices the stop at once: mail
        // read elsewhere, by nobody shown, changes nothing.
        let yard = shot.yard.clone();
        std::thread::spawn(move || {
            for _ in 0..600 {
                if !yard.asked("Conversation").is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            yard.emit(
                r#"{"seq": 99, "kind": "mail_read", "data": {"reader": "nobody", "through": 0}}"#,
            );
            std::thread::sleep(std::time::Duration::from_millis(300));
            yard.stop();
        });
    }
    let mut env = env;
    env.socket = socket;
    (env, Some(shot))
}

/// The settings the `signedout` shot opens: the hosted yard south at the
/// edge's address, with the account it was signed in as, its device token
/// and the session in files beside them, as the sign-in-again test keeps
/// them. The shot script runs against a throwaway config, which keeps no
/// settings; where a settings file already stands, the state is refused
/// rather than overwrite it.
fn write_signedout_settings(address: &str) {
    let Some(file) = crate::settings::file() else {
        eprintln!("footplate: the signedout shot keeps no settings without a home");
        std::process::exit(1);
    };
    if file.exists() {
        eprintln!(
            "footplate: the signedout shot writes its own settings, and {} already stands: shoot it against a throwaway config, as scripts/shot-linux.sh does",
            file.display()
        );
        std::process::exit(2);
    }
    let mut south =
        crate::settings::Yard::hosted("south", address, "yard-south.token", "yard-south.session");
    south.account = crate::fake::KRATOS_EMAIL.to_string();
    crate::settings::Settings::default()
        .with_yard(None, south, Some(&file))
        .and_then(|set| {
            set.opening("south")
                .write(&file)
                .map_err(|error| error.to_string())
        })
        .unwrap_or_else(|error| {
            eprintln!("footplate: the signedout shot keeps no settings: {error}");
            std::process::exit(1);
        });
    let edge = crate::api::Address::parse(address).unwrap_or_else(|error| {
        eprintln!("footplate: the signedout shot's edge has no address: {error}");
        std::process::exit(1);
    });
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    crate::yard::token_place(&dir.join("yard-south.token"))
        .and_then(|place| place.keep(crate::fake::TOKEN, &edge))
        .unwrap_or_else(|error| {
            eprintln!("footplate: the signedout shot keeps no token: {error}");
            std::process::exit(1);
        });
    crate::yard::keep_session(&dir.join("yard-south.session"), crate::fake::SESSION, &edge)
        .unwrap_or_else(|error| {
            eprintln!("footplate: the signedout shot keeps no session: {error}");
            std::process::exit(1);
        });
}

/// The settings the `away` shot opens: the yard far at its address,
/// open, with its device token in a file beside them, as the
/// pair-again test keeps them. The shot script runs against a
/// throwaway config, which keeps no settings; where a settings file
/// already stands, the state is refused rather than overwrite it.
fn write_away_settings(address: &str) {
    let Some(file) = crate::settings::file() else {
        eprintln!("footplate: the away shot keeps no settings without a home");
        std::process::exit(1);
    };
    if file.exists() {
        eprintln!(
            "footplate: the away shot writes its own settings, and {} already stands: shoot it against a throwaway config, as scripts/shot-linux.sh does",
            file.display()
        );
        std::process::exit(2);
    }
    crate::settings::Settings::default()
        .with_yard(
            None,
            crate::settings::Yard::far("far", address, "yard-far.token"),
            Some(&file),
        )
        .and_then(|set| {
            set.opening("far")
                .write(&file)
                .map_err(|error| error.to_string())
        })
        .unwrap_or_else(|error| {
            eprintln!("footplate: the away shot keeps no settings: {error}");
            std::process::exit(1);
        });
    let edge = crate::api::Address::parse(address).unwrap_or_else(|error| {
        eprintln!("footplate: the away shot's yard has no address: {error}");
        std::process::exit(1);
    });
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    crate::yard::token_place(&dir.join("yard-far.token"))
        .and_then(|place| place.keep(crate::fake::TOKEN, &edge))
        .unwrap_or_else(|error| {
            eprintln!("footplate: the away shot keeps no token: {error}");
            std::process::exit(1);
        });
}

/// The settings the `switcher` shot opens: four yards in the screenshot's
/// order, adventuretime at its listener and open, this machine at the
/// shot's socket, droplet away, then plantime at its listener. Droplet
/// keeps no token, so it is never reached: the window asks nothing of the
/// network for it. The shot script runs against a throwaway config, which
/// keeps no settings; where a settings file already stands, the state is
/// refused rather than overwrite it.
fn write_switcher_settings(adventuretime_at: &str, plantime_at: &str) {
    let Some(file) = crate::settings::file() else {
        eprintln!("footplate: the switcher shot keeps no settings without a home");
        std::process::exit(1);
    };
    if file.exists() {
        eprintln!(
            "footplate: the switcher shot writes its own settings, and {} already stands: shoot it against a throwaway config, as scripts/shot-linux.sh does",
            file.display()
        );
        std::process::exit(2);
    }
    let adventuretime = crate::settings::Yard::far(
        "adventuretime.tail5c8d.ts.net",
        adventuretime_at,
        "yard-adventuretime.token",
    );
    // This machine's yard is the shot's socket.
    let home = crate::settings::Yard::near(crate::settings::HERE, "");
    let plantime = crate::settings::Yard::far("plantime", plantime_at, "yard-plantime.token");
    let droplet =
        crate::settings::Yard::far("droplet", "https://droplet.example.net", "droplet.token");
    crate::settings::Settings {
        look: match std::env::var("SHOT_LOOK").as_deref() {
            Ok("light") => crate::theme::Look::Light,
            Ok("dark") => crate::theme::Look::Dark,
            _ => crate::theme::Look::Auto,
        },
        ..Default::default()
    }
    .with_yard(Some(crate::settings::HERE), adventuretime, Some(&file))
    .and_then(|set| set.with_yard(None, home, Some(&file)))
    .and_then(|set| set.with_yard(None, droplet, Some(&file)))
    .and_then(|set| set.with_yard(None, plantime, Some(&file)))
    .and_then(|set| {
        set.opening("adventuretime.tail5c8d.ts.net")
            .write(&file)
            .map_err(|error| error.to_string())
    })
    .unwrap_or_else(|error| {
        eprintln!("footplate: the switcher shot keeps no settings: {error}");
        std::process::exit(1);
    });
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    for (token, at) in [
        ("yard-adventuretime.token", adventuretime_at),
        ("yard-plantime.token", plantime_at),
    ] {
        let edge = crate::api::Address::parse(at).unwrap_or_else(|error| {
            eprintln!("footplate: the switcher shot's yard has no address: {error}");
            std::process::exit(1);
        });
        crate::yard::token_place(&dir.join(token))
            .and_then(|place| place.keep(crate::fake::TOKEN, &edge))
            .unwrap_or_else(|error| {
                eprintln!("footplate: the switcher shot keeps no token: {error}");
                std::process::exit(1);
            });
    }
}

/// The adventuretime yard of the `switcher` shot: jo's, with two mails
/// and one open decision, so its row carries 3 with
/// "1 decision · 2 mail" in its tooltip.
fn switcher_adventuretime(yard: &Yard) {
    populate(yard, "switcher");
    yard.answer(
        "UserInbox",
        r#"{"result": {"mail": [{"seq": 12, "from": "yardmaster", "to": "jo", "text": "From adventuretime.", "read": false},
            {"seq": 13, "from": "assembly", "to": "jo", "text": "Assembled.", "read": false}], "unread": 2, "last": 13, "from": ["yardmaster", "assembly"]}}"#,
    );
    yard.answer(
        "UserAsks",
        r#"{"result": {"open": [
            {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
             "ask": 3, "question": "Move them now?", "asker": "yardmaster",
             "since": "2026-10-10T10:00:00Z"}], "decided": []}}"#,
    );
}

/// The plantime yard of the `switcher` shot: jo's, with twelve mails, so
/// its row carries 12 beside adventuretime's 3: one and two figures in the
/// counts' column.
fn switcher_plantime(yard: &Yard) {
    populate(yard, "switcher");
    let mail: Vec<_> = (0..12)
        .map(|n| {
            json!({"seq": 40 + n, "from": "yardmaster", "to": "jo",
                   "text": format!("No. {} from plantime.", n + 1), "read": false})
        })
        .collect();
    yard.answer(
        "UserInbox",
        &json!({"result": {"mail": mail, "unread": 12, "last": 51, "from": ["yardmaster"]}})
            .to_string(),
    );
    yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
}

/// Doctor's answer for the `setup` shot: git and herdr installed, and
/// two installed providers with the labels and the model offers the
/// second step shows. The first run stands at that step with it, having
/// fetched and run nothing.
pub(crate) fn setup_doctor() -> crate::setup::Doctor {
    serde_json::from_value(json!({
        "yard": false,
        "machine": [
            {"program": "herdr", "for": "it starts every agent in a pane of its own", "found": true,
             "path": "/usr/bin/herdr", "server": {"state": "not_running", "output": "no server"}},
            {"program": "git", "for": "a depot is a git repository", "found": true, "path": "/usr/bin/git"},
            {"program": "claude", "for": "a harness a yard can be created on", "found": true,
             "path": "/usr/bin/claude", "harness": "claude", "harness_login": true,
             "provider": "Anthropic", "label": "Claude Code",
             "offers": [
                 {"model": "opus", "label": "Opus 5.5", "recommended": "yardmaster"},
                 {"model": "fable", "label": "Fable 5.1"},
                 {"model": "sonnet", "label": "Sonnet 5"}]},
            {"program": "codex", "for": "a harness a yard can be created on", "found": true,
             "path": "/usr/bin/codex", "harness": "codex",
             "provider": "OpenAI", "label": "Codex",
             "offers": [
                 {"model": "gpt-6-luna", "label": "GPT-6 Luna", "recommended": "yardmaster"},
                 {"model": "gpt-6-astra", "label": "GPT-6 Astra"}]}
        ],
        "faults": []
    }))
    .expect("the setup shot's doctor")
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
    // The arrivals board every state but old-lines loads: three rows
    // in served order, one with an amber remark, one with none, and one
    // with a red remark and no expected arrival. The window counts it
    // in the arrivals row; the departures state draws the rows flat in
    // this order. Old-lines serves neither call, so the yard answers
    // no such method to both, as a yardr from before the Lines view.
    if state != "old-lines" {
        yard.answer(
            "Arrivals",
            &json!({"result": {"rows": [
            {"bead": "yd-zulu", "title": "Zulu's late turn",
             "platform": "draft", "line": "yardr", "expected": "5h–8h",
             "remark": "late", "tone": "amber"},
            {"bead": "yd-alpha", "title": "Alpha moves first",
             "platform": "review", "line": "yardr", "expected": "20m–35m",
             "remark": "", "tone": "none"},
            {"bead": "footplate-p512", "title": "The sign-in's password label",
             "platform": "new", "line": "footplate", "expected": "—",
             "remark": "held", "tone": "red"},
        ], "platforms": ["new", "review", "draft"]}})
            .to_string(),
        );
        // The flows view every state but old-lines loads: the three arrivals
        // travelling, yardr's two on its default flow's turn and quiet stages,
        // footplate's one on a fault stage, each stage with its terminal
        // flag and its routed group's seats. The window counts the totals
        // in the timetable's rows.
        yard.answer(
            "Flows",
            &json!({"result": {
                "travelling": 3,
                "depots": [
                    {"name": "yardr", "kind": "git", "travelling": 2, "flows": [
                        {"name": "default", "type": "", "source": "store", "stages": [
                            {"stage": "new", "group": "muse-builders", "human": false,
                             "lamp": "turn", "terminal": false, "limit": 3, "taken": 2,
                             "beads": [
                                {"bead": "yd-zulu", "title": "Zulu's late turn",
                                 "expected": "5h–8h", "remark": "late",
                                 "tone": "amber", "working": false}]},
                            {"stage": "review", "group": "mckean", "human": true,
                             "lamp": "none", "terminal": false, "limit": 1, "taken": 0,
                             "beads": [
                                {"bead": "yd-alpha", "title": "Alpha moves first",
                                 "expected": "20m–35m", "remark": "",
                                 "tone": "none", "working": false}]},
                        ]},
                    ], "elsewhere": []},
                    {"name": "footplate", "kind": "git", "travelling": 1, "flows": [
                        {"name": "default", "type": "", "source": "store", "stages": [
                            {"stage": "new", "group": "muse-builders", "human": false,
                             "lamp": "fault", "terminal": false, "limit": 2, "taken": 1,
                             "beads": [
                                {"bead": "footplate-p512",
                                 "title": "The sign-in's password label",
                                 "expected": "—", "remark": "held",
                                 "tone": "red", "working": false}]},
                        ]},
                    ], "elsewhere": []},
                ],
                "unassigned": [],
            }})
            .to_string(),
        );
    }
    match state {
        // The yard at an address is opened from the settings, not from
        // this yard, and the settings open over a running one, each
        // state on its section: both serve the yardmaster's
        // conversation, as does the yard that is stopped once it is
        // read, the one that answers under the first screen's open
        // fields, the signed-out yard's, which the window never reaches
        // past the edge, the timetable states', which the window
        // opens on the panel, the board, the bead and the flow, and the
        // switcher's, which the window opens with the switcher's list
        // standing.
        "yardmaster"
        | "far"
        | "settings"
        | "settings-appearance"
        | "settings-yards"
        | "settings-agents"
        | "settings-notifications"
        | "settings-connection"
        | "reconnecting"
        | "answered"
        | "signedout"
        | "away"
        | "arrivals"
        | "departures"
        | "arrival"
        | "flow"
        | "old-lines"
        | "switcher" => {
            // The conversation reads over two days, with a mail in each
            // sender's group: the yardmaster shot shows the column, the
            // grouping and the separators.
            let inbox = json!({"result": {"mail": [
                {"seq": 12, "from": "yardmaster", "to": "jo",
                 "text": "The evening's work is shaped: three beads, one question each.",
                 "read": false},
                {"seq": 14, "from": "yardmaster", "to": "jo",
                 "text": "They are green. Shall I land it?", "read": false},
                {"seq": 15, "from": "yardmaster", "to": "jo",
                 "text": "The second lands behind it.", "read": false},
            ], "unread": 3, "last": 15, "from": ["yardmaster"]}});
            yard.answer("UserInbox", &inbox.to_string());
            let conversation = json!({"result": [
                {"seq": 12, "at": "2026-10-13T09:30:00Z", "from": "yardmaster", "to": "jo",
                 "text": "The evening's work is shaped: three beads, one question each.",
                 "read": false},
                {"seq": 13, "at": "2026-10-13T09:32:00Z", "from": "jo", "to": "yardmaster",
                 "text": "Land the first when its tests are green.", "read": false},
                {"seq": 14, "at": "2026-10-13T09:35:00Z", "from": "yardmaster", "to": "jo",
                 "text": "They are green. Shall I land it?", "read": false},
                {"seq": 15, "at": "2026-10-13T09:37:00Z", "from": "yardmaster", "to": "jo",
                 "text": "The second lands behind it.", "read": false},
                {"seq": 16, "at": "2026-10-14T08:05:00Z", "from": "jo", "to": "yardmaster",
                 "text": "Both are landed.", "read": false},
                {"seq": 17, "at": "2026-10-14T08:09:00Z", "from": "jo", "to": "yardmaster",
                 "text": "What is next?", "read": false},
            ]});
            yard.answer("Conversation", &conversation.to_string());
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
            // One arrival's bead, which the arrival state opens: the
            // body, two labels, two notes, and three asks, two of them
            // open.
            if state == "arrival" {
                yard.answer("Bead", &bead_zulu());
            }
        }
        "crew" => {
            // Chat with the sidebar's groups full: the yardmaster
            // working, brakeman idle and assembly stopped, north's clerk
            // and warden with rue, jo and ada for people, and three
            // unread notices. The unread mail is elsewhere than the open
            // conversation, so Crew carries its badge, as Decide does for
            // the open ask and Lines for one of jo's beads added past
            // the head.
            yard.answer(
                "Crew",
                r#"{"result": [{"name": "assembly", "enabled": true, "state": "stopped"},
                               {"name": "yardmaster", "enabled": true, "state": "running", "status": "working"},
                               {"name": "brakeman", "enabled": true, "state": "running", "status": "idle"}]}"#,
            );
            yard.answer(
                "Peers",
                r#"{"result": [{"name": "north", "send": ["mail"], "receive": ["mail"],
                    "said": {"roster": [{"name": "clerk", "kind": "crew", "role": "Keeps the books", "mail": true},
                                        {"name": "warden", "kind": "crew", "role": "Watches the north gate", "mail": true},
                                        {"name": "rue", "kind": "user", "role": "Operates the north link", "mail": true}],
                             "at": "2026-10-08T09:30:00Z"}}]}"#,
            );
            yard.answer(
                "Users",
                r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0},
                               {"name": "ada", "cursor": 9, "notification_cursor": 4}]}"#,
            );
            yard.answer(
                "UserNotifications",
                &json!({"result": {"notifications": [
                    {"seq": 10, "kind": "created", "bead": "yd-barn",
                     "text": "Paint the barn was opened", "read": false},
                    {"seq": 11, "kind": "closed", "bead": "yd-shed",
                     "text": "Shed the tools was closed", "read": false},
                    {"seq": 12, "kind": "advanced", "bead": "yd-barn",
                     "text": "Paint the barn moved to review", "read": false},
                ], "unread": 3, "last": 12}})
                .to_string(),
            );
            yard.answer(
                "UserInbox",
                &json!({"result": {"mail": [
                    {"seq": 12, "from": "yardmaster", "to": "jo",
                     "text": "The evening's work is shaped: three beads, one question each.",
                     "read": false},
                    {"seq": 13, "from": "brakeman", "to": "jo",
                     "text": "The second lands behind it.", "read": false},
                    {"seq": 14, "from": "brakeman", "to": "jo",
                     "text": "Green across the board.", "read": false},
                    {"seq": 15, "from": "clerk@north", "to": "jo",
                     "text": "The books balance.", "read": false},
                ], "unread": 4, "last": 15,
                "from": ["yardmaster", "brakeman", "clerk@north"]}})
                .to_string(),
            );
            yard.answer(
                "Conversation",
                r#"{"result": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "The evening's work is shaped: three beads, one question each.", "read": false}]}"#,
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
            yard.emit(r#"{"seq": 21, "kind": "created", "actor": "yardmaster", "bead": "yd-barn", "data": {"title": "Paint the barn", "stage": "new", "type": "task", "originator": "jo"}}"#);
        }
        "plain" | "lines" | "keys" => {
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
            // The lines badge's bead: one of jo's added past the head,
            // so the rail's Lines carries its badge beside the others.
            if state == "lines" {
                yard.emit(r#"{"seq": 21, "kind": "created", "actor": "yardmaster", "bead": "yd-barn", "data": {"title": "Paint the barn", "stage": "new", "type": "task", "originator": "jo"}}"#);
            }
        }
        "page" => {
            // The plain ask above, whose note carries a page, an image and
            // a file: the yard holds the image, and the page and the file
            // are never asked for, since Open takes them to the browser,
            // run beside Open for the page alone. The mail carries a page
            // too, which Open takes to the browser, with no run.
            yard.blob(IMAGE_HASH, IMAGE);
            yard.answer(
                "UserInbox",
                &json!({"result": {"mail": [
                    {"seq": 12, "from": "yardmaster", "to": "jo",
                     "text": "The mockup is on the bead.", "read": false},
                ], "unread": 1, "last": 12, "from": ["yardmaster"]}})
                .to_string(),
            );
            yard.answer(
                "Conversation",
                &json!({"result": [
                    {"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo",
                     "text": "The mockup is on the bead.",
                     "files": [{"hash": MAIL_PAGE_HASH, "type": "text/html; charset=utf-8",
                                "bytes": 18000}],
                     "read": false},
                ]})
                .to_string(),
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
                    "body": "",
                    "stage": "new", "labels": ["topic:linear-out"],
                    "notes": [{"seq": 2, "kind": "note", "author": "yardmaster",
                               "text": "the mockup, like this:", "at": "2026-10-10T09:30:00Z",
                               "files": [
                                   {"hash": PAGE_HASH, "type": "text/html; charset=utf-8",
                                    "bytes": 42000},
                                   {"hash": IMAGE_HASH, "type": "image/png",
                                    "bytes": IMAGE.len()},
                                   {"hash": FILE_HASH, "type": "application/pdf",
                                    "bytes": 900, "name": "figures.pdf"},
                               ]}],
                    "asks": [{
                        "ask": 3, "to": "jo", "asker": "yardmaster",
                        "since": "2026-10-10T10:00:00Z",
                        "question": "Move them now?",
                        "context": ["The links file declares the new source."],
                        "options": [
                            {"label": "Move them now",
                             "consequence": "all 64 links point tonight"},
                            {"label": "You decide",
                             "consequence": "leave it to yardmaster"}
                        ],
                        "recommend": 0, "why": "",
                        "state": "open"
                    }]
                }})
                .to_string(),
            );
        }
        "thread" => {
            // A long wrapped thread from alternating senders, the
            // newest mail with an image: the window opens on it, so
            // the shot shows the reading column with wrapped mail. The
            // yard holds the image, as in the mail state.
            yard.blob(IMAGE_HASH, IMAGE);
            yard.answer(
                "UserInbox",
                r#"{"result": {"mail": [], "unread": 0, "last": 0, "from": []}}"#,
            );
            yard.answer(
                "Conversation",
                &json!({"result": [
                    {"seq": 12, "at": "2026-10-13T09:30:00Z", "from": "yardmaster", "to": "jo",
                     "text": "New draft is in Gmail on Shai's thread: it says managers cannot change names today, people can edit their own, a manager option is coming with no date promised, and I'll let him know.",
                     "read": true},
                    {"seq": 13, "at": "2026-10-13T09:40:00Z", "from": "jo", "to": "yardmaster",
                     "text": "Can we get permission to edit?", "read": true},
                    {"seq": 14, "at": "2026-10-13T09:50:00Z", "from": "yardmaster", "to": "jo",
                     "text": "Not with the Gmail pack as it is: it has four calls, and by design no edit, delete, send or labels. To allow draft edits I'd write a second source file for Gmail, and you'd approve it as a new source plus a grant.",
                     "read": true},
                    {"seq": 15, "at": "2026-10-13T10:00:00Z", "from": "jo", "to": "yardmaster",
                     "text": "Write it, and send Shai the new draft once the grant lands. Tell him the manager option is coming.",
                     "read": true},
                    {"seq": 16, "at": "2026-10-13T10:10:00Z", "from": "yardmaster", "to": "jo",
                     "text": "The grant is in: the second source is merged to main, reviewed with screenshots, check:all passed. Shai's draft still says 'coming'; you can now tell him it's live once deployed.",
                     "read": true},
                    {"seq": 17, "at": "2026-10-13T10:20:00Z", "from": "jo", "to": "yardmaster",
                     "text": "Can I see the screenshot?", "read": true},
                    {"seq": 18, "at": "2026-10-13T10:30:00Z", "from": "yardmaster", "to": "jo",
                     "text": "The three screenshots from the builder's note: the editor with the override typed in and the reset button, the list and header after saving, and after reset.",
                     "files": [{"hash": IMAGE_HASH, "type": "image/png",
                                "bytes": IMAGE.len()}],
                     "read": true},
                ]})
                .to_string(),
            );
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        }
        "column" => {
            // A long conversation over three days, grouped by sender, with
            // a call link, a calendar link and a correction: the shot shows
            // the reading column, the grouping, the separators and the
            // short links. The days count back from today, so the
            // separators read Today, Yesterday and the weekday.
            yard.answer(
                "UserInbox",
                r#"{"result": {"mail": [], "unread": 0, "last": 0, "from": []}}"#,
            );
            let zone = chrono::Local::now().format("%:z").to_string();
            let day = |ago: i64, at: &str| {
                let date = chrono::Local::now().date_naive() - chrono::Duration::days(ago);
                format!("{date}T{at}{zone}")
            };
            yard.answer(
                "Conversation",
                &json!({"result": [
                    {"seq": 12, "at": day(2, "20:14:00"), "from": "yardmaster", "to": "jo",
                     "text": "The evening's work is shaped: three beads, one question each.",
                     "read": true},
                    {"seq": 13, "at": day(2, "20:16:00"), "from": "yardmaster", "to": "jo",
                     "text": "The first is ready to land.",
                     "read": true},
                    {"seq": 14, "at": day(2, "20:17:00"), "from": "jo", "to": "yardmaster",
                     "text": "here's the teams link for the video call: https://teams.microsoft.com/l/meetup-join/19%3ameeting_N2Q4YzQefBDwL/0?context=chat",
                     "read": true},
                    {"seq": 15, "at": day(2, "20:19:00"), "from": "yardmaster", "to": "jo",
                     "text": "I'll put it on the event as its location.",
                     "read": true},
                    {"seq": 16, "at": day(2, "20:21:00"), "from": "yardmaster", "to": "jo",
                     "text": "correction the edit card is ask 6893, not 6891",
                     "read": true},
                    {"seq": 17, "at": day(1, "09:30:00"), "from": "jo", "to": "yardmaster",
                     "text": "applied the docs pack",
                     "read": true},
                    {"seq": 18, "at": day(1, "09:33:00"), "from": "yardmaster", "to": "jo",
                     "text": "Thanks, google-docs now has replace. The invite moved: https://calendar.google.com/calendar/event?action=TEMPLATE&tmeid=abc123&dates=20261019",
                     "read": true},
                    {"seq": 19, "at": day(1, "09:35:00"), "from": "yardmaster", "to": "jo",
                     "text": "Once you approve the grant I read it and make the edits.",
                     "read": true},
                    {"seq": 20, "at": day(0, "08:05:00"), "from": "jo", "to": "yardmaster",
                     "text": "approved",
                     "read": true},
                    {"seq": 21, "at": day(0, "08:07:00"), "from": "yardmaster", "to": "jo",
                     "text": "Done: the Teams link is on Monday's call with Liana, as the location and in the description.",
                     "read": true},
                    {"seq": 22, "at": day(0, "08:09:00"), "from": "yardmaster", "to": "jo",
                     "text": "The second lands behind it.",
                     "read": true},
                ]})
                .to_string(),
            );
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        }
        "refs" => {
            // The yardmaster's conversation naming an ask and two bead
            // ids: the ask's chip with its preview under the mail, and
            // the beads' chips. The ask is a run ask's, as the grant in
            // the review is; yardr-e9f0 is its own bead, and yd-zulu
            // travels on the board every state serves.
            yard.answer(
                "UserInbox",
                &json!({"result": {"mail": [
                    {"seq": 12, "from": "yardmaster", "to": "jo",
                     "text": "I put the grant to you as a card (ask 6926).",
                     "read": false},
                ], "unread": 1, "last": 12, "from": ["yardmaster"]}})
                .to_string(),
            );
            yard.answer(
                "Conversation",
                &json!({"result": [
                    {"seq": 12, "at": "2026-10-14T08:00:00Z", "from": "yardmaster", "to": "jo",
                     "text": "I put the grant to you as a card (ask 6926).",
                     "read": false},
                    {"seq": 13, "at": "2026-10-14T08:02:00Z", "from": "jo", "to": "yardmaster",
                     "text": "On it. Is the turn (yd-zulu) still waiting?",
                     "read": false},
                    {"seq": 14, "at": "2026-10-14T08:05:00Z", "from": "yardmaster", "to": "jo",
                     "text": "It waits on the grant. The grant's bead (yardr-e9f0) holds it.",
                     "read": false},
                ]})
                .to_string(),
            );
            yard.answer(
                "UserAsks",
                &json!({"result": {"open": [
                    {"bead": "yardr-e9f0", "bead_title": "Docs grant",
                     "ask": 6926, "question": "Grant yardmaster \"replace\" on google-docs?",
                     "asker": "yardmaster", "since": "2026-10-14T07:55:00Z",
                     "run": run_of()},
                ], "decided": []}})
                .to_string(),
            );
        }
        "reactions" | "reactions-hover" => {
            // The yardmaster's conversation with the parties' reactions
            // under two mails: jo's on the question, and both parties'
            // on the answer, with a third mail without any. The hover
            // state poses the pointer on the answer, so its row of
            // reactions to tap stands under it.
            yard.answer(
                "UserInbox",
                r#"{"result": {"mail": [], "unread": 0, "last": 0, "from": []}}"#,
            );
            yard.answer(
                "Conversation",
                &json!({"result": [
                    {"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo",
                     "text": "Shall I land it?", "read": true,
                     "reactions": [{"from": "jo", "emoji": "👍"}]},
                    {"seq": 13, "at": "2026-10-08T09:36:00Z", "from": "jo", "to": "yardmaster",
                     "text": "Land it.", "read": true,
                     "reactions": [{"from": "jo", "emoji": "✅"},
                                   {"from": "yardmaster", "emoji": "👀"}]},
                    {"seq": 14, "at": "2026-10-08T09:42:00Z", "from": "yardmaster", "to": "jo",
                     "text": "Landed.", "read": true},
                ]})
                .to_string(),
            );
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        }
        "mail" | "image" => {
            // The yardmaster's mail carries a page and an image, as the
            // bead's note does in the page state: the yard holds the
            // image, and the page is never asked for, since Open takes
            // it to the browser, with no run. The image state serves
            // the same mail, and the window opens its image over it.
            yard.blob(IMAGE_HASH, IMAGE);
            yard.answer(
                "UserInbox",
                &json!({"result": {"mail": [
                    {"seq": 12, "from": "yardmaster", "to": "jo",
                     "text": "The mockup is in the mail.", "read": false},
                ], "unread": 1, "last": 12, "from": ["yardmaster"]}})
                .to_string(),
            );
            yard.answer(
                "Conversation",
                &json!({"result": [
                    {"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo",
                     "text": "The mockup is in the mail.",
                     "files": [{"hash": MAIL_PAGE_HASH, "type": "text/html; charset=utf-8",
                                "bytes": 18000},
                               {"hash": IMAGE_HASH, "type": "image/png",
                                "bytes": IMAGE.len()}],
                     "read": false},
                ]})
                .to_string(),
            );
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        }
        "undelivered" => {
            // The conversation with one of a linked yard whose mail the
            // yard gave up over the hour: its line says what the link
            // takes and when the hour ends, from the mail the yard wrote
            // of it. The failed message and the telling cross as the
            // stream tells of them, past the head the window reads from.
            yard.answer(
                "UserInbox",
                r#"{"result": {"mail": [], "unread": 0, "last": 0, "from": []}}"#,
            );
            yard.converse(
                "clerk@north",
                vec![
                    json!({"seq": 12, "at": "2026-10-13T09:30:00Z", "from": "clerk@north",
                           "to": "jo", "text": "The ledger closes at noon.", "read": true}),
                    json!({"seq": 13, "at": "2026-10-13T09:32:00Z", "from": "jo",
                           "to": "clerk@north", "text": "Is the ledger closed?",
                           "read": false, "delivery": "failed"}),
                ],
            );
            yard.answer(
                "Conversation",
                r#"{"result": [{"seq": 11, "at": "2026-10-13T09:29:00Z", "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}]}"#,
            );
            yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
            yard.emit(r#"{"seq": 21, "kind": "peer_message_failed", "actor": "yard", "data": {"peer": "north", "kind": "mail", "id": "m1", "attempts": 1, "reason": "peer north: refused by the peer: it takes 30 mails an hour from this yard; send again after 21:00"}}"#);
            yard.emit(r#"{"seq": 22, "kind": "mail", "actor": "yard", "at": "2026-10-13T20:31:00Z", "data": {"to": "jo", "to_user": true, "undelivered": "m1", "limit": 30, "until": "2026-10-13T21:00:00Z", "text": "your mail to clerk@north of 10-13 20:31 was not delivered: refused by the peer: it takes 30 mails an hour from this yard; send again after 21:00"}}"#);
        }
        "secret" | "secret-shown" | "secret-refused" => {
            // An open card of a secret ask, posed with a value typed:
            // hidden, with the toggle to Show, or shown where the yard
            // marks the value no secret, with the toggle to Hide. The
            // refused shot poses the same ask with the yard's refusal
            // of its answer on the card instead, the field empty.
            let unconcealed = state == "secret-shown";
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
                &json!({"result": {"open": [queued_secret(unconcealed)], "decided": []}})
                    .to_string(),
            );
            yard.answer("Bead", &bead_secret(unconcealed));
        }
        "decisions" | "decisions-filtered" => {
            // One open ask with a question that wraps, and a history of
            // 156 decided: three outcomes asked today, a folded look-alike
            // run of four with a put-aside, a decline and a secret given
            // yesterday, and the older weeks behind them. The filtered
            // state poses the filter with `docs`, which matches today
            // above the fold and yesterday below it.
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
                &json!({"result": {
                    "open": [queued_links()],
                    "decided": decided_history(),
                }})
                .to_string(),
            );
            yard.answer("Bead", &bead_links());
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
    if state == "arrivals" || state == "departures" {
        // The board these states shoot answers over the shared one: a
        // late bead with the overrun the yard will print, a bead that
        // waits on jo, and a held one. The window draws the panel on
        // the arrivals row, and the board with its banner under
        // Departures.
        yard.answer(
            "Arrivals",
            &json!({"result": {"rows": [
                {"bead": "yd-zulu", "title": "Zulu's late turn",
                 "platform": "review", "line": "yardr", "expected": "5h–8h",
                 "remark": "late by 40m", "tone": "amber"},
                {"bead": "yd-alpha", "title": "Alpha moves first",
                 "platform": "review-security", "line": "yardr", "expected": "20m–35m",
                 "remark": "waits for jo, yardmaster", "tone": "amber"},
                {"bead": "footplate-p512", "title": "The sign-in's password label",
                 "platform": "new", "line": "yardr", "expected": "—",
                 "remark": "held", "tone": "red"},
            ], "platforms": ["new", "review", "review-security"]}})
            .to_string(),
        );
    }
    if state == "arrivals" || state == "departures" || state == "flow" {
        // The flows these states shoot answer over the shared ones:
        // five tracks with a backlog to count, a held and a blocked
        // bead under a fault lamp, a late and a worked bead with one
        // more in review's full group, a wait and a person's turn under
        // a turn lamp, a terminal stage with its buffer stop, an
        // Elsewhere bead, two beads with no line, and drafts quiet.
        // Each stage carries its terminal flag and its routed group's
        // seats, each claimed bead its worker and its stage's clock.
        // The longest names this yard's own flows have stand among them:
        // `contributor` and `approved` as stages of people, and the line
        // `contributors.issue · issue`, so a cell too narrow for a real
        // name shows in the picture.
        yard.answer(
            "Flows",
            &with_the_longest_names(json!({"result": {
                "travelling": 14,
                "depots": [
                    {"name": "yardr", "kind": "git", "travelling": 9, "flows": [
                        {"name": "default", "type": "", "source": "store", "stages": [
                            {"stage": "backlog", "group": "mckean", "human": true,
                             "lamp": "none", "terminal": false, "limit": 1, "taken": 0,
                             "beads": [
                                {"bead": "yd-b1", "title": "First to shape",
                                 "expected": "—", "remark": "", "tone": "none",
                                 "working": false},
                                {"bead": "yd-b2", "title": "Second to shape",
                                 "expected": "—", "remark": "", "tone": "none",
                                 "working": false}]},
                            {"stage": "new", "group": "muse-builders", "human": false,
                             "lamp": "fault", "terminal": false, "limit": 3, "taken": 2,
                             "beads": [
                                {"bead": "footplate-p512",
                                 "title": "The sign-in's password label",
                                 "expected": "—", "remark": "held",
                                 "tone": "red", "working": false},
                                {"bead": "yd-x1", "title": "Blocked behind it",
                                 "expected": "1h–2h", "remark": "blocked by footplate-p512",
                                 "tone": "amber", "working": false}]},
                            {"stage": "decide", "group": "decisions", "human": true,
                             "lamp": "none", "terminal": false, "limit": 1, "taken": 0,
                             "beads": []},
                            {"stage": "review", "group": "muse-reviewers", "human": false,
                             "lamp": "none", "terminal": false, "limit": 2, "taken": 2,
                             "beads": [
                                {"bead": "yd-zulu", "title": "Zulu's late turn",
                                 "expected": "5h–8h", "remark": "late by 40m",
                                 "tone": "amber", "working": false},
                                {"bead": "yd-r2", "title": "Second in review",
                                 "expected": "2h–3h", "remark": "", "tone": "none",
                                 "working": true, "worker": "herdr yd-r2-review",
                                 "entered_stage_at": "2026-10-10T09:30:00Z"},
                                {"bead": "yd-r3", "title": "Third in review",
                                 "expected": "3h–4h", "remark": "", "tone": "none",
                                 "working": false}]},
                            {"stage": "review-security", "group": "muse-security", "human": false,
                             "lamp": "turn", "terminal": false, "limit": 1, "taken": 0,
                             "beads": [
                                {"bead": "yd-alpha", "title": "Alpha moves first",
                                 "expected": "20m–35m", "remark": "waits for jo, yardmaster",
                                 "tone": "amber", "working": false},
                                {"bead": "yd-w1", "title": "A person's turn",
                                 "expected": "1h–2h", "remark": "",
                                 "tone": "none", "working": false,
                                 "entered_stage_at": "2026-10-10T09:45:00Z"}]},
                            {"stage": "approved", "group": "shippers", "human": false,
                             "lamp": "none", "terminal": true, "limit": 2, "taken": 0,
                             "beads": []},
                        ]},
                        {"name": "design.design", "type": "design", "source": "store", "stages": [
                            {"stage": "draft", "group": "mckean", "human": true,
                             "lamp": "none", "terminal": false, "limit": 1, "taken": 0,
                             "beads": [
                                {"bead": "yd-d1", "title": "Drafts the follow-up",
                                 "expected": "3h–5h", "remark": "",
                                 "tone": "none", "working": false}]},
                            {"stage": "critique", "group": "mckean", "human": true,
                             "lamp": "none", "terminal": true, "beads": []},
                        ]},
                    ], "elsewhere": [
                        {"bead": "yardr-p9", "title": "A pinned flow's bead",
                         "expected": "—", "remark": "", "tone": "none",
                         "working": false},
                    ]},
                    {"name": "footplate", "kind": "git", "travelling": 2, "flows": [
                        {"name": "default", "type": "task", "source": "store", "stages": [
                            {"stage": "new", "group": "muse-builders", "human": false,
                             "lamp": "none", "terminal": false, "limit": 2, "taken": 1,
                             "beads": [
                                {"bead": "footplate-f1", "title": "A quiet bead",
                                 "expected": "1h–2h", "remark": "",
                                 "tone": "none", "working": false}]},
                        ]},
                    ], "elsewhere": []},
                    {"name": "drafts", "kind": "dir", "travelling": 0,
                     "flows": [], "elsewhere": []},
                ],
                "unassigned": [
                    {"bead": "yd-s1", "title": "Travels light",
                     "expected": "2h–3h", "remark": "", "tone": "none",
                     "working": false},
                    {"bead": "yd-s2", "title": "Waits with no line",
                     "expected": "—", "remark": "waits for yardmaster, jo",
                     "tone": "amber", "working": false},
                ],
            }}))
            .to_string(),
        );
    }
}

/// The rich flows with this yard's longest names added: footplate's flow
/// of pull requests, whose stages of people have the longest heads, and
/// signalbox, whose flow has the longest line. They are added to the
/// literal, not written in it: it is long enough as it stands for json!
/// to reach its recursion limit.
fn with_the_longest_names(mut flows: serde_json::Value) -> serde_json::Value {
    let depots = flows["result"]["depots"].as_array_mut().unwrap();
    depots[1]["flows"].as_array_mut().unwrap().push(json!(
        {"name": "contributors.pr", "type": "pr", "source": "store", "stages": [
            {"stage": "triage", "group": "mckean", "human": true,
             "lamp": "none", "terminal": false, "limit": 1, "taken": 0, "beads": []},
            {"stage": "review", "group": "muse-reviewers", "human": false,
             "lamp": "none", "terminal": false, "limit": 2, "taken": 2, "beads": []},
            {"stage": "contributor", "group": "mckean", "human": true,
             "lamp": "turn", "terminal": false, "limit": 1, "taken": 0,
             "beads": [
                {"bead": "footplate-c1", "title": "A contributor's pull request",
                 "expected": "1h–2h", "remark": "waits for jo",
                 "tone": "amber", "working": false}]},
            {"stage": "approved", "group": "mckean", "human": true,
             "lamp": "none", "terminal": false, "limit": 1, "taken": 0, "beads": []},
            {"stage": "decide", "group": "decisions", "human": true,
             "lamp": "none", "terminal": false, "limit": 1, "taken": 0, "beads": []},
            {"stage": "merged", "group": "", "human": false,
             "lamp": "none", "terminal": true, "beads": []},
            {"stage": "closed", "group": "", "human": false,
             "lamp": "none", "terminal": true, "beads": []},
        ]}
    ));
    depots.push(json!(
        {"name": "signalbox", "kind": "git", "travelling": 1, "flows": [
            {"name": "contributors.issue", "type": "issue", "source": "store", "stages": [
                {"stage": "triage", "group": "mckean", "human": true,
                 "lamp": "turn", "terminal": false, "limit": 1, "taken": 0,
                 "beads": [
                    {"bead": "signalbox-i1", "title": "An issue to triage",
                     "expected": "—", "remark": "waits for yardmaster",
                     "tone": "amber", "working": false}]},
                {"stage": "backlog", "group": "mckean", "human": true,
                 "lamp": "none", "terminal": false, "limit": 1, "taken": 0, "beads": []},
                {"stage": "new", "group": "muse-builders", "human": false,
                 "lamp": "none", "terminal": false, "limit": 2, "taken": 1, "beads": []},
                {"stage": "review", "group": "muse-reviewers", "human": false,
                 "lamp": "none", "terminal": false, "limit": 2, "taken": 2, "beads": []},
                {"stage": "approved", "group": "shippers", "human": false,
                 "lamp": "none", "terminal": false, "limit": 2, "taken": 0, "beads": []},
                {"stage": "decide", "group": "decisions", "human": true,
                 "lamp": "none", "terminal": false, "limit": 1, "taken": 0, "beads": []},
                {"stage": "merged", "group": "", "human": false,
                 "lamp": "none", "terminal": true, "beads": []},
                {"stage": "closed", "group": "", "human": false,
                 "lamp": "none", "terminal": true, "beads": []},
            ]},
        ], "elsewhere": []}
    ));
    flows
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

/// The arrival yd-zulu whole, as the Bead call answers it.
fn bead_zulu() -> String {
    json!({"result": {
        "id": "yd-zulu", "title": "Zulu's late turn",
        "body": "The turn the yard waits on: its tests are green, and the landing waits for the review.",
        "stage": "draft", "labels": ["size:medium", "eyes"],
        "notes": [
            {"seq": 3, "kind": "note", "author": "assembly",
             "text": "running late: the review has two beads ahead of it",
             "at": "2026-10-10T10:30:00Z"},
            {"seq": 5, "kind": "note", "author": "yardmaster",
             "text": "still late, and still first in line",
             "at": "2026-10-10T11:30:00Z"},
        ],
        "asks": [
            {"ask": 7, "to": "jo", "asker": "yardmaster",
             "since": "2026-10-10T11:00:00Z",
             "question": "Wait for it?", "state": "open"},
            {"ask": 8, "to": "jo", "asker": "assembly",
             "since": "2026-10-10T11:30:00Z",
             "question": "Land it now?", "state": "open"},
            {"ask": 6, "to": "jo", "asker": "yardmaster",
             "since": "2026-10-10T10:00:00Z",
             "question": "Shape it?", "state": "answered"},
        ],
    }})
    .to_string()
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

/// What the secret ask asks for, in the yard's words.
fn secret_of() -> serde_json::Value {
    json!({"source": "books", "env": ["BOOKS_TOKEN"],
           "line": "a secret: BOOKS_TOKEN, for the source books (https://books.example.com)"})
}

/// The secret ask 7 on bead-s as the queue lists it, marked no secret
/// where the state shows it.
fn queued_secret(unconcealed: bool) -> serde_json::Value {
    let mut ask = json!({
        "bead": "bead-s", "bead_title": "The books", "topic": "", "ask": 7,
        "question": "The token for the books", "asker": "yardmaster",
        "since": "2026-10-10T10:00:00Z", "secret": secret_of()
    });
    if unconcealed {
        ask["concealed"] = json!(false);
    }
    ask
}

/// The bead of that ask, marked as the queue marks it.
fn bead_secret(unconcealed: bool) -> String {
    let mut ask = json!(
        {"ask": 7, "to": "jo", "asker": "yardmaster",
         "since": "2026-10-10T10:00:00Z",
         "question": "The token for the books",
         "context": ["The books were moved to the declared source."],
         "options": [], "state": "open", "secret": secret_of()});
    if unconcealed {
        ask["concealed"] = json!(false);
    }
    json!({"result": {"id": "bead-s", "title": "The books", "body": "The body.",
        "stage": "new", "labels": [], "notes": [], "asks": [ask]}})
    .to_string()
}

/// The open ask of the decisions states: a question that wraps to two
/// lines in the list, with four options on its card.
fn queued_links() -> serde_json::Value {
    json!({
        "bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
        "ask": 3,
        "question": "Point this yard's 64 links at the declared source now, before Friday's release?",
        "asker": "yardmaster", "since": "2026-10-10T10:00:00Z"
    })
}

/// The bead of that ask, with its four options.
fn bead_links() -> String {
    json!({"result": {
        "id": "bead-a", "title": "Move the links",
        "body": "The body of bead-a.",
        "stage": "new", "labels": ["topic:linear-out"],
        "notes": [{"seq": 2, "kind": "note", "author": "yardmaster",
                   "text": "the test passed", "at": "2026-10-10T09:30:00Z"}],
        "asks": [{
            "ask": 3, "to": "jo", "asker": "yardmaster",
            "since": "2026-10-10T10:00:00Z",
            "question": "Point this yard's 64 links at the declared source now, before Friday's release?",
            "context": ["The links file declares the new source.",
                        "The move was tested."],
            "options": [
                {"label": "Move them now",
                 "consequence": "all 64 links point tonight"},
                {"label": "After Friday",
                 "consequence": "keep the old source"},
                {"label": "One lane first",
                 "consequence": "move the drafts lane tonight"},
                {"label": "You decide",
                 "consequence": "leave it to yardmaster"}
            ],
            "recommend": 1, "why": "the test passed",
            "state": "open"
        }]
    }})
    .to_string()
}

/// Midnight this morning, in the person shooting's zone: the history's
/// days stand against it, so the shot always shows a Today and a
/// Yesterday whatever day it is taken.
fn shot_midnight() -> DateTime<Local> {
    Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
}

/// An asked time `days` days ago at `hour`:`min`, as the yard serves it.
fn shot_since(midnight: &DateTime<Local>, days: i64, hour: u32, min: u32) -> String {
    (*midnight - Duration::days(days)
        + Duration::hours(hour as i64)
        + Duration::minutes(min as i64))
    .to_rfc3339()
}

/// The decisions states' history, newest first: 156 dealt with asks.
fn decided_history() -> serde_json::Value {
    let midnight = shot_midnight();
    let at = |days: i64, hour: u32, min: u32| shot_since(&midnight, days, hour, min);
    let mut decided = vec![
        json!({"bead": "yd-cal", "bead_title": "The calendar", "topic": "topic:linear-out",
               "ask": 101, "question": "Let me edit Google Docs in place", "asker": "yardmaster",
               "since": at(0, 11, 53), "state": "answered",
               "option": 1, "option_label": "Let them"}),
        json!({"bead": "yd-cal", "bead_title": "The calendar", "topic": "topic:linear-out",
               "ask": 102, "question": "Which inbox set should count as attention?", "asker": "mayor",
               "since": at(0, 10, 5), "state": "answered", "words": "just the direct ones"}),
        json!({"bead": "yd-train", "bead_title": "The release train", "topic": "",
               "ask": 103, "question": "Take the release train tonight?", "asker": "yardmaster",
               "since": at(0, 9, 12), "state": "taken back",
               "by": "yardmaster", "words": "the train left"}),
        json!({"bead": "yd-docs", "bead_title": "The quota", "topic": "topic:linear-out",
               "ask": 107, "question": "Raise Google Docs to 4000 calls", "asker": "mayor",
               "since": at(1, 20, 21), "state": "answered",
               "option": 1, "option_label": "Raise them"}),
        json!({"bead": "yd-docs", "bead_title": "The quota", "topic": "topic:linear-out",
               "ask": 106, "question": "Raise Google Docs to 3000 calls", "asker": "mayor",
               "since": at(1, 20, 19), "state": "answered",
               "option": 1, "option_label": "Raise them"}),
        json!({"bead": "yd-docs", "bead_title": "The quota", "topic": "topic:linear-out",
               "ask": 105, "question": "Raise Google Docs to 2000 calls", "asker": "mayor",
               "since": at(1, 20, 18), "state": "answered",
               "option": 1, "option_label": "Raise them"}),
        json!({"bead": "yd-docs", "bead_title": "The quota", "topic": "topic:linear-out",
               "ask": 104, "question": "Raise Google Docs to 1000 calls", "asker": "mayor",
               "since": at(1, 20, 17), "state": "answered",
               "option": 1, "option_label": "Raise them"}),
        json!({"bead": "yd-index", "bead_title": "The index", "topic": "",
               "ask": 108, "question": "Rebuild the search index tonight?", "asker": "yardmaster",
               "since": at(1, 18, 2), "state": "put aside",
               "return_at": at(-1, 9, 30)}),
        json!({"bead": "yd-ship", "bead_title": "The migration", "topic": "topic:linear-out",
               "ask": 109, "question": "Land the half-written migration?", "asker": "assembly",
               "since": at(1, 14, 44), "state": "declined", "words": "not yet"}),
        json!({"bead": "yd-books", "bead_title": "The books", "topic": "",
               "ask": 110, "question": "The token for the books", "asker": "yardmaster",
               "since": at(1, 9, 30), "state": "answered", "given": ["BOOKS_TOKEN"]}),
    ];
    // The older weeks: 146 asks over eleven days, newest first. The asker
    // turns every ask, so no two neighbours ever fold: the frame's rows
    // stand as written.
    let askers = ["mayor", "yardmaster", "assembly"];
    let services = ["api", "web", "worker", "store", "mail"];
    for (i, n) in (111..=256).enumerate() {
        let days = 2 + (i as i64) / 14;
        let hour = 18 - (i as u32 % 14);
        let min = (i as u32 * 7) % 60;
        let asker = askers[i % askers.len()];
        let mut ask = json!({
            "bead": format!("yd-past-{n}"), "bead_title": "The older weeks",
            "topic": "topic:linear-out", "ask": n,
            "question": format!("Approve the {} {} deploy?", services[i % services.len()], 100 + i),
            "asker": asker, "since": at(days, hour, min),
        });
        if n == 160 || n == 230 {
            ask["state"] = json!("declined");
            ask["words"] = json!("not now");
        } else if i % 13 == 7 {
            ask["state"] = json!("put aside");
        } else if i % 11 == 5 {
            ask["state"] = json!("taken back");
            ask["by"] = json!(asker);
            ask["words"] = json!("changed my mind");
        } else if i % 7 == 3 {
            ask["state"] = json!("answered");
            ask["words"] = json!("on it");
        } else {
            ask["state"] = json!("answered");
            ask["option"] = json!(1);
            ask["option_label"] = json!("Approve it");
        }
        decided.push(ask);
    }
    json!(decided)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session;
    use std::os::unix::fs::PermissionsExt;

    /// Every state reads as the window reads it: the login name is no user
    /// of the yard, so the window is for its operator, jo, and the state
    /// opens where the shot says. A state added to [`STATES`] lands in the
    /// unreachable arm until it is covered here. `stopped`, `refusal`,
    /// `setup` and `hosted` listen nowhere: their run's socket names a
    /// yard that is down.
    #[test]
    fn every_state_loads_as_the_window_reads_it() {
        for state in STATES {
            if *state == "stopped" || *state == "refusal" || *state == "setup" || *state == "hosted"
            {
                let env = Env {
                    socket: PathBuf::from("/none"),
                    ..Env::default()
                };
                let (env, shot) = start_for_state(env, state);
                assert!(shot.is_none(), "{state}");
                assert!(!env.socket.exists(), "{state}");
                if *state == "setup" {
                    // The canned doctor the second step stands with: two
                    // installed providers with labels and offers, one
                    // recommended each.
                    let doctor = setup_doctor();
                    assert!(doctor.ready(), "{state}");
                    let kinds: Vec<_> = doctor
                        .installed()
                        .iter()
                        .map(|item| item.harness.clone().unwrap())
                        .collect();
                    assert_eq!(kinds, ["claude", "codex"], "{state}");
                    for item in doctor.installed() {
                        assert!(item.label.is_some(), "{state}");
                        assert!(item.recommended_offer().is_some(), "{state}");
                    }
                }
                continue;
            }
            let yard = Yard::start();
            populate(&yard, state);
            let loaded = session::load(&yard.client(), "shot").unwrap();
            assert_eq!(loaded.me, "jo", "{state}");
            match *state {
                "decisions" | "decisions-filtered" => {
                    assert_eq!(loaded.asks.decided.len(), 156, "{state}")
                }
                _ => assert_eq!(loaded.asks.decided.len(), 0, "{state}"),
            }
            match *state {
                "yardmaster"
                | "far"
                | "settings"
                | "settings-appearance"
                | "settings-yards"
                | "settings-agents"
                | "settings-notifications"
                | "settings-connection"
                | "reconnecting"
                | "answered"
                | "signedout"
                | "away"
                | "switcher" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 6, "{state}");
                    assert_eq!(letters[1].from, "jo", "{state}");
                }
                "arrivals" | "departures" | "arrival" | "flow" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 6, "{state}");
                    assert_eq!(letters[1].from, "jo", "{state}");
                    // The board the window opens: three rows, with an
                    // amber remark, none, and a red one. The arrivals and
                    // departures states shoot their own board over that
                    // one: a late bead with its overrun, a bead that
                    // waits on jo, and a held one.
                    assert_eq!(loaded.arrivals.rows.len(), 3, "{state}");
                    assert_eq!(loaded.arrivals.rows[0].bead, "yd-zulu", "{state}");
                    let overrun = *state == "arrivals" || *state == "departures";
                    assert_eq!(
                        (
                            loaded.arrivals.rows[0].remark.as_str(),
                            loaded.arrivals.rows[0].tone.as_str()
                        ),
                        match overrun {
                            true => ("late by 40m", "amber"),
                            false => ("late", "amber"),
                        },
                        "{state}"
                    );
                    assert_eq!(
                        loaded.arrivals.rows[1].remark.as_str(),
                        match overrun {
                            true => "waits for jo, yardmaster",
                            false => "",
                        },
                        "{state}"
                    );
                    assert_eq!(
                        (
                            loaded.arrivals.rows[2].expected.as_str(),
                            loaded.arrivals.rows[2].remark.as_str(),
                            loaded.arrivals.rows[2].tone.as_str()
                        ),
                        ("—", "held", "red"),
                        "{state}"
                    );
                    // The flows view beside the board: three travelling,
                    // yardr's two on a turn and a quiet stage, footplate's
                    // one on a fault stage, and nothing without a line.
                    // The arrivals, departures and flow states shoot the
                    // richer one instead: yardr's two flows with a
                    // backlog, an Elsewhere bead and two beads with no
                    // line, footplate's two, drafts quiet, and signalbox
                    // with the longest line name.
                    let rich = overrun || *state == "flow";
                    assert_eq!(
                        loaded.flows.travelling,
                        match rich {
                            true => 14,
                            false => 3,
                        },
                        "{state}"
                    );
                    assert_eq!(loaded.flows.depots[0].name, "yardr", "{state}");
                    assert_eq!(
                        loaded.flows.depots[0].travelling,
                        match rich {
                            true => 9,
                            false => 2,
                        },
                        "{state}"
                    );
                    assert_eq!(
                        loaded.flows.depots.len(),
                        match rich {
                            true => 4,
                            false => 2,
                        },
                        "{state}"
                    );
                    if rich {
                        let stages = &loaded.flows.depots[0].flows[0].stages;
                        assert_eq!(
                            stages
                                .iter()
                                .map(|s| (s.stage.as_str(), s.human))
                                .collect::<Vec<_>>(),
                            [
                                ("backlog", true),
                                ("new", false),
                                ("decide", true),
                                ("review", false),
                                ("review-security", false),
                                ("approved", false),
                            ],
                            "{state}"
                        );
                        assert_eq!(stages[1].lamp, "fault", "{state}");
                        assert_eq!(stages[4].lamp, "turn", "{state}");
                        assert_eq!(stages[1].beads.len(), 2, "{state}");
                        assert_eq!(stages[3].beads.len(), 3, "{state}");
                        assert!(!stages[1].terminal, "{state}");
                        assert!(stages[5].terminal, "{state}");
                        assert_eq!(
                            (stages[3].limit, stages[3].taken),
                            (Some(2), Some(2)),
                            "{state}"
                        );
                        assert_eq!(stages[3].beads[1].worker, "herdr yd-r2-review", "{state}");
                        assert_eq!(
                            stages[3].beads[1].entered_stage_at, "2026-10-10T09:30:00Z",
                            "{state}"
                        );
                        assert_eq!(loaded.flows.depots[0].flows.len(), 2, "{state}");
                        assert_eq!(loaded.flows.depots[0].elsewhere.len(), 1, "{state}");
                        assert_eq!(loaded.flows.depots[1].travelling, 2, "{state}");
                        assert_eq!(loaded.flows.depots[2].travelling, 0, "{state}");
                        // This yard's longest names: the stages of people
                        // with the longest heads, and the longest line.
                        let contributors = &loaded.flows.depots[1].flows[1];
                        assert_eq!(contributors.name, "contributors.pr", "{state}");
                        assert_eq!(
                            contributors
                                .stages
                                .iter()
                                .filter(|s| s.human)
                                .map(|s| s.stage.as_str())
                                .collect::<Vec<_>>(),
                            ["triage", "contributor", "approved", "decide"],
                            "{state}"
                        );
                        let signalbox = &loaded.flows.depots[3];
                        assert_eq!(signalbox.name, "signalbox", "{state}");
                        assert_eq!(signalbox.flows[0].name, "contributors.issue", "{state}");
                        assert_eq!(signalbox.flows[0].flow_type, "issue", "{state}");
                        assert_eq!(
                            loaded
                                .flows
                                .unassigned
                                .iter()
                                .map(|b| b.bead.as_str())
                                .collect::<Vec<_>>(),
                            ["yd-s1", "yd-s2"],
                            "{state}"
                        );
                    } else {
                        assert!(loaded.flows.unassigned.is_empty(), "{state}");
                    }
                    assert_eq!(
                        loaded.flows.depots[0].flows[0].stages[0].lamp,
                        match rich {
                            true => "none",
                            false => "turn",
                        },
                        "{state}"
                    );
                    // One arrival's bead, which the arrival state opens:
                    // the body, two labels, two notes, and three asks, two
                    // of them open.
                    if *state == "arrival" {
                        let bead = yard.client().bead("yd-zulu").unwrap();
                        assert_eq!(bead.title, "Zulu's late turn", "{state}");
                        assert_eq!(bead.stage, "draft", "{state}");
                        assert_eq!(bead.labels.len(), 2, "{state}");
                        assert!(!bead.body.is_empty(), "{state}");
                        assert_eq!(bead.notes.len(), 2, "{state}");
                        assert_eq!(bead.asks.len(), 3, "{state}");
                        assert_eq!(
                            bead.asks.iter().filter(|a| a.state == "open").count(),
                            2,
                            "{state}"
                        );
                    }
                }
                "crew" => {
                    // The sidebar's groups full: three crew with their
                    // states, three of the linked yard, two people, three
                    // unread notices, one open ask, and one of jo's beads
                    // added past the head.
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    let crew: Vec<(&str, &str, &str)> = loaded
                        .crew
                        .iter()
                        .map(|r| (r.name.as_str(), r.state.as_str(), r.status.as_str()))
                        .collect();
                    assert_eq!(
                        crew,
                        [
                            ("assembly", "stopped", ""),
                            ("yardmaster", "running", "working"),
                            ("brakeman", "running", "idle"),
                        ],
                        "{state}"
                    );
                    let roster: Vec<&str> = loaded.peers[0]
                        .said
                        .as_ref()
                        .unwrap()
                        .roster
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect();
                    assert_eq!(roster, ["clerk", "warden", "rue"], "{state}");
                    let users: Vec<&str> = loaded.users.iter().map(|u| u.name.as_str()).collect();
                    assert_eq!(users, ["jo", "ada"], "{state}");
                    assert_eq!(
                        loaded.notifications.iter().filter(|n| !n.read).count(),
                        3,
                        "{state}"
                    );
                    let mut events = yard.client().events(20).unwrap();
                    let added = events.next().unwrap().unwrap();
                    assert_eq!(added.kind.as_str(), "created", "{state}");
                    assert_eq!(added.text("originator"), "jo", "{state}");
                }
                "plain" | "lines" | "keys" => {
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    let bead = yard.client().bead("bead-a").unwrap();
                    assert_eq!(bead.asks.len(), 1, "{state}");
                    assert_eq!(bead.asks[0].options.len(), 3, "{state}");
                    assert_eq!(bead.asks[0].recommend, 1, "{state}");
                    // The lines badge's bead, added past the head: one of
                    // jo's.
                    if *state == "lines" {
                        let mut events = yard.client().events(20).unwrap();
                        let added = events.next().unwrap().unwrap();
                        assert_eq!(added.kind.as_str(), "created", "{state}");
                        assert_eq!(added.text("originator"), "jo", "{state}");
                    }
                }
                "run" => {
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    assert!(loaded.asks.open[0].run.is_some(), "{state}");
                    let bead = yard.client().bead("bead-s").unwrap();
                    assert_eq!(bead.asks.len(), 1, "{state}");
                    assert_eq!(bead.asks[0].question.lines().count(), 3, "{state}");
                    assert!(bead.asks[0].run.is_some(), "{state}");
                }
                "secret" | "secret-shown" | "secret-refused" => {
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    assert!(loaded.asks.open[0].secret.is_some(), "{state}");
                    assert_eq!(
                        loaded.asks.open[0].concealed,
                        *state != "secret-shown",
                        "{state}"
                    );
                    let bead = yard.client().bead("bead-s").unwrap();
                    assert_eq!(bead.asks.len(), 1, "{state}");
                    assert!(bead.asks[0].secret.is_some(), "{state}");
                    assert_eq!(bead.asks[0].concealed, *state != "secret-shown", "{state}");
                }
                "page" => {
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    let bead = yard.client().bead("bead-a").unwrap();
                    assert_eq!(bead.notes.len(), 1, "{state}");
                    assert_eq!(bead.notes[0].files.len(), 3, "{state}");
                    assert!(crate::model::is_page(&bead.notes[0].files[0]), "{state}");
                    assert!(crate::model::is_image(&bead.notes[0].files[1]), "{state}");
                    // The third row is a file, named; the yard holds the
                    // image, and the page and the file are named only.
                    assert!(!crate::model::is_page(&bead.notes[0].files[2]), "{state}");
                    assert!(!crate::model::is_image(&bead.notes[0].files[2]), "{state}");
                    assert_eq!(bead.notes[0].files[2].name, "figures.pdf", "{state}");
                    let image = yard.client().blob(IMAGE_HASH).unwrap().unwrap();
                    assert_eq!(image.len(), IMAGE.len(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 1, "{state}");
                    assert_eq!(letters[0].files.len(), 1, "{state}");
                    assert!(crate::model::is_page(&letters[0].files[0]), "{state}");
                }
                "thread" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    // Seven mails from alternating senders, the newest
                    // with the image the yard holds.
                    let image = yard.client().blob(IMAGE_HASH).unwrap().unwrap();
                    assert_eq!(image.len(), IMAGE.len(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 7, "{state}");
                    assert_eq!(letters[1].from, "jo", "{state}");
                    assert_eq!(letters[6].files.len(), 1, "{state}");
                    assert!(crate::model::is_image(&letters[6].files[0]), "{state}");
                }
                "column" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    // Eleven mails over three days, grouped by sender,
                    // with a call link, a calendar link and a correction.
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 11, "{state}");
                    assert!(letters[2].text.contains("teams.microsoft.com"), "{state}");
                    assert!(letters[6].text.contains("calendar.google.com"), "{state}");
                    assert!(letters[4].text.starts_with("correction"), "{state}");
                }
                "mail" | "image" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    // The mail carries the page and the image; the yard
                    // holds the image, and the page is named only.
                    let image = yard.client().blob(IMAGE_HASH).unwrap().unwrap();
                    assert_eq!(image.len(), IMAGE.len(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 1, "{state}");
                    assert_eq!(letters[0].files.len(), 2, "{state}");
                    assert!(crate::model::is_page(&letters[0].files[0]), "{state}");
                    assert!(!crate::model::is_page(&letters[0].files[1]), "{state}");
                }
                "reactions" | "reactions-hover" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    // Three mails, the first with one reaction and the
                    // second with both parties', the third with none.
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 3, "{state}");
                    assert_eq!(letters[0].reactions.len(), 1, "{state}");
                    assert_eq!(letters[1].reactions.len(), 2, "{state}");
                    assert!(letters[2].reactions.is_empty(), "{state}");
                }
                "refs" => {
                    // The run ask the first mail names, and the two bead
                    // ids the mails name: the ask's own, and yd-zulu's
                    // from the board.
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    assert_eq!(loaded.asks.open[0].ask, 6926, "{state}");
                    assert_eq!(loaded.asks.open[0].bead, "yardr-e9f0", "{state}");
                    assert!(loaded.asks.open[0].run.is_some(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 3, "{state}");
                    assert!(letters[0].text.contains("ask 6926"), "{state}");
                    assert!(letters[1].text.contains("yd-zulu"), "{state}");
                    assert!(letters[2].text.contains("yardr-e9f0"), "{state}");
                }
                "undelivered" => {
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    let letters = yard.client().conversation("jo", "clerk@north", 0).unwrap();
                    assert_eq!(letters.len(), 2, "{state}");
                    assert_eq!(letters[1].delivery, "failed", "{state}");
                    // The failed message and the telling of it cross past
                    // the head, as the stream tells of them.
                    let mut events = yard.client().events(20).unwrap();
                    let failed = events.next().unwrap().unwrap();
                    assert_eq!(failed.text("peer"), "north", "{state}");
                    assert_eq!(failed.text("id"), "m1", "{state}");
                    let telling = events.next().unwrap().unwrap();
                    assert_eq!(telling.text("undelivered"), "m1", "{state}");
                    assert_eq!(telling.number("limit"), 30, "{state}");
                    assert!(!telling.text("until").is_empty(), "{state}");
                }
                "decisions" | "decisions-filtered" => {
                    // One open ask with four options on its card.
                    assert_eq!(loaded.asks.open.len(), 1, "{state}");
                    let bead = yard.client().bead("bead-a").unwrap();
                    assert_eq!(bead.asks.len(), 1, "{state}");
                    assert_eq!(bead.asks[0].options.len(), 4, "{state}");
                    assert_eq!(bead.asks[0].recommend, 1, "{state}");
                    // Three outcomes asked today, then yesterday's folded
                    // run of four, newest first.
                    let asked: Vec<u64> = loaded.asks.decided.iter().map(|ask| ask.ask).collect();
                    assert_eq!(
                        &asked[..10],
                        &[101, 102, 103, 107, 106, 105, 104, 108, 109, 110],
                        "{state}"
                    );
                    assert_eq!(
                        loaded.asks.decided[0].question, "Let me edit Google Docs in place",
                        "{state}"
                    );
                    // The history the window draws from it: a Today and a
                    // Yesterday first, with the run folded to one row.
                    let (model, _) = crate::model::Model::new(
                        loaded.clone(),
                        crate::model::Mode::Decisions,
                        None,
                    );
                    let groups = model.decided_groups(&Local::now());
                    assert_eq!(groups[0].day, "Today", "{state}");
                    assert_eq!(groups[1].day, "Yesterday", "{state}");
                    assert!(groups.len() > 2, "{state}");
                    let yesterday = &groups[1].rows[0];
                    match yesterday {
                        crate::model::DecidedEntry::Fold(fold) => {
                            assert_eq!(fold.ask, 107, "{state}");
                            assert_eq!(fold.rows.len(), 4, "{state}");
                        }
                        entry => panic!("the run folds: {entry:?}"),
                    }
                }
                "old-lines" => {
                    // The yard serves neither Lines call, as one from
                    // before the Lines view: both halves read as empty,
                    // with no notice, and too old instead.
                    assert!(loaded.asks.open.is_empty(), "{state}");
                    let letters = yard.client().conversation("jo", "yardmaster", 0).unwrap();
                    assert_eq!(letters.len(), 6, "{state}");
                    assert!(loaded.arrivals.rows.is_empty(), "{state}");
                    assert_eq!(loaded.arrivals_notice, None, "{state}");
                    assert!(loaded.arrivals_too_old, "{state}");
                    assert_eq!(loaded.flows.travelling, 0, "{state}");
                    assert!(loaded.flows.depots.is_empty(), "{state}");
                    assert_eq!(loaded.flows_notice, None, "{state}");
                    assert!(loaded.flows_too_old, "{state}");
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

    #[test]
    fn the_switcher_shot_s_held_yards_wait_two_and_one_and_twelve() {
        let adventuretime = Yard::start();
        switcher_adventuretime(&adventuretime);
        let loaded = session::load(&adventuretime.client(), "shot").unwrap();
        assert_eq!(loaded.me, "jo");
        assert_eq!(loaded.unread.len(), 2);
        assert_eq!(loaded.asks.open.len(), 1);
        let plantime = Yard::start();
        switcher_plantime(&plantime);
        let loaded = session::load(&plantime.client(), "shot").unwrap();
        assert_eq!(loaded.me, "jo");
        assert_eq!(loaded.unread.len(), 12);
        assert!(loaded.asks.open.is_empty());
    }
}
