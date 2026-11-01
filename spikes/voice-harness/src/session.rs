//! The harness's session: one thread that takes everything that happens (the
//! server's events, the microphone's chunks, a typed line) in the order it
//! comes, sends what the session needs, and writes down what a person reads
//! back afterwards: the delegation seen, the typed answer beside what was
//! spoken, how long an interruption took to stop the sound, an
//! acknowledgement before the 800 ms delay and its cancellation by an append,
//! and the captions against the audio's position.
//!
//! The socket has a thread of its own (`spawn_socket`), as the yard's event
//! stream does in footplate's src/session.rs: it polls with a short read
//! timeout and between polls sends what the session queued.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::audio::{self, ONSET_CHUNKS, PlayQueue, Vad, VadEvent};
use crate::live::{self, Append, Fragment, Server};
use crate::ws::{self, Incoming};

/// What reaches the session's thread.
pub enum Msg {
    Server(Instant, Server),
    /// Twenty milliseconds of what the person said, mono 16-bit at 24 kHz.
    Mic(Vec<i16>),
    Line(String),
    /// A ping from the server, answered by the socket's thread.
    Ping,
    Note(String),
    /// The socket ended.
    Down(String),
}

/// The microphone's chunks, as the session's messages.
pub fn mic_sink(tx: Sender<Msg>) -> audio::MicSink {
    Box::new(move |chunk| {
        let _ = tx.send(Msg::Mic(chunk));
    })
}

/// What the session asks the socket's thread to send.
pub enum Out {
    Text(String),
    Close,
}

pub struct Config {
    pub counterpart: String,
    pub instructions: String,
    pub ack_delay_ms: u32,
    /// Send an empty thinking append the moment a delegation arrives (the
    /// guide's way to keep the automatic acknowledgement from being spoken).
    pub quiet_on_delegation: bool,
    /// Drop the queued speech the moment the person is heard to speak over it.
    pub barge_in: bool,
    /// After a drop, audio that arrives within this long is thrown away too:
    /// what the server had already sent, or sends before it hears the person.
    pub holdoff: Duration,
    pub vad_db: f32,
    pub stop_after: Option<Duration>,
    /// Answer every delegation with this text after this many milliseconds
    /// where nobody has typed one: to try an append inside the delay.
    pub auto_answer: Option<(u64, String)>,
    pub sync_every: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            counterpart: "yardmaster".into(),
            instructions: String::new(),
            ack_delay_ms: 800,
            quiet_on_delegation: false,
            barge_in: false,
            holdoff: Duration::from_millis(300),
            vad_db: -35.0,
            stop_after: None,
            auto_answer: None,
            sync_every: Duration::from_secs(15),
        }
    }
}

/// The log: what is written to stderr and, where there is a file, to it, a
/// line at a time, each with its time since the harness began. The key is
/// never given to it.
#[derive(Clone)]
pub struct Log {
    start: Instant,
    sink: Arc<Mutex<Sink>>,
}

struct Sink {
    file: Option<File>,
    quiet: bool,
    lines: Vec<String>,
}

impl Log {
    pub fn new(file: Option<File>, quiet: bool) -> Log {
        Log {
            start: Instant::now(),
            sink: Arc::new(Mutex::new(Sink {
                file,
                quiet,
                lines: Vec::new(),
            })),
        }
    }

    /// A log that keeps its lines in memory and prints nothing.
    #[cfg(test)]
    pub fn memory() -> Log {
        Log::new(None, true)
    }

    pub fn line(&self, kind: &str, text: &str) {
        let secs = self.start.elapsed().as_secs_f64();
        let mut body = text.lines();
        let first = body.next().unwrap_or_default();
        let mut lines = vec![format!("[{secs:8.3}s] {kind:<9} {first}")];
        lines.extend(body.map(|more| format!("{:>21}{more}", "")));
        let mut sink = self.sink.lock().unwrap();
        for line in lines {
            if !sink.quiet {
                eprintln!("{line}");
            }
            if let Some(file) = sink.file.as_mut() {
                let _ = writeln!(file, "{line}");
            }
            sink.lines.push(line);
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.sink.lock().unwrap().lines.clone()
    }
}

/// What came of an acknowledgement.
#[derive(Clone, Debug, PartialEq)]
pub enum Ack {
    /// The model spoke before anything was appended, `ms` after the
    /// delegation; `early` is before the delay it was given.
    Spoke {
        ms: u64,
        early: bool,
    },
    /// An append at `append_ms`, inside the delay, and no speech before it.
    Cancelled {
        append_ms: u64,
    },
    /// An append after the delay had passed and no speech before it.
    NoneBeforeAppend {
        append_ms: u64,
    },
    Pending,
}

pub fn classify_ack(
    delay_ms: u64,
    first_speech_ms: Option<u64>,
    first_append_ms: Option<u64>,
) -> Ack {
    match (first_speech_ms, first_append_ms) {
        (Some(ms), _) => Ack::Spoke {
            ms,
            early: ms < delay_ms,
        },
        (None, Some(append_ms)) if append_ms < delay_ms => Ack::Cancelled { append_ms },
        (None, Some(append_ms)) => Ack::NoneBeforeAppend { append_ms },
        (None, None) => Ack::Pending,
    }
}

impl Ack {
    fn words(&self, delay_ms: u64) -> String {
        match self {
            Ack::Spoke { ms, early: true } => {
                format!("spoke {ms} ms after the delegation: BEFORE the {delay_ms} ms delay")
            }
            Ack::Spoke { ms, early: false } => {
                format!("spoke {ms} ms after the delegation: at or after the {delay_ms} ms delay")
            }
            Ack::Cancelled { append_ms } => format!(
                "an append at {append_ms} ms, inside the {delay_ms} ms delay, and no speech came first: CANCELLED"
            ),
            Ack::NoneBeforeAppend { append_ms } => format!(
                "no speech before the append at {append_ms} ms, which was after the {delay_ms} ms delay"
            ),
            Ack::Pending => "nothing spoken yet, nothing appended".into(),
        }
    }
}

/// Least squares of `(seconds, value)`, as value per minute.
pub fn slope_per_min(samples: &[(f64, f64)]) -> Option<f64> {
    if samples.len() < 3 {
        return None;
    }
    let n = samples.len() as f64;
    let (sx, sy) = samples
        .iter()
        .fold((0.0, 0.0), |(x, y), s| (x + s.0, y + s.1));
    let (mx, my) = (sx / n, sy / n);
    let num: f64 = samples.iter().map(|s| (s.0 - mx) * (s.1 - my)).sum();
    let den: f64 = samples.iter().map(|s| (s.0 - mx).powi(2)).sum();
    if den < 1e-9 {
        return None;
    }
    Some(num / den * 60.0)
}

/// The microphone's level for a log line, with how long the level gate was
/// open in it: a gate that never closes is a noisy room or someone talking.
fn mic_words(level: &audio::Level) -> String {
    if level.samples == 0 || level.is_digital_silence() {
        return level.to_string();
    }
    format!(
        "{level} (level gate open {} of {} ms)",
        level.marked_ms,
        audio::samples_ms(level.samples)
    )
}

/// The model's audio level for a log line, with how much of it was above the
/// speech floor: the rest is the server's padding.
fn model_words(level: &audio::Level) -> String {
    if level.samples == 0 || level.is_digital_silence() {
        return level.to_string();
    }
    format!(
        "{level} ({} of {} ms above the {} dBFS speech floor)",
        level.marked_ms,
        audio::samples_ms(level.samples),
        audio::SOUND_DB
    )
}

fn series_words(name: &str, samples: &[(f64, f64)]) -> String {
    let Some(first) = samples.first() else {
        return format!("{name}: no samples");
    };
    let last = samples.last().unwrap();
    let (lo, hi) = samples.iter().fold((f64::MAX, f64::MIN), |(lo, hi), s| {
        (lo.min(s.1), hi.max(s.1))
    });
    let span = last.0 - first.0;
    match slope_per_min(samples) {
        Some(slope) => format!(
            "{name}: {} samples over {span:.0} s; first {:.0} ms, last {:.0} ms, range {lo:.0}..{hi:.0} ms, slope {slope:+.0} ms/min",
            samples.len(),
            first.1,
            last.1
        ),
        None => format!(
            "{name}: {} samples, too few for a slope; {:.0}..{:.0} ms",
            samples.len(),
            lo,
            hi
        ),
    }
}

/// What became of a delegation. The log is what a person reads; these are
/// what the tests read.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct DelegationReport {
    pub id: String,
    pub task: String,
    pub ack: Ack,
    pub answer: Option<String>,
    pub spoken: Option<String>,
    pub comparison: Option<live::Comparison>,
}

#[derive(Clone, Debug)]
pub struct Interruption {
    /// Audio queued and not yet played when the person was heard speaking.
    pub queued_ms: u64,
    /// What a local drop threw away, where `--barge-in` dropped.
    pub dropped_ms: Option<u64>,
    /// Audio that came within the holdoff after a drop and was thrown away.
    pub discarded_ms: u64,
    /// When the last audio from the server came after the first loud chunk.
    pub last_delta_after_ms: Option<u64>,
    /// From the person's first loud chunk to the speaker's being silent.
    pub stop_took_ms: Option<u64>,
}

#[derive(Default)]
pub struct Report {
    pub session_id: Option<String>,
    pub limit_secs: Option<i64>,
    pub closed_reason: Option<String>,
    pub usage_seconds: Option<f64>,
    pub delegations: Vec<DelegationReport>,
    pub interruptions: Vec<Interruption>,
    pub echo_suspects: usize,
    pub pings: usize,
    pub chunks_sent: u64,
    pub append_acks: Vec<(String, u64)>,
    pub errors: Vec<String>,
    /// (seconds since the session began, milliseconds), for the three drifts.
    pub timeline_vs_wall: Vec<(f64, f64)>,
    pub input_lag: Vec<(f64, f64)>,
    pub output_lead: Vec<(f64, f64)>,
    /// The level of the microphone's audio and of the model's over the whole session.
    pub mic_level: Option<audio::Level>,
    pub model_level: Option<audio::Level>,
    pub finished_cleanly: bool,
}

struct Pending {
    kind: Append,
    sent: Instant,
}

struct Delegation {
    id: String,
    n: usize,
    seen: Instant,
    task: String,
    first_speech: Option<Duration>,
    first_append: Option<Duration>,
    answer: Option<(String, Instant)>,
    spoken: Option<String>,
    comparison: Option<live::Comparison>,
}

/// The model's speech, collected from its transcript deltas until it stops
/// for more than two seconds (the SDK's own gap for a turn).
struct Utterance {
    text: String,
    start_ms: u64,
    end_ms: u64,
    last: Instant,
    /// The delegation whose answer had been sent before this began.
    answer_of: Option<usize>,
    ack_of: Option<usize>,
}

struct Interrupt {
    first_loud: Instant,
    queued_ms: u64,
    dropped_ms: Option<u64>,
    discarded_ms: u64,
    /// The last audio of the burst that went on from before the person spoke:
    /// a gap of `TAIL_GAP` ends the burst, and later audio is a new answer.
    last_delta: Option<Instant>,
    tail_closed: bool,
}

impl Interrupt {
    /// Audio of `ms` came from the server at `at`. Only speech counts: the
    /// stream goes on in padding, which would keep the tail open for as long as
    /// the interruption is watched.
    fn heard(&mut self, at: Instant, ms: u64, speech: bool, held_off: bool) {
        if !speech {
            return;
        }
        if !self.tail_closed {
            let since = self.last_delta.unwrap_or(self.first_loud);
            if at.duration_since(since) > TAIL_GAP {
                self.tail_closed = true;
            } else {
                self.last_delta = Some(at);
            }
        }
        if held_off {
            self.discarded_ms += ms;
        }
    }
}

/// A pause this long in the server's audio ends what it was still saying when
/// the person began to speak.
const TAIL_GAP: Duration = Duration::from_millis(250);

/// How long an interruption is watched: the server's last audio after it, and
/// the sound's stopping, are looked for in this time.
const WATCH: Duration = Duration::from_secs(1);
const WATCH_OVERDUE: Duration = Duration::from_secs(10);

pub struct Io {
    pub rx: Receiver<Msg>,
    pub out: Sender<Out>,
    pub queue: Arc<PlayQueue>,
    pub log: Log,
    pub stop: Arc<AtomicBool>,
}

const SPEECH_GAP: Duration = Duration::from_secs(2);
const START_TIMEOUT: Duration = Duration::from_secs(20);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_APPEND_BYTES: usize = 400;

/// The thread that owns the socket.
pub fn spawn_socket(mut conn: ws::Conn, tx: Sender<Msg>, out: Receiver<Out>) -> JoinHandle<()> {
    thread::spawn(move || {
        let _ = conn.set_poll(Duration::from_millis(5));
        let mut closing = None;
        loop {
            loop {
                match out.try_recv() {
                    Ok(Out::Text(text)) => {
                        if let Err(err) = conn.send_text(&text) {
                            let _ = tx.send(Msg::Down(format!("a send failed: {err}")));
                            return;
                        }
                    }
                    Ok(Out::Close) => {
                        let _ = conn.send_close(1000, "");
                        closing = Some(Instant::now());
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        let _ = conn.send_close(1000, "");
                        return;
                    }
                }
            }
            match conn.poll() {
                Ok(events) => {
                    for event in events {
                        match event {
                            Incoming::Text(text) => {
                                let msg = match live::parse(&text) {
                                    Ok(server) => Msg::Server(Instant::now(), server),
                                    Err(why) => Msg::Note(format!(
                                        "an event that could not be read ({why})"
                                    )),
                                };
                                if tx.send(msg).is_err() {
                                    return;
                                }
                            }
                            Incoming::Ping(_) => {
                                let _ = tx.send(Msg::Ping);
                            }
                            Incoming::Pong(_) => {}
                            Incoming::Binary(bytes) => {
                                let _ = tx.send(Msg::Note(format!(
                                    "a binary message of {} bytes",
                                    bytes.len()
                                )));
                            }
                            Incoming::Close(code, reason) => {
                                let _ = tx.send(Msg::Down(format!(
                                    "the server closed the connection ({}) {reason}",
                                    code.map(|c| c.to_string())
                                        .unwrap_or_else(|| "no code".into())
                                )));
                                return;
                            }
                        }
                    }
                }
                Err(err) => {
                    let _ = tx.send(Msg::Down(err.to_string()));
                    return;
                }
            }
            if closing.is_some_and(|since: Instant| since.elapsed() > Duration::from_secs(3)) {
                return;
            }
        }
    })
}

struct State {
    cfg: Config,
    io: Io,
    report: Report,
    started: Option<Instant>,
    delegations: Vec<Delegation>,
    input_frags: Vec<Fragment>,
    in_utterance: String,
    utterance: Option<Utterance>,
    recent_output: String,
    next_event: u64,
    pending: HashMap<String, Pending>,
    in_sent_ms: u64,
    vad: Vad,
    speaking: bool,
    /// What each direction measured, for the SYNC lines and the summary.
    mic_meter: audio::Meter,
    rx_meter: audio::Meter,
    /// Chunks in a row that were exact digital silence, and whether that was said.
    zero_run: u32,
    warned_zero: bool,
    interrupt: Option<Interrupt>,
    discard_until: Option<Instant>,
    closing: Option<Instant>,
    closed: bool,
    last_sync: Instant,
    last_usage_log: Option<Instant>,
}

pub fn run(cfg: Config, io: Io) -> Result<Report, String> {
    let begin = Instant::now();
    let vad = Vad::new(cfg.vad_db);
    let mut st = State {
        cfg,
        io,
        report: Report::default(),
        started: None,
        delegations: Vec::new(),
        input_frags: Vec::new(),
        in_utterance: String::new(),
        utterance: None,
        recent_output: String::new(),
        next_event: 0,
        pending: HashMap::new(),
        in_sent_ms: 0,
        vad,
        speaking: false,
        mic_meter: audio::Meter::default(),
        rx_meter: audio::Meter::default(),
        zero_run: 0,
        warned_zero: false,
        interrupt: None,
        discard_until: None,
        closing: None,
        closed: false,
        last_sync: begin,
        last_usage_log: None,
    };
    st.send(live::session_start(
        &st.cfg.instructions,
        st.cfg.ack_delay_ms,
        audio::RATE,
    ));
    st.io.log.line(
        "START",
        &format!(
            "session.start sent: gpt-live-1, client delegation, acknowledgement_delay_ms {}, pcm {} Hz, store false, barge-in {}, level gate {} dBFS",
            st.cfg.ack_delay_ms,
            audio::RATE,
            if st.cfg.barge_in { "drop" } else { "off (measure only)" },
            st.cfg.vad_db
        ),
    );
    loop {
        match st.io.rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Msg::Server(at, server)) => {
                if let Some(why) = st.on_server(at, server) {
                    return Err(why);
                }
            }
            Ok(Msg::Mic(chunk)) => st.on_mic(chunk),
            Ok(Msg::Line(line)) => st.on_line(&line),
            Ok(Msg::Ping) => {
                st.report.pings += 1;
                st.io.log.line(
                    "PING",
                    &format!(
                        "from the server, answered with a pong ({} so far)",
                        st.report.pings
                    ),
                );
            }
            Ok(Msg::Note(note)) => st.io.log.line("NOTE", &note),
            Ok(Msg::Down(why)) => {
                if st.started.is_none() {
                    return Err(format!(
                        "the connection ended before the session began: {why}"
                    ));
                }
                st.io.log.line("DOWN", &why);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if st.started.is_none() && begin.elapsed() > START_TIMEOUT {
            return Err("no session.started within 20 seconds".into());
        }
        st.tick();
        if st.closed {
            break;
        }
        if let Some(since) = st.closing
            && since.elapsed() > CLOSE_TIMEOUT
        {
            st.io.log.line(
                "CLOSING",
                "session.closed did not come within 15 s: final usage is unconfirmed",
            );
            break;
        }
    }
    st.finish();
    Ok(st.report)
}

impl State {
    fn event_id(&mut self, prefix: &str) -> String {
        self.next_event += 1;
        format!("{prefix}-{}", self.next_event)
    }

    fn send(&self, event: serde_json::Value) {
        let _ = self.io.out.send(Out::Text(event.to_string()));
    }

    fn secs(&self) -> f64 {
        self.started
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0)
    }

    fn wall_ms(&self) -> f64 {
        self.secs() * 1000.0
    }

    fn begin_close(&mut self, why: &str) {
        if self.closing.is_none() && self.started.is_some() {
            self.io.log.line(
                "CLOSING",
                &format!("{why}: session.close sent, waiting for session.closed"),
            );
            self.send(live::session_close());
            self.closing = Some(Instant::now());
        }
    }

    fn on_server(&mut self, at: Instant, server: Server) -> Option<String> {
        match server {
            Server::Started {
                id,
                expires_at,
                resolved,
            } => {
                self.started = Some(at);
                self.report.session_id = Some(id.clone());
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                self.report.limit_secs = expires_at.map(|e| e - now);
                let limit = match expires_at {
                    Some(e) => format!(
                        "expires_at {e}, which is {} s ({:.1} min) from now",
                        e - now,
                        (e - now) as f64 / 60.0
                    ),
                    None => "no expires_at".into(),
                };
                self.io
                    .log
                    .line("SESSION", &format!("started, id {id}; {limit}"));
                self.io.log.line(
                    "SESSION",
                    &format!("the server resolved it as: {}", resolved.words()),
                );
                if let Some(why) = resolved.audio_differs(audio::RATE) {
                    self.io.log.line("WARN", &why);
                }
            }
            Server::InputTranscript {
                text,
                start_ms,
                end_ms,
            } => self.on_input(at, text, start_ms, end_ms),
            Server::OutputTranscript {
                text,
                start_ms,
                end_ms,
            } => self.on_output_text(at, text, start_ms, end_ms),
            Server::OutputAudio(pcm) => {
                let ms = audio::samples_ms(pcm.len() as u64);
                // The server's stream runs on through its silences, so audio
                // arriving is not the model speaking: only audio above the floor is.
                let speech = audio::has_sound(&pcm);
                self.rx_meter.add(&pcm, speech);
                let held_off = self.discard_until.is_some_and(|until| at < until);
                if let Some(interrupt) = self.interrupt.as_mut() {
                    interrupt.heard(at, ms, speech, held_off);
                }
                if !held_off {
                    self.io.queue.push(&pcm);
                }
                if speech {
                    self.speech_began(at);
                }
            }
            Server::Delegation {
                id,
                offset_ms,
                target,
            } => self.on_delegation(at, id, offset_ms, target),
            Server::Appended {
                client_event_id, ..
            } => {
                if let Some(pending) =
                    client_event_id.and_then(|id| self.pending.remove(&id).map(|p| (id, p)))
                {
                    let ms = at.duration_since(pending.1.sent).as_millis() as u64;
                    self.report
                        .append_acks
                        .push((pending.1.kind.name().to_string(), ms));
                    self.io.log.line(
                        "APPEND-OK",
                        &format!(
                            "{} {} acknowledged {ms} ms after it was sent",
                            pending.1.kind.name(),
                            pending.0
                        ),
                    );
                }
            }
            Server::Usage { seconds } => {
                self.report.usage_seconds = Some(seconds);
                if self
                    .last_usage_log
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
                {
                    self.last_usage_log = Some(Instant::now());
                    self.io.log.line(
                        "USAGE",
                        &format!(
                            "{seconds:.1} s of voice so far (${:.3} at $0.05 a minute)",
                            seconds / 60.0 * 0.05
                        ),
                    );
                }
            }
            Server::Closed { reason, seconds } => {
                self.report.closed_reason = Some(reason.clone());
                if seconds.is_some() {
                    self.report.usage_seconds = seconds;
                }
                self.io.log.line(
                    "CLOSED",
                    &format!("session.closed, reason {reason}, final usage {seconds:?} s"),
                );
                self.closed = true;
            }
            Server::Error {
                code,
                message,
                client_event_id,
            } => {
                let text = format!(
                    "{code}: {} (for {})",
                    ws::redact(&message),
                    client_event_id.as_deref().unwrap_or("no event")
                );
                self.report.errors.push(text.clone());
                self.io.log.line("ERROR", &text);
                if self.started.is_none() {
                    return Some(format!("the session was refused: {text}"));
                }
            }
            Server::Other(kind, raw) => self
                .io
                .log
                .line("EVENT", &format!("{kind} {}", ws::redact(&raw))),
        }
        None
    }

    fn sample_sync(&mut self, out_end_ms: Option<u64>, in_end_ms: Option<u64>, end_ms: u64) {
        let secs = self.secs();
        self.report
            .timeline_vs_wall
            .push((secs, end_ms as f64 - self.wall_ms()));
        if let Some(end) = in_end_ms {
            self.report
                .input_lag
                .push((secs, self.in_sent_ms as f64 - end as f64));
        }
        if let Some(end) = out_end_ms {
            let played = self.io.queue.stats().played_ms;
            self.report
                .output_lead
                .push((secs, end as f64 - played as f64));
        }
    }

    fn on_input(&mut self, at: Instant, text: String, start_ms: u64, end_ms: u64) {
        let echo = self.report_recent_speech(at) && live::echo_suspect(&text, &self.recent_output);
        if echo {
            self.report.echo_suspects += 1;
        }
        // The caption grows as fragments come, and starts again after a gap.
        if let Some(last) = self.input_frags.last()
            && start_ms >= last.end_ms + 700
        {
            self.in_utterance.clear();
        }
        self.in_utterance.push_str(&text);
        self.input_frags.push(Fragment {
            text: text.clone(),
            start_ms,
            end_ms,
        });
        let stats = self.io.queue.stats();
        self.io.log.line(
            "IN",
            &format!(
                "[{start_ms}..{end_ms} ms] caption: {:?}{}   (sent {} ms, audio played {} ms)",
                self.in_utterance.trim(),
                if echo {
                    "   ECHO? this is the model's own words coming back through the microphone"
                } else {
                    ""
                },
                self.in_sent_ms,
                stats.played_ms
            ),
        );
        self.sample_sync(None, Some(end_ms), end_ms);
    }

    /// Whether the model spoke in the last few seconds (so that what the
    /// microphone heard may be its own voice).
    fn report_recent_speech(&self, at: Instant) -> bool {
        self.utterance
            .as_ref()
            .is_some_and(|u| at.duration_since(u.last) < Duration::from_secs(4))
            || self.io.queue.stats().queued_ms > 0
    }

    /// The model began to speak (audio above the floor, or words), for the
    /// acknowledgement.
    fn speech_began(&mut self, at: Instant) {
        for d in self.delegations.iter_mut() {
            if d.first_speech.is_none() && d.first_append.is_none() && d.answer.is_none() {
                d.first_speech = Some(at.duration_since(d.seen));
            }
        }
    }

    fn on_output_text(&mut self, at: Instant, text: String, start_ms: u64, end_ms: u64) {
        self.speech_began(at);
        match self.utterance.as_mut() {
            Some(u) => {
                u.text.push_str(&text);
                u.end_ms = end_ms;
                u.last = at;
            }
            None => {
                // Whose words these are: the answer last sent, if it was sent
                // before they began, or the acknowledgement of the newest delegation.
                let answer_of = self.delegations.iter().rposition(|d| {
                    d.answer.as_ref().is_some_and(|(_, sent)| *sent <= at) && d.spoken.is_none()
                });
                let ack_of = self.delegations.iter().rposition(|d| d.answer.is_none());
                self.utterance = Some(Utterance {
                    text: text.clone(),
                    start_ms,
                    end_ms,
                    last: at,
                    answer_of,
                    ack_of,
                });
            }
        }
        self.recent_output.push_str(&text);
        if self.recent_output.len() > 600 {
            let cut = self.recent_output.len() - 400;
            let cut = (cut..)
                .find(|i| self.recent_output.is_char_boundary(*i))
                .unwrap_or(0);
            self.recent_output.drain(..cut);
        }
        self.sample_sync(Some(end_ms), None, end_ms);
    }

    fn finish_utterance(&mut self) {
        let Some(u) = self.utterance.take() else {
            return;
        };
        let words = format!("[{}..{} ms] {:?}", u.start_ms, u.end_ms, u.text.trim());
        if let Some(i) = u.answer_of {
            let d = &mut self.delegations[i];
            let typed = d
                .answer
                .as_ref()
                .map(|(t, _)| t.clone())
                .unwrap_or_default();
            let comparison = live::compare(&typed, &u.text);
            self.io.log.line("SPOKEN", &format!("d{} {words}", d.n));
            self.io.log.line("COMPARE", &format!(
                "d{}\n  typed : {:?}\n  spoken: {:?}\n  {:.0}% of the typed content words were kept; words spoken that were not typed: {}",
                d.n,
                typed,
                u.text.trim(),
                comparison.kept * 100.0,
                if comparison.added.is_empty() { "none".into() } else { comparison.added.join(", ") }
            ));
            d.spoken = Some(u.text.trim().to_string());
            d.comparison = Some(comparison);
        } else if let Some(i) = u
            .ack_of
            .filter(|i| self.delegations[*i].first_speech.is_some())
        {
            self.io
                .log
                .line("ACK-SPEECH", &format!("d{} {words}", self.delegations[i].n));
        } else {
            self.io.log.line("SAID", &words);
        }
    }

    fn on_delegation(&mut self, at: Instant, id: String, offset_ms: u64, target: String) {
        let n = self.delegations.len() + 1;
        let task = live::task_from(&self.input_frags, 700);
        self.input_frags.clear();
        self.in_utterance.clear();
        self.io.log.line("DELEGATION", &format!("d{n} id {id} at offset {offset_ms} ms, target {target} (the event carries no task text)"));
        self.io
            .log
            .line("TASK", &format!("d{n} from the transcript: {task:?}"));
        self.delegations.push(Delegation {
            id: id.clone(),
            n,
            seen: at,
            task,
            first_speech: None,
            first_append: None,
            answer: None,
            spoken: None,
            comparison: None,
        });
        if self.cfg.quiet_on_delegation {
            self.append(Append::Thinking, Some(n - 1), "");
        }
    }

    /// Sends an append for a delegation (or none), noting the first one.
    fn append(&mut self, kind: Append, delegation: Option<usize>, content: &str) {
        let event_id = self.event_id("h");
        let id = delegation.map(|i| self.delegations[i].id.clone());
        let now = Instant::now();
        if let Some(i) = delegation {
            let d = &mut self.delegations[i];
            if d.first_append.is_none() {
                d.first_append = Some(now.duration_since(d.seen));
            }
        }
        // Any thinking or commentary cancels every pending acknowledgement.
        if matches!(kind, Append::Thinking | Append::Commentary) {
            for d in self.delegations.iter_mut() {
                if d.first_append.is_none() && d.answer.is_none() {
                    d.first_append = Some(now.duration_since(d.seen));
                }
            }
        }
        self.pending
            .insert(event_id.clone(), Pending { kind, sent: now });
        self.send(live::append(kind, &event_id, id.as_deref(), content));
    }

    fn answer(&mut self, text: &str, auto: Option<u64>) {
        let target = self.delegations.iter().position(|d| d.answer.is_none());
        self.finish_utterance();
        let (label, since) = match target {
            Some(i) => (
                format!("d{}", self.delegations[i].n),
                self.delegations[i].seen.elapsed().as_millis(),
            ),
            None => ("no delegation (delegation_id null)".to_string(), 0),
        };
        let how = match auto {
            Some(ms) => format!("sent by the harness after {ms} ms"),
            None => "typed".to_string(),
        };
        self.io.log.line(
            "ANSWER",
            &format!("{label} {how}, {since} ms after the delegation: {text:?}"),
        );
        let chunks = live::chunk_text(text, MAX_APPEND_BYTES);
        if chunks.len() > 1 {
            self.io.log.line(
                "ANSWER",
                &format!(
                    "sent in {} appends under one id (the 500-token limit)",
                    chunks.len()
                ),
            );
        }
        for chunk in chunks {
            self.append(Append::Commentary, target, &chunk);
        }
        if let Some(i) = target {
            self.delegations[i].answer = Some((text.to_string(), Instant::now()));
        }
    }

    /// Writes down the interruption being watched. `ended`: the session ended
    /// first, so the sound may not have stopped yet.
    fn finish_interrupt(&mut self, ended: bool) {
        let Some(interrupt) = self.interrupt.take() else {
            return;
        };
        let latency = self.io.queue.stats().latency;
        let stopped = self
            .io
            .queue
            .first_silence_after(interrupt.first_loud)
            .map(|t| t + latency);
        let took = stopped.map(|t| t.duration_since(interrupt.first_loud).as_millis() as u64);
        let after = interrupt.last_delta.map(|t| {
            t.saturating_duration_since(interrupt.first_loud)
                .as_millis() as u64
        });
        self.io.log.line(
            "INTERRUPT",
            &format!(
                "{}; the last audio from the server came {}{}",
                match took {
                    Some(ms) => format!("the sound stopped {ms} ms after you first spoke (the device's own {} ms counted)", latency.as_millis()),
                    None if ended => "the session ended before the sound stopped".to_string(),
                    None => "the sound had NOT stopped 10 s after you first spoke".to_string(),
                },
                match after {
                    Some(ms) => format!("{ms} ms after you first spoke"),
                    None => "before you spoke (none came after)".to_string(),
                },
                if interrupt.discarded_ms > 0 { format!("; {} ms of it was thrown away in the holdoff", interrupt.discarded_ms) } else { String::new() }
            ),
        );
        self.report.interruptions.push(Interruption {
            queued_ms: interrupt.queued_ms,
            dropped_ms: interrupt.dropped_ms,
            discarded_ms: interrupt.discarded_ms,
            last_delta_after_ms: after,
            stop_took_ms: took,
        });
    }

    fn on_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        match line {
            "/q" | "/quit" => self.begin_close("you asked to end it"),
            "/status" => self.status(),
            _ if line == "/t" || line.starts_with("/t ") => {
                let target = self.delegations.iter().position(|d| d.answer.is_none());
                let text = line.strip_prefix("/t").unwrap_or_default().trim().to_string();
                self.io.log.line("THINKING", &format!("{} {text:?}", if text.is_empty() { "(empty: quiet)" } else { "(not spoken)" }));
                self.append(Append::Thinking, target, &text);
            }
            _ if line.starts_with('/') => self.io.log.line("NOTE", "commands: /q ends the session, /status, /t [text] a thinking append; any other line is the yard's answer"),
            _ => self.answer(line, None),
        }
    }

    fn status(&self) {
        let stats = self.io.queue.stats();
        self.io.log.line(
            "STATUS",
            &format!(
                "{:.0} s in; {} delegations ({} unanswered); sent {} ms of audio; received {} ms ({} ms of it speech), played {} ms, queued {} ms, dropped {} ms; levels so far: microphone {}; model audio {}",
                self.secs(),
                self.delegations.len(),
                self.delegations.iter().filter(|d| d.answer.is_none()).count(),
                self.in_sent_ms,
                stats.rx_ms,
                stats.speech_rx_ms,
                stats.played_ms,
                stats.queued_ms,
                stats.dropped_ms,
                mic_words(&self.mic_meter.overall()),
                model_words(&self.rx_meter.overall())
            ),
        );
    }

    fn on_mic(&mut self, chunk: Vec<i16>) {
        if self.started.is_none() || self.closing.is_some() {
            return;
        }
        self.in_sent_ms += audio::samples_ms(chunk.len() as u64);
        self.report.chunks_sent += 1;
        // macOS gives a program silence, not an error, where the microphone's
        // consent was refused; a live microphone is never exactly zero.
        self.zero_run = if chunk.iter().all(|&s| s == 0) {
            self.zero_run + 1
        } else {
            0
        };
        if self.zero_run == 150 && !self.warned_zero {
            self.warned_zero = true;
            self.io.log.line(
                "WARN",
                "the microphone has given exact silence for 3 s. macOS does that, and no error, where the \
                 microphone's consent was refused: System Settings, Privacy & Security, Microphone, allow \
                 the terminal this was run from (and run it again). A muted or unplugged input does it too.",
            );
        }
        match self.vad.feed(&chunk) {
            Some(VadEvent::Onset) => {
                self.speaking = true;
                let first_loud =
                    Instant::now() - Duration::from_millis(20 * (ONSET_CHUNKS as u64 - 1));
                let stats = self.io.queue.stats();
                let playing = stats.queued_ms > 0
                    || stats
                        .last_audio
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(200));
                if playing && self.interrupt.is_none() {
                    let dropped_ms = self.cfg.barge_in.then(|| self.io.queue.drop_all());
                    if dropped_ms.is_some() {
                        self.discard_until = Some(Instant::now() + self.cfg.holdoff);
                    }
                    self.io.log.line(
                        "INTERRUPT",
                        &format!(
                            "you began to speak while the model was speaking; {} ms of speech was queued ahead{}",
                            stats.queued_ms,
                            match dropped_ms {
                                Some(ms) => format!(", and {ms} ms of it was dropped at once (--barge-in)"),
                                None => ", and nothing was dropped (the server must stop, and the queue play out)".into(),
                            }
                        ),
                    );
                    self.interrupt = Some(Interrupt {
                        first_loud,
                        queued_ms: stats.queued_ms,
                        dropped_ms,
                        discarded_ms: 0,
                        last_delta: None,
                        tail_closed: false,
                    });
                } else {
                    self.io.log.line("SPEECH", "you began to speak");
                }
            }
            Some(VadEvent::Offset) => {
                self.speaking = false;
                self.io.log.line("SPEECH", "you stopped speaking");
            }
            None => {}
        }
        self.mic_meter.add(&chunk, self.speaking);
        self.send(live::input_audio_append(&chunk));
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.io.stop.load(Ordering::SeqCst) {
            self.begin_close("Ctrl-C");
        }
        if let (Some(limit), Some(started)) = (self.cfg.stop_after, self.started)
            && started.elapsed() >= limit
        {
            self.begin_close(&format!(
                "the {} s asked for with --stop-after is up",
                limit.as_secs()
            ));
        }
        if let Some(u) = &self.utterance
            && now.duration_since(u.last) > SPEECH_GAP
        {
            self.finish_utterance();
        }
        // An interruption is watched for a second, and ends when the sound has
        // stopped by then (or, if it has not, when it is overdue).
        if let Some(interrupt) = &self.interrupt {
            let age = now.duration_since(interrupt.first_loud);
            let silent = self
                .io
                .queue
                .first_silence_after(interrupt.first_loud)
                .is_some();
            if (silent && age >= WATCH) || age >= WATCH_OVERDUE {
                self.finish_interrupt(false);
            }
        }
        if let Some((ms, text)) = self.cfg.auto_answer.clone()
            && let Some(d) = self.delegations.iter().find(|d| d.answer.is_none())
            && now.duration_since(d.seen) >= Duration::from_millis(ms)
        {
            self.answer(&text, Some(ms));
        }
        if self.started.is_some() && now.duration_since(self.last_sync) >= self.cfg.sync_every {
            self.last_sync = now;
            let stats = self.io.queue.stats();
            let mic = self.mic_meter.take_interval();
            let model = self.rx_meter.take_interval();
            self.io.log.line(
                "SYNC",
                &format!(
                    "{:.0} s in: audio sent {} ms; received {} ms, played {} ms; caption timeline vs wall {}; microphone this interval: {}; model audio this interval: {}",
                    self.secs(),
                    self.in_sent_ms,
                    stats.rx_ms,
                    stats.played_ms,
                    self.report.timeline_vs_wall.last().map(|s| format!("{:+.0} ms", s.1)).unwrap_or_else(|| "(no caption yet)".into()),
                    mic_words(&mic),
                    model_words(&model)
                ),
            );
        }
    }

    fn finish(&mut self) {
        self.finish_utterance();
        self.finish_interrupt(true);
        let delay = self.cfg.ack_delay_ms as u64;
        for d in &self.delegations {
            let ack = classify_ack(
                delay,
                d.first_speech.map(|t| t.as_millis() as u64),
                d.first_append.map(|t| t.as_millis() as u64),
            );
            self.io
                .log
                .line("ACK", &format!("d{} {}", d.n, ack.words(delay)));
            self.report.delegations.push(DelegationReport {
                id: d.id.clone(),
                task: d.task.clone(),
                ack,
                answer: d.answer.as_ref().map(|(t, _)| t.clone()),
                spoken: d.spoken.clone(),
                comparison: d.comparison.clone(),
            });
        }
        let stats = self.io.queue.stats();
        let minutes = self.secs() / 60.0;
        let log = &self.io.log;
        log.line("SUMMARY", &format!(
            "{:.1} min; {} delegations; {} interruptions; {} echo suspects; {} pings answered; {} errors; closed: {}",
            minutes,
            self.report.delegations.len(),
            self.report.interruptions.len(),
            self.report.echo_suspects,
            self.report.pings,
            self.report.errors.len(),
            self.report.closed_reason.as_deref().unwrap_or("not confirmed by session.closed"),
        ));
        log.line("SUMMARY", &format!(
            "audio: sent {} chunks ({} ms); model audio received {} ms, played {} ms, dropped {} ms",
            self.report.chunks_sent, self.in_sent_ms, stats.rx_ms, stats.played_ms, stats.dropped_ms
        ));
        let (mic, model) = (self.mic_meter.overall(), self.rx_meter.overall());
        log.line(
            "SUMMARY",
            &format!(
                "levels over the whole session: microphone {}; model audio {}",
                mic_words(&mic),
                model_words(&model)
            ),
        );
        self.report.mic_level = Some(mic);
        self.report.model_level = Some(model);
        if let (Some(limit), Some(seconds)) = (self.report.limit_secs, self.report.usage_seconds) {
            log.line("SUMMARY", &format!("the session was given {limit} s at its start; {seconds:.1} s of voice used (${:.3})", seconds / 60.0 * 0.05));
        }
        log.line(
            "SUMMARY",
            &series_words(
                "caption timeline minus wall clock",
                &self.report.timeline_vs_wall,
            ),
        );
        log.line(
            "SUMMARY",
            &series_words("audio sent minus input caption end", &self.report.input_lag),
        );
        log.line(
            "SUMMARY",
            &series_words(
                "output caption end minus audio played",
                &self.report.output_lead,
            ),
        );
        for i in &self.report.interruptions {
            log.line("SUMMARY", &format!(
                "interruption: stopped in {:?} ms; {} ms queued ahead; dropped {:?} ms, thrown away in the holdoff {} ms; last audio from the server {:?} ms after",
                i.stop_took_ms, i.queued_ms, i.dropped_ms, i.discarded_ms, i.last_delta_after_ms
            ));
        }
        self.report.finished_cleanly = self.report.closed_reason.is_some();
        let _ = self.io.out.send(Out::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_acknowledgement_is_told_from_its_speech_and_its_append() {
        // Spoke before anything was appended: early if inside the delay.
        assert_eq!(
            classify_ack(800, Some(300), None),
            Ack::Spoke {
                ms: 300,
                early: true
            }
        );
        assert_eq!(
            classify_ack(800, Some(900), Some(2000)),
            Ack::Spoke {
                ms: 900,
                early: false
            }
        );
        // An append inside the delay with no speech first: cancelled.
        assert_eq!(
            classify_ack(800, None, Some(300)),
            Ack::Cancelled { append_ms: 300 }
        );
        assert_eq!(
            classify_ack(800, None, Some(0)),
            Ack::Cancelled { append_ms: 0 }
        );
        // An append after the delay and still no speech: nothing was heard.
        assert_eq!(
            classify_ack(800, None, Some(5000)),
            Ack::NoneBeforeAppend { append_ms: 5000 }
        );
        assert_eq!(classify_ack(800, None, None), Ack::Pending);
    }

    #[test]
    fn padding_from_the_server_is_no_part_of_what_it_went_on_saying() {
        let began = Instant::now();
        let at = |ms: u64| began + Duration::from_millis(ms);
        let new = || Interrupt {
            first_loud: began,
            queued_ms: 0,
            dropped_ms: None,
            discarded_ms: 0,
            last_delta: None,
            tail_closed: false,
        };
        // Speech the server was still sending when the person began to speak
        // is the tail of what it said; the silence that follows, at the pace
        // of the clock, is not, however long it goes on.
        let mut interrupt = new();
        interrupt.heard(at(100), 20, true, false);
        interrupt.heard(at(300), 20, true, false);
        for ms in (320..2000).step_by(20) {
            interrupt.heard(at(ms), 20, false, false);
        }
        assert_eq!(interrupt.last_delta, Some(at(300)));
        assert!(!interrupt.tail_closed);
        // Speech after a pause longer than the gap is a new answer.
        interrupt.heard(at(2000), 20, true, false);
        assert!(interrupt.tail_closed);
        assert_eq!(interrupt.last_delta, Some(at(300)));
        // Of what a holdoff threw away, only the speech is counted.
        let mut held = new();
        held.heard(at(50), 480, false, true);
        assert_eq!(held.discarded_ms, 0);
        held.heard(at(60), 20, true, true);
        assert_eq!(held.discarded_ms, 20);
    }

    #[test]
    fn a_drift_is_a_slope_in_milliseconds_a_minute() {
        // 5 ms lost every second is 300 ms a minute.
        let samples: Vec<(f64, f64)> = (0..30)
            .map(|s| (s as f64 * 10.0, 100.0 + s as f64 * 50.0))
            .collect();
        let slope = slope_per_min(&samples).unwrap();
        assert!((slope - 300.0).abs() < 1e-6, "{slope}");
        assert_eq!(slope_per_min(&samples[..2]), None);
        let flat: Vec<(f64, f64)> = (0..10).map(|s| (s as f64, 40.0)).collect();
        assert!(slope_per_min(&flat).unwrap().abs() < 1e-9);
        assert!(
            series_words("x", &flat).contains("slope +0 ms/min")
                || series_words("x", &flat).contains("slope -0 ms/min")
        );
        assert_eq!(series_words("x", &[]), "x: no samples");
    }
}
