//! A stand-in for OpenAI's Live socket, on a loopback port: the server side of
//! the upgrade and of the frames, and a scripted session behind it. It lets
//! the harness's framing and event handling be tested with no key and no
//! network, and lets a person see what the harness logs (`--rehearse`) before
//! spending a minute of a real session.
//!
//! The script is the shape the design expects of GPT-Live, as small as it can
//! be: the person's words as transcript fragments, a delegation sent as a
//! fragmented message with a ping between its fragments, an acknowledgement
//! spoken 900 ms later unless an append came first, the answer spoken as a
//! paraphrase with audio sent ahead of real time, speech that stops when the
//! person's voice comes in, a big message, a ping, and a close. Between its
//! speech the audio goes on in silence at the pace of the clock, as the first
//! live run showed the real server's does (32 s of audio in a 33 s session in
//! which the model said nothing); `pad_silence` off is the stream with the
//! silences left out. It does not pretend to be the model: what it says is
//! fixed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};

use crate::audio::{self, PlayQueue, RATE};
use crate::live;
use crate::session::{self, Config, Io, Log, Msg, Report};
use crate::ws::{self, Assembler, Decoder, Incoming};

pub struct Script {
    /// The key the stand-in accepts as a bearer; another is answered 401.
    pub key: String,
    /// The automatic acknowledgement comes this long after a delegation unless
    /// a thinking or commentary append came first.
    pub ack_after: Duration,
    /// What the model says for an answer, whatever the answer was.
    pub paraphrase: String,
    pub answer_audio_ms: u64,
    /// How far ahead of real time the answer's audio is sent.
    pub lead_ms: u64,
    /// Whether the stream goes on in silence, at the pace of the clock, while
    /// nothing is being said.
    pub pad_silence: bool,
}

impl Script {
    pub fn new(key: &str) -> Script {
        Script {
            key: key.into(),
            ack_after: Duration::from_millis(900),
            paraphrase:
                "Everything else has landed, and the Windows zip is still building, due by four."
                    .into(),
            answer_audio_ms: 2000,
            lead_ms: 600,
            pad_silence: true,
        }
    }
}

/// What the stand-in saw, for a test to ask of it.
#[derive(Default, Debug)]
pub struct Seen {
    pub authorization: Option<String>,
    pub path: String,
    pub start: Option<Value>,
    /// Every text message but audio, as it was received (reassembled).
    pub messages: Vec<Value>,
    pub audio_appends: usize,
    pub pongs: Vec<Vec<u8>>,
    pub protocol_error: Option<String>,
    pub closed_by_request: bool,
}

pub struct Standin {
    pub url: String,
    handle: JoinHandle<Seen>,
}

impl Standin {
    pub fn start(script: Script) -> Standin {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
        let url = format!("ws://{}/v1/live/sessions", listener.local_addr().unwrap());
        let handle = thread::spawn(move || serve(listener, script));
        Standin { url, handle }
    }

    pub fn finish(self) -> Seen {
        self.handle.join().expect("the stand-in did not panic")
    }
}

struct Speech {
    total_ms: u64,
    lead_ms: u64,
    began: Instant,
    sent_ms: u64,
}

struct Server {
    tcp: TcpStream,
    script: Script,
    seen: Seen,
    started: Option<Instant>,
    delegations: usize,
    ack_due: Option<Instant>,
    speech: Option<Speech>,
    /// How much audio has gone out on the stream, speech and padding together.
    pos_ms: u64,
    loud: usize,
    interrupted: bool,
    asked_first: bool,
}

fn tone(ms: u64, from_ms: u64) -> Vec<u8> {
    let samples = (ms * RATE as u64 / 1000) as usize;
    let first = (from_ms * RATE as u64 / 1000) as usize;
    (0..samples)
        .flat_map(|i| {
            let t = (first + i) as f32 / RATE as f32;
            let v = (3000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()) as i16;
            v.to_le_bytes()
        })
        .collect()
}

fn serve(listener: TcpListener, script: Script) -> Seen {
    let (mut tcp, _) = listener.accept().expect("the harness connects");
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut seen = Seen::default();
    // The upgrade: read the request, check the key, answer.
    let mut head = Vec::new();
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        match tcp.read(&mut chunk) {
            Ok(0) | Err(_) => return seen,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    };
    let text = String::from_utf8_lossy(&head[..end]).to_string();
    let mut lines = text.split("\r\n");
    seen.path = lines
        .next()
        .unwrap_or_default()
        .split(' ')
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let header = |name: &str| -> Option<String> {
        text.split("\r\n")
            .filter_map(|l| l.split_once(':'))
            .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    };
    seen.authorization = header("authorization");
    if seen.authorization.as_deref() != Some(&format!("Bearer {}", script.key)) {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-test***KEY. You can find your API key at https://platform.openai.com.","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        let _ = write!(
            tcp,
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        return seen;
    }
    let key = header("sec-websocket-key").unwrap_or_default();
    let _ = write!(
        tcp,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        ws::accept_key(&key)
    );
    tcp.set_read_timeout(Some(Duration::from_millis(5))).ok();
    let mut decoder = Decoder::new(true);
    decoder.push(&head[end + 4..]);
    let mut assembler = Assembler::default();
    let mut server = Server {
        tcp,
        script,
        seen,
        started: None,
        delegations: 0,
        ack_due: None,
        speech: None,
        pos_ms: 0,
        loud: 0,
        interrupted: false,
        asked_first: false,
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    'serve: while Instant::now() < deadline {
        loop {
            match decoder.next_frame() {
                Ok(Some(frame)) => match assembler.accept(frame) {
                    Ok(Some(Incoming::Text(text))) => {
                        if server.on_text(&text) {
                            break 'serve;
                        }
                    }
                    Ok(Some(Incoming::Pong(body))) => server.seen.pongs.push(body),
                    Ok(Some(Incoming::Ping(body))) => server.frame(true, ws::PONG, &body),
                    Ok(Some(Incoming::Close(..))) => {
                        server.frame(true, ws::CLOSE, &1000u16.to_be_bytes());
                        break 'serve;
                    }
                    Ok(Some(Incoming::Binary(_))) | Ok(None) => {}
                    Err(err) => {
                        server.seen.protocol_error = Some(err.to_string());
                        break 'serve;
                    }
                },
                Ok(None) => break,
                Err(err) => {
                    server.seen.protocol_error = Some(err.to_string());
                    break 'serve;
                }
            }
        }
        server.timers();
        match server.tcp.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => decoder.push(&chunk[..n]),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    server.seen
}

impl Server {
    fn frame(&mut self, fin: bool, opcode: u8, payload: &[u8]) {
        let _ = self
            .tcp
            .write_all(&ws::encode_frame(fin, opcode, payload, None));
    }

    fn send(&mut self, event: Value) {
        self.frame(true, ws::TEXT, event.to_string().as_bytes());
    }

    fn now_ms(&self) -> u64 {
        self.started
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }

    /// Sends a message cut in three, with a ping between the first two parts.
    fn send_fragmented(&mut self, event: Value) {
        let text = event.to_string();
        let bytes = text.as_bytes();
        let (a, b) = (bytes.len() / 3, bytes.len() * 2 / 3);
        self.frame(false, ws::TEXT, &bytes[..a]);
        self.frame(true, ws::PING, b"mid-fragment");
        self.frame(false, ws::CONTINUATION, &bytes[a..b]);
        self.frame(true, ws::CONTINUATION, &bytes[b..]);
    }

    fn speak(&mut self, text: &str, total_ms: u64, lead_ms: u64) {
        let now = self.now_ms();
        let words: Vec<&str> = text.split_inclusive(' ').collect();
        let (first, second) = (
            words[..words.len() / 2].concat(),
            words[words.len() / 2..].concat(),
        );
        let half = total_ms / 2;
        self.send(json!({"type":"session.output_transcript.delta","event_id":"s-t1","delta":first,"start_ms":now,"end_ms":now + half}));
        self.send(json!({"type":"session.output_transcript.delta","event_id":"s-t2","delta":second,"start_ms":now + half,"end_ms":now + total_ms}));
        self.speech = Some(Speech {
            total_ms,
            lead_ms,
            began: Instant::now(),
            sent_ms: 0,
        });
        self.interrupted = false;
        self.loud = 0;
    }

    fn ask(&mut self, words: [&str; 2]) {
        let now = self.now_ms();
        self.send(json!({"type":"session.input_transcript.delta","event_id":"s-i1","delta":words[0],"start_ms":now.saturating_sub(900),"end_ms":now.saturating_sub(400)}));
        self.send(json!({"type":"session.input_transcript.delta","event_id":"s-i2","delta":words[1],"start_ms":now.saturating_sub(400),"end_ms":now}));
        self.delegations += 1;
        self.send_fragmented(json!({
            "type":"session.delegation.created","event_id":format!("s-d{}", self.delegations),"offset_ms":now,
            "delegation":{"id":format!("del_{}", self.delegations),"type":"delegation","target":"client"}
        }));
        self.ack_due = Some(Instant::now() + self.script.ack_after);
    }

    /// True when the session is over.
    fn on_text(&mut self, text: &str) -> bool {
        let Ok(event) = serde_json::from_str::<Value>(text) else {
            self.seen.protocol_error = Some("the harness sent text that is not JSON".into());
            return true;
        };
        match event["type"].as_str().unwrap_or_default() {
            "session.start" => {
                self.seen.start = Some(event.clone());
                self.started = Some(Instant::now());
                let expires = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() + 3600)
                    .unwrap_or(0);
                // As the page's example resolves it: what was asked, and the default voice.
                let asked = &event["session"];
                self.send(json!({"type":"session.started","event_id":"s-start","session":{
                    "id":"live_standin","expires_at":expires,"status":"active","model":"gpt-live-1",
                    "audio":{"format":asked["audio"]["format"].clone(),"output":{"voice":"marin"}},
                    "delegation":asked["delegation"].clone()
                }}));
                // A message past 64 KiB, which takes the 64-bit length.
                self.send(json!({"type":"info","event_id":"s-info","code":"padding","message":"x".repeat(70_000)}));
                self.frame(true, ws::PING, b"keepalive-1");
            }
            "session.input_audio.append" => {
                self.seen.audio_appends += 1;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(event["audio"].as_str().unwrap_or_default())
                    .unwrap_or_default();
                let pcm: Vec<i16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if audio::rms_dbfs(&pcm) > -35.0 {
                    self.loud += 1;
                    if self.loud >= 2 && self.speech.is_some() {
                        // The model yields: no more audio.
                        self.interrupted = true;
                    }
                    if self.loud == 5 && self.asked_first && self.delegations == 1 {
                        self.ask(["Wait, tell them", " to land without it."]);
                    }
                } else {
                    self.loud = 0;
                }
                if self.seen.audio_appends == 10 && !self.asked_first {
                    self.asked_first = true;
                    self.ask(["What's blocking", " the release?"]);
                }
            }
            "session.commentary.append" | "session.thinking.append" => {
                self.seen.messages.push(event.clone());
                self.ack_due = None;
                let kind = event["type"]
                    .as_str()
                    .unwrap_or_default()
                    .replace(".append", ".appended");
                let now = self.now_ms();
                self.send(json!({"type":kind,"event_id":"s-ack","client_event_id":event["event_id"],"start_ms":now,"end_ms":now}));
                if event["type"] == "session.commentary.append"
                    && event["content"].as_str().is_some_and(|c| !c.is_empty())
                {
                    let paraphrase = self.script.paraphrase.clone();
                    let (total, lead) = (self.script.answer_audio_ms, self.script.lead_ms);
                    self.speak(&paraphrase, total, lead);
                }
            }
            "session.close" => {
                self.seen.messages.push(event.clone());
                self.seen.closed_by_request = true;
                let seconds = self.now_ms() as f64 / 1000.0;
                self.send(json!({"type":"session.closed","event_id":"s-closed","reason":"close_requested","usage":{"seconds":seconds},"session":{"id":"live_standin","status":"active"}}));
                self.frame(true, ws::CLOSE, &1000u16.to_be_bytes());
                return true;
            }
            _ => self.seen.messages.push(event),
        }
        false
    }

    fn timers(&mut self) {
        if let Some(due) = self.ack_due
            && Instant::now() >= due
        {
            self.ack_due = None;
            self.speak("Let me check.", 500, 500);
        }
        if self.interrupted {
            self.speech = None;
        }
        let Some(speech) = self.speech.as_mut() else {
            self.pad();
            return;
        };
        let due = (speech.began.elapsed().as_millis() as u64 + speech.lead_ms).min(speech.total_ms);
        let mut bytes = Vec::new();
        while speech.sent_ms + 20 <= due {
            bytes.extend(tone(20, speech.sent_ms));
            speech.sent_ms += 20;
            self.pos_ms += 20;
        }
        let done = speech.sent_ms >= speech.total_ms;
        if !bytes.is_empty() {
            let delta = base64::engine::general_purpose::STANDARD.encode(&bytes);
            self.send(json!({"type":"session.output_audio.delta","delta": delta}));
        }
        if done {
            self.speech = None;
        }
    }

    /// While nothing is being said the stream goes on in silence, at the pace
    /// of the clock: speech that was sent ahead of the clock leaves the stream
    /// ahead of it, and no silence goes out until the clock has caught up.
    fn pad(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        if !self.script.pad_silence {
            return;
        }
        let wall = started.elapsed().as_millis() as u64;
        let mut bytes = Vec::new();
        while self.pos_ms + 20 <= wall {
            bytes.extend(std::iter::repeat_n(0u8, 2 * (RATE as usize / 50)));
            self.pos_ms += 20;
        }
        if !bytes.is_empty() {
            let delta = base64::engine::general_purpose::STANDARD.encode(&bytes);
            self.send(json!({"type":"session.output_audio.delta","delta": delta}));
        }
    }
}

/// A whole session against the stand-in, with fake devices and a driver that
/// plays the person: waits for the acknowledgement, types an answer, talks
/// over the answer, answers the correction at once, and ends it. Used by the
/// test and by `--rehearse`.
pub fn rehearsal(log: Log, barge_in: bool, key: &str) -> Result<(Report, Seen), String> {
    rehearse(log, barge_in, Script::new(key), Duration::from_secs(1))
}

/// `rehearsal` with the script and the interval of the SYNC lines chosen.
pub fn rehearse(
    log: Log,
    barge_in: bool,
    script: Script,
    sync_every: Duration,
) -> Result<(Report, Seen), String> {
    let bearer = format!("Bearer {}", script.key);
    let standin = Standin::start(script);
    let url = ws::Url::parse(&standin.url).map_err(|e| e.to_string())?;
    let conn = ws::Conn::connect(&url, &[("Authorization", bearer.as_str())])
        .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    let (out_tx, out_rx) = mpsc::channel();
    let socket = session::spawn_socket(conn, tx.clone(), out_rx);
    let queue = PlayQueue::new();
    let devices = audio::start_fake(session::mic_sink(tx.clone()), queue.clone());
    let driver = {
        let (tx, queue, log, speak) = (
            tx.clone(),
            queue.clone(),
            log.clone(),
            devices.speak.clone(),
        );
        thread::spawn(move || {
            let wait = |what: &str, mut ready: Box<dyn FnMut() -> bool>| {
                let until = Instant::now() + Duration::from_secs(15);
                while !ready() {
                    if Instant::now() > until {
                        log.line("DRIVER", &format!("gave up waiting for {what}"));
                        return false;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                true
            };
            let q = queue.clone();
            // The acknowledgement of the first delegation is speech arriving:
            // audio alone is not, the stream goes on in silence.
            if !wait(
                "the first acknowledgement",
                Box::new(move || q.stats().speech_rx_ms > 0),
            ) {
                return;
            }
            let _ = tx.send(Msg::Line(
                "Everything else has landed. The Windows zip is still building.".into(),
            ));
            let q = queue.clone();
            // The answer's speech is queued: talk over it.
            if !wait(
                "the answer's speech",
                Box::new(move || q.stats().speech_rx_ms >= 1300 && q.stats().queued_ms > 100),
            ) {
                return;
            }
            speak.store(true, Ordering::SeqCst);
            let l = log.clone();
            if !wait(
                "the second delegation",
                Box::new(move || {
                    l.lines()
                        .iter()
                        .any(|line| line.contains("DELEGATION") && line.contains(" d2 "))
                }),
            ) {
                return;
            }
            speak.store(false, Ordering::SeqCst);
            // The answer comes a little after the correction, as one does, and
            // inside the delay the acknowledgement was given.
            thread::sleep(Duration::from_millis(500));
            let _ = tx.send(Msg::Line("Told them to land without it.".into()));
            thread::sleep(Duration::from_millis(1500));
            let _ = tx.send(Msg::Line("/status".into()));
            let _ = tx.send(Msg::Line("/q".into()));
        })
    };
    let cfg = Config {
        counterpart: "yardmaster".into(),
        instructions: live::instructions("yardmaster"),
        barge_in,
        sync_every,
        ..Config::default()
    };
    let io = Io {
        rx,
        out: out_tx,
        queue,
        log,
        stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let result = session::run(cfg, io);
    drop(devices);
    let _ = driver.join();
    let _ = socket.join();
    let seen = standin.finish();
    result.map(|report| (report, seen))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Ack;

    /// Not a key: a string that must appear nowhere the harness writes.
    const KEY: &str = "standin-secret-0f3a9c1d77be";

    fn lines_with<'a>(lines: &'a [String], word: &str) -> Vec<&'a String> {
        lines.iter().filter(|l| l.contains(word)).collect()
    }

    #[test]
    fn a_whole_session_is_framed_read_and_logged() {
        let log = Log::memory();
        // SYNC lines every 300 ms, so that the silent start and the loud middle
        // each have one whatever the machine's pace.
        let (report, seen) = rehearse(
            log.clone(),
            true,
            Script::new(KEY),
            Duration::from_millis(300),
        )
        .expect("the session ran");
        let lines = log.lines();
        let text = lines.join("\n");

        // The upgrade carried the key, and the key is nowhere in what was written.
        assert_eq!(seen.authorization, Some(format!("Bearer {KEY}")));
        assert_eq!(seen.path, "/v1/live/sessions");
        assert!(!text.contains(KEY), "the key reached the log");
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        // The framing held both ways: every client frame was masked (the
        // stand-in refuses one that is not), the pings were answered with
        // their own bytes, a 70 KB message and fragmented ones were read.
        assert_eq!(seen.protocol_error, None);
        let mut pongs = seen.pongs.clone();
        pongs.sort();
        assert_eq!(
            pongs,
            [
                b"keepalive-1".to_vec(),
                b"mid-fragment".to_vec(),
                b"mid-fragment".to_vec()
            ]
        );
        assert!(report.pings >= 3, "{} pings seen", report.pings);
        let big = lines_with(&lines, "EVENT");
        assert_eq!(big.len(), 1);
        assert!(big[0].contains("info") && big[0].len() < 400, "{}", big[0]);

        // What the session asked for.
        let start = seen.start.expect("session.start was sent");
        assert_eq!(start["session"]["model"], "gpt-live-1");
        assert_eq!(start["session"]["delegation"]["type"], "client");
        assert_eq!(
            start["session"]["delegation"]["acknowledgement_delay_ms"],
            800
        );
        assert_eq!(start["session"]["audio"]["format"]["rate"], 24000);
        assert_eq!(start["session"]["store"], false);
        assert!(
            start["session"]["instructions"]
                .as_str()
                .unwrap()
                .contains("talking with yardmaster")
        );
        assert!(seen.audio_appends >= 10);

        // Delegation seen, with the task taken from the transcript.
        assert_eq!(report.delegations.len(), 2);
        let (d1, d2) = (&report.delegations[0], &report.delegations[1]);
        assert_eq!((d1.id.as_str(), d2.id.as_str()), ("del_1", "del_2"));
        assert_eq!(d1.task, "What's blocking the release?");
        assert_eq!(d2.task, "Wait, tell them to land without it.");
        assert!(
            lines_with(&lines, "caption: \"What's blocking the release?\"").len() == 1,
            "{text}"
        );

        // The typed answer, under its delegation's id, beside what was spoken.
        let appends: Vec<_> = seen
            .messages
            .iter()
            .filter(|m| m["type"] == "session.commentary.append")
            .collect();
        assert_eq!(appends.len(), 2);
        assert_eq!(appends[0]["delegation_id"], "del_1");
        assert_eq!(
            appends[0]["content"],
            "Everything else has landed. The Windows zip is still building."
        );
        assert_eq!(appends[1]["delegation_id"], "del_2");
        assert_eq!(d1.answer.as_deref(), appends[0]["content"].as_str());
        assert_eq!(
            d1.spoken.as_deref(),
            Some("Everything else has landed, and the Windows zip is still building, due by four.")
        );
        let comparison = d1.comparison.as_ref().unwrap();
        assert_eq!(comparison.added, ["due", "four"]);
        assert!(comparison.kept > 0.99);
        assert_eq!(lines_with(&lines, "COMPARE").len(), 2);
        assert_eq!(report.append_acks.len(), 2);

        // The acknowledgement: spoken at the delay when nothing was appended,
        // cancelled where the answer came at once.
        assert!(
            matches!(d1.ack, Ack::Spoke { early: false, ms } if (800..1500).contains(&ms)),
            "{:?}",
            d1.ack
        );
        assert!(
            matches!(d2.ack, Ack::Cancelled { append_ms } if append_ms < 800),
            "{:?}",
            d2.ack
        );
        assert!(!lines_with(&lines, "ACK-SPEECH").is_empty());

        // The interruption: the queue was dropped and the sound stopped soon.
        assert_eq!(report.interruptions.len(), 1, "{text}");
        let interruption = &report.interruptions[0];
        assert!(interruption.queued_ms > 100, "{interruption:?}");
        assert!(
            interruption.dropped_ms.is_some_and(|ms| ms > 0),
            "{interruption:?}"
        );
        assert!(
            interruption.stop_took_ms.is_some_and(|ms| ms < 400),
            "{interruption:?}"
        );
        // What the server sent in the moments before it heard the person was
        // thrown away in the holdoff, not played again.
        assert!(
            interruption.last_delta_after_ms.is_none_or(|ms| ms < 400),
            "{interruption:?}"
        );

        // Captions against the audio's position were sampled, and the end is clean.
        assert!(
            report.input_lag.len() >= 3,
            "{} input samples",
            report.input_lag.len()
        );
        assert!(!report.output_lead.is_empty() && !report.timeline_vs_wall.is_empty());
        assert!(seen.closed_by_request);
        assert_eq!(report.closed_reason.as_deref(), Some("close_requested"));
        assert!(report.finished_cleanly);
        assert!(
            report
                .limit_secs
                .is_some_and(|s| (3500..=3600).contains(&s))
        );
        assert!(lines_with(&lines, "SUMMARY").len() >= 6);

        // What the server resolved is said, and said to match what was sent.
        assert_eq!(
            lines_with(&lines, "the server resolved it as").len(),
            1,
            "{text}"
        );
        assert!(
            text.contains("audio audio/pcm at 24000 Hz, voice marin, delegation client"),
            "{text}"
        );
        assert!(!text.contains("the server resolved the audio as"), "{text}");

        // Levels on the SYNC lines, in both directions, silent and loud: the
        // microphone is silent until the driver speaks, and the server's stream
        // is padding until the first acknowledgement is spoken. Exact zeros are
        // said as such, and speech as its RMS and peak.
        let syncs = lines_with(&lines, "SYNC");
        let has = |what: &str| syncs.iter().any(|line| line.contains(what));
        assert!(has("microphone this interval: digital silence"), "{text}");
        assert!(has("microphone this interval: rms -"), "{text}");
        assert!(has("model audio this interval: digital silence"), "{text}");
        assert!(has("model audio this interval: rms -"), "{text}");
        assert!(has("above the -50 dBFS speech floor"), "{text}");
        assert!(has("level gate open"), "{text}");
        // And over the whole session, in the summary and in the report; and so
        // far, on /status, without waiting for the next SYNC.
        let levels = lines_with(&lines, "levels over the whole session");
        assert_eq!(levels.len(), 1, "{text}");
        let status = lines_with(&lines, "STATUS");
        assert_eq!(status.len(), 1, "{text}");
        assert!(
            status[0].contains("levels so far: microphone rms -")
                && status[0].contains("; model audio rms -"),
            "{}",
            status[0]
        );
        let (mic, model) = (report.mic_level.unwrap(), report.model_level.unwrap());
        assert!(!mic.is_digital_silence() && mic.marked_ms > 0, "{mic:?}");
        assert!(
            (mic.peak_db() + 14.7).abs() < 0.2,
            "the fake microphone speaks at 6000: {mic:?}"
        );
        assert!(
            !model.is_digital_silence() && model.marked_ms > 1000,
            "{model:?}"
        );
        assert!(
            (model.peak_db() + 20.8).abs() < 0.2,
            "the stand-in speaks at 3000: {model:?}"
        );
        assert!(
            model.samples > samples_of(model.marked_ms),
            "padding is in what was received, but is not speech: {model:?}"
        );

        // The old shape: nothing is sent between the speech. The
        // acknowledgement and interruption read as they do with padding, and
        // what is not there says so, not that it was silent.
        let log = Log::memory();
        let script = Script {
            pad_silence: false,
            ..Script::new(KEY)
        };
        let (report, seen) = rehearse(log.clone(), true, script, Duration::from_millis(300))
            .expect("the session ran");
        let text = log.lines().join("\n");
        assert_eq!(seen.protocol_error, None);
        assert_eq!(report.delegations.len(), 2, "{text}");
        assert!(
            matches!(report.delegations[0].ack, Ack::Spoke { early: false, ms } if (800..1500).contains(&ms)),
            "{:?}",
            report.delegations[0].ack
        );
        assert!(
            matches!(report.delegations[1].ack, Ack::Cancelled { append_ms } if append_ms < 800),
            "{:?}",
            report.delegations[1].ack
        );
        assert_eq!(report.interruptions.len(), 1, "{text}");
        assert!(
            report.interruptions[0]
                .stop_took_ms
                .is_some_and(|ms| ms < 400),
            "{:?}",
            report.interruptions[0]
        );
        assert!(
            text.contains("model audio this interval: no audio"),
            "{text}"
        );
        assert!(
            !text.contains("model audio this interval: digital silence"),
            "{text}"
        );
        let model = report.model_level.unwrap();
        assert_eq!(model.samples, samples_of(model.marked_ms), "{model:?}");
    }

    /// The samples in `ms` of audio.
    fn samples_of(ms: u64) -> u64 {
        ms * RATE as u64 / 1000
    }

    #[test]
    fn without_a_local_drop_the_speech_plays_out() {
        let log = Log::memory();
        let (report, seen) = rehearsal(log.clone(), false, KEY).expect("the session ran");
        assert_eq!(seen.protocol_error, None);
        assert_eq!(report.interruptions.len(), 1, "{}", log.lines().join("\n"));
        let interruption = &report.interruptions[0];
        assert_eq!(interruption.dropped_ms, None);
        assert_eq!(interruption.discarded_ms, 0);
        assert!(interruption.queued_ms > 100);
        // Nothing was dropped, so the queue plays out: the sound stops late
        // (or, here, after the session's end, which is also said).
        assert!(
            interruption.stop_took_ms.is_none_or(|ms| ms > 300),
            "{interruption:?}"
        );
    }

    #[test]
    fn a_refused_upgrade_is_said_without_the_key() {
        let standin = Standin::start(Script::new("the-right-key"));
        let url = ws::Url::parse(&standin.url).unwrap();
        let bearer = format!("Bearer {KEY}");
        let err = match ws::Conn::connect(&url, &[("Authorization", bearer.as_str())]) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("a wrong key was accepted"),
        };
        assert!(err.contains("401"), "{err}");
        assert!(err.contains("invalid_api_key"), "{err}");
        assert!(!err.contains(KEY) && !err.contains("sk-test"), "{err}");
        assert!(err.contains("sk-…"), "{err}");
        assert_eq!(standin.finish().authorization, Some(bearer));
    }

    #[test]
    fn a_long_message_goes_out_in_fragments_and_arrives_whole() {
        let standin = Standin::start(Script::new(KEY));
        let url = ws::Url::parse(&standin.url).unwrap();
        let bearer = format!("Bearer {KEY}");
        let mut conn = ws::Conn::connect(&url, &[("Authorization", bearer.as_str())]).unwrap();
        conn.max_frame = 16;
        conn.set_poll(Duration::from_millis(5)).unwrap();
        let long = json!({"type":"session.thinking.append","event_id":"e1","delegation_id":null,"content":"héllo ".repeat(40)});
        conn.send_text(&long.to_string()).unwrap();
        conn.send_ping(b"are you there").unwrap();
        conn.send_close(1000, "").unwrap();
        // Read until the stand-in's close comes back.
        let until = Instant::now() + Duration::from_secs(5);
        let (mut closed, mut ponged) = (false, false);
        while !closed && Instant::now() < until {
            for message in conn.poll().unwrap() {
                closed |= matches!(message, Incoming::Close(..));
                ponged |= message == Incoming::Pong(b"are you there".to_vec());
            }
        }
        assert!(closed, "the stand-in answered the close");
        assert!(ponged, "the stand-in answered the ping with its own bytes");
        let seen = standin.finish();
        assert_eq!(seen.protocol_error, None);
        assert_eq!(seen.messages, [long]);
    }
}
