//! voice-harness: a throwaway program for bead footplate-4hd3 of footplate's
//! voice design. Microphone to GPT-Live (`gpt-live-1`) over a hand-written
//! WebSocket, its speech played as it comes, a hand-typed yard answer returned
//! by `session.commentary.append` to be spoken, and a log for a person to read
//! back. Nothing here is footplate's product code.
//!
//! The key is read from FOOTPLATE_OPENAI_KEY and is never printed or logged.

mod audio;
mod live;
mod session;
mod standin;
mod ws;

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use session::{Config, Io, Log, Msg};

const DEFAULT_URL: &str = "wss://api.openai.com/v1/live/sessions";
const KEY_VARIABLE: &str = "FOOTPLATE_OPENAI_KEY";

const HELP: &str = "voice-harness: a GPT-Live voice session with a typed yard answer, and a log.

  FOOTPLATE_OPENAI_KEY=sk-... voice-harness [options]

Speak to the model. When it hands a request to the yard the log shows
DELEGATION and the task taken from what you said; type the yard's answer on a
line and press Enter, and the model speaks it in its own words. /q ends the
session (Ctrl-C does too); /t [text] sends a thinking append; /status.

options:
  --url URL               endpoint for --probe; live sessions use OpenAI's endpoint
  --counterpart NAME      who the person is talking with (default yardmaster)
  --ack-delay-ms N        acknowledgement_delay_ms, 0 to 800 (default 800)
  --quiet-on-delegation   send an empty thinking append the moment a delegation
                          arrives, as the guide does, to cancel the acknowledgement
  --barge-in              drop the queued speech the moment you speak over it
                          (a headset keeps the model's voice out of the level gate)
  --holdoff-ms N          with --barge-in, audio that arrives within N ms after the
                          drop is thrown away too (default 300)
  --vad-db DB             the level gate, in dBFS (default -35)
  --auto-answer MS:TEXT   answer a delegation nobody has answered after MS ms
                          (to try an append inside the delay, which takes no typing)
  --stop-after SECS       end the session after SECS seconds (five minutes: 300)
  --instructions FILE     the session's instructions from a file, {counterpart} replaced
  --log FILE              the log's file (default voice-harness-<time>.log here)
  --rehearse              no key, no network: a session against the stand-in
  --probe                 no key: connect to the url and show how it refuses
  --devices [--play-silence]
                          show the default microphone and speaker; with
                          --play-silence a second of silence through the speaker
";

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("voice-harness: {why}");
    std::process::exit(2);
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

struct Args {
    url: String,
    cfg: Config,
    log: Option<String>,
    instructions: Option<String>,
    rehearse: bool,
    probe: bool,
    devices: bool,
    play_silence: bool,
}

/// Keep the project key on the one endpoint this harness is meant to call.
fn is_live_endpoint(url: &ws::Url) -> bool {
    url.tls
        && url.host.eq_ignore_ascii_case("api.openai.com")
        && url.port == 443
        && url.path == "/v1/live/sessions"
}

fn parse_args() -> Args {
    let mut args = Args {
        url: DEFAULT_URL.into(),
        cfg: Config::default(),
        log: None,
        instructions: None,
        rehearse: false,
        probe: false,
        devices: false,
        play_silence: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .unwrap_or_else(|| fail(format!("{name} needs a value")))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--url" => args.url = value("--url"),
            "--counterpart" => args.cfg.counterpart = value("--counterpart"),
            "--ack-delay-ms" => {
                args.cfg.ack_delay_ms = value("--ack-delay-ms")
                    .parse()
                    .ok()
                    .filter(|ms| *ms <= 800)
                    .unwrap_or_else(|| fail("--ack-delay-ms is a number from 0 to 800"))
            }
            "--quiet-on-delegation" => args.cfg.quiet_on_delegation = true,
            "--barge-in" => args.cfg.barge_in = true,
            "--holdoff-ms" => {
                let ms: u64 = value("--holdoff-ms")
                    .parse()
                    .unwrap_or_else(|_| fail("--holdoff-ms is a number of milliseconds"));
                args.cfg.holdoff = Duration::from_millis(ms);
            }
            "--vad-db" => {
                args.cfg.vad_db = value("--vad-db")
                    .parse()
                    .unwrap_or_else(|_| fail("--vad-db is a number of dBFS, such as -35"))
            }
            "--auto-answer" => {
                let spec = value("--auto-answer");
                let (ms, text) = spec
                    .split_once(':')
                    .unwrap_or_else(|| fail("--auto-answer is MS:TEXT"));
                let ms = ms
                    .parse()
                    .unwrap_or_else(|_| fail("--auto-answer is MS:TEXT, MS a number"));
                args.cfg.auto_answer = Some((ms, text.to_string()));
            }
            "--stop-after" => {
                let secs: u64 = value("--stop-after")
                    .parse()
                    .unwrap_or_else(|_| fail("--stop-after is a number of seconds"));
                args.cfg.stop_after = Some(Duration::from_secs(secs));
            }
            "--instructions" => {
                let path = value("--instructions");
                args.instructions = Some(
                    std::fs::read_to_string(&path)
                        .unwrap_or_else(|err| fail(format!("{path}: {err}"))),
                );
            }
            "--log" => args.log = Some(value("--log")),
            "--rehearse" => args.rehearse = true,
            "--probe" => args.probe = true,
            "--devices" => args.devices = true,
            "--play-silence" => args.play_silence = true,
            other => fail(format!("{other} is not an option (--help lists them)")),
        }
    }
    args
}

/// The log's file: this directory's, for its owner alone.
fn open_log(path: Option<&str>) -> std::fs::File {
    use std::os::unix::fs::OpenOptionsExt;
    let path = path.map(str::to_string).unwrap_or_else(|| {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("voice-harness-{secs}.log")
    });
    eprintln!("voice-harness: the log is also written to {path}");
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap_or_else(|err| fail(format!("{path}: {err}")))
}

fn main() {
    let args = parse_args();
    if args.devices {
        #[cfg(feature = "device")]
        {
            print!("{}", audio::device::probe(args.play_silence));
            return;
        }
        #[cfg(not(feature = "device"))]
        fail("this build has no audio devices (the `device` feature is off)");
    }
    if args.rehearse {
        // A rehearsal keeps no file unless one is asked for: it is all stand-in.
        let log = Log::new(args.log.as_deref().map(|path| open_log(Some(path))), false);
        match standin::rehearsal(log, args.cfg.barge_in, "rehearsal-key-not-a-secret") {
            Ok((report, _)) => std::process::exit(if report.finished_cleanly { 0 } else { 1 }),
            Err(why) => fail(why),
        }
    }
    let url = ws::Url::parse(&args.url).unwrap_or_else(|err| fail(err));
    if args.probe {
        match ws::Conn::connect(&url, &[]) {
            Err(err @ ws::Error::Refused(_)) => {
                println!(
                    "reached {} over {} and was refused, as a request with no key should be: {err}",
                    url.host,
                    if url.tls { "TLS" } else { "TCP" }
                );
            }
            Err(err) => fail(err),
            Ok(_) => fail("the server accepted an upgrade with no key"),
        }
        return;
    }

    if !is_live_endpoint(&url) {
        fail(
            "live sessions send the project key only to wss://api.openai.com/v1/live/sessions; --url is for --probe",
        );
    }

    let key = std::env::var(KEY_VARIABLE)
        .ok()
        .filter(|k| !k.trim().is_empty());
    let bearer = key.as_ref().map(|k| format!("Bearer {}", k.trim()));
    if bearer.is_none() {
        fail(format!("set {KEY_VARIABLE} to your OpenAI project key"));
    }
    let mut cfg = args.cfg;
    cfg.instructions = match args.instructions {
        Some(text) => text.replace("{counterpart}", &cfg.counterpart),
        None => live::instructions(&cfg.counterpart),
    };
    let log = Log::new(Some(open_log(args.log.as_deref())), false);
    let headers: Vec<(&str, &str)> = bearer
        .iter()
        .map(|b| ("Authorization", b.as_str()))
        .collect();
    let conn = ws::Conn::connect(&url, &headers).unwrap_or_else(|err| fail(err));
    drop(headers);

    let (tx, rx) = mpsc::channel();
    let (out_tx, out_rx) = mpsc::channel();
    let socket = session::spawn_socket(conn, tx.clone(), out_rx);
    let queue = audio::PlayQueue::new();
    #[cfg(feature = "device")]
    let devices = audio::device::start(session::mic_sink(tx.clone()), queue.clone())
        .unwrap_or_else(|why| fail(why));
    #[cfg(not(feature = "device"))]
    let devices = audio::start_fake(session::mic_sink(tx.clone()), queue.clone());
    log.line(
        "DEVICES",
        &format!("microphone: {}; speaker: {}", devices.input, devices.output),
    );

    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        thread::spawn(move || {
            while !STOP.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(50));
            }
            stop.store(true, Ordering::SeqCst);
        });
    }
    {
        let tx = tx.clone();
        thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tx.send(Msg::Line(line)).is_err() {
                    break;
                }
            }
        });
    }
    eprintln!(
        "voice-harness: speak; type the yard's answer when DELEGATION shows; /q or Ctrl-C ends it"
    );
    let io = Io {
        rx,
        out: out_tx,
        queue,
        log,
        stop,
    };
    let result = session::run(cfg, io);
    drop(devices);
    let _ = socket.join();
    match result {
        Ok(report) => std::process::exit(if report.finished_cleanly { 0 } else { 1 }),
        Err(why) => fail(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_session_is_pinned_to_the_openai_endpoint() {
        assert!(is_live_endpoint(
            &ws::Url::parse("wss://api.openai.com/v1/live/sessions").unwrap()
        ));
        assert!(is_live_endpoint(
            &ws::Url::parse("wss://API.OPENAI.COM:443/v1/live/sessions").unwrap()
        ));
        for address in [
            "ws://api.openai.com/v1/live/sessions",
            "wss://example.com/v1/live/sessions",
            "wss://api.openai.com:8443/v1/live/sessions",
            "wss://api.openai.com/v1/other",
        ] {
            assert!(
                !is_live_endpoint(&ws::Url::parse(address).unwrap()),
                "accepted {address}"
            );
        }
    }
}
