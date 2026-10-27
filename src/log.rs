//! What footplate says on stderr, kept where it can be found after the
//! fact. An app started by the Finder or by launchd has its stderr on
//! /dev/null, and a release on Windows has none, so a line said there is
//! lost where a hosted yard's timeouts happen. The line is kept as well, led
//! by the time it was said, in one small file in the platform's log
//! directory: `~/Library/Logs/footplate/footplate.log` on macOS,
//! `$XDG_STATE_HOME/footplate/footplate.log` elsewhere (`~/.local/state` when
//! that is not set), `%LOCALAPPDATA%\footplate\footplate.log` on Windows.
//! Past [`CAP`] the file is moved to `footplate.log.1`, the one before it
//! gone, and a new one begun: the log is never more than twice the cap, and
//! what came just before a new file is still there to read. It stays on this
//! machine; nothing sends it anywhere.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, FixedOffset, Local, SecondsFormat};

/// The most bytes the log holds before it is moved aside for a new one.
pub const CAP: u64 = 1 << 20;

/// Where the log is kept, or nowhere for a person without a home. As for
/// the settings ([`crate::settings::file_under`]), an absolute
/// `$XDG_STATE_HOME` wins on either platform and a relative one is
/// ignored, as the XDG base directory specification has it.
fn file_under(
    macos: bool,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    let dir = match xdg_state_home.and_then(absolute) {
        Some(dir) => dir,
        None if macos => home.and_then(absolute)?.join("Library/Logs"),
        None => home.and_then(absolute)?.join(".local/state"),
    };
    Some(dir.join(crate::NAME).join("footplate.log"))
}

/// Where the log is kept on Windows: `%LOCALAPPDATA%\footplate`, not the
/// roaming `%APPDATA%` the settings are in, which follows a person from
/// one machine to the next; this machine's calls are for this machine. A
/// `LOCALAPPDATA` that is not an absolute path is none.
fn file_under_local_appdata(local_appdata: Option<OsString>) -> Option<PathBuf> {
    let dir = Some(PathBuf::from(local_appdata?)).filter(|dir| dir.is_absolute())?;
    Some(dir.join(crate::NAME).join("footplate.log"))
}

/// Where this run keeps the log. A test never asks: it keeps in a file of
/// its own, and the person's is left alone.
#[cfg_attr(test, allow(dead_code))]
pub fn file() -> Option<PathBuf> {
    if cfg!(windows) {
        return file_under_local_appdata(std::env::var_os("LOCALAPPDATA"));
    }
    file_under(
        cfg!(target_os = "macos"),
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Says `line` through `err`, led by the program's name as footplate's other
/// lines are, and keeps it in `file` with the time, where there is one. `err`
/// is where stderr's lines go, so that a test can read them. A log that
/// cannot be written is said so and no more: the line was said, and what
/// failed to be kept is no reason for the call that left it to fail again.
pub fn say(err: impl Fn(&str), file: Option<&Path>, line: &str) {
    err(&format!("footplate: {line}"));
    let Some(file) = file else { return };
    if let Err(why) = keep(file, Local::now().fixed_offset(), line) {
        err(&format!(
            "footplate: no line in the log {}: {why}",
            file.display()
        ));
    }
}

/// A failed call is made on any thread. One line at a time, so that two
/// that cross the cap together move the file once, and not the second over
/// the history the first kept.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Appends `line` to `file`, led by `at`, with the file's directory made if
/// it is not there. A line that would take the file past [`CAP`] begins a
/// new one: the file is moved to [`rotated`] first.
fn keep(file: &Path, at: DateTime<FixedOffset>, line: &str) -> io::Result<()> {
    let entry = format!(
        "{} {line}\n",
        at.to_rfc3339_opts(SecondsFormat::Millis, false)
    );
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(file).is_ok_and(|kept| kept.len() + entry.len() as u64 > CAP) {
        // A move that does not work (Windows, with a reader holding the
        // file) leaves the file as it is, and the next line tries again.
        let _ = fs::rename(file, rotated(file));
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(entry.as_bytes())
}

/// Where the log goes when it is full: its own name and `.1`.
fn rotated(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::SubsecRound;
    use std::cell::RefCell;

    fn at(stamp: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(stamp).unwrap()
    }

    const LINE: &str = "call failed: request=r-1 method=POST path=/call/Flows outcome=timeout status=none timeout=30s";

    // Its paths are a unix system's: `/elsewhere` is no absolute path on
    // Windows, which `file_under_local_appdata`'s test stands for.
    #[cfg(unix)]
    #[test]
    fn the_log_is_where_the_platform_has_a_program_s_logs() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            file_under(true, None, os("/Users/jo")),
            Some(PathBuf::from(
                "/Users/jo/Library/Logs/footplate/footplate.log"
            ))
        );
        assert_eq!(
            file_under(true, os("/elsewhere"), os("/Users/jo")),
            Some(PathBuf::from("/elsewhere/footplate/footplate.log"))
        );
        assert_eq!(
            file_under(true, os("relative"), os("/Users/jo")),
            Some(PathBuf::from(
                "/Users/jo/Library/Logs/footplate/footplate.log"
            ))
        );
        assert_eq!(
            file_under(false, None, os("/home/jo")),
            Some(PathBuf::from(
                "/home/jo/.local/state/footplate/footplate.log"
            ))
        );
        assert_eq!(
            file_under(false, os("/home/jo/.state"), os("/home/jo")),
            Some(PathBuf::from("/home/jo/.state/footplate/footplate.log"))
        );
        assert_eq!(
            file_under(false, os("relative"), os("/home/jo")),
            Some(PathBuf::from(
                "/home/jo/.local/state/footplate/footplate.log"
            ))
        );
        assert_eq!(file_under(false, None, None), None);
        assert_eq!(
            file_under(true, os("/elsewhere"), None),
            Some(PathBuf::from("/elsewhere/footplate/footplate.log"))
        );
    }

    #[test]
    fn on_windows_the_log_is_under_local_appdata_when_that_is_a_whole_path() {
        // A whole path on any system the test runs on: the temp dir stands
        // for the profile's `AppData\Local`.
        let local = std::env::temp_dir().join("Local");
        assert_eq!(
            file_under_local_appdata(Some(local.clone().into_os_string())),
            Some(local.join("footplate").join("footplate.log"))
        );
        assert_eq!(
            file_under_local_appdata(Some(OsString::from("Local"))),
            None
        );
        assert_eq!(file_under_local_appdata(Some(OsString::new())), None);
        assert_eq!(file_under_local_appdata(None), None);
    }

    #[test]
    fn a_line_is_kept_with_the_time_it_was_said() {
        let dir = tempfile::tempdir().unwrap();
        // The directory is not there before the first line.
        let file = dir.path().join("footplate/footplate.log");
        keep(&file, at("2026-10-26T21:47:03.512+01:00"), LINE).unwrap();
        // The zone is the person's, and the time is to the millisecond:
        // where the edge gave no id (a connect that failed, a head that
        // never came), the time is all the edge's own log can be joined by.
        keep(&file, at("2026-10-26T15:48:03.007-05:00"), "second").unwrap();
        keep(&file, at("2026-10-26T20:49:00Z"), "third").unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!(
                "2026-10-26T21:47:03.512+01:00 {LINE}\n\
                 2026-10-26T15:48:03.007-05:00 second\n\
                 2026-10-26T20:49:00.000+00:00 third\n"
            )
        );
        assert!(!rotated(&file).exists());
    }

    #[test]
    fn a_line_that_would_pass_the_cap_begins_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate.log");
        let when = at("2026-10-26T21:47:03.512+01:00");
        let entry = format!("2026-10-26T21:47:03.512+01:00 {LINE}\n");
        // One line short of the cap.
        let room = CAP as usize - entry.len();
        fs::write(&file, "a".repeat(room)).unwrap();

        // The line that fills it exactly still goes in.
        keep(&file, when, LINE).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().len(), CAP);
        assert!(!rotated(&file).exists());

        // One more would pass it: the file is moved aside whole.
        keep(&file, when, LINE).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), entry);
        assert_eq!(
            fs::read_to_string(rotated(&file)).unwrap(),
            format!("{}{entry}", "a".repeat(room))
        );

        // The next full file takes the moved one's place: no more than two
        // are kept.
        fs::write(&file, "b".repeat(room)).unwrap();
        keep(&file, when, LINE).unwrap();
        keep(&file, when, LINE).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), entry);
        assert_eq!(
            fs::read_to_string(rotated(&file)).unwrap(),
            format!("{}{entry}", "b".repeat(room))
        );
    }

    #[test]
    fn stderr_has_the_line_as_ever_and_the_log_has_it_with_the_time() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate.log");
        let said = RefCell::new(Vec::new());
        let err = |text: &str| said.borrow_mut().push(text.to_string());

        let before = Local::now().trunc_subsecs(3);
        say(err, Some(&file), LINE);
        let after = Local::now();
        // Without the time: the terminal or the journal it is read in has
        // its own.
        assert_eq!(said.take(), [format!("footplate: {LINE}")]);
        let kept = fs::read_to_string(&file).unwrap();
        let (stamp, rest) = kept.split_once(' ').unwrap();
        assert_eq!(rest, format!("{LINE}\n"));
        let stamp = DateTime::parse_from_rfc3339(stamp).unwrap();
        assert!(
            before <= stamp && stamp <= after,
            "{before} {stamp} {after}"
        );

        // A person without a home has no log, and stderr alone.
        say(err, None, LINE);
        assert_eq!(said.take(), [format!("footplate: {LINE}")]);
        assert_eq!(fs::read_to_string(&file).unwrap(), kept);
    }

    #[test]
    fn a_log_that_cannot_be_written_is_said_after_the_line_and_is_no_failure() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the log's directory would be.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "").unwrap();
        let file = blocker.join("footplate.log");
        let said = RefCell::new(Vec::new());
        say(
            |text: &str| said.borrow_mut().push(text.to_string()),
            Some(&file),
            LINE,
        );
        let said = said.take();
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!(said[0], format!("footplate: {LINE}"));
        assert!(
            said[1].starts_with(&format!(
                "footplate: no line in the log {}: ",
                file.display()
            )),
            "{said:?}"
        );
    }
}
