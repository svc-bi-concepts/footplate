//! GPT-Live's events as the harness speaks them: the ones it sends, the ones
//! it reads, and the plain rules the design (sections 3.3 and 3.4) puts in
//! the voice module, here as functions with tests: what text a delegation
//! stands for, how an answer is cut for the 500-token limit, how a typed
//! answer is set beside what was spoken, and whether the model heard itself.
//!
//! Field names are OpenAI's primary-websocket reference as of 31 Oct 2026
//! (developers.openai.com/api/reference/resources/live/primary-websocket).

use base64::Engine;
use serde_json::{Value, json};

/// The session's instructions: the design's section 3.4 text, with the two
/// headings OpenAI's prompting guide says to keep (Backchannel policy and
/// Interruption policy), which the design's text leaves out.
pub const INSTRUCTIONS: &str =
    "You are the voice of footplate, the window onto this yard. Speak plainly
and briefly. While you wait for the yard, say \"let me check\" or \"let me
think\"; never say you will ask anyone, and never name the machinery. If
a decision waits in the window, take no answer to it; tell the person
it waits there. The person is talking with {counterpart} in the yard;
everything that needs the yard is delegated, and footplate sends it as
their message and brings the answer back for you to speak. Speak each
answer briefly in your own words from what footplate gives you; the
written mail in the window is the record, so add nothing to an answer
that is not in what you were given.

Backchannel policy: Use no backchannels; listen until the person is done.

Interruption policy: Stop speaking when the user interrupts. Listen to what they say.

Delegation policy:
Backend tools:
- The open conversation: ask {counterpart}, and hear their answer.

Generate <|SpawnThinking|> when:
- The person asks anything of the yard, of {counterpart}, or that needs
  a fact you have not already been given in this session.
- A correction changes what was already asked.
- The person asks to hear an answer again; footplate gives it again.

Do not generate <|SpawnThinking|> when:
- The person greets you.
- You need a brief clarification before the request makes sense.

Generate <|SpawnThinking|> before giving an answer that depends on the
yard's work. Do not guess the result while waiting, and do not state a
yard fact you have not been given in this session.";

pub fn instructions(counterpart: &str) -> String {
    INSTRUCTIONS.replace("{counterpart}", counterpart)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Append {
    /// Spoken, in the model's own words.
    Commentary,
    /// Not spoken; usable in later replies. An empty one cancels the pending
    /// automatic acknowledgement.
    Thinking,
    /// A standing instruction, which may interrupt speech in progress.
    Instructions,
}

impl Append {
    pub fn name(self) -> &'static str {
        match self {
            Append::Commentary => "commentary",
            Append::Thinking => "thinking",
            Append::Instructions => "instructions",
        }
    }
}

pub fn session_start(instructions: &str, ack_delay_ms: u32, rate: u32) -> Value {
    json!({
        "type": "session.start",
        "event_id": "h-start",
        "session": {
            "model": "gpt-live-1",
            "instructions": instructions,
            "audio": { "format": { "type": "audio/pcm", "rate": rate } },
            "delegation": { "type": "client", "acknowledgement_delay_ms": ack_delay_ms },
            "store": false
        }
    })
}

pub fn input_audio_append(pcm: &[i16]) -> Value {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    json!({
        "type": "session.input_audio.append",
        "audio": base64::engine::general_purpose::STANDARD.encode(bytes)
    })
}

pub fn append(kind: Append, event_id: &str, delegation_id: Option<&str>, content: &str) -> Value {
    json!({
        "type": format!("session.{}.append", kind.name()),
        "event_id": event_id,
        "delegation_id": delegation_id,
        "content": content
    })
}

pub fn session_close() -> Value {
    json!({ "type": "session.close", "event_id": "h-close" })
}

/// What `session.started` says the session came out as ("the resolved session
/// configuration, including server defaults", the page says): the audio the
/// harness sends and plays, the voice, and who handles delegations. A server
/// that resolved the audio otherwise than the harness asked would explain
/// sound that is the wrong speed or none.
#[derive(Debug, Default, PartialEq)]
pub struct Resolved {
    pub format: Option<String>,
    pub rate: Option<u64>,
    pub voice: Option<String>,
    pub delegation: Option<String>,
    pub ack_delay_ms: Option<u64>,
}

impl Resolved {
    pub fn words(&self) -> String {
        let said = |value: &Option<String>| value.clone().unwrap_or_else(|| "not given".into());
        format!(
            "audio {} at {} Hz, voice {}, delegation {}{}",
            said(&self.format),
            self.rate
                .map_or_else(|| "not given".into(), |rate| rate.to_string()),
            said(&self.voice),
            said(&self.delegation),
            self.ack_delay_ms
                .map(|ms| format!(" (acknowledgement_delay_ms {ms})"))
                .unwrap_or_default(),
        )
    }

    /// Words for a session whose audio came out other than the PCM at `rate`
    /// that the harness sends and plays; nothing where it did, or where the
    /// server said nothing of its audio.
    pub fn audio_differs(&self, rate: u32) -> Option<String> {
        match (self.format.as_deref(), self.rate) {
            (None, None) => None,
            (Some("audio/pcm"), Some(got)) if got == u64::from(rate) => None,
            (format, got) => Some(format!(
                "the server resolved the audio as {} at {} Hz, and the harness sends and plays audio/pcm at {rate} Hz: \
                 what it hears and says would be at the wrong speed",
                format.unwrap_or("no format"),
                got.map_or_else(|| "no".into(), |got| got.to_string()),
            )),
        }
    }
}

/// What the server says, as far as the harness reads it.
#[derive(Debug, PartialEq)]
pub enum Server {
    Started {
        id: String,
        expires_at: Option<i64>,
        resolved: Resolved,
    },
    InputTranscript {
        text: String,
        start_ms: u64,
        end_ms: u64,
    },
    OutputTranscript {
        text: String,
        start_ms: u64,
        end_ms: u64,
    },
    /// 16-bit samples at the session's rate.
    OutputAudio(Vec<i16>),
    Delegation {
        id: String,
        offset_ms: u64,
        target: String,
    },
    Appended {
        kind: Append,
        client_event_id: Option<String>,
    },
    Usage {
        seconds: f64,
    },
    Closed {
        reason: String,
        seconds: Option<f64>,
    },
    Error {
        code: String,
        message: String,
        client_event_id: Option<String>,
    },
    /// Anything else, named and kept short for the log.
    Other(String, String),
}

pub fn parse(text: &str) -> Result<Server, String> {
    let v: Value = serde_json::from_str(text).map_err(|err| format!("not JSON: {err}"))?;
    let kind = v["type"].as_str().ok_or("an event with no type")?;
    let string = |value: &Value| value.as_str().unwrap_or_default().to_string();
    let ms = |value: &Value| value.as_u64().unwrap_or_default();
    // A string as it is, and anything else (a custom voice is an object) as its JSON.
    let plain = |value: &Value| match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    };
    Ok(match kind {
        "session.started" => Server::Started {
            id: string(&v["session"]["id"]),
            expires_at: v["session"]["expires_at"].as_i64(),
            resolved: Resolved {
                format: plain(&v["session"]["audio"]["format"]["type"]),
                rate: v["session"]["audio"]["format"]["rate"].as_u64(),
                voice: plain(&v["session"]["audio"]["output"]["voice"]),
                delegation: plain(&v["session"]["delegation"]["type"]),
                ack_delay_ms: v["session"]["delegation"]["acknowledgement_delay_ms"].as_u64(),
            },
        },
        "session.input_transcript.delta" => Server::InputTranscript {
            text: string(&v["delta"]),
            start_ms: ms(&v["start_ms"]),
            end_ms: ms(&v["end_ms"]),
        },
        "session.output_transcript.delta" => Server::OutputTranscript {
            text: string(&v["delta"]),
            start_ms: ms(&v["start_ms"]),
            end_ms: ms(&v["end_ms"]),
        },
        "session.output_audio.delta" => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(v["delta"].as_str().unwrap_or_default())
                .map_err(|err| format!("audio delta is not base64: {err}"))?;
            if bytes.len() % 2 != 0 {
                return Err("an audio delta of an odd number of bytes".into());
            }
            Server::OutputAudio(
                bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect(),
            )
        }
        "session.delegation.created" => Server::Delegation {
            id: string(&v["delegation"]["id"]),
            offset_ms: ms(&v["offset_ms"]),
            target: string(&v["delegation"]["target"]),
        },
        "session.commentary.appended"
        | "session.thinking.appended"
        | "session.instructions.appended" => Server::Appended {
            kind: match kind {
                "session.commentary.appended" => Append::Commentary,
                "session.thinking.appended" => Append::Thinking,
                _ => Append::Instructions,
            },
            client_event_id: v["client_event_id"].as_str().map(str::to_string),
        },
        "session.usage.updated" => Server::Usage {
            seconds: v["usage"]["seconds"].as_f64().unwrap_or_default(),
        },
        "session.closed" => Server::Closed {
            reason: string(&v["reason"]),
            seconds: v["usage"]["seconds"].as_f64(),
        },
        "error" => Server::Error {
            code: string(&v["error"]["code"]),
            message: string(&v["error"]["message"]),
            client_event_id: v["error"]["client_event_id"]
                .as_str()
                .or(v["client_event_id"].as_str())
                .map(str::to_string),
        },
        other => {
            let mut raw = v.to_string();
            if raw.len() > 300 {
                let mut end = 300;
                while !raw.is_char_boundary(end) {
                    end -= 1;
                }
                raw.truncate(end);
                raw.push('…');
            }
            Server::Other(other.to_string(), raw)
        }
    })
}

// ---- the plain rules ---------------------------------------------------------

/// An answer cut below the 500-token append limit: at sentence ends first and
/// at words where a sentence is longer than the limit. UTF-8 bytes are a safe
/// upper bound on tokens, even where a character takes several bytes.
pub fn chunk_text(text: &str, max_bytes: usize) -> Vec<String> {
    assert!(max_bytes >= 4, "the limit must fit one UTF-8 character");
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        let piece = current.trim();
        if !piece.is_empty() {
            chunks.push(current.clone());
        }
        current.clear();
    };
    // Sentences keep their ending and the space after it.
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, (at, c)) in chars.iter().enumerate() {
        let ends = matches!(c, '.' | '!' | '?' | '\n')
            && chars
                .get(i + 1)
                .is_none_or(|(_, next)| next.is_whitespace());
        if ends {
            let end = at + c.len_utf8();
            sentences.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        sentences.push(&text[start..]);
    }
    for sentence in sentences {
        if current.len() + sentence.len() <= max_bytes {
            current.push_str(sentence);
            continue;
        }
        flush(&mut current, &mut chunks);
        if sentence.len() <= max_bytes {
            current.push_str(sentence);
            continue;
        }
        // A sentence longer than the limit: by words, and a word longer than
        // the limit at UTF-8 character boundaries.
        for word in sentence.split_inclusive(char::is_whitespace) {
            if current.len() + word.len() > max_bytes {
                flush(&mut current, &mut chunks);
            }
            if word.len() > max_bytes {
                let mut part = String::new();
                for character in word.chars() {
                    if part.len() + character.len_utf8() > max_bytes {
                        chunks.push(std::mem::take(&mut part));
                    }
                    part.push(character);
                }
                if !part.is_empty() {
                    chunks.push(part);
                }
            } else {
                current.push_str(word);
            }
        }
    }
    flush(&mut current, &mut chunks);
    chunks
}

/// One piece of a transcript, with its place on the session's timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The text a delegation stands for, taken the way section 3.3 step 2 has it:
/// what the person said since the last delegation, joined as it came. A new
/// utterance begins where the timeline has a gap of `gap_ms`; the fragments
/// carry their own spacing, so a gap only adds one space.
pub fn task_from(fragments: &[Fragment], gap_ms: u64) -> String {
    let mut task = String::new();
    let mut last_end = None;
    for fragment in fragments {
        if let Some(end) = last_end
            && fragment.start_ms >= end + gap_ms
            && !task.ends_with(' ')
        {
            task.push(' ');
        }
        task.push_str(&fragment.text);
        last_end = Some(fragment.end_ms);
    }
    task.trim().to_string()
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Little words that say nothing about whether a fact was kept or added.
const FILLER: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "it", "its", "it's", "that",
    "this", "of", "to", "and", "or", "in", "on", "at", "for", "with", "as", "by", "so", "but",
    "if", "you", "your", "i", "i'm", "we", "they", "them", "their", "there", "just", "now",
    "still", "has", "have", "had", "will", "would", "can", "could", "do", "does", "did", "not",
    "no", "yes", "then", "than", "from", "up", "out", "about", "also", "here", "what", "which",
    "who", "let", "me", "know", "says", "said", "say", "tell", "told",
];

/// A typed answer set beside what was spoken, for a person to judge a
/// paraphrase by: how much of what was typed was kept, and which words were
/// said that were not typed. Rough, on words, and only a pointer to read the
/// two lines by.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub kept: f64,
    pub added: Vec<String>,
}

pub fn compare(typed: &str, spoken: &str) -> Comparison {
    let content = |text: &str| -> Vec<String> {
        words(text)
            .into_iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .collect()
    };
    let typed = content(typed);
    let spoken = content(spoken);
    let kept = if typed.is_empty() {
        1.0
    } else {
        typed.iter().filter(|w| spoken.contains(w)).count() as f64 / typed.len() as f64
    };
    let mut added: Vec<String> = Vec::new();
    for word in &spoken {
        if !typed.contains(word) && !added.contains(word) {
            added.push(word.clone());
        }
    }
    Comparison { kept, added }
}

/// Whether a piece of what the person is heard to say is the model's own
/// recent speech coming back through the microphone (a speaker without a
/// headset): most of its words were spoken by the model in the last moments.
pub fn echo_suspect(heard: &str, recent_output: &str) -> bool {
    let heard = words(heard);
    if heard.len() < 3 {
        return false;
    }
    let spoken = words(recent_output);
    let shared = heard.iter().filter(|w| spoken.contains(w)).count();
    shared * 10 >= heard.len() * 7
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(text: &str, start_ms: u64, end_ms: u64) -> Fragment {
        Fragment {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn the_instructions_carry_the_designs_delegation_words() {
        let text = instructions("yardmaster");
        assert!(!text.contains("{counterpart}"));
        assert!(text.contains("The person is talking with yardmaster in the yard"));
        for words in [
            "let me check",
            "never say you will ask anyone",
            "Generate <|SpawnThinking|> when:",
            "Do not generate <|SpawnThinking|> when:",
            "do not state a\nyard fact you have not been given",
            "Interruption policy:",
        ] {
            assert!(text.contains(words), "{words:?} is missing");
        }
    }

    #[test]
    fn what_is_sent_has_the_references_field_names() {
        let start = session_start("hi", 800, 24000);
        assert_eq!(start["type"], "session.start");
        assert_eq!(start["session"]["model"], "gpt-live-1");
        assert_eq!(start["session"]["audio"]["format"]["type"], "audio/pcm");
        assert_eq!(start["session"]["audio"]["format"]["rate"], 24000);
        assert_eq!(start["session"]["delegation"]["type"], "client");
        assert_eq!(
            start["session"]["delegation"]["acknowledgement_delay_ms"],
            800
        );
        assert_eq!(start["session"]["store"], false);
        let audio = input_audio_append(&[1, -2, 3]);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(audio["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(bytes, [1, 0, 0xfe, 0xff, 3, 0]);
        let answer = append(Append::Commentary, "e1", Some("del_1"), "It shipped.");
        assert_eq!(answer["type"], "session.commentary.append");
        assert_eq!(answer["delegation_id"], "del_1");
        assert_eq!(answer["content"], "It shipped.");
        let general = append(Append::Thinking, "e2", None, "");
        assert_eq!(general["type"], "session.thinking.append");
        assert!(general["delegation_id"].is_null());
    }

    #[test]
    fn what_is_read_is_the_references_events() {
        let started = parse(
            r#"{"type":"session.started","event_id":"e","session":{"id":"live_1","expires_at":1788307200,"status":"active","model":"gpt-live-1"}}"#,
        )
        .unwrap();
        assert_eq!(
            started,
            Server::Started {
                id: "live_1".into(),
                expires_at: Some(1788307200),
                resolved: Resolved::default()
            }
        );
        // The page's own example of what the server resolved the session as.
        let Server::Started { resolved, .. } = parse(
            r#"{"type":"session.started","event_id":"evt_started_001","client_event_id":"evt_start_001","session":{"id":"live_abc123","model":"gpt-live-1","status":"active","expires_at":1788555600,"instructions":"Help.","input":[],"audio":{"format":{"type":"audio/pcm","rate":24000},"output":{"voice":"marin"}},"delegation":{"type":"client","acknowledgement_delay_ms":800}}}"#,
        )
        .unwrap() else {
            panic!("not a started session");
        };
        assert_eq!(
            resolved.words(),
            "audio audio/pcm at 24000 Hz, voice marin, delegation client (acknowledgement_delay_ms 800)"
        );
        assert_eq!(resolved.audio_differs(24000), None);
        // Audio resolved otherwise than the harness sends and plays is said; silence about it is not.
        let at_16k = Resolved {
            rate: Some(16000),
            ..resolved
        };
        assert!(
            at_16k
                .audio_differs(24000)
                .is_some_and(|why| why.contains("audio/pcm at 16000 Hz"))
        );
        assert_eq!(Resolved::default().audio_differs(24000), None);
        assert_eq!(
            Resolved::default().words(),
            "audio not given at not given Hz, voice not given, delegation not given"
        );
        assert_eq!(
            parse(r#"{"type":"session.delegation.created","event_id":"e","offset_ms":3600,"delegation":{"id":"del_abc","type":"delegation","target":"client"}}"#).unwrap(),
            Server::Delegation { id: "del_abc".into(), offset_ms: 3600, target: "client".into() }
        );
        assert_eq!(
            parse(r#"{"type":"session.output_transcript.delta","event_id":"e","delta":"Hi","start_ms":5400,"end_ms":7200}"#).unwrap(),
            Server::OutputTranscript { text: "Hi".into(), start_ms: 5400, end_ms: 7200 }
        );
        assert_eq!(
            parse(r#"{"type":"session.output_audio.delta","delta":"AQAC/w=="}"#).unwrap(),
            Server::OutputAudio(vec![1, -254])
        );
        assert!(parse(r#"{"type":"session.output_audio.delta","delta":"AQ=="}"#).is_err());
        assert_eq!(
            parse(r#"{"type":"session.commentary.appended","event_id":"e","client_event_id":"c1","start_ms":1,"end_ms":2}"#).unwrap(),
            Server::Appended { kind: Append::Commentary, client_event_id: Some("c1".into()) }
        );
        assert_eq!(
            parse(r#"{"type":"error","event_id":"e","error":{"type":"invalid_request_error","code":"unknown_parameter","message":"Unknown parameter.","client_event_id":"c2"}}"#).unwrap(),
            Server::Error {
                code: "unknown_parameter".into(),
                message: "Unknown parameter.".into(),
                client_event_id: Some("c2".into())
            }
        );
        assert_eq!(
            parse(r#"{"type":"session.closed","reason":"expired","usage":{"seconds":12.5},"session":{}}"#).unwrap(),
            Server::Closed { reason: "expired".into(), seconds: Some(12.5) }
        );
        assert!(matches!(
            parse(r#"{"type":"info","code":"x","message":"y"}"#).unwrap(),
            Server::Other(kind, _) if kind == "info"
        ));
        assert!(parse("[1]").is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn an_answer_is_cut_at_sentences_then_words() {
        assert_eq!(chunk_text("One. Two! Three?", 100), ["One. Two! Three?"]);
        assert_eq!(
            chunk_text("One. Two! Three?", 8).concat(),
            "One. Two! Three?"
        );
        let pieces = chunk_text("alpha beta gamma delta epsilon", 12);
        assert_eq!(pieces.concat(), "alpha beta gamma delta epsilon");
        assert!(pieces.iter().all(|p| p.len() <= 12));
        // A word longer than the limit is cut by characters; nothing is lost.
        let pieces = chunk_text("abcdefghij", 4);
        assert_eq!(pieces.concat(), "abcdefghij");
        assert!(pieces.iter().all(|p| p.len() <= 4));
        let unicode = "界🙂界";
        let pieces = chunk_text(unicode, 4);
        assert_eq!(pieces.concat(), unicode);
        assert!(pieces.iter().all(|p| p.len() <= 4));
        // A version number's dots do not end a sentence.
        assert_eq!(
            chunk_text("It is v1.2.3 now. Done.", 18).concat(),
            "It is v1.2.3 now. Done."
        );
        // The pieces, joined, are the answer's words in order.
        let long = "The release waits on the Windows zip. ".repeat(60);
        let pieces = chunk_text(&long, 300);
        assert!(pieces.len() > 5 && pieces.iter().all(|p| p.len() <= 300));
        assert_eq!(pieces.concat(), long.trim());
        assert_eq!(words(&pieces.join(" ")), words(&long));
        assert!(chunk_text("   ", 10).is_empty());
    }

    #[test]
    fn a_task_is_what_the_person_said_since_the_last_delegation() {
        let said = [
            frag("What's blocking", 1000, 1800),
            frag(" the release?", 1800, 2500),
            frag("And who", 4000, 4400),
            frag(" owns it?", 4400, 5000),
        ];
        assert_eq!(
            task_from(&said, 700),
            "What's blocking the release? And who owns it?"
        );
        assert_eq!(task_from(&said[..2], 700), "What's blocking the release?");
        assert_eq!(task_from(&[], 700), "");
    }

    #[test]
    fn a_typed_answer_is_set_beside_what_was_spoken() {
        let typed = "Everything else has landed. The Windows zip is still building.";
        let said = "Everything else has landed, and the Windows zip is still building.";
        let same = compare(typed, said);
        assert!(same.kept > 0.99, "{same:?}");
        assert!(same.added.is_empty(), "{same:?}");
        let invented = compare(
            typed,
            "Everything has landed except the Windows zip, due by four.",
        );
        assert!(invented.kept < 1.0);
        assert!(invented.added.contains(&"four".to_string()), "{invented:?}");
        assert_eq!(compare("", "anything").kept, 1.0);
    }

    #[test]
    fn the_models_own_words_through_the_microphone_are_noticed() {
        let spoken = "everything else has landed and the windows zip is still building";
        assert!(echo_suspect("the windows zip is still building", spoken));
        assert!(!echo_suspect("tell them to land without it", spoken));
        assert!(!echo_suspect("okay", spoken));
    }
}
