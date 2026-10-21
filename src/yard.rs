//! Which yard the window opens: one of this machine, by its socket, or one
//! at an address, as a device of its user. The environment names the one
//! yard of a run, else the settings list several and say which is open. A
//! device's token is kept in a file only its user reads, one for each yard
//! and never in the settings; the yard gives it for a pairing code.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::api::{self, Address, Client};
use crate::settings::Settings;

/// The file beside the settings that holds the token of the device of the
/// yard the environment names, and of the one yard of a settings file from
/// before there were several.
pub const TOKEN_FILE: &str = "yard.token";

/// What is said where a token's file would be beside the settings and
/// there are none.
const NO_HOME: &str = "There is no home directory to keep this device's token in. Name a file for it: FOOTPLATE_TOKEN_FILE";

/// Marks a token file that footplate wrote after a pairing. A token from
/// elsewhere may have the same first line, but removing a yard must not
/// remove that file.
const FOOTPLATE_TOKEN: &str = "!footplate-token-v1";

/// Marks a session file that footplate wrote after signing in: a token
/// file's marker never stands in one, and this one never in a token's.
const FOOTPLATE_SESSION: &str = "!footplate-session-v1";

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
        if let Some(env_yard) = self.yard.as_deref() {
            let file = self.token_file(settings).ok_or(NO_HOME.to_string())?;
            let address = Address::parse(env_yard)?;
            if let Some(settings_file) = settings {
                if same_file(&file, settings_file) {
                    return Err(format!(
                        "The environment's token file {} is the file the settings are kept in, and cannot be used for a token.",
                        file.display()
                    ));
                }
                for listed in set.yards.iter().filter(|yard| yard.is_far()) {
                    let Ok(listed_file) = token_path(&listed.token_file, Some(settings_file))
                    else {
                        continue;
                    };
                    let listed_address_differs = Address::parse(&listed.address)
                        .map_or(true, |listed_address| !listed_address.is(&address));
                    if same_file(&file, &listed_file) && listed_address_differs {
                        return Err(format!(
                            "The environment's yard at {address} and the listed yard {} at {} share one token file, {}, and neither may use it for the other: set FOOTPLATE_TOKEN_FILE to a different file.",
                            listed.name,
                            listed.address,
                            file.display()
                        ));
                    }
                }
            }
            check_token_file_target(&file)?;
            return Ok(file);
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
        let file = token_path(&yard.token_file, settings)?;
        check_token_file_target(&file)?;
        Ok(file)
    }

    /// A client of the yard to show, or why there is none: an address that
    /// is refused, or a token that is missing or open to others. It is the
    /// yard the environment names, else the one `set` opens; `settings` is
    /// the file they are kept in. The environment's yard is reached direct;
    /// a hosted yard is opened from the settings, with its session beside
    /// its device token, and where neither is there signing in is what is
    /// said first.
    pub fn open(&self, set: &Settings, settings: Option<&Path>) -> Result<Client, String> {
        let opened = set.opened();
        let Some(address) = self.address(set) else {
            let socket = Path::new(&opened.socket);
            if opened.socket.is_empty() {
                return Ok(Client::new(&self.socket));
            }
            // The settings take no such socket where one is typed, and a
            // list edited by hand is held to the same: a path from
            // wherever footplate was started would be whoever's socket
            // lies there.
            if !socket.is_absolute() {
                return Err(format!(
                    "{:?} is no path of a socket: the yard {} wants it whole, from /, where the settings are kept.",
                    opened.socket, opened.name
                ));
            }
            return Ok(Client::new(socket));
        };
        let address = Address::parse(address)?;
        let session = match self.session_of(set, &opened.name, settings)? {
            Some(file) => Some(session_for(&file, &address)?),
            None => None,
        };
        let file = self.token_of(set, &opened.name, settings)?;
        let token = token_for(&file, &address)?;
        Ok(match session {
            Some(session) => Client::remote_hosted(address, token, session),
            None => Client::remote(address, token),
        })
    }

    /// The file the session token of the far yard called `name` is in,
    /// where that yard is a hosted one: where it is read from, and where
    /// signing in writes it. `None` is a yard reached direct, and the one
    /// yard the environment names, which is direct too. A file another
    /// yard's token or session is in, and the yard's own token file, are
    /// refused as a session file.
    pub fn session_of(
        &self,
        set: &Settings,
        name: &str,
        settings: Option<&Path>,
    ) -> Result<Option<PathBuf>, String> {
        if self.yard.is_some() {
            return Ok(None);
        }
        let Some(yard) = set.yard(name) else {
            return Err(format!("The settings list no yard called {name}."));
        };
        if !yard.is_hosted() {
            return Ok(None);
        }
        if let Some((other, shared)) = set.session_shares(name, settings) {
            return Err(format!(
                "The yard {name} keeps its session in the file the yard {} keeps a token in, {}, and is not opened until they differ: name another for one of them where the settings are kept.",
                other.name,
                shared.display()
            ));
        }
        let file = token_path(&yard.session_file, settings)?;
        if token_path(&yard.token_file, settings).is_ok_and(|token| same_file(&file, &token)) {
            return Err(format!(
                "The yard {name} keeps its session in its device token's file, {}, and is not opened until they differ: name another for one of them where the settings are kept.",
                file.display()
            ));
        }
        check_token_file_target(&file)?;
        Ok(Some(file))
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
    let path = if Path::new(name).is_absolute() && !name.ends_with('/') {
        PathBuf::from(name)
    } else if is_bare(name) {
        settings.ok_or(NO_HOME)?.with_file_name(name)
    } else {
        return Err(format!(
            "{name:?} names no token file: one is a bare name, for a file beside the settings, or a whole path from /."
        ));
    };
    // A pairing writes this file and removing the yard deletes it: the
    // settings' own is neither's to take.
    if settings.is_some_and(|settings| same_file(&path, settings)) {
        return Err(format!(
            "{name:?} names no token file: it is the file the settings are kept in."
        ));
    }
    Ok(path)
}

/// A pairing must not spend its one-time code on a target that cannot be a
/// token file. A link is allowed here because pairing replaces the link
/// itself atomically; reading follows it and checks the target's type.
fn check_token_file_target(file: &Path) -> Result<(), String> {
    match fs::symlink_metadata(file) {
        Ok(meta) if meta.file_type().is_file() || meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(format!(
            "The token file {} is not a regular file, and is refused.",
            file.display()
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "The token file {} cannot be checked: {err}",
            file.display()
        )),
    }
}

/// Whether two paths are one file, by whichever way its directory is
/// named: a link to the directory is the directory.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(a_meta), Ok(b_meta)) = (fs::metadata(a), fs::metadata(b)) {
        return a_meta.dev() == b_meta.dev() && a_meta.ino() == b_meta.ino();
    }
    resolved_path(a) == resolved_path(b)
}

/// The canonical path when it exists; otherwise, the longest existing
/// prefix resolved through links, with the rest normalized and appended.
/// Token files may not exist yet when two settings entries are compared.
fn resolved_path(path: &Path) -> PathBuf {
    if let Ok(path) = fs::canonicalize(path) {
        return path;
    }

    let mut prefix = path.to_path_buf();
    let mut suffix = Vec::new();
    while !prefix.exists() {
        let Some(name) = prefix.file_name() else {
            return normalize_path(path);
        };
        suffix.push(name.to_os_string());
        let Some(parent) = prefix.parent() else {
            return normalize_path(path);
        };
        prefix = parent.to_path_buf();
    }

    let Ok(mut resolved) = fs::canonicalize(&prefix) else {
        return normalize_path(path);
    };
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    normalize_path(&resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some_and(|name| name != "..") {
                    normalized.pop();
                } else if !normalized.is_absolute() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// A credential kept in a file of its own: a device's token or a hosted
/// yard's session token. Both are read the same way; only the marker and
/// the words differ.
#[derive(Clone, Copy)]
pub(crate) struct KeptKind {
    /// The marker footplate writes under a token it kept.
    marker: &'static str,
    /// `token` or `session`, as a refusal names the file and what it holds.
    noun: &'static str,
    /// Why the file may not be read by others, past the mode.
    open: &'static str,
    /// What gave the token: `the yard` or `signing in`.
    giver: &'static str,
}

pub(crate) const TOKEN_KIND: KeptKind = KeptKind {
    marker: FOOTPLATE_TOKEN,
    noun: "token",
    open: "whoever reads it calls the yard as you",
    giver: "the yard",
};

pub(crate) const SESSION_KIND: KeptKind = KeptKind {
    marker: FOOTPLATE_SESSION,
    noun: "session",
    open: "whoever reads it is signed in as this yard's user",
    giver: "signing in",
};

impl KeptKind {
    /// The token `file` holds, and the address it is for where the file
    /// says.
    fn read(&self, file: &Path) -> Result<KeptToken, String> {
        kept_as(file, *self)
    }

    /// Why there is no token where the file is not there: pairing for a
    /// device, signing in for a session.
    fn missing(&self, at: &std::path::Display<'_>) -> String {
        match self.noun {
            "session" => {
                format!("Not signed in: {at} is not there. Sign in to this hosted yard again.")
            }
            _ => format!(
                "This device has no token: {at} is not there. Pair it in the settings, with a code the yard's operator opens: yardr user device pair <user> <device>"
            ),
        }
    }
}

/// Reads a device's token from its file, and the address of the yard it
/// is for where the file says. A file others can read is refused, as every
/// file of a secret is in yardr: the token is all that says who a request
/// is.
struct KeptToken {
    token: String,
    address: Option<Address>,
    ends_newline: bool,
    managed: bool,
}

fn kept(file: &Path) -> Result<KeptToken, String> {
    TOKEN_KIND.read(file)
}

fn kept_session(file: &Path) -> Result<KeptToken, String> {
    SESSION_KIND.read(file)
}

fn kept_as(file: &Path, kind: KeptKind) -> Result<KeptToken, String> {
    let at = file.display();
    let noun = kind.noun;
    let meta = match fs::metadata(file) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(kind.missing(&at));
        }
        Err(err) => return Err(format!("The {noun} file {at} cannot be read: {err}")),
    };
    if !meta.file_type().is_file() {
        return Err(format!(
            "The {noun} file {at} is not a regular file, and is refused."
        ));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "The {noun} file {at} is open to others (mode {mode:04o}), and is refused: {}. Make it yours alone: chmod 600 {at}",
            kind.open
        ));
    }
    let text = fs::read_to_string(file)
        .map_err(|err| format!("The {noun} file {at} cannot be read: {err}"))?;
    let ends_newline = text.ends_with('\n');
    let none = || {
        format!(
            "The {noun} file {at} holds no {noun}: one is a line, as {} gave it, with footplate's marker and the address of its {} where it wrote them.",
            kind.giver,
            match noun {
                "session" => "edge",
                _ => "yard",
            }
        )
    };
    let lines: Vec<&str> = text.trim().lines().collect();
    let token = lines.first().copied().unwrap_or_default().trim();
    let (managed, address_line) = match lines.get(1) {
        Some(line) if line.trim() == kind.marker => (true, lines.get(2).copied()),
        Some(line) => (false, Some(*line)),
        None => (false, None),
    };
    let line_count = match managed {
        true if address_line.is_some() => 3,
        true => 2,
        false if address_line.is_some() => 2,
        false => 1,
    };
    if !api::is_token(token) || lines.len() != line_count {
        return Err(none());
    }
    match address_line.map(Address::parse).transpose() {
        Ok(address) => Ok(KeptToken {
            token: token.to_string(),
            address,
            ends_newline,
            managed,
        }),
        Err(_) => Err(none()),
    }
}

/// The token `file` holds, whichever yard it is for: what a test reads
/// back. A request takes its token from [`token_for`].
#[cfg(test)]
pub fn read_token(file: &Path) -> Result<String, String> {
    kept(file).map(|kept| kept.token)
}

/// The session `file` holds, whichever edge it is for: what a test reads
/// back. A request takes its session from [`session_for`].
#[cfg(test)]
pub fn read_session(file: &Path) -> Result<String, String> {
    kept_session(file).map(|kept| kept.token)
}

/// Whether the tests run as root, which no file mode holds: a test of a
/// refusal a mode carries says so and returns instead of trying it.
#[cfg(test)]
pub fn running_as_root() -> bool {
    // SAFETY: geteuid takes nothing, touches nothing, and fails nowhere.
    unsafe { libc::geteuid() == 0 }
}

/// Whether this is a regular token file that a pairing wrote. A settings
/// entry may name an existing token from another program; removing that
/// entry must leave it there.
pub(crate) fn is_managed_token_file(file: &Path) -> bool {
    if !fs::symlink_metadata(file).is_ok_and(|meta| meta.file_type().is_file()) {
        return false;
    }
    kept(file).is_ok_and(|kept| kept.managed)
}

/// Whether this is a regular session file that signing in wrote. A
/// settings entry may name an existing session from another program;
/// removing that entry must leave it there.
pub(crate) fn is_managed_session_file(file: &Path) -> bool {
    if !fs::symlink_metadata(file).is_ok_and(|meta| meta.file_type().is_file()) {
        return false;
    }
    kept_session(file).is_ok_and(|kept| kept.managed)
}

/// The token `file` holds, to call the yard at `address` with. A token is
/// for the yard that gave it and for no other: the file names that yard's
/// address under the token, and with another address the token is refused
/// and sent nowhere, until the device is paired there. So an address
/// changed where the settings are kept, or a file two yards came to name,
/// gives one yard's token to nobody else. A pairing writes the address
/// with the token. A file that names no address, as every file was before
/// this and as one from another program is, is bound
/// to the first yard it is opened for only when that address can be saved;
/// otherwise no request is sent.
pub fn token_for(file: &Path, address: &Address) -> Result<String, String> {
    kept_for(file, address, TOKEN_KIND, |at, given, address| {
        format!(
            "The token in {at} is of this device at {given}, and is not sent to {address}: whoever answers there would call that yard as you. Pair this device with the yard at {address} in the settings, with a code its operator opens: yardr user device pair <user> <device>"
        )
    })
}

/// The session `file` holds, to call the hosted yard at `address` with. A
/// session is for the edge that signed it and for no other: the file names
/// that edge's address under the session, and with another address the
/// session is refused and sent nowhere, until signed in there. Signing in
/// writes the address with the session. A file that names no address, as
/// one from another program is, is bound to the first edge it is opened
/// for only when that address can be saved; otherwise no request is sent.
pub fn session_for(file: &Path, address: &Address) -> Result<String, String> {
    kept_for(file, address, SESSION_KIND, |at, given, address| {
        format!(
            "The session in {at} is this yard's sign-in at {given}, and is not sent to {address}: whoever answers there could sign in as you. Sign in to the yard at {address}."
        )
    })
}

/// The token `file` holds for the yard at `address`: refused and sent
/// nowhere with another address than the one the file names, and bound
/// to `address` where the file names none. `wrong` says a token of
/// another address, with what to do instead.
fn kept_for(
    file: &Path,
    address: &Address,
    kind: KeptKind,
    wrong: impl FnOnce(&std::path::Display<'_>, &Address, &Address) -> String,
) -> Result<String, String> {
    let saved = kind.read(file)?;
    let mut token = saved.token;
    let at = file.display();
    let noun = kind.noun;
    let edge = match noun {
        "session" => "edge",
        _ => "yard",
    };
    let instead = match noun {
        "session" => "Sign in to that yard.",
        _ => "Pair this device with that yard.",
    };
    match saved.address {
        Some(given) if given.is(address) => {}
        Some(given) => {
            return Err(wrong(&at, &given, address));
        }
        // A token without a recorded address is not safe to send until the
        // address is durably bound to it.
        None => {
            let line = match saved.ends_newline {
                true => format!("{address}\n"),
                false => format!("\n{address}\n"),
            };
            let bound = fs::OpenOptions::new()
                .append(true)
                .open(file)
                .and_then(|mut out| out.write_all(line.as_bytes()));
            if bound.is_err() {
                return Err(format!(
                    "The {noun} file {at} has no saved {edge} address and cannot be safely bound to {address}; no request was sent. {instead}",
                ));
            }
            let verified = kind.read(file).map_err(|_| {
                format!(
                    "The {noun} file {at} could not be safely bound to {address}; no request was sent. {instead}",
                )
            })?;
            if !verified
                .address
                .as_ref()
                .is_some_and(|given| given.is(address))
            {
                return Err(format!(
                    "The {noun} file {at} could not be safely bound to {address}; no request was sent. {instead}",
                ));
            }
            token = verified.token;
        }
    }
    Ok(token)
}

/// Where a pairing's token will go: the private sibling of its file (mode
/// 0600) that is renamed into place once the yard has given the token. It
/// is made before the code is sent, since a code is used up by the yard it
/// is sent to: a file that cannot be written is found while the code is
/// still good. Dropped without a token, it leaves nothing behind.
pub struct TokenPlace {
    file: PathBuf,
    new: PathBuf,
    out: fs::File,
    kept: bool,
}

/// Makes the place a token is written to before it becomes `file`, or says
/// why `file` cannot be written: something other than a file is there, or
/// its directory cannot be made or written to.
pub fn token_place(file: &Path) -> io::Result<TokenPlace> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    match fs::symlink_metadata(file) {
        Ok(meta) if meta.file_type().is_file() || meta.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "token path is not a regular file",
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut name = std::ffi::OsString::from(".footplate-token-");
    name.push(std::process::id().to_string());
    name.push("-");
    name.push(file.file_name().unwrap_or_default());

    // Never clear a fixed sibling: a settings entry may name that path as
    // its own token file. Create a private candidate and leave collisions
    // untouched, including leftovers from an interrupted write.
    loop {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let mut candidate = name.clone();
        candidate.push(format!("-{sequence}.new"));
        let new = file.with_file_name(candidate);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&new)
        {
            Ok(out) => {
                return Ok(TokenPlace {
                    file: file.to_path_buf(),
                    new,
                    out,
                    kept: false,
                });
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
}

impl TokenPlace {
    /// Keeps a device's token in the file, for its user alone, in place of
    /// the one that was there, with the address of the yard that gave it
    /// under it: a token is never on disk without its yard. The file is
    /// whole or not there: it is written beside itself and renamed.
    pub fn keep(self, token: &str, address: &Address) -> io::Result<()> {
        self.keep_as(token, Some(address), TOKEN_KIND)
    }

    /// The same private, atomic write for a device token or hosted yard's
    /// session. The marker distinguishes the two files; an addressed
    /// session can be written by the hosted sign-in flow without ever
    /// putting its value in settings.
    pub(crate) fn keep_as(
        mut self,
        token: &str,
        address: Option<&Address>,
        kind: KeptKind,
    ) -> io::Result<()> {
        let mut text = format!("{token}\n{}\n", kind.marker);
        if let Some(address) = address {
            text.push_str(&format!("{address}\n"));
        }
        self.out.write_all(text.as_bytes())?;
        self.out.sync_all()?;
        fs::rename(&self.new, &self.file)?;
        self.kept = true;
        Ok(())
    }
}

impl Drop for TokenPlace {
    /// What was not renamed into place is removed: no pairing, or a write
    /// that failed. Once it is the token file its old name is not touched,
    /// which may by then be another's.
    fn drop(&mut self) {
        if !self.kept {
            let _ = fs::remove_file(&self.new);
        }
    }
}

/// Keeps a session token in `file`, for its user alone, with the
/// address of the edge that signed it under it: a session is never on
/// disk without its edge. The file is whole or not there: it is written
/// beside itself and renamed.
pub fn keep_session(file: &Path, token: &str, address: &Address) -> io::Result<()> {
    token_place(file)?.keep_as(token, Some(address), SESSION_KIND)
}

/// A legacy token alone in `file`, for tests that need a file before its
/// address has been bound by [`token_for`].
#[cfg(test)]
pub fn keep_token(file: &Path, token: &str) -> io::Result<()> {
    token_place(file)?.keep_as(token, None, TOKEN_KIND)
}

/// A session token alone in `file`, for tests that need a file before its
/// address has been bound by [`session_for`].
#[cfg(test)]
pub fn keep_session_token(file: &Path, token: &str) -> io::Result<()> {
    token_place(file)?.keep_as(token, None, SESSION_KIND)
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
        yard.answer("Arrivals", r#"{"result": []}"#);
        yard.answer(
            "Flows",
            r#"{"result": {"travelling": 0, "depots": [], "unassigned": []}}"#,
        );
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        yard.answer("SourceGrants", r#"{"result": []}"#);
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
        // Whose window it is is read, with the grants where it is the
        // operator's.
        assert_eq!(far.asked("Settings"), vec![serde_json::json!({})]);
        assert_eq!(state.operator, "jo");
        assert!(state.grants.is_empty());
        assert_eq!(far.asked("SourceGrants"), vec![serde_json::json!({})]);
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
    fn the_environment_cannot_use_the_settings_or_a_special_file_as_its_token_file() {
        use std::os::unix::fs::FileTypeExt;
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, "{}").unwrap();
        let set = Settings::default();
        let env = |token_file| Env {
            yard: Some("https://mac.example.net".into()),
            token_file: Some(token_file),
            ..Env::default()
        };

        let why = env(settings.clone())
            .token_of(&set, "", Some(&settings))
            .unwrap_err();
        assert!(why.contains("file the settings are kept in"), "{why}");

        let socket = dir.path().join("token.socket");
        let listener = UnixListener::bind(&socket).unwrap();
        drop(listener);
        let why = env(socket.clone())
            .token_of(&set, "", Some(&settings))
            .unwrap_err();
        assert!(why.contains("not a regular file"), "{why}");
        assert!(
            fs::symlink_metadata(socket)
                .unwrap()
                .file_type()
                .is_socket()
        );
    }

    #[test]
    fn only_a_well_formed_paired_token_file_is_managed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("yard.token");
        keep_token(&file, TOKEN).unwrap();
        assert!(is_managed_token_file(&file));

        fs::write(
            &file,
            format!("{TOKEN}\n{FOOTPLATE_TOKEN}\nnot-an-address\nextra\n"),
        )
        .unwrap();
        assert!(!is_managed_token_file(&file));
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
    fn token_paths_that_are_file_links_or_hard_links_are_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let token = file_of_mode(dir.path(), "yard.token", 0o600, TOKEN);
        let symbolic = dir.path().join("symbolic.token");
        let hard = dir.path().join("hard.token");
        std::os::unix::fs::symlink(&token, &symbolic).unwrap();
        std::fs::hard_link(&token, &hard).unwrap();

        assert!(same_file(&token, &symbolic));
        assert!(same_file(&token, &hard));

        let settings = dir.path().join("settings.json");
        let set = Settings::default()
            .with_yard(
                None,
                settings::Yard::far("mac", "https://mac.example.net", "yard.token"),
                Some(&settings),
            )
            .unwrap();
        for alias in ["symbolic.token", "hard.token"] {
            let why = set
                .clone()
                .with_yard(
                    None,
                    settings::Yard::far("plantime", "https://plantime.example.net", alias),
                    Some(&settings),
                )
                .unwrap_err();
            assert!(why.contains("two yards may not share one"), "{why}");
        }
    }

    #[test]
    fn keeping_a_token_does_not_remove_a_neighboring_yard_s_token_file() {
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("yard-mac.token");
        let other = dir.path().join("yard-mac.token.new");
        fs::write(&other, "yardr-dev-other\n").unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o600)).unwrap();

        keep_token(&token, "yardr-dev-mac").unwrap();

        assert_eq!(read_token(&token).unwrap(), "yardr-dev-mac");
        assert_eq!(read_token(&other).unwrap(), "yardr-dev-other");
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
        // This machine's yard is listed after it, and it is the one opened.
        assert_eq!(set.yards.len(), 2);
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
        // A token as a pairing left it, of no yard yet, for each address.
        let env = |yard: &str| Env {
            yard: Some(yard.into()),
            token_file: Some(file_of_mode(dir.path(), "mac.token", 0o600, TOKEN)),
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
    fn a_token_goes_to_the_yard_it_was_first_opened_for_and_to_no_other_address() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let token = dir.path().join("yard-mac.token");
        keep_token(&token, TOKEN).unwrap();
        let at = |address: &str| {
            Settings::default()
                .with_yard(
                    None,
                    settings::Yard::far("mac", address, "yard-mac.token"),
                    Some(&settings),
                )
                .unwrap()
                .opening("mac")
        };
        let env = Env::default();
        let open = |address: &str| env.open(&at(address), Some(&settings));

        // Opened once, the file says whose the token is, and it is the
        // same yard by another spelling of its address.
        assert!(open("https://mac.example.net").is_ok());
        assert_eq!(
            fs::read_to_string(&token).unwrap(),
            format!("{TOKEN}\n{FOOTPLATE_TOKEN}\nhttps://mac.example.net\n")
        );
        assert_eq!(read_token(&token).unwrap(), TOKEN);
        assert!(open("https://MAC.example.net:443/").is_ok());

        // The address changed where the settings are kept: the token is
        // of the yard that gave it, and is not read out for another host,
        // port, path or scheme, nor named in the refusal.
        for other in [
            "https://plantime.example.net",
            "https://mac.example.net:8443",
            "https://mac.example.net/yard",
        ] {
            let why = open(other).unwrap_err();
            assert!(
                why.contains(&format!(
                    "is of this device at https://mac.example.net, and is not sent to {other}"
                )),
                "{why}"
            );
            assert!(!why.contains(TOKEN), "{why}");
            assert_eq!(token_for(&token, &Address::parse(other).unwrap()), Err(why));
        }
        // Nor for the one yard the environment names, whose token file
        // without FOOTPLATE_TOKEN_FILE is the one beside the settings: a
        // yard of the settings may keep its token there.
        let beside = dir.path().join(TOKEN_FILE);
        keep_token(&beside, TOKEN).unwrap();
        let listed = Settings::default()
            .with_yard(
                None,
                settings::Yard::far("mac", "https://mac.example.net", TOKEN_FILE),
                Some(&settings),
            )
            .unwrap()
            .opening("mac");
        assert!(env.open(&listed, Some(&settings)).is_ok());
        let named = Env {
            yard: Some("https://trial.example.net".into()),
            ..Env::default()
        };
        let trial = Yard::start_tcp();
        a_yard_of_jo(&trial);
        let named = Env {
            yard: Some(trial.address().to_string()),
            ..named
        };
        // Before the old yard has ever been opened by this version, its
        // one-line token has no recorded address. An environment override
        // must not claim it for another yard just because it opens first.
        let result = named.open(&listed, Some(&settings));
        if let Ok(client) = &result {
            let _ = client.head();
        }
        let why = result.unwrap_err();
        assert!(
            why.contains("share one token file") && why.contains("listed yard mac"),
            "{why}"
        );
        assert!(trial.asked("Head").is_empty());

        // Paired again, the token is the new yard's.
        keep_token(&token, "yardr-dev-plantime").unwrap();
        assert!(open("https://plantime.example.net").is_ok());
        assert!(open("https://mac.example.net").is_err());

        // A file from before ends as it was written, and is told the same.
        let old = file_of_mode(dir.path(), "old.token", 0o600, TOKEN);
        let mac = Address::parse("https://mac.example.net").unwrap();
        assert_eq!(token_for(&old, &mac).unwrap(), TOKEN);
        assert_eq!(
            fs::read_to_string(&old).unwrap(),
            format!("{TOKEN}\nhttps://mac.example.net\n")
        );
        // What stands under a token and is no address is no token file.
        for none in [format!("{TOKEN}\nmac"), format!("{TOKEN}\n{mac}\n{mac}")] {
            let file = file_of_mode(dir.path(), "none.token", 0o600, &none);
            let why = token_for(&file, &mac).unwrap_err();
            assert!(why.contains("holds no token"), "{why}");
        }
    }

    #[test]
    fn a_hosted_yard_opens_with_its_session_beside_its_token() {
        use crate::fake::{Router, SESSION};

        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let yard = Yard::start_tcp();
        a_yard_of_jo(&yard);
        let edge = Router::start(&yard, SESSION);
        let set = Settings::default()
            .with_yard(
                None,
                settings::Yard::hosted(
                    "south",
                    edge.address(),
                    "yard-south.token",
                    "yard-south.session",
                ),
                Some(&settings),
            )
            .unwrap()
            .opening("south");
        let env = Env::default();
        // Neither file there: signing in is said first.
        let why = env.open(&set, Some(&settings)).unwrap_err();
        assert!(why.starts_with("Not signed in: "), "{why}");
        assert!(why.contains("yard-south.session"), "{why}");
        // The session alone: the device is still to pair.
        keep_session_token(&dir.path().join("yard-south.session"), SESSION).unwrap();
        let why = env.open(&set, Some(&settings)).unwrap_err();
        assert!(why.starts_with("This device has no token"), "{why}");
        // Both: every call carries both.
        keep_token(&dir.path().join("yard-south.token"), TOKEN).unwrap();
        let client = env.open(&set, Some(&settings)).unwrap();
        let state = session::load(&client, "mac-login").unwrap();
        assert_eq!(state.me, "jo");
        let request = edge.last_request();
        assert!(
            request.contains(&format!("X-Edge-Token: {SESSION}\r\n")),
            "{request}"
        );
        assert!(
            request.contains(&format!("Authorization: Bearer {TOKEN}\r\n")),
            "{request}"
        );
        // A bare session is bound to the edge on its first opening, and
        // the settings hold neither token.
        assert_eq!(
            fs::read_to_string(dir.path().join("yard-south.session")).unwrap(),
            format!("{SESSION}\n{FOOTPLATE_SESSION}\n{}\n", edge.address())
        );
        set.write(&settings).unwrap();
        let kept = fs::read_to_string(&settings).unwrap();
        assert!(!kept.contains(TOKEN) && !kept.contains(SESSION));

        // The environment names a direct yard: no session rides along,
        // and the edge says to sign in.
        let direct = Env {
            yard: Some(edge.address().to_string()),
            token_file: Some(dir.path().join("yard-south.token")),
            ..Env::default()
        };
        let client = direct.open(&set, Some(&settings)).unwrap();
        assert!(matches!(client.head(), Err(api::Error::NotSignedIn)));
    }

    #[test]
    fn a_session_goes_to_the_edge_it_was_first_opened_for_and_to_no_other() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        keep_session_token(&dir.path().join("yard-south.session"), "kratos-dev-south").unwrap();
        keep_token(&dir.path().join("yard-south.token"), TOKEN).unwrap();
        let at = |address: &str| {
            Settings::default()
                .with_yard(
                    None,
                    settings::Yard::hosted(
                        "south",
                        address,
                        "yard-south.token",
                        "yard-south.session",
                    ),
                    Some(&settings),
                )
                .unwrap()
                .opening("south")
        };
        let env = Env::default();
        let open = |address: &str| env.open(&at(address), Some(&settings));

        // Opened once, the file says whose the session is.
        assert!(open("https://south.example.net").is_ok());
        assert_eq!(
            read_session(&dir.path().join("yard-south.session")).unwrap(),
            "kratos-dev-south"
        );
        // The address changed where the settings are kept: the session is
        // of the edge that signed it, and is not read out for another,
        // nor named in the refusal.
        for other in [
            "https://plantime.example.net",
            "https://south.example.net:8443",
        ] {
            let why = open(other).unwrap_err();
            assert!(
                why.contains(
                    "is this yard's sign-in at https://south.example.net, and is not sent to"
                ),
                "{why}"
            );
            assert!(why.contains("Sign in to the yard at"), "{why}");
            assert!(!why.contains("kratos-dev-south"), "{why}");
            assert_eq!(
                session_for(
                    &dir.path().join("yard-south.session"),
                    &Address::parse(other).unwrap()
                ),
                Err(why)
            );
        }
    }

    #[test]
    fn a_kept_session_is_private_and_bound_to_its_edge() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("yard.session");
        let address = Address::parse("https://south.example.net").unwrap();
        let session = "kratos-dev-south";

        token_place(&file)
            .unwrap()
            .keep_as(session, Some(&address), SESSION_KIND)
            .unwrap();

        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("{session}\n{FOOTPLATE_SESSION}\n{address}\n")
        );
        assert_eq!(session_for(&file, &address).unwrap(), session);
        assert!(is_managed_session_file(&file));
    }

    #[test]
    fn a_session_file_open_to_others_or_holding_none_is_refused() {
        // No file mode holds root: what is refused below for another
        // user is let through here.
        if running_as_root() {
            eprintln!("skipped: root (euid 0) is not held by a file's mode");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let at = |session: &str| {
            Settings::default()
                .with_yard(
                    None,
                    settings::Yard::hosted(
                        "south",
                        "https://south.example.net",
                        "yard-south.token",
                        session,
                    ),
                    Some(&settings),
                )
                .unwrap()
                .opening("south")
        };
        let env = Env::default();
        for mode in [0o644, 0o640] {
            let file = file_of_mode(dir.path(), "open.session", mode, "kratos-dev-south\n");
            let why = env.open(&at("open.session"), Some(&settings)).unwrap_err();
            assert!(
                why.contains(&format!("is open to others (mode {mode:04o})")),
                "{why}"
            );
            assert!(
                why.ends_with(&format!("chmod 600 {}", file.display())),
                "{why}"
            );
        }
        for none in ["", "\n", "two words"] {
            let file = file_of_mode(dir.path(), "none.session", 0o600, none);
            let why = env.open(&at("none.session"), Some(&settings)).unwrap_err();
            assert!(why.contains("holds no session"), "{none:?}: {why}");
            assert!(
                session_for(&file, &Address::parse("https://south.example.net").unwrap()).is_err()
            );
        }
    }

    #[test]
    fn a_session_file_shared_or_the_token_s_is_refused_until_they_differ() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        file_of_mode(dir.path(), "yard.token", 0o600, TOKEN);
        // Only a list edited by hand comes to this.
        fs::write(
            &settings,
            r#"{"yards": [{"name": "mac", "address": "https://mac.example.net", "token_file": "yard.token"},
                          {"name": "south", "address": "https://south.example.net", "token_file": "yard-south.token", "session_file": "yard.token"},
                          {"name": "twin", "address": "https://twin.example.net", "token_file": "yard-twin.token", "session_file": "yard-twin.token"}]}"#,
        )
        .unwrap();
        let set = Settings::read(&settings);
        let env = Env::default();
        let why = env
            .open(&set.clone().opening("south"), Some(&settings))
            .unwrap_err();
        assert!(
            why.starts_with(&format!(
                "The yard south keeps its session in the file the yard mac keeps a token in, {}",
                dir.path().join("yard.token").display()
            )),
            "{why}"
        );
        let why = env
            .open(&set.clone().opening("twin"), Some(&settings))
            .unwrap_err();
        assert!(
            why.starts_with(&format!(
                "The yard twin keeps its session in its device token's file, {}",
                dir.path().join("yard-twin.token").display()
            )),
            "{why}"
        );
        // The direct yard opens as ever: the sharing is the hosted one's
        // to answer.
        assert_eq!(env.session_of(&set, "mac", Some(&settings)), Ok(None));
    }

    #[test]
    fn only_a_well_formed_session_file_is_managed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("yard.session");
        keep_session_token(&file, "kratos-dev-south").unwrap();
        assert!(is_managed_session_file(&file));
        // The markers do not cross: neither file is the other's kind.
        assert!(!is_managed_token_file(&file));
        let token = dir.path().join("yard.token");
        keep_token(&token, TOKEN).unwrap();
        assert!(is_managed_token_file(&token));
        assert!(!is_managed_session_file(&token));

        fs::write(
            &file,
            format!("kratos-dev-south\n{FOOTPLATE_SESSION}\nnot-an-address\nextra\n"),
        )
        .unwrap();
        assert!(!is_managed_session_file(&file));
    }

    #[test]
    fn signing_in_keeps_the_session_with_its_edge_for_its_user_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("yard-south.session");
        let address = Address::parse("https://south.example.net").unwrap();
        keep_session(&file, "kratos-dev-south", &address).unwrap();
        // Mode 0600, the session's marker, the edge's address, and the
        // session reads back for that edge.
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "kratos-dev-south\n!footplate-session-v1\nhttps://south.example.net\n"
        );
        assert!(is_managed_session_file(&file));
        assert!(!is_managed_token_file(&file));
        assert_eq!(session_for(&file, &address).unwrap(), "kratos-dev-south");
        // And for no other edge.
        let elsewhere = Address::parse("https://north.example.net").unwrap();
        assert!(session_for(&file, &elsewhere).is_err());
    }

    #[test]
    fn the_settings_file_is_no_token_file_and_a_socket_is_a_whole_path() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        // Only a list edited by hand comes to either.
        fs::write(
            &settings,
            r#"{"yards": [{"name": "mac", "address": "https://mac.example.net", "token_file": "settings.json"},
                          {"name": "shop", "socket": "run/yard.sock"}]}"#,
        )
        .unwrap();
        let set = Settings::read(&settings);
        let whole = settings.to_str().unwrap();
        let link = dir.path().join("link.token");
        std::os::unix::fs::symlink(&settings, &link).unwrap();
        for own in ["settings.json", whole, "link.token"] {
            let why = token_path(own, Some(&settings)).unwrap_err();
            assert!(
                why.contains("it is the file the settings are kept in"),
                "{own}: {why}"
            );
        }
        let env = Env::default();
        assert!(env.token_of(&set, "mac", Some(&settings)).is_err());
        assert!(
            env.open(&set.clone().opening("mac"), Some(&settings))
                .is_err()
        );
        // Removing that yard leaves the settings where they are.
        let (less, left, _) = set.clone().without("mac", Some(&settings));
        assert_eq!(left, settings::TokenLeft::None);
        assert!(settings.exists() && less.yard("mac").is_none());

        let why = env.open(&set.opening("shop"), Some(&settings)).unwrap_err();
        assert!(
            why.starts_with("\"run/yard.sock\" is no path of a socket: the yard shop"),
            "{why}"
        );
    }

    #[test]
    fn a_token_file_open_to_others_is_refused_with_the_reason() {
        // No file mode holds root: what is refused below for another
        // user is let through here.
        if running_as_root() {
            eprintln!("skipped: root (euid 0) is not held by a file's mode");
            return;
        }
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
        let read_only = file_of_mode(dir.path(), "mine.token", 0o400, TOKEN);
        let why = token_for(
            &read_only,
            &Address::parse("https://mac.example.net").unwrap(),
        )
        .unwrap_err();
        assert!(
            why.contains("cannot be safely bound") && why.contains("no request was sent"),
            "{why}"
        );
        assert!(env(read_only).open(&Settings::default(), None).is_err());
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
    fn a_token_is_kept_with_the_address_of_its_yard_before_it_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tokens/yard.token");
        let mac = Address::parse("https://mac.example.net").unwrap();
        let other = Address::parse("https://plantime.example.net").unwrap();

        // The place is there before the token is, and private; without a
        // token it leaves nothing, and the file that was there stays.
        let place = token_place(&file).unwrap();
        let beside = || fs::read_dir(dir.path().join("tokens")).unwrap().count();
        assert_eq!(beside(), 1);
        assert!(!file.exists());
        assert_eq!(fs::metadata(&place.new).unwrap().mode() & 0o777, 0o600);
        drop(place);
        assert_eq!(beside(), 0);

        token_place(&file).unwrap().keep(TOKEN, &mac).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("{TOKEN}\n{FOOTPLATE_TOKEN}\nhttps://mac.example.net\n")
        );
        assert_eq!(beside(), 1);
        assert!(is_managed_token_file(&file));
        // It was never opened, and is of its yard alone already.
        assert!(
            token_for(&file, &other)
                .unwrap_err()
                .contains("is not sent to")
        );
        assert_eq!(token_for(&file, &mac).unwrap(), TOKEN);

        // What cannot be a token's file is said before there is a token.
        assert!(token_place(dir.path()).is_err());
        assert!(token_place(&file.join("yard.token")).is_err());
    }

    #[test]
    fn a_token_path_of_another_kind_is_refused_without_reading_or_replacing_it() {
        use std::os::unix::fs::FileTypeExt;
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yard.token");
        let listener = UnixListener::bind(&socket).unwrap();
        drop(listener);

        let why =
            token_for(&socket, &Address::parse("https://mac.example.net").unwrap()).unwrap_err();
        assert!(why.contains("not a regular file"), "{why}");
        assert!(keep_token(&socket, TOKEN).is_err());
        assert!(
            fs::symlink_metadata(&socket)
                .unwrap()
                .file_type()
                .is_socket()
        );
    }

    #[test]
    fn reading_a_symlink_follows_it_but_pairing_replaces_only_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = file_of_mode(dir.path(), "external.token", 0o600, TOKEN);
        let link = dir.path().join("yard.token");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let address = Address::parse("https://mac.example.net").unwrap();

        assert_eq!(token_for(&link, &address).unwrap(), TOKEN);
        assert!(
            fs::read_to_string(&target)
                .unwrap()
                .contains("https://mac.example.net")
        );

        keep_token(&link, "yardr-dev-new").unwrap();
        assert!(
            !fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read_token(&link).unwrap(), "yardr-dev-new");
        assert!(
            fs::read_to_string(target)
                .unwrap()
                .contains("https://mac.example.net")
        );
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
