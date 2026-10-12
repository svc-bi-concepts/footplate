//! Which yard the window opens: this machine's, by its socket, or one at an
//! address, as a device of its user. The environment says which, else the
//! settings do. A device's token is kept in a file only its user reads,
//! never in the settings; the yard gives it for a pairing code.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::api::{self, Address, Client};

/// The file beside the settings that holds the token of a device paired in
/// the window.
const TOKEN_FILE: &str = "yard.token";

/// What the environment says of the yard, read once when footplate starts.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `FOOTPLATE_YARD`: the address of a yard. It wins over the settings.
    pub yard: Option<String>,
    /// `FOOTPLATE_TOKEN_FILE`: the file the device's token is read from,
    /// and written to by a pairing. It wins over the file beside the
    /// settings.
    pub token_file: Option<PathBuf>,
    /// The socket of this machine's yard.
    pub socket: PathBuf,
}

impl Env {
    pub fn read() -> Env {
        let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        Env {
            yard: set("FOOTPLATE_YARD").map(|yard| yard.to_string_lossy().into_owned()),
            token_file: set("FOOTPLATE_TOKEN_FILE").map(PathBuf::from),
            socket: api::socket_path(),
        }
    }

    /// The address of the yard at an address that is opened: the
    /// environment's, else the one the settings chose. None is this
    /// machine's yard.
    pub fn address<'a>(&'a self, chosen: Option<&'a str>) -> Option<&'a str> {
        self.yard.as_deref().or(chosen)
    }

    /// The file the token is in: the environment's, else the one beside
    /// the settings, where there are settings.
    pub fn token_file(&self, settings: Option<&Path>) -> Option<PathBuf> {
        self.token_file
            .clone()
            .or_else(|| Some(settings?.with_file_name(TOKEN_FILE)))
    }

    /// A client of the yard to show, or why there is none: an address that
    /// is refused, or a token that is missing or open to others. `chosen`
    /// is the address the settings chose, `settings` their file.
    pub fn open(&self, chosen: Option<&str>, settings: Option<&Path>) -> Result<Client, String> {
        let Some(address) = self.address(chosen) else {
            return Ok(Client::new(&self.socket));
        };
        let address = Address::parse(address)?;
        let file = self.token_file(settings).ok_or(
            "There is no home directory to keep this device's token in. Name a file for it: FOOTPLATE_TOKEN_FILE",
        )?;
        Ok(Client::remote(address, read_token(&file)?))
    }
}

/// Reads a device's token from its file. A file others can read is
/// refused, as every file of a secret is in yardr: the token is all that
/// says who a request is.
pub fn read_token(file: &Path) -> Result<String, String> {
    let at = file.display();
    let meta = match fs::metadata(file) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "This device has no token: {at} is not there. Pair it in the settings, with a code the yard's operator opens: yardr user device pair <user> <device>"
            ));
        }
        Err(err) => return Err(format!("The token file {at} cannot be read: {err}")),
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "The token file {at} is open to others (mode {mode:04o}), and is refused: whoever reads it calls the yard as you. Make it yours alone: chmod 600 {at}"
        ));
    }
    let text = fs::read_to_string(file)
        .map_err(|err| format!("The token file {at} cannot be read: {err}"))?;
    let token = text.trim();
    if !api::is_token(token) {
        return Err(format!(
            "The token file {at} holds no token: one is a single line, as the yard gave it."
        ));
    }
    Ok(token.to_string())
}

/// Keeps a device's token in `file`, for its user alone (mode 0600), in
/// place of the one that was there. The file is whole or not there: it is
/// written beside itself and renamed.
pub fn keep_token(file: &Path, token: &str) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".new");
    let new = file.with_file_name(name);
    // One left by a run that was killed between the two steps.
    let _ = fs::remove_file(&new);
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&new)?;
    out.write_all(token.as_bytes())?;
    out.write_all(b"\n")?;
    out.sync_all()?;
    fs::rename(&new, file)
}

/// The address and the code as they were typed. What a pairing's QR code
/// holds, `<address>#<code>`, may be given as the address: it is split at
/// its last `#`, and a code typed beside it wins.
pub fn typed(address: &str, code: &str) -> (String, String) {
    let (address, code) = (address.trim(), code.trim());
    let (address, held) = address.rsplit_once('#').unwrap_or((address, ""));
    let code = if code.is_empty() { held.trim() } else { code };
    (
        address.trim().trim_end_matches('/').to_string(),
        code.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{TOKEN, Yard};
    use crate::session;

    fn file_of_mode(dir: &Path, name: &str, mode: u32, text: &str) -> PathBuf {
        let file = dir.join(name);
        fs::write(&file, text).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
        file
    }

    fn a_yard_of_jo(yard: &Yard) {
        yard.answer("Head", r#"{"result": 20}"#);
        yard.answer("Crew", r#"{"result": []}"#);
        yard.answer("Peers", r#"{"result": []}"#);
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
        );
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "jo", "cursor": 5, "notification_cursor": 0}}"#,
        );
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

    #[test]
    fn an_address_and_a_token_file_of_mode_0600_open_that_yard_as_the_user_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let far = Yard::start_tcp();
        a_yard_of_jo(&far);
        let env = Env {
            yard: Some(far.address().to_string()),
            token_file: Some(file_of_mode(
                dir.path(),
                "mac.token",
                0o600,
                &format!("{TOKEN}\n"),
            )),
            socket: dir.path().join("yard.sock"),
        };
        let client = env.open(None, None).unwrap();
        assert!(client.is_remote());
        assert_eq!(client.place(), format!("The yard: {}", far.address()));
        // The name of this machine's login is not asked after: the token's
        // user is.
        let state = session::load(&client, "mac-login").unwrap();
        assert_eq!(state.me, "jo");
        let me = state.yard.unwrap();
        assert_eq!((me.said_by.as_str(), me.yard.as_str()), ("laptop", "south"));
        assert_eq!(
            far.asked("GetUser"),
            vec![serde_json::json!({"name": "jo"})]
        );
        assert_eq!(far.asked("Settings"), Vec::<serde_json::Value>::new());
    }

    #[test]
    fn without_them_it_is_this_machine_s_socket_as_before() {
        let near = Yard::start();
        a_yard_of_jo(&near);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let client = env.open(None, None).unwrap();
        assert!(!client.is_remote());
        assert_eq!(
            client.place(),
            format!("The socket: {}", near.socket().display())
        );
        let state = session::load(&client, "jo").unwrap();
        assert_eq!((state.me.as_str(), state.yard), ("jo", None));
        // A token file alone names no yard.
        let env = Env {
            token_file: Some("/nowhere/mac.token".into()),
            ..env
        };
        assert!(!env.open(None, None).unwrap().is_remote());
    }

    #[test]
    fn the_environment_wins_over_what_the_settings_chose() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let beside = file_of_mode(dir.path(), "yard.token", 0o600, "yardr-dev-kept\n");
        let env = Env::default();
        assert_eq!(env.address(None), None);
        assert_eq!(
            env.address(Some("https://kept.example.net")),
            Some("https://kept.example.net")
        );
        assert_eq!(env.token_file(Some(&settings)), Some(beside));
        assert_eq!(env.token_file(None), None);
        let client = env
            .open(Some("https://kept.example.net"), Some(&settings))
            .unwrap();
        assert_eq!(client.place(), "The yard: https://kept.example.net");

        let named = file_of_mode(dir.path(), "mac.token", 0o600, "yardr-dev-named");
        let env = Env {
            yard: Some("https://mac.example.net".into()),
            token_file: Some(named.clone()),
            ..Env::default()
        };
        assert_eq!(env.token_file(Some(&settings)), Some(named));
        let client = env
            .open(Some("https://kept.example.net"), Some(&settings))
            .unwrap();
        assert_eq!(client.place(), "The yard: https://mac.example.net");
    }

    #[test]
    fn plain_http_off_this_machine_is_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let token = file_of_mode(dir.path(), "mac.token", 0o600, TOKEN);
        let env = |yard: &str| Env {
            yard: Some(yard.into()),
            token_file: Some(token.clone()),
            ..Env::default()
        };
        let why = env("http://mac.example.net:8790")
            .open(None, None)
            .unwrap_err();
        assert!(
            why.starts_with("http://mac.example.net:8790 is refused: plain http is for a yard on this machine alone"),
            "{why}"
        );
        assert!(why.contains("https://"), "{why}");
        assert!(env("http://192.168.1.20").open(None, None).is_err());
        // This machine by any of its names, and anything with TLS.
        for here in [
            "http://127.0.0.1:8790",
            "http://localhost:8790",
            "http://[::1]:8790",
            "https://mac.example.net",
            "https://mac.example.net/yard/",
        ] {
            assert!(env(here).open(None, None).is_ok(), "{here}");
        }
        assert!(env("mac.example.net").open(None, None).is_err());
    }

    #[test]
    fn a_token_file_open_to_others_is_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let env = |file: PathBuf| Env {
            yard: Some("https://mac.example.net".into()),
            token_file: Some(file),
            ..Env::default()
        };
        for mode in [0o644, 0o640, 0o604, 0o660] {
            let file = file_of_mode(dir.path(), "open.token", mode, TOKEN);
            let why = env(file.clone()).open(None, None).unwrap_err();
            assert!(
                why.contains(&format!("is open to others (mode {mode:04o})")),
                "{why}"
            );
            assert!(
                why.ends_with(&format!("chmod 600 {}", file.display())),
                "{why}"
            );
        }
        assert!(
            env(file_of_mode(dir.path(), "mine.token", 0o400, TOKEN))
                .open(None, None)
                .is_ok()
        );
        // No file, and one that holds no token.
        let why = env(dir.path().join("gone.token"))
            .open(None, None)
            .unwrap_err();
        assert!(why.starts_with("This device has no token"), "{why}");
        for none in ["", "\n", "two words", "a\nb"] {
            let file = file_of_mode(dir.path(), "none.token", 0o600, none);
            let why = env(file).open(None, None).unwrap_err();
            assert!(why.contains("holds no token"), "{none:?}: {why}");
        }
    }

    #[test]
    fn a_token_is_kept_for_its_user_alone_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        // The directory is not there before the first pairing.
        let file = dir.path().join("footplate/yard.token");
        keep_token(&file, "yardr-dev-one").unwrap();
        assert_eq!(read_token(&file).unwrap(), "yardr-dev-one");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // The next pairing's replaces it, also where the last was left open.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        keep_token(&file, "yardr-dev-two").unwrap();
        assert_eq!(read_token(&file).unwrap(), "yardr-dev-two");
        assert!(!dir.path().join("footplate/yard.token.new").exists());
    }

    #[test]
    fn what_a_qr_code_holds_is_an_address_and_a_code() {
        let both = |a: &str, c: &str| (a.to_string(), c.to_string());
        assert_eq!(
            typed(" https://mac.example.net/ ", " K7QM-2XPD "),
            both("https://mac.example.net", "K7QM-2XPD")
        );
        assert_eq!(
            typed("https://mac.example.net#K7QM-2XPD", ""),
            both("https://mac.example.net", "K7QM-2XPD")
        );
        // The code typed beside it is the newer one.
        assert_eq!(
            typed("https://mac.example.net#K7QM-2XPD", "abcd efgh"),
            both("https://mac.example.net", "abcd efgh")
        );
        assert_eq!(typed("", ""), both("", ""));
    }
}
