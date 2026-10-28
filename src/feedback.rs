//! The Feedback form's send: what a person typed, as a response to yardr's
//! PostHog survey 'footplate feedback', from where the yard hears it (the
//! posthog pack's feedback destination, in the yardr repository). It goes to
//! PostHog's EU ingestion host with the project's write key and nothing of a
//! yard's: no token, no session, no device. A message goes once, when a
//! person presses Send; nothing is queued, retried or kept.
//!
//! PostHog's capture takes one event as a POST of `{api_key, event,
//! distinct_id, properties}` to `/i/v0/e/`, and a survey's answer is the
//! event `survey sent` whose properties carry the survey's `$survey_id` and
//! the answer. A survey of one question is answered in the bare
//! `$survey_response`, as the custom-surveys page has it for a first
//! question; the page keys an answer by the question's id
//! (`$survey_response_<question id>`) in what it recommends, and the yard
//! hears either. If the survey as made records only the keyed form, the
//! question's id is a third constant beside [`WRITE_KEY`] and [`SURVEY`].
//! Each send has a distinct id of its own, made for it and kept nowhere, so
//! that nothing ties two messages of one person together. (Checked 28 Oct
//! 2026 against posthog.com/docs/api/capture, its "Survey" section, and
//! posthog.com/docs/surveys/implementing-custom-surveys.)

use std::{fmt, io};

use serde_json::{Value, json};

use crate::api::{self, Address};

/// PostHog's ingestion host for its EU cloud, where yardr's project is, and
/// the path of one event there.
const HOST: &str = "https://eu.i.posthog.com";
const CAPTURE: &str = "/i/v0/e/";

/// The write key of yardr's PostHog project (EU). A `phc_` key is public and
/// writes only by PostHog's design: it sends events and reads nothing, so it
/// stands in the source. There is no override at build time: a fork or an
/// open-source build sends its feedback to this project, on purpose, as the
/// place feedback goes.
const WRITE_KEY: &str = "phc_s9ANvMv9vKG3FLfuGFHLQQWPdHsyqv3fqC6hkcFgNkBA";

/// The id of the API-type survey 'footplate feedback', whose one open-text
/// question this form answers. Empty until the survey's id is had: while this
/// or the key is empty the Feedback button is not shown ([`Project::at`]), so
/// that no build offers a form that sends nowhere.
const SURVEY: &str = "";

/// The event a survey's answer is, and the properties of it that are PostHog's.
const EVENT: &str = "survey sent";
const SURVEY_ID: &str = "$survey_id";
const SURVEY_RESPONSE: &str = "$survey_response";

/// What the form says before anything is sent: all of what goes, and nothing
/// is sent beside it. [`payload`] is held to it by a test.
pub const PLAIN: &str = "Sends your message, the footplate version and this system (OS), and your email if you add it, to yardr's PostHog in the EU. Nothing else is sent.";

/// Where feedback goes: the PostHog project's capture address, its write key,
/// and the survey the messages answer. A test or a shot puts another where
/// the real one would be, so that nothing it sends leaves the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    address: Address,
    key: String,
    survey: String,
}

impl Project {
    /// yardr's own project, where it is offered: the constants, with the
    /// survey's still to come none, and so nothing to show.
    pub fn yardr() -> Option<Project> {
        Project::at(HOST, WRITE_KEY, SURVEY)
    }

    /// The project whose capture is at `host`, with the write `key` and the
    /// `survey`'s id. None where the key or the survey is empty, or the host is
    /// none: then there is no form to offer.
    pub fn at(host: &str, key: &str, survey: &str) -> Option<Project> {
        let (key, survey) = (key.trim(), survey.trim());
        if key.is_empty() || survey.is_empty() {
            return None;
        }
        Some(Project {
            address: Address::parse(host).ok()?,
            key: key.to_string(),
            survey: survey.to_string(),
        })
    }
}

/// Why a message did not go, in words for a person: shown, never parsed.
#[derive(Debug)]
pub enum Refusal {
    /// The system gave no random bytes for the send's id: nothing was sent.
    NoId,
    /// Nothing answered at PostHog's address: the connection failed, or the
    /// answer was cut off.
    Down(io::Error),
    /// PostHog gave no answer in time. `sent` says the request went out: a
    /// connect that gave up sent nothing.
    Slow { sent: bool },
    /// PostHog answered, and did not take the event: any status but 200.
    Answered(u16),
    /// What answered is no answer of an HTTP server's.
    Unreadable(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoId => f.write_str("this system gave no random id to send it under"),
            Refusal::Down(err) => write!(f, "PostHog does not answer ({err})"),
            Refusal::Slow { sent: false } => f.write_str("PostHog did not answer in time"),
            Refusal::Slow { sent: true } => {
                f.write_str("PostHog did not answer in time, and may have taken it")
            }
            Refusal::Answered(status) => write!(f, "PostHog answered {status}"),
            Refusal::Unreadable(what) => {
                write!(f, "PostHog answered what footplate cannot read: {what}")
            }
        }
    }
}

impl From<api::Error> for Refusal {
    fn from(err: api::Error) -> Self {
        match err {
            api::Error::Down(err) => Refusal::Down(err),
            api::Error::Slow { sent, .. } => Refusal::Slow { sent },
            api::Error::Protocol(what) => Refusal::Unreadable(what),
            // The answers of a yard and an edge: a post to a site that is
            // neither reads none of them.
            other => Refusal::Unreadable(other.to_string()),
        }
    }
}

/// The event one submission is: a survey's answer, from a sender no one can
/// tell from another. `email` is a property only where one was given.
fn payload(project: &Project, distinct_id: &str, message: &str, email: &str) -> Value {
    let mut properties = json!({
        SURVEY_ID: project.survey,
        SURVEY_RESPONSE: message,
        "footplate_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    });
    if !email.is_empty() {
        properties["email"] = json!(email);
    }
    json!({
        "api_key": project.key,
        "event": EVENT,
        "distinct_id": distinct_id,
        "properties": properties,
    })
}

/// 16 bytes as the text of a UUID of version 4, which is how an id made of
/// nothing but chance is usually written.
fn uuid_of(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex[..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..].concat()
    )
}

/// A new id for one submission, from the system's random bytes by way of the
/// TLS stack's provider, which api.rs speaks through already. It is not kept:
/// the next send makes its own.
fn fresh_id() -> Result<String, Refusal> {
    let mut bytes = [0u8; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| Refusal::NoId)?;
    Ok(uuid_of(bytes))
}

/// Sends `message` to the project as a survey's answer, with `email` where
/// the person gave one, and answers whether PostHog took it. Both are sent as
/// they are trimmed; a message is required, and the form asks for one before
/// it comes here. Called off the window's thread: it waits for the answer as
/// long as a call to a yard does.
pub fn send(project: &Project, message: &str, email: &str) -> Result<(), Refusal> {
    let body = payload(project, &fresh_id()?, message.trim(), email.trim()).to_string();
    match api::post_json(&project.address, CAPTURE, &body)? {
        200 => Ok(()),
        status => Err(Refusal::Answered(status)),
    }
}

/// What the form says when a message did not go: that it could not be sent,
/// why, that what was typed is still there, and to try again.
pub fn not_sent(why: &Refusal) -> String {
    format!("It could not be sent: {why}. What you wrote is still here; try again.")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::fake::Posthog;

    /// The project of a test: the stand-in's address, and a key and a survey
    /// that are none of the real ones.
    fn project_at(posthog: &Posthog) -> Project {
        Project::at(posthog.address(), "phc_test_key", "survey-test-id").unwrap()
    }

    /// The JSON body of the request the stand-in kept as its `n`th.
    fn body_of(posthog: &Posthog, n: usize) -> Value {
        serde_json::from_str(&posthog.sent()[n].1).unwrap()
    }

    #[test]
    fn what_is_sent_is_the_survey_s_answer_and_four_named_properties() {
        let posthog = Posthog::start();
        let project = project_at(&posthog);
        send(&project, "  The rail hides my longest depot name.\n", "").unwrap();
        send(&project, "Dark mode, please.", " ada@example.com ").unwrap();

        let sent = posthog.sent();
        assert_eq!(sent.len(), 2);
        // One POST to the capture path, with the body's length and type.
        let head = &sent[0].0;
        assert!(head.starts_with("POST /i/v0/e/ HTTP/1.1\r\n"), "{head}");
        assert!(
            head.contains("Content-Type: application/json\r\n"),
            "{head}"
        );
        assert!(
            head.contains(&format!("Content-Length: {}\r\n", sent[0].1.len())),
            "{head}"
        );

        let (first, second) = (body_of(&posthog, 0), body_of(&posthog, 1));
        // The message as typed but for its ends; no email where none was
        // given, and the email trimmed where one was.
        assert_eq!(
            first["properties"],
            json!({
                "$survey_id": "survey-test-id",
                "$survey_response": "The rail hides my longest depot name.",
                "footplate_version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
            })
        );
        assert_eq!(
            second["properties"],
            json!({
                "$survey_id": "survey-test-id",
                "$survey_response": "Dark mode, please.",
                "footplate_version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "email": "ada@example.com",
            })
        );
        // Nothing but the key, the event, the id and the properties.
        for body in [&first, &second] {
            assert_eq!(body["api_key"], "phc_test_key");
            assert_eq!(body["event"], "survey sent");
            let keys: BTreeSet<&str> = body
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from(["api_key", "distinct_id", "event", "properties"])
            );
        }
        // Each send is its own sender: the ids are fresh, UUID-shaped, and
        // not the same twice.
        let ids: Vec<&str> = [&first, &second]
            .into_iter()
            .map(|body| body["distinct_id"].as_str().unwrap())
            .collect();
        assert_ne!(ids[0], ids[1]);
        for id in ids {
            assert_eq!(id.len(), 36, "{id}");
            assert_eq!(id.as_bytes()[14], b'4', "{id}");
            assert!(
                id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
                "{id}"
            );
        }
    }

    /// The request is the write key's alone: none of what a yard or an edge
    /// is spoken to with rides on it.
    #[test]
    fn the_request_carries_no_token_session_or_device() {
        let posthog = Posthog::start();
        send(&project_at(&posthog), "hello", "").unwrap();

        let head = posthog.sent()[0].0.clone();
        let names: BTreeSet<String> = head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        assert_eq!(
            names,
            BTreeSet::from(
                [
                    "host",
                    "user-agent",
                    "content-type",
                    "content-length",
                    "connection"
                ]
                .map(String::from)
            )
        );
        assert!(!head.contains("Bearer"), "{head}");
        assert!(!head.contains("phc_test_key"), "{head}");
    }

    #[test]
    fn what_posthog_does_not_take_is_not_sent_and_says_why() {
        let posthog = Posthog::start();
        let project = project_at(&posthog);
        posthog.answer(401);
        let Err(refused) = send(&project, "hello", "") else {
            panic!("a 401 was taken for sent");
        };
        assert!(matches!(refused, Refusal::Answered(401)), "{refused:?}");
        assert_eq!(
            not_sent(&refused),
            "It could not be sent: PostHog answered 401. What you wrote is still here; try again."
        );
        // Anything but 200: a redirect is not followed, and a 204 is none
        // that capture gives.
        for status in [204, 301, 400, 413, 429, 503] {
            posthog.answer(status);
            assert!(
                matches!(send(&project, "hello", ""), Err(Refusal::Answered(s)) if s == status),
                "{status}"
            );
        }

        // Nothing listens: the connection is refused, and that is said.
        let gone = Posthog::start();
        let lost = project_at(&gone);
        gone.stop();
        let Err(refused) = send(&lost, "hello", "") else {
            panic!("a send to nothing was taken for sent");
        };
        assert!(matches!(refused, Refusal::Down(_)), "{refused:?}");
        assert!(
            refused.to_string().starts_with("PostHog does not answer ("),
            "{refused}"
        );
    }

    #[test]
    fn a_slow_answer_says_whether_the_message_went_out() {
        for (slow, words) in [
            (
                api::Error::Slow {
                    idempotent: false,
                    sent: false,
                },
                "PostHog did not answer in time",
            ),
            (
                api::Error::Slow {
                    idempotent: false,
                    sent: true,
                },
                "PostHog did not answer in time, and may have taken it",
            ),
        ] {
            assert_eq!(Refusal::from(slow).to_string(), words);
        }
        assert_eq!(
            Refusal::from(api::Error::Protocol("no HTTP status".into())).to_string(),
            "PostHog answered what footplate cannot read: no HTTP status"
        );
    }

    /// No build offers a form that sends nowhere: the project is none while
    /// the key or the survey is empty, or the host is no address.
    #[test]
    fn a_project_needs_its_key_its_survey_and_a_host() {
        let host = "http://127.0.0.1:1";
        assert!(Project::at(host, "phc_key", "survey").is_some());
        for (host, key, survey) in [
            (host, "", "survey"),
            (host, "phc_key", ""),
            (host, "  ", "survey"),
            (host, "phc_key", " \n"),
            ("eu.i.posthog.com", "phc_key", "survey"),
            ("http://example.com", "phc_key", "survey"),
        ] {
            assert_eq!(
                Project::at(host, key, survey),
                None,
                "{host} {key:?} {survey:?}"
            );
        }
        // The constants: the real host is https and PostHog's EU ingestion,
        // and the key is a write key, as it stands in the source.
        let real = Address::parse(HOST).unwrap();
        assert_eq!(real.to_string(), "https://eu.i.posthog.com");
        assert!(WRITE_KEY.starts_with("phc_") && WRITE_KEY.trim() == WRITE_KEY);
    }

    /// The line the form says is held to what is sent: every property that
    /// is a person's is named in it, and the payload has no other.
    #[test]
    fn the_plain_line_names_all_that_is_sent() {
        assert_eq!(
            PLAIN,
            "Sends your message, the footplate version and this system (OS), and your email if you add it, to yardr's PostHog in the EU. Nothing else is sent."
        );
        for named in [
            "your message",
            "the footplate version",
            "this system (OS)",
            "your email if you add it",
            "PostHog in the EU",
            "Nothing else is sent.",
        ] {
            assert!(PLAIN.contains(named), "{named}");
        }
        let project = Project::at("http://127.0.0.1:1", "phc_key", "survey").unwrap();
        let body = payload(&project, "id", "message", "jo@example.net");
        let properties: BTreeSet<&str> = body["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            properties,
            BTreeSet::from([
                "$survey_id",
                "$survey_response",
                "email",
                "footplate_version",
                "os"
            ])
        );
    }

    #[test]
    fn an_id_is_a_version_4_uuid_made_of_chance() {
        assert_eq!(uuid_of([0; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_of([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let ids: BTreeSet<String> = (0..32).map(|_| fresh_id().unwrap()).collect();
        assert_eq!(ids.len(), 32);
    }
}
