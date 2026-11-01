//! The OpenAI key of a live session: `FOOTPLATE_OPENAI_KEY` where that is
//! set, else the key typed in footplate's Settings, Voice, which footplate
//! keeps in `voice.key` beside its `settings.json`. Nobody has to put a key
//! in a terminal.
//!
//! This is a small copy of what footplate does, because the harness is its
//! own package and depends on nothing of footplate's. It copies `voice_key_as`,
//! `kept_as` with `VOICE_KIND`, and the file's format (`keep_as`) from
//! src/yard.rs, and `file_under` from src/settings.rs, where the settings lie.
//! When that file's format changes there, change it here too. The copy goes
//! with the harness, when src/voice.rs takes what the harness proved.
//!
//! A refusal names the file and what is wrong with it. It never says what it
//! read, so the key is in none of them.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The environment variable that holds the key. A nonempty trimmed value
/// wins, and the file is not read.
pub const VARIABLE: &str = "FOOTPLATE_OPENAI_KEY";

/// The file beside footplate's settings that holds the key (`VOICE_KEY_FILE`).
const FILE: &str = "voice.key";

/// The marker under the key in a file footplate wrote (`FOOTPLATE_VOICE`). A
/// file without it was not written by footplate, and its first line is not
/// taken for a key.
const MARKER: &str = "!footplate-voice-v1";

/// The key of this run, from the real environment and the real settings
/// directory. Only `main` calls it. A test gives `voice_key_as` its own
/// variable and directory.
pub fn voice_key() -> Result<String, String> {
    voice_key_as(
        std::env::var(VARIABLE).ok(),
        settings_file_under(
            cfg!(target_os = "macos"),
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )
        .as_deref(),
    )
}

/// The key in `env`, the variable's value, else the one in the file. A test
/// passes `env` in and never sets the variable.
fn voice_key_as(env: Option<String>, settings: Option<&Path>) -> Result<String, String> {
    if let Some(key) = env
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
    {
        if !is_token(&key) {
            return Err(format!(
                "{VARIABLE} is no voice key: one is a line, as OpenAI gave it."
            ));
        }
        return Ok(key);
    }
    let Some(settings) = settings else {
        return Err(format!(
            "There is no home directory to keep the voice key in. Set {VARIABLE}."
        ));
    };
    kept(&settings.with_file_name(FILE))
}

/// The key `file` holds. It must be a regular file for its user alone, with
/// the key on its first line, the marker under it and nothing else.
fn kept(file: &Path) -> Result<String, String> {
    let at = file.display();
    let meta = match fs::metadata(file) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "There is no voice key: {at} is not there. Type your OpenAI key in footplate's Settings, Voice section, or set {VARIABLE}."
            ));
        }
        Err(err) => return Err(format!("The voice key file {at} cannot be read: {err}")),
    };
    if !meta.file_type().is_file() {
        return Err(format!(
            "The voice key file {at} is not a regular file, and is refused."
        ));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "The voice key file {at} is open to others (mode {mode:04o}), and is refused: whoever reads it calls OpenAI as you. Make it yours alone: chmod 600 {at}"
        ));
    }
    let text = fs::read_to_string(file)
        .map_err(|err| format!("The voice key file {at} cannot be read: {err}"))?;
    match text.trim().lines().collect::<Vec<_>>()[..] {
        [key, marker] if marker.trim() == MARKER && is_token(key.trim()) => {
            Ok(key.trim().to_string())
        }
        _ => Err(format!(
            "The voice key file {at} holds no voice key: one is a line, as OpenAI gave it, with footplate's marker under it."
        )),
    }
}

/// Whether `token` can stand in a request's head: one line of visible
/// characters, as `is_token` in footplate's src/api.rs has it.
fn is_token(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic())
}

/// Where footplate keeps its settings, or nowhere for a person without a home.
/// An absolute `$XDG_CONFIG_HOME` wins on either system and a relative one is
/// ignored. There is no arm for Windows's `%APPDATA%`, since the harness is for
/// a Mac or Linux, as `open_log` is.
fn settings_file_under(
    macos: bool,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    let dir = match xdg_config_home.and_then(absolute) {
        Some(dir) => dir,
        None if macos => home.and_then(absolute)?.join("Library/Application Support"),
        None => home.and_then(absolute)?.join(".config"),
    };
    Some(dir.join("footplate").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Key-shaped, as a real one is, so that a refusal which repeated what it
    /// read would show in `names_no_key`.
    const KEY: &str = "sk-test-voice-key-5e9a4b21";

    /// A directory of the test's own under the system's temporary one, removed
    /// when it drops. A test never reads the person's own settings.
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Dir {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            // The time too, so that a directory a crashed run left under the
            // same process id is never in the way.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos());
            let dir = std::env::temp_dir().join(format!(
                "voice-harness-key-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Dir(dir)
        }

        /// Where footplate's settings would be. Its directory is not there
        /// until a key is kept.
        fn settings(&self) -> PathBuf {
            self.0.join("footplate/settings.json")
        }

        /// `voice.key` beside the settings, holding `text`, with `mode`.
        fn keep(&self, text: &str, mode: u32) -> PathBuf {
            let file = self.settings().with_file_name(FILE);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, text).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
            file
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Fails if a refusal repeats any of what the file or the variable held.
    /// Every key-shaped string in these tests begins `sk-test`.
    fn names_no_key(why: &str) {
        assert!(!why.contains("sk-test"), "{why}");
    }

    #[test]
    fn a_key_kept_as_footplate_keeps_it_is_read_back() {
        let dir = Dir::new();
        // The file footplate's `keep_voice_key` leaves. `keep_as` in
        // src/yard.rs writes it, and its test
        // `the_voice_key_is_kept_for_its_user_alone_and_read_back` asserts the
        // same text. The key, then the marker, a newline after each, mode 0600,
        // in `voice.key` beside the settings.
        dir.keep(&format!("{KEY}\n!footplate-voice-v1\n"), 0o600);
        assert_eq!(voice_key_as(None, Some(&dir.settings())).unwrap(), KEY);
    }

    #[test]
    fn the_variable_wins_and_the_file_is_then_not_read() {
        let dir = Dir::new();
        let settings = dir.settings();
        let file = dir.keep(&format!("{KEY}-filed\n!footplate-voice-v1\n"), 0o600);
        let from =
            |env: &str, settings: Option<&Path>| voice_key_as(Some(env.to_string()), settings);
        // Set, the variable is the key, with its ends trimmed, and the file is
        // not read. Not even a file that would be refused is read, and with no
        // home there is nothing to look in.
        for env in [KEY, "  sk-test-voice-key-5e9a4b21\n"] {
            assert_eq!(from(env, Some(&settings)).unwrap(), KEY);
        }
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(from(KEY, Some(&settings)).unwrap(), KEY);
        assert_eq!(from(KEY, None).unwrap(), KEY);
        // An empty variable counts as unset, so the file is read. Unset
        // altogether, the file alone is.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        for empty in ["", "  \n "] {
            assert_eq!(
                from(empty, Some(&settings)).unwrap(),
                format!("{KEY}-filed")
            );
        }
        assert_eq!(
            voice_key_as(None, Some(&settings)).unwrap(),
            format!("{KEY}-filed")
        );
    }

    #[test]
    fn a_variable_that_is_no_key_is_refused_whatever_the_file_holds() {
        let dir = Dir::new();
        dir.keep(&format!("{KEY}\n!footplate-voice-v1\n"), 0o600);
        // Anything but one line of visible characters. A second line would
        // become a header of its own.
        for env in [
            format!("{KEY} two words"),
            format!("{KEY}\nsecond line"),
            format!("{KEY}\r\nX-Injected: 1"),
            format!("{KEY}é"),
        ] {
            // The variable wins even when it is wrong. A good file is not
            // taken in its place.
            for settings in [Some(dir.settings()), None] {
                let why = voice_key_as(Some(env.clone()), settings.as_deref()).unwrap_err();
                assert_eq!(
                    why,
                    "FOOTPLATE_OPENAI_KEY is no voice key: one is a line, as OpenAI gave it."
                );
                names_no_key(&why);
            }
        }
    }

    #[test]
    fn a_file_open_to_others_holding_none_or_not_a_file_is_refused() {
        let dir = Dir::new();
        let settings = dir.settings();
        let read = || voice_key_as(None, Some(&settings));
        // Open to others, whatever the other bits grant. It is refused before
        // it is read, and the refusal says why.
        for mode in [0o644, 0o640, 0o604, 0o660] {
            dir.keep(&format!("{KEY}\n!footplate-voice-v1\n"), mode);
            let why = read().unwrap_err();
            assert!(
                why.contains(&format!("is open to others (mode {mode:04o})")),
                "{why}"
            );
            assert!(why.contains("calls OpenAI as you"), "{why}");
            names_no_key(&why);
        }
        // Files that hold no key.
        for none in [
            // Nothing.
            String::new(),
            "\n".to_string(),
            // A key that is not one line.
            format!("{KEY} two words\n!footplate-voice-v1\n"),
            format!("{KEY}\nsecond-key\n!footplate-voice-v1\n"),
            // A key without the marker, as a hand would write it, or with a
            // yard's address under it.
            KEY.to_string(),
            format!("{KEY}\n"),
            format!("{KEY}\nhttps://mac.example.net\n"),
            // What footplate wrote for a yard's device or a hosted session has
            // a marker of its own, with the address of its yard or edge under
            // it or not. It is never taken for the voice key.
            format!("{KEY}\n!footplate-token-v1\n"),
            format!("{KEY}\n!footplate-token-v1\nhttps://mac.example.net\n"),
            format!("{KEY}\n!footplate-session-v1\n"),
            format!("{KEY}\n!footplate-session-v1\nhttps://edge.example.net\n"),
            // Something under the voice marker, where footplate writes nothing.
            format!("{KEY}\n!footplate-voice-v1\nextra\n"),
            format!("{KEY}\n!footplate-voice-v1\nhttps://mac.example.net\n"),
        ] {
            dir.keep(&none, 0o600);
            let why = read().unwrap_err();
            assert!(why.contains("holds no voice key"), "{none:?}: {why}");
            names_no_key(&why);
        }
        // Not a file at all.
        let file = dir.keep("", 0o600);
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        let why = read().unwrap_err();
        assert!(why.contains("is not a regular file"), "{why}");
    }

    #[test]
    fn with_no_key_at_all_it_says_where_to_type_one() {
        let dir = Dir::new();
        let settings = dir.settings();
        let file = settings.with_file_name(FILE).display().to_string();
        // An unset or empty variable and no file. The refusal names the file it
        // looked for, then says where to type the key or what to set.
        for env in [None, Some("".to_string()), Some(" \n".to_string())] {
            let why = voice_key_as(env, Some(&settings)).unwrap_err();
            assert!(
                why.starts_with(&format!("There is no voice key: {file} is not there.")),
                "{why}"
            );
            assert!(
                why.contains("Settings, Voice section") && why.contains("FOOTPLATE_OPENAI_KEY"),
                "{why}"
            );
        }
        // With no home there is no file to look for.
        assert_eq!(
            voice_key_as(None, None).unwrap_err(),
            "There is no home directory to keep the voice key in. Set FOOTPLATE_OPENAI_KEY."
        );
    }

    #[test]
    fn the_settings_lie_where_footplate_keeps_them() {
        let os = |path: &str| Some(OsString::from(path));
        let at = |path: &str| Some(PathBuf::from(path));
        // On a Mac they are in the home's Library, on Linux in its .config.
        assert_eq!(
            settings_file_under(true, None, os("/Users/jo")),
            at("/Users/jo/Library/Application Support/footplate/settings.json")
        );
        assert_eq!(
            settings_file_under(false, None, os("/home/jo")),
            at("/home/jo/.config/footplate/settings.json")
        );
        // An absolute `XDG_CONFIG_HOME` wins on either system, and a relative
        // one is ignored.
        assert_eq!(
            settings_file_under(true, os("/elsewhere"), os("/Users/jo")),
            at("/elsewhere/footplate/settings.json")
        );
        assert_eq!(
            settings_file_under(false, os("/home/jo/.cfg"), os("/home/jo")),
            at("/home/jo/.cfg/footplate/settings.json")
        );
        assert_eq!(
            settings_file_under(true, os("relative"), os("/Users/jo")),
            at("/Users/jo/Library/Application Support/footplate/settings.json")
        );
        assert_eq!(
            settings_file_under(false, os("relative"), os("/home/jo")),
            at("/home/jo/.config/footplate/settings.json")
        );
        // With no home, or a relative one, there is nowhere, unless
        // `XDG_CONFIG_HOME` is absolute.
        assert_eq!(settings_file_under(false, None, None), None);
        assert_eq!(settings_file_under(true, None, os("jo")), None);
        assert_eq!(
            settings_file_under(true, os("/elsewhere"), None),
            at("/elsewhere/footplate/settings.json")
        );
    }
}
