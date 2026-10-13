//! Which yard the window opens: one of this machine, by its socket, or one
//! at an address, as a device of its user. The environment names the one
//! yard of a run, else the settings list several and say which is open. A
//! device's token is kept in a file only its user reads, one for each yard
//! and never in the settings; the yard gives it for a pairing code.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::api::{self, Address, Client};
use crate::settings::Settings;

/// The file beside the settings that holds the token of the device of the
/// yard the environment names, and of the one yard of a settings file from
/// before there were several.
pub const TOKEN_FILE: &str = "yard.token";

/// What is said where a token's file would be beside the settings and
/// there are none.
const NO_HOME: &str = "There is no home directory to keep this device's token in. Name a file for it: FOOTPLATE_TOKEN_FILE";

/// What the environment says of the yard, read once when footplate starts.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `FOOTPLATE_YARD`: the address of a yard. The window is of that one
    /// yard then, whatever the settings list.
    pub yard: Option<String>,
    /// `FOOTPLATE_TOKEN_FILE`: the file the token of the device of
    /// `FOOTPLATE_YARD`'s yard is read from, and written to by a pairing.
    /// It wins over the file beside the settings, and without
    /// `FOOTPLATE_YARD` it says nothing: a yard of the settings has its
    /// own file, and the environment's is never lent to one.
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
    /// environment's, else that of the yard the settings open, where that
    /// is a far one. None is a yard of this machine.
    pub fn address<'a>(&'a self, set: &'a Settings) -> Option<&'a str> {
        let opened = set.opened();
        let far = opened.is_far().then_some(opened.address.as_str());
        self.yard.as_deref().or(far)
    }

    /// The file the token of the yard the environment names is in: the
    /// environment's, else the one beside the settings, where there are
    /// settings.
    pub fn token_file(&self, settings: Option<&Path>) -> Option<PathBuf> {
        self.token_file
            .clone()
            .or_else(|| Some(settings?.with_file_name(TOKEN_FILE)))
    }

    /// The file the token of the far yard called `name` is in, or of the
    /// yard the environment names where it names one: where it is read
    /// from, and where a pairing writes it. Two yards of a list edited by
    /// hand that have one file between them are refused, both, until their
    /// files differ.
    pub fn token_of(
        &self,
        set: &Settings,
        name: &str,
        settings: Option<&Path>,
    ) -> Result<PathBuf, String> {
        if self.yard.is_some() {
            return self.token_file(settings).ok_or(NO_HOME.to_string());
        }
        if let Some((other, shared)) = set.shares(name, settings) {
            return Err(format!(
                "The yards {name} and {} have one token file between them, {}, and neither is opened or paired until their files differ: name another for one of them where the settings are kept.",
                other.name,
                shared.display()
            ));
        }
        let yard = set
            .yard(name)
            .ok_or_else(|| format!("The settings list no yard called {name}."))?;
        token_path(&yard.token_file, settings)
    }

    /// A client of the yard to show, or why there is none: an address that
    /// is refused, or a token that is missing or open to others. It is the
    /// yard the environment names, else the one `set` opens; `settings` is
    /// the file they are kept in.
    pub fn open(&self, set: &Settings, settings: Option<&Path>) -> Result<Client, String> {
        let opened = set.opened();
        let Some(address) = self.address(set) else {
            return Ok(Client::new(match opened.socket.is_empty() {
                true => &self.socket,
                false => Path::new(&opened.socket),
            }));
        };
        let address = Address::parse(address)?;
        let file = self.token_of(set, &opened.name, settings)?;
        Ok(Client::remote(address, read_token(&file)?))
    }
}

/// Whether a yard's token file is named by a bare name, which is a file
/// beside the settings: footplate's own to make and to delete.
pub fn is_bare(token_file: &str) -> bool {
    let name = token_file.trim();
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

/// Where the token file a yard of the settings names is: beside the
/// settings for a bare name, and where it says for a whole path. Anything
/// else is refused with its reason: a path from wherever footplate was
/// started would be another file the next time.
pub fn token_path(token_file: &str, settings: Option<&Path>) -> Result<PathBuf, String> {
    let name = token_file.trim();
    if Path::new(name).is_absolute() && !name.ends_with('/') {
        return Ok(PathBuf::from(name));
    }
    if !is_bare(name) {
        return Err(format!(
            "{name:?} names no token file: one is a bare name, for a file beside the settings, or a whole path from /."
        ));
    }
    Ok(settings.ok_or(NO_HOME)?.with_file_name(name))
}

/// Whether two paths are one file, by whichever way its directory is
/// named: a link to the directory is the directory.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let whole = |path: &Path| {
        let dir = path.parent().and_then(|dir| fs::canonicalize(dir).ok());
        match (dir, path.file_name()) {
            (Some(dir), Some(name)) => dir.join(name),
            _ => path.to_path_buf(),
        }
    };
    whole(a) == whole(b)
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
    use crate::settings;

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
        let client = env.open(&Settings::default(), None).unwrap();
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
        let client = env.open(&Settings::default(), None).unwrap();
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
        assert!(!env.open(&Settings::default(), None).unwrap().is_remote());
    }

    #[test]
    fn the_environment_s_yard_is_the_one_yard_whatever_the_settings_list() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let beside = file_of_mode(dir.path(), "yard.token", 0o600, "yardr-dev-kept\n");
        let kept = Settings::default()
            .with_yard(
                None,
                settings::Yard::far("kept", "https://kept.example.net", "yard-kept.token"),
                Some(&settings),
            )
            .unwrap()
            .opening("kept");
        file_of_mode(dir.path(), "yard-kept.token", 0o600, "yardr-dev-kept\n");
        let env = Env::default();
        assert_eq!(env.address(&Settings::default()), None);
        assert_eq!(env.address(&kept), Some("https://kept.example.net"));
        let client = env.open(&kept, Some(&settings)).unwrap();
        assert_eq!(client.place(), "The yard: https://kept.example.net");

        // With FOOTPLATE_YARD the token is the file beside the settings,
        // as it was, or FOOTPLATE_TOKEN_FILE's.
        let env = Env {
            yard: Some("https://mac.example.net".into()),
            ..Env::default()
        };
        assert_eq!(env.token_file(Some(&settings)), Some(beside.clone()));
        assert_eq!(env.token_file(None), None);
        assert_eq!(env.token_of(&kept, "kept", Some(&settings)), Ok(beside));
        let named = file_of_mode(dir.path(), "mac.token", 0o600, "yardr-dev-named");
        let env = Env {
            token_file: Some(named.clone()),
            ..env
        };
        assert_eq!(env.token_of(&kept, "kept", Some(&settings)), Ok(named));
        let client = env.open(&kept, Some(&settings)).unwrap();
        assert_eq!(client.place(), "The yard: https://mac.example.net");
    }

    #[test]
    fn each_yard_of_the_settings_is_opened_by_its_own_socket_or_token_file() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let near = Yard::start();
        let second = Yard::start();
        let far = Yard::start_tcp();
        let set = Settings::default()
            .with_yard(
                None,
                settings::Yard::near("second home", second.socket().to_str().unwrap()),
                None,
            )
            .unwrap()
            .with_yard(
                None,
                settings::Yard::far("far", far.address(), "yard-far.token"),
                Some(&settings),
            )
            .unwrap();
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let place = |open: &str| {
            env.open(&set.clone().opening(open), Some(&settings))
                .map(|client| client.place())
        };
        // No socket named is this run's; a path is another home's yard.
        assert_eq!(
            place(settings::HERE).unwrap(),
            format!("The socket: {}", near.socket().display())
        );
        assert_eq!(
            place("second home").unwrap(),
            format!("The socket: {}", second.socket().display())
        );
        // The far one has no token yet, and says where one is looked for.
        let why = place("far").unwrap_err();
        assert!(
            why.starts_with(&format!(
                "This device has no token: {} is not there.",
                dir.path().join("yard-far.token").display()
            )),
            "{why}"
        );
        file_of_mode(dir.path(), "yard-far.token", 0o600, TOKEN);
        assert_eq!(
            place("far").unwrap(),
            format!("The yard: {}", far.address())
        );

        // FOOTPLATE_TOKEN_FILE without FOOTPLATE_YARD says nothing: a yard
        // of the settings is never called with the environment's token.
        let elsewhere = file_of_mode(dir.path(), "env.token", 0o600, "yardr-dev-env");
        let lent = Env {
            token_file: Some(elsewhere),
            ..env.clone()
        };
        assert_eq!(
            lent.token_of(&set, "far", Some(&settings)),
            Ok(dir.path().join("yard-far.token"))
        );
        fs::remove_file(dir.path().join("yard-far.token")).unwrap();
        assert!(
            lent.open(&set.clone().opening("far"), Some(&settings))
                .is_err()
        );
        // With no home there is nowhere beside the settings.
        let why = env.token_of(&set, "far", None).unwrap_err();
        assert!(why.starts_with("There is no home directory"), "{why}");
        assert!(env.token_of(&set, "nobody", Some(&settings)).is_err());
    }

    #[test]
    fn two_yards_on_one_token_file_are_refused_both_until_they_differ() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        file_of_mode(dir.path(), "yard.token", 0o600, TOKEN);
        // Only a list edited by hand comes to this.
        fs::write(
            &settings,
            r#"{"yards": [{"name": "mac", "address": "https://mac.example.net", "token_file": "yard.token"},
                          {"name": "twin", "address": "https://twin.example.net", "token_file": "yard.token"},
                          {"name": "shop", "address": "https://shop.example.net", "token_file": "shop.token"}]}"#,
        )
        .unwrap();
        let set = Settings::read(&settings);
        let env = Env::default();
        for (name, other) in [("mac", "twin"), ("twin", "mac")] {
            let why = env
                .open(&set.clone().opening(name), Some(&settings))
                .unwrap_err();
            assert!(
                why.starts_with(&format!(
                    "The yards {name} and {other} have one token file between them, {}",
                    dir.path().join("yard.token").display()
                )),
                "{why}"
            );
            assert_eq!(env.token_of(&set, name, Some(&settings)), Err(why));
        }
        assert_eq!(
            env.token_of(&set, "shop", Some(&settings)),
            Ok(dir.path().join("shop.token"))
        );
    }

    #[test]
    fn a_token_file_is_a_bare_name_beside_the_settings_or_a_whole_path() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        assert_eq!(
            token_path(" yard-mac.token ", Some(&settings)),
            Ok(dir.path().join("yard-mac.token"))
        );
        assert_eq!(
            token_path("/home/jo/secrets/mac.token", None),
            Ok(PathBuf::from("/home/jo/secrets/mac.token"))
        );
        for none in [
            "",
            "a/b.token",
            "../b.token",
            "./b.token",
            ".",
            "..",
            "/home/jo/",
        ] {
            let why = token_path(none, Some(&settings)).unwrap_err();
            assert!(why.contains("names no token file"), "{none:?}: {why}");
        }
        assert!(is_bare("yard.token") && !is_bare("/home/jo/yard.token"));
        // One file by two ways to its directory is one file.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        assert!(same_file(
            &dir.path().join("yard.token"),
            &link.join("yard.token")
        ));
        assert!(!same_file(
            &dir.path().join("yard.token"),
            &dir.path().join("yard-2.token")
        ));
    }

    #[test]
    fn a_settings_file_from_before_with_yard_token_beside_it_opens_paired_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let far = Yard::start_tcp();
        a_yard_of_jo(&far);
        fs::write(
            &settings,
            format!(
                "{{\n  \"text\": 14,\n  \"look\": \"auto\",\n  \"remote\": true,\n  \"address\": \"{}\"\n}}\n",
                far.address()
            ),
        )
        .unwrap();
        file_of_mode(dir.path(), "yard.token", 0o600, &format!("{TOKEN}\n"));
        let set = Settings::read(&settings);
        assert_eq!(set.yards.len(), 1);
        assert_eq!(set.opened().name, "127.0.0.1");
        let client = Env::default().open(&set, Some(&settings)).unwrap();
        assert_eq!(client.place(), format!("The yard: {}", far.address()));
        let state = session::load(&client, "mac-login").unwrap();
        let me = state.yard.unwrap();
        assert_eq!(
            (state.me.as_str(), me.said_by.as_str(), me.yard.as_str()),
            ("jo", "laptop", "south")
        );
        // The environment's token file is not this yard's: with it alone
        // the yard is as it was.
        let env = Env {
            token_file: Some("/nowhere/env.token".into()),
            ..Env::default()
        };
        assert!(env.open(&set, Some(&settings)).is_ok());
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
            .open(&Settings::default(), None)
            .unwrap_err();
        assert!(
            why.starts_with("http://mac.example.net:8790 is refused: plain http is for a yard on this machine alone"),
            "{why}"
        );
        assert!(why.contains("https://"), "{why}");
        assert!(
            env("http://192.168.1.20")
                .open(&Settings::default(), None)
                .is_err()
        );
        // This machine by any of its names, and anything with TLS.
        for here in [
            "http://127.0.0.1:8790",
            "http://localhost:8790",
            "http://[::1]:8790",
            "https://mac.example.net",
            "https://mac.example.net/yard/",
        ] {
            assert!(env(here).open(&Settings::default(), None).is_ok(), "{here}");
        }
        assert!(
            env("mac.example.net")
                .open(&Settings::default(), None)
                .is_err()
        );
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
            let why = env(file.clone())
                .open(&Settings::default(), None)
                .unwrap_err();
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
                .open(&Settings::default(), None)
                .is_ok()
        );
        // No file, and one that holds no token.
        let why = env(dir.path().join("gone.token"))
            .open(&Settings::default(), None)
            .unwrap_err();
        assert!(why.starts_with("This device has no token"), "{why}");
        for none in ["", "\n", "two words", "a\nb"] {
            let file = file_of_mode(dir.path(), "none.token", 0o600, none);
            let why = env(file).open(&Settings::default(), None).unwrap_err();
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
