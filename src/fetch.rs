//! yardr is a dependency: footplate uses the one on this machine, found by
//! whole path ([`Machine::found`]), and where there is none it offers to run
//! yardr's own installer ([`Machine::install`]). It keeps no copy, and
//! neither downloads nor checks yardr itself: a yardr that is found is used
//! as it is. An older footplate did keep one, and that one is moved out of
//! footplate's directory the first time nothing else is found
//! ([`Machine::yardr`]).
//!
//! The installer comes from one host, [`SITE`], over TLS, and is not
//! followed elsewhere: an answer that is no 200, a redirect among them, is a
//! refusal. It carries no checksum, so that host is what is trusted, as it
//! is by anyone who pipes the same script to a shell. What it installs it
//! checks itself.
//!
//! What `yardr doctor --json` names for a program fetched on a Mac
//! (docs/doctor.md of the yardr repository) goes another way, by
//! [`program`]: its [`sha256`] is compared, the one [`member`] of a gzipped
//! tar taken, and the program [`keep`]-ed. It comes from its maker's address,
//! which may send on to another, and is trusted for its checksum alone,
//! which yardr carries.

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Cursor, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{env, fmt, thread};

use flate2::bufread::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::api::{self, Address};
use crate::log;
use crate::setup::{StatusError, Yardr};

/// The site yardr's installer is on: the one host it is ever fetched from.
pub const SITE: &str = "https://yardr.dev";

/// Where the site has the installer.
const INSTALLER_AT: &str = "/install.sh";

/// The most bytes that are read of the installer, a script of some 10 kB.
const INSTALLER_MOST: u64 = 1024 * 1024;

/// The program, as the machine names it, and the file an older footplate
/// kept beside its copy of it.
const PROGRAM: &str = "yardr";
const VERSION_FILE: &str = "yardr.version";

/// The most a program's archive unpacks to.
const UNPACKED_MOST: u64 = 256 * 1024 * 1024;

/// How long the login shell has to say where yardr is. A profile that takes
/// longer has not found it, and the person is offered the installer instead
/// of waiting.
#[cfg_attr(test, allow(dead_code))]
const SHELL_WITHIN: Duration = Duration::from_secs(5);

/// The one question the login shell is asked, as yardr's doctor asks it.
const SHELL_ASKS: &str = "command -v yardr";

/// What is said where no yardr is found. The window offers the installer
/// under exactly these words, and "try again" under any others.
pub const NOT_THERE: &str = "There is no yardr on this machine.";

/// Each writer gets its own temporary file before atomically replacing the
/// stored program. The pid also keeps names distinct across footplate runs.
static NEXT_PART: AtomicU64 = AtomicU64::new(0);

/// Why the installer was not fetched. Each is said in words for a person
/// (`Display`).
#[derive(Debug)]
pub enum Refusal {
    /// The person has no home for yardr to go under.
    NoHome,
    /// yardr has no installer for this system.
    NoSystem { system: String },
    /// The site did not answer.
    Down { site: String, err: io::Error },
    /// The site answered, and has no such file: any status but 200.
    Answered {
        site: String,
        path: String,
        status: u16,
    },
    /// What the site answered cannot be read as what it should be.
    Unreadable { what: String, why: String },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoHome => f.write_str("There is no home directory to install yardr under."),
            Refusal::NoSystem { system } => write!(
                f,
                "There is no yardr to install on this system ({system}): it runs on macOS and Linux."
            ),
            Refusal::Down { site, err } => write!(
                f,
                "{site} does not answer, so yardr's installer was not fetched ({err}). This needs the network once."
            ),
            Refusal::Answered { site, path, status } => write!(
                f,
                "{site} did not serve {path} directly (status {status}), so yardr was not installed."
            ),
            Refusal::Unreadable { what, why } => {
                write!(
                    f,
                    "footplate cannot read {what}: {why}. yardr was not installed."
                )
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// Where footplate keeps the programs it fetched: a `bin` beside its
/// settings. Nowhere for a person without a home.
#[cfg_attr(test, allow(dead_code))]
pub fn dir() -> Option<PathBuf> {
    directory_for(&crate::settings::file()?)
}

fn directory_for(settings: &Path) -> Option<PathBuf> {
    Some(settings.parent()?.join("bin"))
}

/// Whether yardr runs on the system Rust names `os`. Windows has no yard of
/// its own, so an old or misplaced file under its data directory must not be
/// started.
fn runs_on(os: &str) -> bool {
    matches!(os, "macos" | "linux")
}

/// What yardr is looked for by, on this machine or in a test's directory.
/// Nothing here is read before it is asked for. The default finds nothing.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// `<home>/binary` of the local yard: the file the yard records the
    /// yardr it runs in (docs/configuration.md of the yardr repository,
    /// "Which yardr a yard runs").
    pub binary: Option<PathBuf>,
    /// The person's home, which `.local/bin` is under.
    pub home: Option<PathBuf>,
    /// The PATH footplate was started with.
    pub path: OsString,
    /// The person's login shell, and how long it has to answer.
    pub shell: Option<PathBuf>,
    pub within: Duration,
    /// footplate's own directory for programs ([`dir`]).
    pub own: Option<PathBuf>,
    /// The yards of this machine, by their homes: each may have a login
    /// service that names the yardr an older footplate kept.
    pub yards: Vec<PathBuf>,
}

impl Machine {
    /// This machine: the local yard's by its socket, the rest from the
    /// environment and the settings.
    #[cfg_attr(test, allow(dead_code))]
    pub fn here() -> Machine {
        let socket = api::socket_path();
        let local = socket.parent().map(Path::to_path_buf);
        let listed = crate::settings::file()
            .map(|file| crate::settings::Settings::read(&file).yards)
            .unwrap_or_default();
        let mut yards: Vec<PathBuf> = local.iter().cloned().collect();
        for yard in listed {
            let home = Path::new(&yard.socket).parent().map(Path::to_path_buf);
            if let Some(home) = home.filter(|home| home.is_absolute() && !yards.contains(home)) {
                yards.push(home);
            }
        }
        Machine {
            binary: local.map(|home| home.join("binary")),
            home: env::home_dir(),
            path: env::var_os("PATH").unwrap_or_default(),
            shell: env::var_os("SHELL").map(PathBuf::from),
            within: SHELL_WITHIN,
            own: dir(),
            yards,
        }
    }

    /// The yardr of `~/.local/bin`, where the installer puts it.
    fn local(&self) -> Option<PathBuf> {
        Some(self.home.as_ref()?.join(".local/bin").join(PROGRAM))
    }

    /// The yardr an older footplate kept in its own directory, a regular
    /// file its user can run.
    fn kept(&self) -> Option<PathBuf> {
        let path = self.own.as_ref()?.join(PROGRAM);
        let metadata = fs::symlink_metadata(&path).ok()?;
        (metadata.file_type().is_file() && runnable(&metadata)).then_some(path)
    }

    /// The yardr on this machine, in this order: the one the local yard
    /// records, `~/.local/bin`'s, the PATH's, and the login shell's. It
    /// reads, and writes and fetches nothing.
    ///
    /// The kept yardr is none of them, wherever it is named: a yard it
    /// started records it in `<home>/binary`, and taking it for found would
    /// leave it where it is for good ([`Machine::yardr`] moves it).
    pub fn found(&self) -> Option<PathBuf> {
        let kept = self.kept();
        let usable = |path: &PathBuf| {
            runs(path)
                && !kept
                    .as_ref()
                    .is_some_and(|kept| crate::yard::same_file(path, kept))
        };
        recorded(self.binary.as_deref())
            .filter(usable)
            .or_else(|| self.local().filter(usable))
            .or_else(|| {
                env::split_paths(&self.path)
                    .map(|dir| dir.join(PROGRAM))
                    .find(usable)
            })
            .or_else(|| {
                let shell = self.shell.as_deref()?;
                login_shell(shell, self.within).filter(usable)
            })
    }

    /// The yardr to run. Where nothing but the kept one is there, the kept
    /// one is moved first, and if that fails it is still the one to run,
    /// with why beside it. With none at all, [`NOT_THERE`]; the installer is
    /// the person's to ask for.
    pub fn yardr(&self) -> Result<(PathBuf, Option<String>), String> {
        if let Some(found) = self.found() {
            return Ok((found, None));
        }
        let Some(kept) = self.kept() else {
            return Err(NOT_THERE.to_string());
        };
        match self.moved(&kept) {
            Ok(place) => Ok((place, None)),
            Err(why) => {
                let note = format!(
                    "The yardr footplate kept is still used, from {}: {why}",
                    kept.display()
                );
                say(&note);
                Ok((kept, Some(note)))
            }
        }
    }

    /// Moves the kept yardr to `~/.local/bin`, for the machine to have it
    /// where the installer would have put it. The old one goes last: until
    /// the login services name the new place, the old one is what they
    /// start.
    fn moved(&self, kept: &Path) -> Result<PathBuf, String> {
        let place = self.local().ok_or(Refusal::NoHome.to_string())?;
        let dir = place.parent().ok_or(Refusal::NoHome.to_string())?;
        let cannot =
            |err: io::Error| format!("it could not be copied to {} ({err}).", dir.display());
        let bytes = fs::read(kept).map_err(cannot)?;
        keep(dir, PROGRAM, &bytes).map_err(cannot)?;
        let reinstalled = match self.reinstalled(&place) {
            Ok(reinstalled) => reinstalled,
            // No service names the copy yet, so it can go, and the move is
            // tried again. Left, it would be found, and the service would go
            // on naming the old one with nothing left to move it.
            Err((0, why)) => {
                let _ = fs::remove_file(&place);
                return Err(why);
            }
            // A service names the copy, so it stays, and so does the old
            // one, which the next is still started from.
            Err((_, why)) => return Err(why),
        };
        for file in [kept.with_file_name(VERSION_FILE), kept.to_path_buf()] {
            match fs::remove_file(&file) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => {
                    // With no service switched, the old copy still serves
                    // every start. Remove the new one too so the move can
                    // be tried again; after any service switched, both are
                    // needed until that service can be changed.
                    if reinstalled == 0 {
                        let _ = fs::remove_file(&place);
                    }
                    return Err(format!("{} could not be removed ({err}).", file.display()));
                }
                _ => {}
            }
        }
        Ok(place)
    }

    /// Runs each login service's install from `program` on each yard
    /// with one: the daemon's, the web view's, and the BDP server's,
    /// whose units name the yardr that installed them. Doctor widens the
    /// PATH first: a unit keeps the PATH it was installed with. Where one
    /// fails, how many were done before it, and why.
    fn reinstalled(&self, program: &Path) -> Result<usize, (usize, String)> {
        let mut done = 0;
        for home in &self.yards {
            let mut yardr = Yardr {
                program: program.to_path_buf(),
                home: Some(home.clone()),
                path: self.path.clone(),
                actor: None,
            };
            let _ = yardr.needs();
            match yardr.status() {
                Ok(status) => {
                    if status.daemon_service().installed {
                        yardr.daemon_install().map_err(|why| (done, why))?;
                        done += 1;
                    }
                    if status.web_service().installed {
                        yardr.web_install().map_err(|why| (done, why))?;
                        done += 1;
                    }
                    // `status` says nothing of the BDP server's service,
                    // so it is asked itself.
                    let bdp = yardr.bdp_status().map_err(|why| (done, why))?;
                    if bdp.service.installed {
                        yardr.bdp_install().map_err(|why| (done, why))?;
                        done += 1;
                    }
                }
                Err(StatusError::NoYard) => {}
                Err(StatusError::Failed(why)) => return Err((done, why)),
            }
        }
        Ok(done)
    }

    /// Fetches yardr's installer from [`SITE`], runs it, and gives the
    /// yardr it left. A failure says what the installer said.
    pub fn install(
        &self,
        progress: Option<async_channel::Sender<String>>,
    ) -> Result<PathBuf, String> {
        let site = Address::parse(SITE).map_err(|why| unreadable(SITE, why).to_string())?;
        self.install_from(&site, env::consts::OS, progress)
    }

    fn install_from(
        &self,
        site: &Address,
        os: &str,
        progress: Option<async_channel::Sender<String>>,
    ) -> Result<PathBuf, String> {
        if !runs_on(os) {
            let system = os.to_string();
            return Err(Refusal::NoSystem { system }.to_string());
        }
        let (Some(place), Some(own)) = (self.local(), self.own.as_ref()) else {
            return Err(Refusal::NoHome.to_string());
        };
        let script = get(site, INSTALLER_AT, INSTALLER_MOST).map_err(|why| why.to_string())?;
        // A site may answer 200 with a page for an address it does not have.
        if !script.starts_with(b"#!") {
            return Err(unreadable(INSTALLER_AT, "it is no script").to_string());
        }
        let said = run_aside(own, PROGRAM, &script, &[], &self.path, progress)?;
        match runs(&place) {
            true => Ok(place),
            false => Err(format!(
                "The installer of yardr ended, and {} is not there. It said: {said}",
                place.display()
            )),
        }
    }

    /// What the yardr that is used says of itself, for the About card. The
    /// kept one counts, as it is what runs until it is moved. Nothing is
    /// moved or fetched for a card.
    pub fn seen(&self) -> Seen {
        seen_on(env::consts::OS, self)
    }
}

/// What the yardr of `<home>/binary` is: its one line where that is a whole
/// path, and none for anything else, a missing file among them.
fn recorded(file: Option<&Path>) -> Option<PathBuf> {
    let text = fs::read_to_string(file?).ok()?;
    let line = text.strip_suffix('\n').unwrap_or(&text);
    let path = PathBuf::from(line);
    (!line.is_empty() && !line.contains('\n') && path.is_absolute()).then_some(path)
}

/// Whether `path` is a whole path to a program its user can run. A link
/// counts by what it leads to: Homebrew's is one.
fn runs(path: &Path) -> bool {
    path.is_absolute()
        && fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && runnable(&metadata))
}

/// Where the login shell finds yardr, asked once. The shell has its own
/// process group, so that a profile that hangs is stopped with what it
/// started. Its output is read without waiting for the pipe to close: a
/// profile may leave something running after the shell has ended. Anything
/// but a whole path on the last line is no answer: a shell that fails or
/// takes too long, or an alias.
#[cfg(unix)]
fn login_shell(shell: &Path, within: Duration) -> Option<PathBuf> {
    let mut child = Command::new(shell)
        .args(["-l", "-c", SHELL_ASKS])
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(mut out) = child.stdout.take() else {
        stop_group(&mut child);
        return None;
    };
    // SAFETY: out owns a live descriptor until this function returns.
    let flags = unsafe { libc::fcntl(out.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        stop_group(&mut child);
        return None;
    }
    // SAFETY: the same owned descriptor remains open while its flags change.
    if unsafe { libc::fcntl(out.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        stop_group(&mut child);
        return None;
    }
    let asked = Instant::now();
    let mut said = Vec::new();
    let status = loop {
        let mut chunk = [0; 1024];
        match out.read(&mut chunk) {
            Ok(n) => {
                let keep = (VERSION_MOST as usize).saturating_sub(said.len()).min(n);
                said.extend_from_slice(&chunk[..keep]);
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => {
                stop_group(&mut child);
                return None;
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if asked.elapsed() > within => {
                stop_group(&mut child);
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                stop_group(&mut child);
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    // Get bytes written just before the shell ended. A background process
    // may still hold the pipe, so a non-blocking read must stop at WouldBlock.
    loop {
        let mut chunk = [0; 1024];
        match out.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let keep = (VERSION_MOST as usize).saturating_sub(said.len()).min(n);
                said.extend_from_slice(&chunk[..keep]);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let said = String::from_utf8_lossy(&said);
    let line = said.lines().rev().find(|line| !line.trim().is_empty())?;
    Some(PathBuf::from(line.trim())).filter(|path| path.is_absolute())
}

/// Stops the shell and the processes it started in its own group.
#[cfg(unix)]
fn stop_group(child: &mut Child) {
    // SAFETY: the negative pid addresses only the process group made for this child.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
fn login_shell(_: &Path, _: Duration) -> Option<PathBuf> {
    None
}

/// What the yardr that is used says of itself, for the About card.
#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    /// There is none on this machine.
    Nothing,
    /// Its own answer to `--version`, the first line of it: the release and
    /// the commit it was built from, as yardr words them.
    Says { line: String, path: PathBuf },
    /// One is there and would not say which it is: why, in a few words.
    Silent { why: String, path: PathBuf },
}

/// How long a yardr has to answer `--version`, which takes it a few
/// milliseconds: one that has not by then is stopped.
const VERSION_WITHIN: Duration = Duration::from_secs(5);

/// The most that is read of the answer, and the most of its line that is
/// kept. Only the first line is wanted.
const VERSION_MOST: u64 = 4 * 1024;
const VERSION_LINE: usize = 80;

/// [`Machine::seen`], on the system Rust names `os`.
fn seen_on(os: &str, machine: &Machine) -> Seen {
    if !runs_on(os) {
        return Seen::Nothing;
    }
    let Some(path) = machine.found().or_else(|| machine.kept()) else {
        return Seen::Nothing;
    };
    match version_of(&path, VERSION_WITHIN) {
        Ok(line) => Seen::Says { line, path },
        Err(why) => Seen::Silent { why, path },
    }
}

/// What the program at `path` answers to `--version`: the first line it
/// prints, with anything that is not text taken out, or why there is none.
/// It is started with no shell and nothing on its input, and one that has
/// not ended in `within` is stopped.
fn version_of(path: &Path, within: Duration) -> Result<String, String> {
    let mut child = started(
        Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|err| err.to_string())?;
    let asked = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if asked.elapsed() > within => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("no answer in time".to_string());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(err) => return Err(err.to_string()),
        }
    };
    if !status.success() {
        return Err(match status.code() {
            Some(code) => format!("ended with status {code}"),
            None => "ended by a signal".to_string(),
        });
    }
    // Read once it has ended: an answer longer than a pipe holds never
    // ends, and is stopped above.
    let mut said = Vec::new();
    if let Some(out) = child.stdout.take() {
        let _ = out.take(VERSION_MOST).read_to_end(&mut said);
    }
    String::from_utf8_lossy(&said)
        .lines()
        .map(|line| {
            let text: String = line
                .chars()
                .filter(|c| !c.is_control())
                .take(VERSION_LINE)
                .collect();
            text.trim().to_string()
        })
        .find(|line| !line.is_empty())
        .ok_or_else(|| "printed nothing".to_string())
}

/// Whether the file is one its user can run: a mode says so on a unix
/// system. Windows runs a file by its name, and fetches no yardr at all
/// ([`target_of`]), so what is kept there is taken as runnable.
#[cfg(unix)]
fn runnable(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn runnable(_: &fs::Metadata) -> bool {
    true
}

/// Says `line` on stderr, led by the program's name as its other lines are,
/// and keeps it in the log with the time ([`log`]): an app started by the
/// Finder or by launchd has its stderr on /dev/null, and would lose it. A
/// test keeps in the file its thread was given, where it was given one, and
/// never in the person's own log.
fn say(line: &str) {
    #[cfg(test)]
    let file = test_log_file();
    #[cfg(not(test))]
    let file = log::file();
    log::say(|text| eprintln!("{text}"), file.as_deref(), line);
}

#[cfg(test)]
thread_local! {
    /// The file this test keeps its lines in, where it set one.
    static LOG: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The file [`say`] keeps its line in during a test: the one this test
/// set for its thread, else none, so that no test writes the person's
/// own log.
#[cfg(test)]
fn test_log_file() -> Option<PathBuf> {
    LOG.with(|file| file.borrow().clone())
}

fn unreadable(what: &str, why: impl ToString) -> Refusal {
    Refusal::Unreadable {
        what: what.to_string(),
        why: why.to_string(),
    }
}

/// Reads the file the site has at `path`, of `most` bytes at the most.
pub fn get(site: &Address, path: &str, most: u64) -> Result<Vec<u8>, Refusal> {
    match api::read(site, path, most) {
        Ok((200, bytes)) => Ok(bytes),
        Ok((status, _)) => Err(Refusal::Answered {
            site: site.to_string(),
            path: path.to_string(),
            status,
        }),
        Err(api::Error::Down(err)) => Err(Refusal::Down {
            site: site.to_string(),
            err,
        }),
        Err(api::Error::Protocol(why)) => Err(unreadable(&format!("{site}{path}"), why)),
        Err(err) => Err(unreadable(&format!("{site}{path}"), err)),
    }
}

/// The SHA-256 of `bytes`, in small hex.
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Takes the member `name` out of a gzipped tar. The archive may hold
/// nothing else: no other name, no directory, no link, no name twice,
/// nothing after its end. It is read in
/// memory and nothing of it is written anywhere; what it unpacks to is
/// read to `most` bytes. A refusal says why, to follow the archive's name
/// in a sentence.
pub fn member(archive: &[u8], name: &str, most: u64) -> Result<Vec<u8>, String> {
    const BLOCK: usize = 512;
    const CUT: &str = "it ends in the middle of a file";
    let mut tar = Vec::new();
    let mut compressed = Cursor::new(archive);
    {
        GzDecoder::new(&mut compressed)
            .take(most.saturating_add(1))
            .read_to_end(&mut tar)
            .map_err(|err| format!("it is no gzipped archive ({err})"))?;
    }
    if tar.len() as u64 > most {
        return Err(format!(
            "it unpacks to more than {} MB",
            most / (1024 * 1024)
        ));
    }
    if compressed.position() != archive.len() as u64 {
        return Err("something follows its gzipped archive".into());
    }
    // A number of a tar's header: octal digits, ended by a space or a NUL.
    let number = |field: &[u8]| {
        let digits = std::str::from_utf8(field)
            .ok()?
            .trim_matches(|c| c == ' ' || c == '\0');
        if digits.is_empty() || !digits.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return None;
        }
        usize::from_str_radix(digits, 8).ok()
    };
    let text = |field: &[u8]| {
        let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
        String::from_utf8(field[..end].to_vec()).ok()
    };
    let mut seen: Vec<String> = Vec::new();
    let mut found = None;
    let mut at = 0;
    loop {
        let Some(head) = tar.get(at..at + BLOCK) else {
            return Err("it has no end".into());
        };
        // The end is blocks of nothing, and nothing but them follows.
        if head.iter().all(|b| *b == 0) {
            if tar[at..].iter().any(|b| *b != 0) {
                return Err("something follows its end".into());
            }
            break;
        }
        // A header carries the sum of its own bytes, its sum's field
        // counted as spaces.
        let sum: usize = head
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    usize::from(*b)
                }
            })
            .sum();
        if number(&head[148..156]) != Some(sum) {
            return Err("it is no tar archive".into());
        }
        let (Some(file), Some(prefix)) = (text(&head[..100]), text(&head[345..500])) else {
            return Err("it holds a name that is no text".into());
        };
        let file = if prefix.is_empty() {
            file
        } else {
            format!("{prefix}/{file}")
        };
        if file != name {
            return Err(format!("it holds {file:?}, which is not expected in it"));
        }
        // '0', or a NUL in an old tar, is a regular file. Everything else
        // is a link, a directory, a device or a header of an extension.
        if !matches!(head[156], b'0' | 0) {
            return Err(format!("its {file} is no regular file"));
        }
        if seen.contains(&file) {
            return Err(format!("it holds {file} twice"));
        }
        let size = number(&head[124..136]).ok_or_else(|| format!("its {file} has no size"))?;
        let start = at + BLOCK;
        let end = start.checked_add(size).ok_or(CUT)?;
        let data = tar.get(start..end).ok_or(CUT)?;
        if file == name {
            found = Some(data.to_vec());
        }
        seen.push(file);
        at = start
            .checked_add(size.div_ceil(BLOCK).checked_mul(BLOCK).ok_or(CUT)?)
            .ok_or(CUT)?;
    }
    match found {
        Some(bytes) if !bytes.is_empty() => Ok(bytes),
        Some(_) => Err(format!("its {name} is empty")),
        None => Err(format!("it holds no {name}")),
    }
}

/// Writes a program as `dir/name`, to be run by its user, and gives its
/// path. The directory is made when it is not there. The file is written
/// whole beside its place and then moved there, so that what is at the
/// path is always a whole program: the one that was there, or this one.
pub fn keep(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    keep_as(dir, name, bytes, 0o755)
}

/// As [`keep`], for a file of the mode `mode`. A system without modes
/// (Windows) sets none: what is kept lies in the person's own profile.
#[cfg_attr(not(unix), allow(unused_variables))]
fn keep_as(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> io::Result<PathBuf> {
    // `name` comes from the program's data for the shared fetch mechanism.
    // Keep it one filename so a caller cannot make this helper write beyond
    // the directory it was given.
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the program name must be one file name",
        ));
    }
    fs::create_dir_all(dir)?;
    let path = dir.join(name);
    // Do not share a fixed `.part` name: two first-run windows can fetch at
    // once. `create_new` also refuses a link or a stale name without
    // following or removing it.
    let (part, mut file) = loop {
        let serial = NEXT_PART.fetch_add(1, Ordering::Relaxed);
        let part = dir.join(format!(".{name}.part.{}.{serial}", std::process::id()));
        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        open.mode(0o600);
        match open.open(&part) {
            Ok(file) => break (part, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };
    let written = (|| {
        file.write_all(bytes)?;
        // Keep the file private until it is whole, then make it runnable;
        // the mode is set again because umask may have cut it.
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        fs::rename(&part, &path)
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&part);
        return Err(err);
    }
    Ok(path)
}

/// A program `yardr doctor --json` names for a Mac where it is missing
/// (its `fetch`; docs/doctor.md of the yardr repository, "A program
/// fetched on a Mac"): one file at its maker's address, of the size and
/// the SHA-256 yardr wrote down when the way was verified.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Wanted {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// The file is a gzipped tar, and this its one member that is the
    /// program.
    #[serde(default)]
    pub member: Option<String>,
    /// The file is the maker's installer: what it is run with.
    #[serde(default)]
    pub run: Vec<String>,
    /// Where the program is afterwards.
    pub path: PathBuf,
}

/// How long a maker's installer may take: it downloads the program again
/// itself.
const INSTALLER: Duration = Duration::from_secs(15 * 60);

/// Puts the program `wanted` names in its place, by the six steps yardr's
/// documentation gives a program that fetches, and says in a sentence why
/// where it could not. Nothing is kept or run that is not the file yardr
/// named: the address is the maker's, but what is trusted is the size and
/// the checksum, which came with yardr. `home` is
/// whose programs these are: the place must be under it. An installer is
/// kept for its run in `own`, a directory of footplate's ([`dir`]), and
/// run with `path` for its PATH, with no shell and nothing to read.
#[cfg(test)]
pub fn program(wanted: &Wanted, home: &Path, own: &Path, path: &OsStr) -> Result<(), String> {
    program_with_progress(wanted, home, own, path, None)
}

/// As [`program`], sending the maker installer's output as it runs when
/// `progress` is there. The first run shows these lines in its busy state.
pub fn program_with_progress(
    wanted: &Wanted,
    home: &Path,
    own: &Path,
    path: &OsStr,
    progress: Option<async_channel::Sender<String>>,
) -> Result<(), String> {
    let place = &wanted.path;
    let name = place
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} is no place for a program.", place.display()))?;
    let dir = place
        .parent()
        .filter(|_| place.is_absolute())
        .ok_or_else(|| format!("{} is no place for a program.", place.display()))?;
    // The place is the person's own: under their home as it is written,
    // and still there once every link on the way is followed.
    let outside = || {
        format!(
            "{} is not under your home ({}), so nothing was put there.",
            place.display(),
            home.display()
        )
    };
    if !dir.starts_with(home) || dir.components().any(|part| part == Component::ParentDir) {
        return Err(outside());
    }
    let cannot =
        |what: &Path, err: io::Error| format!("{} cannot be written ({err}).", what.display());
    if !dir.starts_with(home) || dir.components().any(|part| part == Component::ParentDir) {
        return Err(outside());
    }
    let real_home = fs::canonicalize(home).map_err(|err| cannot(home, err))?;
    // Check the nearest existing ancestor before making any directories.
    // In particular, `~/.local` can be a symlink elsewhere; creating
    // `~/.local/bin` first would write outside the home before noticing.
    let mut existing = dir;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| cannot(existing, err))?;
            }
            Err(err) => return Err(cannot(existing, err)),
        }
    }
    let real_existing = fs::canonicalize(existing).map_err(|err| cannot(existing, err))?;
    if !real_existing.starts_with(&real_home) {
        return Err(outside());
    }
    fs::create_dir_all(dir).map_err(|err| cannot(dir, err))?;
    let real_dir = fs::canonicalize(dir).map_err(|err| cannot(dir, err))?;
    if !real_dir.starts_with(&real_home) {
        return Err(outside());
    }
    // Doctor's PATH check should already have noticed a runnable file at
    // this place. Preserve the useful no-fetch behavior if one is there,
    // but do not let a stale link, directory or non-runnable file make the
    // person retry an install that can never succeed.
    match fs::symlink_metadata(place) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            return Err(format!(
                "The place for {name}, {}, is a folder. Move it out of the way before installing {name}.",
                place.display()
            ));
        }
        Ok(metadata) if metadata.file_type().is_file() && runnable(&metadata) => {
            return Ok(());
        }
        Ok(_) => {} // A verified install below atomically replaces it.
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(cannot(place, err)),
    }
    if !wanted.url.starts_with("https://") && !cfg!(test) {
        return Err(format!(
            "{name} is not fetched: {:?} is no https address.",
            wanted.url
        ));
    }

    let bytes = api::download(&wanted.url, wanted.size).map_err(|err| {
        let why = match err {
            api::Error::Protocol(why) => why,
            api::Error::Down(err) => format!("it does not answer ({err})"),
            err => err.to_string(),
        };
        format!(
            "{name} was not fetched from {}: {why}. This needs the network.",
            host_of(&wanted.url)
        )
    })?;
    if bytes.len() as u64 != wanted.size || !sha256(&bytes).eq_ignore_ascii_case(&wanted.sha256) {
        return Err(format!(
            "What {} gave for {name} is not the file yardr's makers verified: its size or checksum differs. Nothing of it is kept or run.",
            host_of(&wanted.url)
        ));
    }
    let program = match &wanted.member {
        Some(member_name) => member(&bytes, member_name, UNPACKED_MOST).map_err(|why| {
            format!("The archive of {name} is not as expected: {why}. Nothing of it is kept.")
        })?,
        None => bytes,
    };
    if wanted.run.is_empty() {
        keep(&real_dir, name, &program).map_err(|err| cannot(place, err))?;
        return Ok(());
    }

    let said = run_aside(own, name, &program, &wanted.run, path, progress)?;
    match fs::symlink_metadata(place).is_ok() {
        true => Ok(()),
        false => Err(format!(
            "The installer of {name} ended, and {} is not there. It said: {said}",
            place.display()
        )),
    }
}

/// Runs a maker's installer, kept for its run in `own`, a directory of
/// footplate's that nobody else can write to, so that the file that was
/// checked is the file that is run, and gone afterwards.
fn run_aside(
    own: &Path,
    name: &str,
    installer: &[u8],
    args: &[String],
    path: &OsStr,
    progress: Option<async_channel::Sender<String>>,
) -> Result<String, String> {
    let cannot =
        |what: &Path, err: io::Error| format!("{} cannot be written ({err}).", what.display());
    let serial = NEXT_PART.fetch_add(1, Ordering::Relaxed);
    let aside = own.join(format!(".install.{}.{serial}", std::process::id()));
    fs::create_dir_all(own).map_err(|err| cannot(own, err))?;
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(&aside).map_err(|err| cannot(&aside, err))?;
    let ran = keep(&aside, name, installer)
        .map_err(|err| cannot(&aside, err))
        .and_then(|installer| run_installer(&installer, args, path, progress));
    let _ = fs::remove_dir_all(&aside);
    ran
}

/// The host of an address, to name in a sentence.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Starts `command`, a program that was written a moment ago. Linux
/// refuses to start a file that anything still has open for writing, and
/// a process started elsewhere at that moment holds every open file of
/// this one until it has become its own program: the refusal passes by
/// itself, so it is waited out, for a second at the most.
pub fn started(command: &mut Command) -> io::Result<Child> {
    let mut tries = 0;
    loop {
        match command.spawn() {
            Err(err) if err.kind() == io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                thread::sleep(Duration::from_millis(50));
            }
            came => return came,
        }
    }
}

/// Runs a maker's installer with its arguments and gives the last of what
/// it printed. It gets no shell and nothing on its input; one that does not
/// end is stopped.
fn run_installer(
    installer: &Path,
    args: &[String],
    path: &OsStr,
    progress: Option<async_channel::Sender<String>>,
) -> Result<String, String> {
    let name = installer
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut child = started(
        Command::new(installer)
            .args(args)
            .env("PATH", path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .map_err(|err| format!("The installer of {name} could not be started ({err})."))?;
    let tail = |pipe: Option<Box<dyn Read + Send>>,
                progress: Option<async_channel::Sender<String>>| {
        thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let mut chunk = [0; 4096];
                let mut line = Vec::new();
                loop {
                    let n = match pipe.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let keep = (1024 * 1024usize).saturating_sub(text.len()).min(n);
                    text.extend_from_slice(&chunk[..keep]);
                    if let Some(progress) = &progress {
                        for byte in &chunk[..n] {
                            if matches!(*byte, b'\n' | b'\r') {
                                send_progress(progress, &line);
                                line.clear();
                            } else if line.len() < 4096 {
                                line.push(*byte);
                            }
                        }
                    }
                }
                if let Some(progress) = &progress {
                    send_progress(progress, &line);
                }
            }
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let out = tail(
        child.stdout.take().map(|pipe| Box::new(pipe) as _),
        progress.clone(),
    );
    let err = tail(
        child.stderr.take().map(|pipe| Box::new(pipe) as _),
        progress,
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > INSTALLER => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "The installer of {name} did not end in {} minutes, and was stopped.",
                    INSTALLER.as_secs() / 60
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(err) => return Err(format!("The installer of {name} was lost ({err}).")),
        }
    };
    let said = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    let lines: Vec<&str> = said
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let last = lines[lines.len().saturating_sub(4)..].join(" / ");
    match status.success() {
        true => Ok(last),
        false => Err(format!(
            "The installer of {name} failed ({status}). It said: {last}"
        )),
    }
}

/// Sends one line of vendor output without ANSI escapes or other controls,
/// and with a short cap for the setup screen.
fn send_progress(progress: &async_channel::Sender<String>, line: &[u8]) {
    let line: String = String::from_utf8_lossy(line)
        .chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect::<String>()
        .trim()
        .to_string();
    if !line.is_empty() {
        let _ = progress.send_blocking(line);
    }
}

// The lookup and the fetch are unix ones (Windows finds and installs no yardr),
// and so are their tests: they read file modes and make symlinks.
#[cfg(all(test, unix))]
mod tests {
    use std::collections::HashMap;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;
    use crate::setup::standin;

    /// Has this test keep its lines in a log under `dir`, which is not
    /// there yet, and gives its file: a fetch is made on the thread that
    /// asks it.
    fn keep_in(dir: &std::path::Path) -> PathBuf {
        let file = dir.join("footplate/footplate.log");
        super::LOG.with(|kept| *kept.borrow_mut() = Some(file.clone()));
        file
    }

    #[test]
    fn programs_are_kept_beside_settings_under_an_isolated_config_home() {
        let settings = crate::settings::file_under(
            true,
            Some("/tmp/footplate-test/config".into()),
            Some("/Users/jo".into()),
        )
        .unwrap();
        assert_eq!(
            directory_for(&settings),
            Some(PathBuf::from("/tmp/footplate-test/config/footplate/bin"))
        );
    }

    /// A stand-in for the site: a port of this machine that answers each
    /// path with the file it was given, and 404 for every other.
    struct Site {
        address: Address,
        port: u16,
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        statuses: Arc<Mutex<HashMap<String, u16>>>,
        asked: Arc<Mutex<Vec<String>>>,
        stopped: Arc<AtomicBool>,
    }

    impl Site {
        fn start() -> Site {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
            let statuses: Arc<Mutex<HashMap<String, u16>>> = Arc::default();
            let asked: Arc<Mutex<Vec<String>>> = Arc::default();
            let stopped = Arc::new(AtomicBool::new(false));
            let (its_files, its_statuses, its_asked, its_stopped) = (
                files.clone(),
                statuses.clone(),
                asked.clone(),
                stopped.clone(),
            );
            thread::spawn(move || {
                for conn in listener.incoming() {
                    if its_stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(conn) = conn else { continue };
                    let _ = Site::answer(conn, &its_files, &its_statuses, &its_asked);
                }
            });
            Site {
                address: Address::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
                port,
                files,
                statuses,
                asked,
                stopped,
            }
        }

        fn answer(
            mut conn: TcpStream,
            files: &Mutex<HashMap<String, Vec<u8>>>,
            statuses: &Mutex<HashMap<String, u16>>,
            asked: &Mutex<Vec<String>>,
        ) -> io::Result<()> {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if conn.read(&mut byte)? == 0 {
                    return Ok(());
                }
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            // "GET /install.sh HTTP/1.1"
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            asked.lock().unwrap().push(path.clone());
            let file = files.lock().unwrap().get(&path).cloned();
            let status = statuses
                .lock()
                .unwrap()
                .get(&path)
                .copied()
                .unwrap_or(if file.is_some() { 200 } else { 404 });
            match (status, file) {
                (200, Some(bytes)) => {
                    conn.write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .as_bytes(),
                    )?;
                    conn.write_all(&bytes)
                }
                // A status that sends on does so to what its file says.
                (status @ 301..=308, Some(to)) => conn.write_all(
                    format!(
                        "HTTP/1.1 {status} Reply\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        String::from_utf8_lossy(&to)
                    )
                    .as_bytes(),
                ),
                (status, _) => conn.write_all(
                    format!(
                        "HTTP/1.1 {status} Reply\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                ),
            }
        }

        fn put(&self, path: &str, bytes: impl AsRef<[u8]>) {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), bytes.as_ref().to_vec());
        }

        fn status(&self, path: &str, status: u16) {
            self.statuses
                .lock()
                .unwrap()
                .insert(path.to_string(), status);
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Drop for Site {
        fn drop(&mut self) {
            // The thread waits in accept: one connection wakes it to see
            // that it is stopped.
            self.stopped.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    const BINARY: &[u8] = b"#!/bin/sh\necho this is no program, and nothing runs it\n";

    /// One member of a tar: its header, its bytes, and the block filled.
    fn tar_member(name: &str, kind: u8, bytes: &[u8]) -> Vec<u8> {
        let mut head = [0u8; 512];
        head[..name.len()].copy_from_slice(name.as_bytes());
        head[100..108].copy_from_slice(b"0000755\0");
        head[108..116].copy_from_slice(b"0000000\0");
        head[116..124].copy_from_slice(b"0000000\0");
        head[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        head[136..148].copy_from_slice(b"00000000000\0");
        head[148..156].copy_from_slice(b"        ");
        head[156] = kind;
        head[257..265].copy_from_slice(b"ustar\x0000");
        let sum: u32 = head.iter().map(|b| u32::from(*b)).sum();
        head[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut member = head.to_vec();
        member.extend_from_slice(bytes);
        member.resize(512 + bytes.len().div_ceil(512) * 512, 0);
        member
    }

    /// A gzipped tar of `members`: a name, its kind ('0' a regular file)
    /// and its bytes.
    fn archive_of(members: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut tar = Vec::new();
        for (name, kind, bytes) in members {
            tar.extend(tar_member(name, *kind, bytes));
        }
        tar.extend([0u8; 1024]);
        gzip(&tar)
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    }

    /// A home, a yard and footplate's own directory under one temporary
    /// directory, none of them holding anything yet, and a PATH with no
    /// yardr on it.
    struct Stage {
        root: tempfile::TempDir,
    }

    impl Stage {
        fn new() -> Stage {
            let stage = Stage {
                root: tempfile::tempdir().unwrap(),
            };
            for dir in ["home", "yard", "own"] {
                fs::create_dir_all(stage.at(dir)).unwrap();
            }
            stage
        }

        fn at(&self, path: &str) -> PathBuf {
            self.root.path().join(path)
        }

        fn machine(&self) -> Machine {
            Machine {
                binary: Some(self.at("yard/binary")),
                home: Some(self.at("home")),
                path: "/usr/bin:/bin".into(),
                shell: None,
                within: Duration::from_secs(60),
                own: Some(self.at("own")),
                yards: vec![self.at("yard")],
            }
        }

        fn local(&self) -> PathBuf {
            self.at("home/.local/bin/yardr")
        }

        /// A program at `path` under the stage.
        fn program(&self, path: &str) -> PathBuf {
            let path = self.at(path);
            let (dir, name) = (path.parent().unwrap(), path.file_name().unwrap());
            keep(dir, name.to_str().unwrap(), b"#!/bin/sh\necho v0.1.0\n").unwrap()
        }

        /// A login shell that prints `line`, whatever it is asked, and
        /// keeps what it was asked beside it.
        fn shell(&self, line: &str) -> PathBuf {
            let script = format!("#!/bin/sh\nprintf '%s|' \"$@\" >> \"$0.asked\"\n{line}\n");
            keep(&self.at("shell"), "zsh", script.as_bytes()).unwrap()
        }

        /// The yardr an older footplate kept: a stand-in for it, with what
        /// a copy of it at `~/.local/bin` will answer, there being its own
        /// answers beside each program.
        fn kept(&self, status: i32, answer: &serde_json::Value) -> PathBuf {
            let kept = standin::make(&self.at("own"));
            fs::write(self.at("own").join(VERSION_FILE), "v0.14.0\n").unwrap();
            fs::create_dir_all(self.at("home/.local/bin/answers")).unwrap();
            standin::answers(&self.local(), "status", status, answer);
            standin::answers(
                &self.local(),
                "doctor",
                0,
                &serde_json::json!({"machine": [
                    {"program": "herdr", "login_shell_path": "/opt/herdr/bin/herdr"}
                ]}),
            );
            kept
        }

        /// Every file under the stage's three directories: what it is
        /// called, how long it is and when it was written.
        fn tree(&self) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
            let mut files = Vec::new();
            let mut dirs = vec![self.at("home"), self.at("yard"), self.at("own")];
            while let Some(dir) = dirs.pop() {
                for entry in fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    let metadata = fs::symlink_metadata(&path).unwrap();
                    if metadata.is_dir() {
                        dirs.push(path);
                    } else {
                        files.push((path, metadata.len(), metadata.modified().unwrap()));
                    }
                }
            }
            files.sort();
            files
        }
    }

    fn with_services(daemon: bool, web: bool) -> serde_json::Value {
        let service =
            |installed: bool| serde_json::json!({"supported": true, "installed": installed});
        serde_json::json!({
            "daemon": {"service": service(daemon)},
            "web": {"views": [], "service": service(web)},
        })
    }

    /// What `bdp status` answers: whether the BDP server's login service
    /// is installed.
    fn bdp_answer(installed: bool) -> serde_json::Value {
        serde_json::json!({
            "running": false,
            "service": {"supported": true, "installed": installed},
        })
    }

    /// The whole order: each step is found while the ones before it are
    /// there no longer, the last of them is the installer, and nothing up
    /// to it fetches or writes anything.
    #[test]
    fn yardr_is_looked_for_in_order_and_only_the_last_step_reaches_out() {
        let stage = Stage::new();
        let site = Site::start();
        let installer = format!(
            "#!/bin/sh\nmkdir -p {dir}\nprintf '#!/bin/sh\\n' > {place}\nchmod +x {place}\n",
            dir = stage.local().parent().unwrap().display(),
            place = stage.local().display(),
        );
        site.put(INSTALLER_AT, &installer);

        let recorded = stage.program("recorded/yardr");
        let local = stage.program("home/.local/bin/yardr");
        let on_path = stage.program("path/yardr");
        let from_shell = stage.program("profile/yardr");
        fs::write(stage.at("yard/binary"), format!("{}\n", recorded.display())).unwrap();
        let mut machine = stage.machine();
        machine.path = env::join_paths([stage.at("empty"), stage.at("path")]).unwrap();
        let shell = stage.shell(&format!("echo {}", from_shell.display()));
        machine.shell = Some(shell);
        let before = stage.tree();

        assert_eq!(machine.yardr(), Ok((recorded.clone(), None)));
        fs::remove_file(&recorded).unwrap();
        assert_eq!(machine.yardr(), Ok((local.clone(), None)));
        fs::remove_file(&local).unwrap();
        assert_eq!(machine.yardr(), Ok((on_path.clone(), None)));
        fs::remove_file(&on_path).unwrap();
        // The shell is asked only now, once, and the one thing.
        assert!(!stage.at("shell/zsh.asked").exists());
        assert_eq!(machine.yardr(), Ok((from_shell.clone(), None)));
        assert_eq!(
            fs::read_to_string(stage.at("shell/zsh.asked")).unwrap(),
            "-l|-c|command -v yardr|"
        );
        fs::remove_file(&from_shell).unwrap();
        assert_eq!(machine.yardr(), Err(NOT_THERE.to_string()));

        // Four steps and the end of them asked no one and wrote nothing
        // but what the test took away.
        assert_eq!(site.asked(), Vec::<String>::new());
        let left: Vec<_> = before
            .into_iter()
            .filter(|(path, ..)| *path != local)
            .collect();
        assert_eq!(stage.tree(), left);

        // The fifth is the person's to ask for, and step 2 finds what it left.
        machine.path = "/usr/bin:/bin".into();
        let installed = machine.install_from(&site.address, "macos", None).unwrap();
        assert_eq!(installed, stage.local());
        assert_eq!(site.asked(), [INSTALLER_AT]);
        assert_eq!(machine.yardr(), Ok((stage.local(), None)));
        // Its file is gone from footplate's directory.
        assert!(fs::read_dir(stage.at("own")).unwrap().next().is_none());
    }

    /// `<home>/binary` is a contract with a stranger's file: anything but
    /// one whole path to a program is no answer, and the next step's.
    #[test]
    fn a_broken_record_of_the_yards_yardr_falls_through() {
        let stage = Stage::new();
        let local = stage.program("home/.local/bin/yardr");
        let good = stage.program("elsewhere/yardr");
        let plain = stage.at("elsewhere/plain");
        fs::write(&plain, b"not to be run").unwrap();
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
        let machine = stage.machine();

        // No file at all.
        assert_eq!(machine.found(), Some(local.clone()));
        let good_text = good.display().to_string();
        for text in [
            String::new(),
            "\n".to_string(),
            "yardr\n".to_string(),
            format!("{good_text}\n{good_text}\n"),
            format!("{}\n", stage.at("gone/yardr").display()),
            format!("{}\n", plain.display()),
            format!("{}\n", stage.at("elsewhere").display()),
        ] {
            fs::write(stage.at("yard/binary"), &text).unwrap();
            assert_eq!(machine.found(), Some(local.clone()), "{text:?}");
        }
        // No newline is still the one line; a link counts by what it
        // leads to, and is named as it is written.
        fs::write(stage.at("yard/binary"), &good_text).unwrap();
        assert_eq!(machine.found(), Some(good.clone()));
        let link = stage.at("elsewhere/link");
        std::os::unix::fs::symlink(&good, &link).unwrap();
        fs::write(stage.at("yard/binary"), link.display().to_string()).unwrap();
        assert_eq!(machine.found(), Some(link));
    }

    /// A shell that fails, says what is no path, or has no such program is
    /// "not found"; a profile that talks first is still heard, and one
    /// that leaves something running on the pipe does not hold the answer.
    #[test]
    fn the_login_shell_is_heard_only_for_a_whole_path() {
        let stage = Stage::new();
        let there = stage.program("profile/yardr");
        let at = there.display();
        let ask = |line: &str| login_shell(&stage.shell(line), Duration::from_secs(60));

        assert_eq!(ask(&format!("echo {at}")), Some(there.clone()));
        assert_eq!(
            ask(&format!("echo Welcome back; echo {at}; echo")),
            Some(there.clone())
        );
        let pid_file = stage.at("profile/sleep.pid");
        let background = stage.shell(&format!(
            "sleep 30 & echo $! > {}; echo {at}; exit 0",
            pid_file.display()
        ));
        let found = login_shell(&background, Duration::from_millis(500));
        let pid: i32 = fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The background process owns the pipe after the shell ends. Stop
        // this test's child by pid so it does not outlive the test.
        // SAFETY: the pid came from this test's shell's `$!` output.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        assert_eq!(found, Some(there.clone()));
        assert_eq!(ask("exit 1"), None);
        assert_eq!(ask(&format!("echo {at}; exit 1")), None);
        assert_eq!(ask("echo 'alias yardr=/x/yardr'"), None);
        assert_eq!(ask("echo yardr"), None);
        assert_eq!(
            login_shell(&stage.at("no/shell"), Duration::from_secs(1)),
            None
        );
    }

    /// A shell that hangs is stopped at the timeout with what it started,
    /// and the lookup goes on to say there is none.
    #[test]
    fn a_login_shell_that_hangs_falls_through_within_the_timeout() {
        let stage = Stage::new();
        let pid = stage.at("pid");
        let shell = stage.shell(&format!("sleep 30 &\necho $! > {}\nwait", pid.display()));
        // The system assesses a program the first time it starts, a matter
        // of seconds on a busy Mac: once, before anything is measured. It
        // is a stand-in that hangs, so it is asked to stop at once.
        login_shell(&shell, Duration::from_millis(1));
        let _ = fs::remove_file(&pid);

        let mut machine = stage.machine();
        machine.shell = Some(shell);
        machine.within = Duration::from_millis(300);
        let asked = Instant::now();
        assert_eq!(machine.yardr(), Err(NOT_THERE.to_string()));
        assert!(asked.elapsed() < Duration::from_secs(10));

        // What it started went with it.
        let sleep: i32 = fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
        // SAFETY: signal 0 only asks whether the process is there.
        let alive = || unsafe { libc::kill(sleep, 0) } == 0;
        let stopped = Instant::now();
        while alive() && stopped.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!alive());
    }

    /// The installer's failures are its own words, or why it was not run,
    /// and no failure leaves anything behind.
    #[test]
    fn a_failing_installer_says_what_it_said() {
        let stage = Stage::new();
        let site = Site::start();
        let machine = stage.machine();
        let install = |os: &str| machine.install_from(&site.address, os, None);

        // Not on the site yet.
        let why = install("macos").unwrap_err();
        assert!(why.contains("status 404"), "{why}");
        // A page that is no script is not run.
        site.put(INSTALLER_AT, "<html>yardr</html>");
        let why = install("linux").unwrap_err();
        assert!(why.contains("it is no script"), "{why}");
        site.put(
            INSTALLER_AT,
            "#!/bin/sh\necho curl is not on PATH >&2\nexit 3\n",
        );
        let why = install("macos").unwrap_err();
        assert!(why.starts_with("The installer of yardr failed"), "{why}");
        assert!(why.ends_with("It said: curl is not on PATH"), "{why}");
        site.put(INSTALLER_AT, "#!/bin/sh\necho all done\n");
        let why = install("macos").unwrap_err();
        assert!(why.ends_with("is not there. It said: all done"), "{why}");
        assert!(fs::read_dir(stage.at("own")).unwrap().next().is_none());

        // Redirected elsewhere is not followed.
        site.put(INSTALLER_AT, "https://elsewhere.example/install.sh");
        site.status(INSTALLER_AT, 302);
        let why = install("linux").unwrap_err();
        assert!(why.contains("status 302"), "{why}");

        // Windows installs none and asks nothing.
        let asked = site.asked().len();
        let why = install("windows").unwrap_err();
        assert!(why.contains("no yardr to install"), "{why}");
        assert_eq!(site.asked().len(), asked);
    }

    /// A kept yardr, with login services: its copy runs every installed
    /// service's install with the PATH doctor leads to, and only then the
    /// old one goes.
    #[test]
    fn the_kept_yardr_moves_with_its_login_service() {
        for (daemon, web, bdp) in [(true, true, true), (false, false, true)] {
            let stage = Stage::new();
            let kept = stage.kept(0, &with_services(daemon, web));
            fs::write(
                stage.local().parent().unwrap().join("old-binary"),
                kept.display().to_string(),
            )
            .unwrap();
            standin::answers(&stage.local(), "bdp-status", 0, &bdp_answer(bdp));
            for part in ["daemon", "web", "bdp"] {
                standin::answers(
                    &stage.local(),
                    &format!("{part}-install"),
                    0,
                    &serde_json::json!({}),
                );
            }
            // A yard records the yardr that started it: the kept one is not
            // "found" for that.
            fs::write(stage.at("yard/binary"), format!("{}\n", kept.display())).unwrap();
            let machine = stage.machine();

            assert_eq!(machine.yardr(), Ok((stage.local(), None)));

            assert_eq!(fs::read(stage.local()).unwrap(), standin_script());
            assert!(!kept.exists() && !stage.at("own").join(VERSION_FILE).exists());
            let calls = fs::read_to_string(stage.at("home/.local/bin/calls")).unwrap();
            let mut installs = Vec::new();
            for (part, installed) in [("daemon", daemon), ("web", web), ("bdp", bdp)] {
                let at = calls
                    .lines()
                    .position(|line| line.starts_with(&format!("ARGS [{part}] [install]")));
                match (at, installed) {
                    (Some(at), true) => {
                        let line = calls.lines().nth(at).unwrap();
                        assert!(
                            line.contains(&format!("[--home] [{}]", stage.at("yard").display())),
                            "{line}"
                        );
                        assert!(line.ends_with("[old-present]"), "{line}");
                        installs.push(at);
                    }
                    (None, false) => {}
                    (at, installed) => {
                        panic!("{part} install ran: {at:?}, installed: {installed} in {calls}")
                    }
                }
            }
            assert!(installs.windows(2).all(|pair| pair[0] < pair[1]), "{calls}");
            assert_eq!(
                calls.lines().nth(installs[0] - 1).unwrap(),
                "PATH=/usr/bin:/bin:/opt/herdr/bin"
            );
            // The record now leads nowhere, which falls through to the copy.
            assert_eq!(machine.found(), Some(stage.local()));
        }
    }

    #[test]
    fn the_kept_yardr_moves_without_a_login_service() {
        for (status, answer) in [
            (0, with_services(false, false)),
            // No yard in that home at all.
            (2, serde_json::json!({})),
        ] {
            let stage = Stage::new();
            let kept = stage.kept(status, &answer);
            standin::answers(&stage.local(), "bdp-status", 0, &bdp_answer(false));
            assert_eq!(stage.machine().yardr(), Ok((stage.local(), None)));
            assert!(!kept.exists() && !stage.at("own").join(VERSION_FILE).exists());
            let calls = fs::read_to_string(stage.at("home/.local/bin/calls")).unwrap();
            assert!(!calls.contains("[install]"), "{calls}");
        }
    }

    /// A move that fails leaves the old yardr where its service starts it,
    /// says so, and leaves nothing that would keep the next try from
    /// happening.
    #[test]
    fn a_move_that_fails_leaves_the_kept_yardr_running() {
        let still_there = |stage: &Stage, kept: &Path, note: Option<String>, words: &str| {
            let note = note.unwrap();
            assert!(note.contains(words), "{note}");
            assert!(note.contains(&kept.display().to_string()), "{note}");
            assert!(kept.exists() && stage.at("own").join(VERSION_FILE).exists());
        };

        // No copy: `.local` is a file.
        let stage = Stage::new();
        let log = keep_in(&stage.at("log"));
        let kept = standin::make(&stage.at("own"));
        fs::write(stage.at("own").join(VERSION_FILE), "v0.14.0\n").unwrap();
        fs::write(stage.at("home/.local"), b"in the way").unwrap();
        let (used, note) = stage.machine().yardr().unwrap();
        assert_eq!(used, kept);
        still_there(&stage, &kept, note, "could not be copied");
        assert!(
            fs::read_to_string(log)
                .unwrap()
                .contains("could not be copied")
        );

        // The service could not be installed: the copy goes too, or it
        // would be found next time and the service left naming the old one.
        let stage = Stage::new();
        let kept = stage.kept(0, &with_services(true, false));
        standin::answers(
            &stage.local(),
            "daemon-install",
            1,
            &serde_json::json!({"error": "launchctl refused"}),
        );
        let (used, note) = stage.machine().yardr().unwrap();
        assert_eq!(used, kept);
        still_there(&stage, &kept, note, "launchctl refused");
        assert!(!stage.local().exists());

        // The web view's service could not be installed either, with no
        // daemon service before it: the old yardr stays where its
        // services start it, and the copy goes, so the move is tried
        // again.
        let stage = Stage::new();
        let kept = stage.kept(0, &with_services(false, true));
        standin::answers(
            &stage.local(),
            "web-install",
            1,
            &serde_json::json!({"error": "launchctl refused"}),
        );
        let (used, note) = stage.machine().yardr().unwrap();
        assert_eq!(used, kept);
        still_there(&stage, &kept, note, "launchctl refused");
        assert!(!stage.local().exists());

        // Nothing could tell whether there is a service.
        let stage = Stage::new();
        let kept = stage.kept(1, &serde_json::json!({"error": "the store cannot be read"}));
        let (used, note) = stage.machine().yardr().unwrap();
        assert_eq!(used, kept);
        still_there(&stage, &kept, note, "the store cannot be read");
        assert!(!stage.local().exists());

        // One of two services was moved: it names the copy, the other the
        // old one, so both stay.
        let stage = Stage::new();
        let kept = stage.kept(0, &with_services(true, false));
        standin::answers(&stage.local(), "bdp-status", 0, &bdp_answer(false));
        standin::answers(
            &stage.local(),
            "daemon-install.1",
            0,
            &serde_json::json!({}),
        );
        standin::answers(
            &stage.local(),
            "daemon-install",
            1,
            &serde_json::json!({"error": "launchctl refused"}),
        );
        let mut machine = stage.machine();
        machine.yards.push(stage.at("second"));
        let (used, note) = machine.yardr().unwrap();
        assert_eq!(used, kept);
        still_there(&stage, &kept, note, "launchctl refused");
        assert!(stage.local().exists());

        // A failure to clean up the migration's marker keeps the old
        // executable in place and reaches the person in the same note.
        let stage = Stage::new();
        let kept = stage.kept(0, &with_services(false, false));
        standin::answers(&stage.local(), "bdp-status", 0, &bdp_answer(false));
        let marker = kept.with_file_name(VERSION_FILE);
        fs::remove_file(&marker).unwrap();
        fs::create_dir(&marker).unwrap();
        let (used, note) = stage.machine().yardr().unwrap();
        assert_eq!(used, kept);
        still_there(&stage, &kept, note, "could not be removed");
        assert!(!stage.local().exists());
    }

    /// The stand-in's own bytes, to hold a copy to.
    fn standin_script() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        fs::read(standin::make(dir.path())).unwrap()
    }

    /// What the About card says is the yardr's own answer to `--version`:
    /// the first line it prints, as text, with where it is. With none there
    /// is nothing to ask; one that cannot say says why, and one that ends
    /// badly says nothing of its words.
    #[test]
    fn the_yardr_in_use_is_asked_its_version_and_says_why_when_it_cannot() {
        let stage = Stage::new();
        let machine = stage.machine();
        assert_eq!(machine.seen(), Seen::Nothing);
        assert_eq!(Machine::default().seen(), Seen::Nothing);

        // A file that would be found is left alone on a system with no yard:
        // the About card must not try to start it.
        let dir = stage.at("home/.local/bin");
        keep(&dir, PROGRAM, b"not a program").unwrap();
        assert_eq!(seen_on("windows", &machine), Seen::Nothing);

        // The yardr an older footplate kept is the one in use until it moves.
        fs::remove_file(stage.local()).unwrap();
        let kept = keep(&stage.at("own"), PROGRAM, b"#!/bin/sh\necho v0.14.0\n").unwrap();
        let _ = version_of(&kept, Duration::from_secs(60));
        assert_eq!(
            machine.seen(),
            Seen::Says {
                line: "v0.14.0".to_string(),
                path: kept.clone()
            }
        );
        fs::remove_file(&kept).unwrap();

        // The program is asked, whatever the file beside it says. One
        // stand-in answers every row with the shell the row writes to
        // `how`: the system assesses a program the first time it starts,
        // which takes a Mac a second or two, and 5 s and more while other
        // programs are being started for the first time too. The next
        // start takes a few hundredths of a second; a rewritten program
        // is assessed again.
        let program = keep(&dir, PROGRAM, b"#!/bin/sh\n. \"$(dirname \"$0\")/how\"\n").unwrap();
        // Start it once, and wait for it to end (a minute at the most),
        // before a row measures anything. Not through `ask`: `seen` stops
        // a start at VERSION_WITHIN, the assessment goes on after the
        // stop, and the first row would pay what is left of it.
        fs::write(dir.join("how"), "exit 0").unwrap();
        let _ = version_of(&program, Duration::from_secs(60));
        let ask = |how: &str| {
            fs::write(dir.join("how"), how).unwrap();
            machine.seen()
        };
        let says = |line: &str| Seen::Says {
            line: line.to_string(),
            path: program.clone(),
        };
        let silent = |why: &str| Seen::Silent {
            why: why.to_string(),
            path: program.clone(),
        };
        assert_eq!(
            ask("echo 'v0.15.1 (cbceebd22919)'"),
            says("v0.15.1 (cbceebd22919)")
        );
        // A build with no release says its commit, as it prints it.
        assert_eq!(ask("echo 901cdec4be6d"), says("901cdec4be6d"));
        // The first line with words in it, trimmed, and the rest unread.
        assert_eq!(
            ask("printf '\\n   \\n  v1.2.3 (abc)  \\nsecond line\\n'"),
            says("v1.2.3 (abc)")
        );
        // Text only, and not a line of any length: what the program prints
        // is put in a window.
        assert_eq!(ask(r"printf 'v1\033[0m.2\n'"), says("v1[0m.2"));
        assert_eq!(
            ask(&format!("echo {}", "x".repeat(300))),
            says(&"x".repeat(VERSION_LINE))
        );

        // A program that ends badly has no version, whatever it printed.
        assert_eq!(
            ask("echo 'v1.0.0 (x)'; exit 3"),
            silent("ended with status 3")
        );
        assert_eq!(ask("kill -9 $$"), silent("ended by a signal"));
        assert_eq!(ask("exit 0"), silent("printed nothing"));
        assert_eq!(ask("echo; echo '  '"), silent("printed nothing"));

        // One that does not answer is stopped, not waited for: this one
        // would run for half a minute.
        fs::write(dir.join("how"), "exec sleep 30").unwrap();
        let asked = Instant::now();
        assert_eq!(
            version_of(&program, Duration::from_millis(300)),
            Err("no answer in time".to_string())
        );
        assert!(asked.elapsed() < Duration::from_secs(10));

        // One that cannot be started gives the system's reason.
        keep(&dir, PROGRAM, b"#!/no/such/interpreter\n").unwrap();
        assert!(matches!(
            machine.seen(),
            Seen::Silent { why, .. } if why.contains("No such file")
        ));
    }
    /// A program that is still open for writing somewhere when it is
    /// started, as one is on Linux while another process is on its way to
    /// being its own program, is started once that has passed.
    #[test]
    fn a_program_written_a_moment_ago_is_started_once_nothing_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let script = keep(dir.path(), "script", b"#!/bin/sh\nexit 7\n").unwrap();
        let open = OpenOptions::new().append(true).open(&script).unwrap();
        let closing = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(open);
        });
        let status = started(&mut Command::new(&script)).unwrap().wait().unwrap();
        assert_eq!(status.code(), Some(7));
        closing.join().unwrap();
    }

    /// What doctor names for a program on a Mac, on the tests' site.
    fn wanted(site: &Site, file: &str, bytes: &[u8], place: &Path) -> Wanted {
        Wanted {
            version: "1.2.3".to_string(),
            url: format!("{}{file}", site.address),
            sha256: sha256(bytes),
            size: bytes.len() as u64,
            member: None,
            run: Vec::new(),
            path: place.to_path_buf(),
        }
    }

    /// A program yardr names is fetched from its maker's address, which
    /// may send on to another, held to yardr's size and checksum, and put
    /// in its place to be run. One that is there already is not fetched.
    #[test]
    fn a_named_program_is_fetched_checked_and_put_in_its_place() {
        let site = Site::start();
        let home = tempfile::tempdir().unwrap();
        let (own, path) = (home.path().join("own"), OsStr::new("/usr/bin:/bin"));
        let place = home.path().join(".local/bin/herdr");
        site.put("/storage/herdr?signed=yes", b"herdr itself");
        site.put("/download/v1/herdr", format!("{}/elsewhere", site.address));
        site.status("/download/v1/herdr", 302);
        site.put("/elsewhere", "/storage/herdr?signed=yes");
        site.status("/elsewhere", 307);
        let herdr = wanted(&site, "/download/v1/herdr", b"herdr itself", &place);
        program(&herdr, home.path(), &own, path).unwrap();
        assert_eq!(fs::read(&place).unwrap(), b"herdr itself");
        assert_eq!(
            fs::metadata(&place).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            site.asked(),
            [
                "/download/v1/herdr",
                "/elsewhere",
                "/storage/herdr?signed=yes"
            ]
        );
        // There already: installed, and nothing is asked of anyone.
        program(&herdr, home.path(), &own, path).unwrap();
        assert_eq!(site.asked().len(), 3);

        // An archive gives its one member, by the name yardr has for it.
        let codex = home.path().join(".local/bin/codex");
        let archive = archive_of(&[("codex-aarch64-apple-darwin", b'0', b"codex itself")]);
        site.put("/codex.tar.gz", &archive);
        let mut named = wanted(&site, "/codex.tar.gz", &archive, &codex);
        named.member = Some("codex-aarch64-apple-darwin".to_string());
        program(&named, home.path(), &own, path).unwrap();
        assert_eq!(fs::read(&codex).unwrap(), b"codex itself");
    }

    /// A stale non-executable destination can be repaired by the named
    /// installer; a directory is left intact with a clear reason.
    #[test]
    fn an_unusable_program_place_does_not_make_install_retry_forever() {
        let site = Site::start();
        let home = tempfile::tempdir().unwrap();
        let (own, path) = (home.path().join("own"), OsStr::new("/usr/bin:/bin"));
        let place = home.path().join(".local/bin/herdr");
        fs::create_dir_all(place.parent().unwrap()).unwrap();
        fs::write(&place, b"old non-executable file").unwrap();
        fs::set_permissions(&place, fs::Permissions::from_mode(0o644)).unwrap();
        site.put("/herdr", b"verified herdr");
        let herdr = wanted(&site, "/herdr", b"verified herdr", &place);

        program(&herdr, home.path(), &own, path).unwrap();
        assert_eq!(fs::read(&place).unwrap(), b"verified herdr");
        assert_eq!(site.asked(), ["/herdr"]);

        let folder = home.path().join(".local/bin/codex");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("keep"), b"user data").unwrap();
        site.put("/codex", b"verified codex");
        let codex = wanted(&site, "/codex", b"verified codex", &folder);
        let before = site.asked().len();
        let why = program(&codex, home.path(), &own, path).unwrap_err();

        assert!(why.contains("is a folder"), "{why}");
        assert_eq!(site.asked().len(), before);
        assert_eq!(fs::read(folder.join("keep")).unwrap(), b"user data");
    }

    /// What is not the file yardr named is not kept and not run: another
    /// file, a longer one, an archive with more in it, a place outside the
    /// home, an address that sends on without end.
    #[test]
    fn a_program_that_is_not_the_named_file_is_neither_kept_nor_run() {
        let site = Site::start();
        let home = tempfile::tempdir().unwrap();
        let (own, path) = (home.path().join("own"), OsStr::new("/usr/bin:/bin"));
        let place = home.path().join(".local/bin/claude");
        let ran = home.path().join("ran");
        let installer = format!("#!/bin/sh\ntouch {}\n", ran.display());

        // Another file than the one that was verified, as an installer.
        site.put("/claude", installer.replace("touch", "tutch"));
        let mut other = wanted(&site, "/claude", installer.as_bytes(), &place);
        other.run = vec!["install".to_string()];
        let why = program(&other, home.path(), &own, path).unwrap_err();
        assert!(
            why.contains("is not the file yardr's makers verified"),
            "{why}"
        );
        // A longer one is not read to its end.
        site.put("/claude", format!("{installer}# and more"));
        let why = program(&other, home.path(), &own, path).unwrap_err();
        assert!(why.contains("was not fetched"), "{why}");
        // An archive that holds more than the program.
        let archive = archive_of(&[("claude", b'0', b"x"), ("extra", b'0', b"y")]);
        site.put("/claude.tar.gz", &archive);
        let mut more = wanted(&site, "/claude.tar.gz", &archive, &place);
        more.member = Some("claude".to_string());
        let why = program(&more, home.path(), &own, path).unwrap_err();
        assert!(why.contains("is not as expected"), "{why}");
        // A place that is not under the home, as it is written or by a
        // link on the way.
        let outside = tempfile::tempdir().unwrap();
        let away = wanted(&site, "/claude", b"x", &outside.path().join("bin/claude"));
        let why = program(&away, home.path(), &own, path).unwrap_err();
        assert!(why.contains("is not under your home"), "{why}");
        std::os::unix::fs::symlink(outside.path(), home.path().join("link")).unwrap();
        let linked = wanted(&site, "/claude", b"x", &home.path().join("link/bin/claude"));
        let why = program(&linked, home.path(), &own, path).unwrap_err();
        assert!(why.contains("is not under your home"), "{why}");
        assert!(!outside.path().join("bin/claude").exists());
        assert!(!outside.path().join("bin").exists());
        // An address that sends on to itself.
        site.put("/round", "/round");
        site.status("/round", 302);
        let round = wanted(&site, "/round", b"x", &place);
        let why = program(&round, home.path(), &own, path).unwrap_err();
        assert!(why.contains("sent on more than"), "{why}");

        assert!(!ran.exists() && !place.exists());
        assert!(fs::read_dir(&own).map_or(true, |mut dir| dir.next().is_none()));
    }

    /// A program its maker's installer puts in place: the file that was
    /// checked is run with yardr's arguments and the PATH it is given,
    /// with nothing to read, and is gone afterwards.
    #[test]
    fn a_maker_s_installer_is_run_with_its_arguments_and_removed() {
        let site = Site::start();
        let home = tempfile::tempdir().unwrap();
        let own = home.path().join("own");
        let place = home.path().join(".local/bin/claude");
        let installer = format!(
            "#!/bin/sh\nread line && exit 9\nmkdir -p {dir}\nprintf '%s %s %s' \"$1\" \"$2\" \"$PATH\" > {place}\n",
            dir = place.parent().unwrap().display(),
            place = place.display()
        );
        site.put("/claude", &installer);
        let mut claude = wanted(&site, "/claude", installer.as_bytes(), &place);
        claude.run = vec!["install".to_string(), "2.1.296".to_string()];
        program(&claude, home.path(), &own, OsStr::new("/usr/bin:/bin")).unwrap();
        assert_eq!(
            fs::read_to_string(&place).unwrap(),
            "install 2.1.296 /usr/bin:/bin"
        );
        assert!(fs::read_dir(&own).unwrap().next().is_none());

        // One that ends and leaves nothing in the place is said so.
        fs::remove_file(&place).unwrap();
        site.put("/quiet", "#!/bin/sh\necho nothing to do\n");
        let mut quiet = wanted(&site, "/quiet", b"#!/bin/sh\necho nothing to do\n", &place);
        quiet.run = vec!["install".to_string()];
        let why = program(&quiet, home.path(), &own, OsStr::new("/usr/bin:/bin")).unwrap_err();
        assert!(
            why.ends_with("is not there. It said: nothing to do"),
            "{why}"
        );
    }

    #[test]
    fn a_maker_s_installer_reports_progress_before_it_ends() {
        let site = Site::start();
        let home = tempfile::tempdir().unwrap();
        let place = home.path().join(".local/bin/claude");
        let installer = format!(
            "#!/bin/sh\necho vendor progress\nsleep 1\nmkdir -p {dir}\nprintf done > {place}\n",
            dir = place.parent().unwrap().display(),
            place = place.display()
        );
        site.put("/claude", &installer);
        let mut claude = wanted(&site, "/claude", installer.as_bytes(), &place);
        claude.run = vec!["install".to_string()];

        let home_path = home.path().to_path_buf();
        let own = home.path().join("own");
        let (send, progress) = async_channel::bounded(1);
        let installer = thread::spawn(move || {
            program_with_progress(
                &claude,
                &home_path,
                &own,
                OsStr::new("/usr/bin:/bin"),
                Some(send),
            )
        });
        assert_eq!(progress.recv_blocking().unwrap(), "vendor progress");
        assert!(!place.exists(), "the installer has not finished yet");
        installer.join().unwrap().unwrap();
        assert_eq!(fs::read_to_string(place).unwrap(), "done");
    }

    #[test]
    fn a_file_longer_than_it_may_be_is_not_read_to_its_end() {
        let site = Site::start();
        site.put("/file", vec![b' '; 65]);
        let refusal = get(&site.address, "/file", 64).unwrap_err();
        assert!(matches!(refusal, Refusal::Unreadable { .. }), "{refusal:?}");
        assert_eq!(get(&site.address, "/file", 65).unwrap().len(), 65);
    }

    #[test]
    fn a_member_is_taken_from_an_archive_that_holds_what_is_expected() {
        let most = 1024 * 1024;
        let alone = archive_of(&[("herdr", b'0', BINARY)]);
        assert_eq!(member(&alone, "herdr", most).unwrap(), BINARY);
        // A file longer than a block, and one that fills its blocks.
        for long in [vec![7u8; 700], vec![7u8; 1024]] {
            let archive = archive_of(&[("herdr", b'0', &long)]);
            assert_eq!(member(&archive, "herdr", most).unwrap(), long);
        }
    }

    #[test]
    fn an_archive_that_holds_anything_else_is_refused() {
        let most = 1024 * 1024;
        let refused = |archive: &[u8]| member(archive, "herdr", most).unwrap_err();

        let why = refused(&archive_of(&[
            ("herdr", b'0', BINARY),
            ("extra", b'0', b"x"),
        ]));
        assert!(why.contains("\"extra\""), "{why}");
        // A name that leaves the directory, or stands in one.
        for name in ["../herdr", "/usr/bin/herdr", "./herdr", "bin/herdr"] {
            let why = refused(&archive_of(&[(name, b'0', BINARY)]));
            assert!(why.contains("not expected"), "{name}: {why}");
        }
        // A link, a hard link and a directory under an expected name, and
        // the header of an extension, which could rename what follows.
        for kind in *b"215xgL" {
            let why = refused(&archive_of(&[("herdr", kind, b"")]));
            assert!(why.contains("no regular file"), "{}: {why}", kind as char);
        }
        let why = refused(&archive_of(&[
            ("herdr", b'0', BINARY),
            ("herdr", b'0', b"x"),
        ]));
        assert!(why.contains("twice"), "{why}");
        let why = refused(&archive_of(&[]));
        assert!(why.contains("holds no herdr"), "{why}");
        let why = refused(&archive_of(&[("herdr", b'0', b"")]));
        assert!(why.contains("empty"), "{why}");
    }

    #[test]
    fn an_archive_that_is_no_whole_gzipped_tar_is_refused() {
        let most = 1024 * 1024;
        let refused = |archive: &[u8]| member(archive, "herdr", most).unwrap_err();
        let whole = tar_member("herdr", b'0', BINARY);

        assert!(refused(b"no gzip at all").contains("no gzipped archive"));
        // No end, and cut in the middle of the file.
        assert!(refused(&gzip(&whole)).contains("no end"));
        assert!(refused(&gzip(&whole[..540])).contains("middle"));
        // Something after its end.
        let mut trailed = whole.clone();
        trailed.extend([0u8; 1024]);
        trailed.extend(tar_member("extra", b'0', b"x"));
        assert!(refused(&gzip(&trailed)).contains("follows its end"));
        // The compressed stream itself must also end with the archive; a
        // valid tar followed by unrelated bytes is not a whole archive.
        let mut compressed_trailing = gzip(&whole);
        compressed_trailing.extend_from_slice(b"extra");
        assert!(refused(&compressed_trailing).contains("follows its gzipped archive"));
        // A header whose sum is wrong.
        let mut wrong = whole.clone();
        wrong[0] = b'Y';
        wrong.extend([0u8; 1024]);
        assert!(refused(&gzip(&wrong)).contains("no tar archive"));
        // More than it may unpack to.
        let big = archive_of(&[("herdr", b'0', &vec![0u8; 4096])]);
        let why = member(&big, "herdr", 2048).unwrap_err();
        assert!(why.contains("unpacks to more than"), "{why}");
    }

    #[test]
    fn keeping_replaces_what_is_there_and_follows_no_link() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("bin");
        let elsewhere = home.path().join("elsewhere");
        fs::write(&elsewhere, b"not to be touched").unwrap();
        fs::create_dir_all(&dir).unwrap();
        // A stale part from the older fixed-name scheme, as a link elsewhere.
        std::os::unix::fs::symlink(&elsewhere, dir.join(".yardr.part")).unwrap();
        fs::write(dir.join("yardr"), b"the old one").unwrap();

        let path = keep(&dir, "yardr", b"the new one").unwrap();
        assert_eq!(path, dir.join("yardr"));
        assert_eq!(fs::read(&path).unwrap(), b"the new one");
        assert_eq!(fs::read(&elsewhere).unwrap(), b"not to be touched");
        assert!(
            fs::symlink_metadata(dir.join(".yardr.part"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn concurrent_keeps_publish_whole_programs() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("bin");
        let start = Arc::new(Barrier::new(3));
        let first = vec![b'a'; 1024 * 1024];
        let second = vec![b'b'; 1024 * 1024];
        let save = |bytes: Vec<u8>| {
            let dir = dir.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                keep(&dir, "yardr", &bytes).unwrap()
            })
        };
        let a = save(first.clone());
        let b = save(second.clone());
        start.wait();
        let a = a.join().unwrap();
        let b = b.join().unwrap();

        assert_eq!(a, dir.join("yardr"));
        assert_eq!(b, a);
        let stored = fs::read(a).unwrap();
        assert!(stored == first || stored == second);
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    }

    #[test]
    fn keeping_refuses_a_name_that_leaves_its_directory() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("bin");

        for name in ["../yardr", "subdir/yardr", "subdir\\yardr", ".", "..", ""] {
            let error = keep(&dir, name, b"not written").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
        assert!(!home.path().join("yardr").exists());
        assert!(!dir.exists());
    }

    /// What is built in: the one host, over TLS.
    #[test]
    fn the_site_is_yardr_dev_over_tls() {
        let site = Address::parse(SITE).unwrap();
        assert_eq!(site.to_string(), "https://yardr.dev");
    }

    #[test]
    fn every_refusal_is_said_in_words_for_a_person() {
        let site = || SITE.to_string();
        let refusals = [
            Refusal::NoHome,
            Refusal::NoSystem {
                system: "freebsd".into(),
            },
            Refusal::Down {
                site: site(),
                err: io::Error::from(io::ErrorKind::ConnectionRefused),
            },
            Refusal::Answered {
                site: site(),
                path: INSTALLER_AT.into(),
                status: 404,
            },
            unreadable(INSTALLER_AT, "it is no script"),
        ];
        for refusal in refusals {
            let words = refusal.to_string();
            // A sentence about yardr, and nothing of how the code names it.
            assert!(words.contains("yardr") && words.ends_with('.'), "{words}");
            assert!(
                !words.contains("Refusal") && !words.contains('{'),
                "{words}"
            );
        }
    }
}
