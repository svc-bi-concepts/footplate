//! yardr itself, fetched: its latest release is read from yardr.dev, proved
//! to be its makers', and its one binary kept in footplate's own directory,
//! to be run from there by the first run (setup.rs). A yardr that is not
//! proved is not kept, so none is ever run.
//!
//! What is trusted is one thing, the public key built into footplate. The
//! release signs its `checksums.txt` with minisign, and the archive is
//! taken for yardr's because its SHA-256 is in the checksums that key
//! signed. `latest.json` only says which version there is and where:
//! nothing is trusted because it says so, and its own `sha256` and `size`
//! are not read. The version it names is in the archive's name, and that
//! name is looked up in the signed checksums, so an older release cannot
//! be handed over as a newer one.
//!
//! The site is the one host yardr is fetched from. It is not followed
//! elsewhere: an answer that is no 200, a redirect among them, is a
//! refusal.
//!
//! What `yardr doctor --json` names for a program fetched on a Mac
//! (docs/doctor.md of the yardr repository) goes the same way, by the same
//! plain functions ([`program`]): its [`sha256`] is compared, the one
//! [`member`] of a gzipped tar taken, and the program [`keep`]-ed. It comes
//! from its maker's address, which may send on to another, and is trusted
//! for its checksum alone, which the proved yardr carries. Only yardr's
//! own fetch has the signature on top.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{env, fmt, thread};

use flate2::bufread::GzDecoder;
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::api::{self, Address};

/// The site yardr's releases are on: the one host yardr is ever fetched
/// from.
pub const SITE: &str = "https://yardr.dev";

/// The public half of the minisign key yardr's releases are signed with,
/// as the second line of its `.pub` file has it: the key of yardr's
/// makers, id 0ECCFE6E81F150B3, made on 13 Oct 2026, and v0.10.0 is the
/// first release it signed. Nothing can replace it in a footplate that is
/// built: a new key means a new footplate.
const KEY: &str = "RWSzUPGBbv7MDqH+nnbNmYdOm20UTkuk7eC5e6XuVtYRwZtaEZRlV/WB";

/// The oldest yardr this footplate takes. A release below it is refused
/// though its signature is good: every release ever signed stays signed,
/// and an old one with a known fault must not be what the site hands out.
const LOWEST: Version = Version(0, 10, 0);

/// What says which release is the latest, the one address that does not
/// change with the version.
const LATEST: &str = "/releases/latest.json";

/// Everything that is fetched is under this.
const RELEASES: &str = "/releases/";

/// The binary, as the archive and footplate's directory name it.
const PROGRAM: &str = "yardr";

/// What a release's archive holds beside the binary. Anything else in it
/// is refused.
const BESIDE: [&str; 3] = ["README.md", "LICENSE", "NOTICE"];

/// The most bytes that are read of `latest.json` and of the checksums, of
/// the signature, of an archive, and of what an archive unpacks to. An
/// archive of yardr 0.10 is 9 MB and unpacks to 21.
const SMALL_MOST: u64 = 64 * 1024;
const SIGNATURE_MOST: u64 = 4 * 1024;
const ARCHIVE_MOST: u64 = 64 * 1024 * 1024;
const UNPACKED_MOST: u64 = 256 * 1024 * 1024;

/// Each writer gets its own temporary file before atomically replacing the
/// stored program. The pid also keeps names distinct across footplate runs.
static NEXT_PART: AtomicU64 = AtomicU64::new(0);

/// Why nothing was fetched, or why what was fetched is not kept. Each is
/// said in words for a person (`Display`), and nothing that is refused is
/// in footplate's directory afterwards.
#[derive(Debug)]
pub enum Refusal {
    /// footplate has no directory of its own: the person has no home.
    NoHome,
    /// This footplate was built without the key releases are signed with.
    NoKey,
    /// There is no yardr for this system, named as the releases name it
    /// where they could.
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
    /// The latest release is older than the oldest this footplate takes.
    TooOld { version: String, lowest: String },
    /// The release has no signature.
    NoSignature { site: String },
    /// The signature is not the built-in key's over these checksums.
    BadSignature,
    /// The signed checksums do not name the archive.
    NotListed { name: String },
    /// The archive is not the one the signed checksums name.
    Differs { name: String },
    /// The archive holds what a release does not, or not the binary.
    Archive { name: String, why: String },
    /// The binary could not be written to footplate's directory.
    NotKept { path: PathBuf, err: io::Error },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoHome => f.write_str(
                "There is no home directory to keep yardr in, so it was not fetched.",
            ),
            Refusal::NoKey => f.write_str(
                "This footplate was built without the key yardr's releases are signed with, so it cannot tell whether a yardr is the real one, and fetches none.",
            ),
            Refusal::NoSystem { system } => write!(
                f,
                "There is no yardr for this system ({system}): its releases are for macOS and Linux, on arm64 and amd64."
            ),
            Refusal::Down { site, err } => write!(
                f,
                "{site} does not answer, so yardr was not fetched ({err}). This needs the network once."
            ),
            Refusal::Answered { site, path, status } => write!(
                f,
                "{site} did not serve {path} directly (status {status}), so yardr was not fetched."
            ),
            Refusal::Unreadable { what, why } => {
                write!(f, "footplate cannot read {what}: {why}. yardr was not fetched.")
            }
            Refusal::TooOld { version, lowest } => write!(
                f,
                "The yardr that is offered is {version}, older than {lowest}, the oldest this footplate takes. It was not fetched."
            ),
            Refusal::NoSignature { site } => write!(
                f,
                "The release of yardr at {site} is not signed, so nothing shows who made it. It was not fetched."
            ),
            Refusal::BadSignature => f.write_str(
                "The signature of yardr's release is not that of yardr's makers. What was fetched is not kept.",
            ),
            Refusal::NotListed { name } => write!(
                f,
                "The checksums yardr's makers signed do not name {name}. It was not fetched."
            ),
            Refusal::Differs { name } => write!(
                f,
                "{name} is not the file yardr's makers signed: its checksum differs. It is not kept."
            ),
            Refusal::Archive { name, why } => write!(
                f,
                "{name} is not what a release holds: {why}. Nothing of it is kept."
            ),
            Refusal::NotKept { path, err } => write!(
                f,
                "yardr was fetched and is its makers', but could not be written to {} ({err}).",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// A release's version: `v0.10.0`, three numbers and nothing after them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    /// Reads a release's tag. One that says more than three numbers (a
    /// pre-release) is none: where it would stand against the lowest
    /// version is not for footplate to guess.
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.strip_prefix('v')?.split('.');
        let mut number = || {
            let part = parts.next()?;
            // `parse` alone would take a sign.
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        };
        let version = Version(number()?, number()?, number()?);
        parts.next().is_none().then_some(version)
    }

    /// The archive of this version for `target`, as a release names it.
    fn archive(&self, target: &str) -> String {
        let Version(major, minor, patch) = self;
        format!("{PROGRAM}_{major}.{minor}.{patch}_{target}.tar.gz")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Version(major, minor, patch) = self;
        write!(f, "v{major}.{minor}.{patch}")
    }
}

/// The yardr that is kept: where, to be run from there by its whole path,
/// and of which version.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub path: PathBuf,
    pub version: String,
}

/// Where footplate keeps the programs it fetched: a `bin` beside its
/// settings. Nowhere for a person without a home.
pub fn dir() -> Option<PathBuf> {
    directory_for(&crate::settings::file()?)
}

fn directory_for(settings: &Path) -> Option<PathBuf> {
    Some(settings.parent()?.join("bin"))
}

/// A system as yardr's releases name it, Go's GOOS and GOARCH: `os` and
/// `arch` are Rust's names (`std::env::consts`).
pub fn target_of(os: &str, arch: &str) -> Option<String> {
    let os = match os {
        "macos" => "darwin",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => return None,
    };
    Some(format!("{os}_{arch}"))
}

/// The file beside the kept yardr that says which version it is: the
/// release's tag, on one line.
const VERSION_FILE: &str = "yardr.version";

/// The yardr to run: the one that is kept, where it is the latest the
/// site has, and else the latest, fetched from [`SITE`], proved and kept in
/// [`dir`] in the other's place. Nothing but `latest.json` is read where
/// the kept one is the latest. A yardr that is kept is never replaced by an
/// older one, whatever the site says is the latest.
///
/// Where a newer one could not be had, for whatever reason, the kept one
/// is still the answer, with why beside it: it was proved when it was
/// fetched, and a yard must start without the network. With none kept, the
/// reason is a refusal.
pub fn current() -> Result<(Stored, Option<Refusal>), Refusal> {
    let dir = dir().ok_or(Refusal::NoHome)?;
    let site = Address::parse(SITE).map_err(|why| Refusal::Unreadable {
        what: SITE.to_string(),
        why,
    })?;
    let target = target_of(env::consts::OS, env::consts::ARCH).ok_or(Refusal::NoSystem {
        system: format!("{} {}", env::consts::OS, env::consts::ARCH),
    })?;
    Source {
        site: &site,
        key: KEY,
        lowest: LOWEST,
        target: &target,
    }
    .current(&dir)
}

/// The yardr that is kept in `dir`, with the version its file says: none
/// where either is not there, or the version is not one.
pub fn stored(dir: &Path) -> Option<Stored> {
    let path = dir.join(PROGRAM);
    let version = fs::read_to_string(dir.join(VERSION_FILE)).ok()?;
    let version = Version::parse(version.trim())?;
    fs::metadata(&path).ok()?.is_file().then(|| Stored {
        path,
        version: version.to_string(),
    })
}

/// Where a release comes from and what it is held to. [`yardr`] has the
/// built-in ones; a test has a stand-in for the site and a key of its own.
struct Source<'a> {
    site: &'a Address,
    key: &'a str,
    lowest: Version,
    target: &'a str,
}

/// `latest.json`, as the README of the yardr.dev repository has it ("A
/// yardr release"), of which only what leads to the files is read.
#[derive(Deserialize)]
struct Latest {
    version: String,
    checksums: String,
    targets: HashMap<String, Target>,
}

#[derive(Deserialize)]
struct Target {
    path: String,
}

impl Source<'_> {
    /// Fetches the latest release whatever is kept.
    #[cfg(test)]
    fn yardr(&self, dir: &Path) -> Result<Stored, Refusal> {
        let (version, latest) = self.latest()?;
        self.release(dir, version, &latest)
    }

    /// [`current`], from this source and in `dir`.
    fn current(&self, dir: &Path) -> Result<(Stored, Option<Refusal>), Refusal> {
        let kept = stored(dir);
        let newer = self.latest().and_then(|(version, latest)| {
            let has = kept
                .as_ref()
                .and_then(|kept| Version::parse(&kept.version))
                .is_some_and(|kept| version <= kept);
            match has {
                true => Ok(None),
                false => self.release(dir, version, &latest).map(Some),
            }
        });
        match (newer, kept) {
            (Ok(Some(new)), _) => Ok((new, None)),
            (Ok(None), Some(kept)) => Ok((kept, None)),
            (Err(refusal), Some(kept)) => Ok((kept, Some(refusal))),
            (Err(refusal), None) => Err(refusal),
            // `has` is true of a kept one alone.
            (Ok(None), None) => Err(Refusal::NoHome),
        }
    }

    /// Which release the site says is the latest: its version, held to the
    /// lowest one, and where its files are.
    fn latest(&self) -> Result<(Version, Latest), Refusal> {
        // Before anything is asked of the site: without the key nothing it
        // answers could be kept.
        PublicKey::from_base64(self.key).map_err(|_| Refusal::NoKey)?;

        let latest = get(self.site, LATEST, SMALL_MOST)?;
        let latest: Latest =
            serde_json::from_slice(&latest).map_err(|err| unreadable(LATEST, err))?;
        let version = Version::parse(&latest.version).ok_or_else(|| {
            unreadable(
                LATEST,
                format!("{:?} is no version of a release", latest.version),
            )
        })?;
        if version < self.lowest {
            return Err(Refusal::TooOld {
                version: version.to_string(),
                lowest: self.lowest.to_string(),
            });
        }
        Ok((version, latest))
    }

    /// Fetches the release of `version` that `latest` leads to, proves it
    /// and keeps its binary in `dir`, with its version beside it.
    fn release(&self, dir: &Path, version: Version, latest: &Latest) -> Result<Stored, Refusal> {
        let key = PublicKey::from_base64(self.key).map_err(|_| Refusal::NoKey)?;
        let target = latest.targets.get(self.target).ok_or(Refusal::NoSystem {
            system: self.target.to_string(),
        })?;
        // The name is made here from the version, and not taken from the
        // path: it is what the signed checksums are asked for, so that the
        // version latest.json claims is the version that was signed.
        let name = version.archive(self.target);
        if file_under_releases(&target.path) != Some(name.as_str()) {
            return Err(unreadable(
                LATEST,
                format!(
                    "it names {:?} for {}, and the archive of {version} is {name} under {RELEASES}",
                    target.path, self.target
                ),
            ));
        }
        if file_under_releases(&latest.checksums).is_none() {
            return Err(unreadable(
                LATEST,
                format!(
                    "it names {:?} for the checksums, which is no file under {RELEASES}",
                    latest.checksums
                ),
            ));
        }

        // The signature before the archive: a release that is not signed
        // costs no download.
        let checksums = get(self.site, &latest.checksums, SMALL_MOST)?;
        let signature = format!("{}.minisig", latest.checksums);
        let signature = match get(self.site, &signature, SIGNATURE_MOST) {
            Err(Refusal::Answered {
                site, status: 404, ..
            }) => return Err(Refusal::NoSignature { site }),
            other => other?,
        };
        signed(&key, &checksums, &signature)?;
        let sum = listed(&checksums, &name)?;

        let archive = get(self.site, &target.path, ARCHIVE_MOST)?;
        if sha256(&archive) != sum {
            return Err(Refusal::Differs { name });
        }
        let program = member(&archive, PROGRAM, &BESIDE, UNPACKED_MOST)
            .map_err(|why| Refusal::Archive { name, why })?;
        let not_kept = |err| Refusal::NotKept {
            path: dir.join(PROGRAM),
            err,
        };
        // The version's file goes first, away: between the two writes the
        // binary has no version, and is fetched again, never taken for
        // another version than it is.
        match fs::remove_file(dir.join(VERSION_FILE)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(not_kept(err)),
            _ => {}
        }
        let path = keep(dir, PROGRAM, &program).map_err(not_kept)?;
        keep_as(dir, VERSION_FILE, format!("{version}\n").as_bytes(), 0o644).map_err(not_kept)?;
        Ok(Stored {
            path,
            version: version.to_string(),
        })
    }
}

fn unreadable(what: &str, why: impl ToString) -> Refusal {
    Refusal::Unreadable {
        what: what.to_string(),
        why: why.to_string(),
    }
}

/// The file's name, when `path` is a file under `/releases/` written in
/// letters, digits, `.`, `_`, `-` and `/` alone. The path is the site's
/// word and goes into the first line of a request as it is.
fn file_under_releases(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(RELEASES)?;
    let plain = rest
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'));
    if !plain
        || rest
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return None;
    }
    rest.rsplit('/').next()
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

/// Whether `signature`, a `.minisig` file, is `key`'s over `text`. A
/// signature that cannot be read is a bad one, and so is one of the kind
/// minisign made before it hashed what it signs: no release has such a one.
fn signed(key: &PublicKey, text: &[u8], signature: &[u8]) -> Result<(), Refusal> {
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|lines| Signature::decode(lines).ok())
        .ok_or(Refusal::BadSignature)?;
    key.verify(text, &signature, false)
        .map_err(|_| Refusal::BadSignature)
}

/// The SHA-256 the checksums have for the file `name`, in small hex. The
/// checksums are lines of `sha256sum`: the sum, then the name.
fn listed(checksums: &[u8], name: &str) -> Result<String, Refusal> {
    const WHAT: &str = "the release's checksums";
    let checksums =
        std::str::from_utf8(checksums).map_err(|_| unreadable(WHAT, "they are no text"))?;
    let mut sums = checksums.lines().filter_map(|line| {
        let mut words = line.split_whitespace();
        let (Some(sum), Some(file), None) = (words.next(), words.next(), words.next()) else {
            return None;
        };
        // sha256sum marks a file it read as binary with a star.
        (file.strip_prefix('*').unwrap_or(file) == name).then_some(sum)
    });
    let sum = sums.next().ok_or_else(|| Refusal::NotListed {
        name: name.to_string(),
    })?;
    if sums.next().is_some() {
        return Err(unreadable(WHAT, format!("they name {name} twice")));
    }
    if sum.len() != 64 || !sum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unreadable(WHAT, format!("{name} has no SHA-256 in them")));
    }
    Ok(sum.to_ascii_lowercase())
}

/// The SHA-256 of `bytes`, in small hex.
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Takes the member `name` out of a gzipped tar. The archive may hold the
/// regular files `beside` too, and nothing else: no other name, no
/// directory, no link, no name twice, nothing after its end. It is read in
/// memory and nothing of it is written anywhere; what it unpacks to is
/// read to `most` bytes. A refusal says why, to follow the archive's name
/// in a sentence.
pub fn member(archive: &[u8], name: &str, beside: &[&str], most: u64) -> Result<Vec<u8>, String> {
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
        if file != name && !beside.contains(&file.as_str()) {
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

/// As [`keep`], for a file of the mode `mode`.
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
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&part)
        {
            Ok(file) => break (part, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };
    let written = (|| {
        file.write_all(bytes)?;
        // Keep the file private until it is whole, then make it runnable;
        // the mode is set again because umask may have cut it.
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
/// the checksum, which came with the yardr that was proved. `home` is
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
    // Installed, and not on PATH: nothing to fetch.
    if fs::symlink_metadata(place).is_ok() {
        return Ok(());
    }
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
    fs::create_dir_all(dir).map_err(|err| cannot(dir, err))?;
    let (real_dir, real_home) = (
        fs::canonicalize(dir).map_err(|err| cannot(dir, err))?,
        fs::canonicalize(home).map_err(|err| cannot(home, err))?,
    );
    if !real_dir.starts_with(&real_home) {
        return Err(outside());
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
        Some(member_name) => member(&bytes, member_name, &[], UNPACKED_MOST).map_err(|why| {
            format!("The archive of {name} is not as expected: {why}. Nothing of it is kept.")
        })?,
        None => bytes,
    };
    if wanted.run.is_empty() {
        keep(&real_dir, name, &program).map_err(|err| cannot(place, err))?;
        return Ok(());
    }

    // The maker's installer: kept in a directory of footplate's own that
    // nobody else can write to, so that the file that was checked is the
    // file that is run, and gone afterwards.
    let serial = NEXT_PART.fetch_add(1, Ordering::Relaxed);
    let aside = own.join(format!(".install.{}.{serial}", std::process::id()));
    fs::create_dir_all(own).map_err(|err| cannot(own, err))?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&aside)
        .map_err(|err| cannot(&aside, err))?;
    let ran = keep(&aside, name, &program)
        .map_err(|err| cannot(&aside, err))
        .and_then(|installer| run_installer(&installer, &wanted.run, path, progress));
    let _ = fs::remove_dir_all(&aside);
    let said = ran?;
    match fs::symlink_metadata(place).is_ok() {
        true => Ok(()),
        false => Err(format!(
            "The installer of {name} ended, and {} is not there. It said: {said}",
            place.display()
        )),
    }
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use minisign::KeyPair;

    use super::*;

    #[test]
    fn yardr_is_kept_beside_settings_under_an_isolated_config_home() {
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
            // "GET /releases/latest.json HTTP/1.1"
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

        fn remove(&self, path: &str) {
            self.files.lock().unwrap().remove(path);
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

    /// The system the tests' releases are for, whatever system runs them.
    const TARGET: &str = "linux_amd64";

    const BINARY: &[u8] = b"#!/bin/sh\necho this is no yardr, and nothing runs it\n";

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

    /// An archive as a release has it.
    fn release_archive() -> Vec<u8> {
        archive_of(&[
            ("yardr", b'0', BINARY),
            ("README.md", b'0', b"# yardr\n"),
            ("LICENSE", b'0', b"Apache License\n"),
            ("NOTICE", b'0', b"yardr\n"),
        ])
    }

    fn sign(keys: &KeyPair, text: &[u8]) -> String {
        minisign::sign(None, &keys.sk, Cursor::new(text), None, None)
            .unwrap()
            .into_string()
    }

    /// A release of the tests on a site of the tests, signed with a key
    /// made for it, and a directory to keep it in.
    struct Fixture {
        site: Site,
        keys: KeyPair,
        key: String,
        home: tempfile::TempDir,
    }

    impl Fixture {
        /// The site with the release `version`, whole and signed.
        fn new(version: &str) -> Fixture {
            let keys = KeyPair::generate_unencrypted_keypair().unwrap();
            let fixture = Fixture {
                site: Site::start(),
                key: keys.pk.to_base64(),
                keys,
                home: tempfile::tempdir().unwrap(),
            };
            fixture.release(version, &release_archive());
            fixture
        }

        fn archive_path(version: &str) -> String {
            let name = Version::parse(version).unwrap().archive(TARGET);
            format!("/releases/{version}/{name}")
        }

        fn checksums_path(version: &str) -> String {
            format!("/releases/{version}/checksums.txt")
        }

        /// Puts the release `version` on the site with `archive` as its
        /// archive: signed checksums that name it, and latest.json.
        fn release(&self, version: &str, archive: &[u8]) {
            let name = Version::parse(version).unwrap().archive(TARGET);
            let checksums = format!(
                "{}  yardr_other_system.tar.gz\n{}  {name}\n",
                sha256(b"another system's"),
                sha256(archive)
            );
            let path = Fixture::checksums_path(version);
            self.site.put(&Fixture::archive_path(version), archive);
            self.site.put(
                &format!("{path}.minisig"),
                sign(&self.keys, checksums.as_bytes()),
            );
            self.site.put(&path, checksums);
            self.latest(version, &Fixture::archive_path(version));
        }

        /// latest.json alone: it says `version`, with its archive at `path`.
        fn latest(&self, version: &str, path: &str) {
            self.site.put(
                LATEST,
                serde_json::json!({
                    "version": version,
                    "date": "2026-10-13",
                    "checksums": Fixture::checksums_path(version),
                    "targets": { TARGET: { "path": path, "size": 1, "sha256": "not read" } },
                })
                .to_string(),
            );
        }

        fn dir(&self) -> PathBuf {
            self.home.path().join("footplate").join("bin")
        }

        fn fetch(&self) -> Result<Stored, Refusal> {
            self.fetch_with(&self.key)
        }

        /// The yardr to run, as the first run asks for it.
        fn current(&self) -> Result<(Stored, Option<Refusal>), Refusal> {
            Source {
                site: &self.site.address,
                key: &self.key,
                lowest: Version(9, 0, 0),
                target: TARGET,
            }
            .current(&self.dir())
        }

        fn fetch_with(&self, key: &str) -> Result<Stored, Refusal> {
            Source {
                site: &self.site.address,
                key,
                lowest: Version(9, 0, 0),
                target: TARGET,
            }
            .yardr(&self.dir())
        }

        /// Whether footplate's directory holds nothing: no yardr and no
        /// part of one.
        fn kept_nothing(&self) -> bool {
            fs::read_dir(self.dir()).map_or(true, |mut dir| dir.next().is_none())
        }
    }

    #[test]
    fn a_signed_release_is_kept_in_footplates_directory() {
        let fixture = Fixture::new("v9.1.0");
        let stored = fixture.fetch().unwrap();

        assert_eq!(stored.path, fixture.dir().join("yardr"));
        assert_eq!(stored.version, "v9.1.0");
        assert_eq!(fs::read(&stored.path).unwrap(), BINARY);
        let mode = fs::metadata(&stored.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // The binary and its version: nothing else of the archive, no
        // part.
        let mut kept: Vec<_> = fs::read_dir(fixture.dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        kept.sort();
        assert_eq!(kept, ["yardr", "yardr.version"]);
        assert_eq!(
            fs::read_to_string(fixture.dir().join("yardr.version")).unwrap(),
            "v9.1.0\n"
        );
        assert_eq!(
            fixture.site.asked(),
            [
                "/releases/latest.json",
                "/releases/v9.1.0/checksums.txt",
                "/releases/v9.1.0/checksums.txt.minisig",
                "/releases/v9.1.0/yardr_9.1.0_linux_amd64.tar.gz",
            ]
        );
    }

    /// The first run's step 0: the release is fetched where no yardr is
    /// kept or the kept one's version is not the latest, and else the kept
    /// one is the answer, with `latest.json` alone read.
    #[test]
    fn the_kept_yardr_is_the_one_to_run_while_it_is_the_latest() {
        let fixture = Fixture::new("v9.1.0");
        assert_eq!(stored(&fixture.dir()), None);
        let (first, stale) = fixture.current().unwrap();
        assert!(stale.is_none());
        assert_eq!(first.version, "v9.1.0");
        assert_eq!(fixture.site.asked().len(), 4);
        assert_eq!(stored(&fixture.dir()), Some(first.clone()));

        // Kept and the latest: nothing but latest.json is asked for.
        let (again, stale) = fixture.current().unwrap();
        assert_eq!((again, stale.is_none()), (first.clone(), true));
        assert_eq!(fixture.site.asked()[4..], ["/releases/latest.json"]);

        // A newer one is out: it is fetched, proved and kept in its place.
        let newer = archive_of(&[("yardr", b'0', b"a newer yardr")]);
        fixture.release("v9.2.0", &newer);
        let (second, _) = fixture.current().unwrap();
        assert_eq!(second.version, "v9.2.0");
        assert_eq!(fs::read(&second.path).unwrap(), b"a newer yardr");
        assert_eq!(fixture.site.asked().len(), 9);

        // The site says an older one is the latest: the kept one is never
        // replaced by it, and nothing of it is fetched.
        fixture.latest("v9.1.0", &Fixture::archive_path("v9.1.0"));
        let (kept, stale) = fixture.current().unwrap();
        assert_eq!((kept.version.as_str(), stale.is_none()), ("v9.2.0", true));
        assert_eq!(fs::read(&kept.path).unwrap(), b"a newer yardr");
        assert_eq!(fixture.site.asked().len(), 10);
    }

    /// A newer release that fails its proof is not kept, and the yardr
    /// that was proved before is still the one to run, with why beside it.
    /// With none kept there is none to run.
    #[test]
    fn a_newer_release_that_fails_leaves_the_kept_one_to_run() {
        let fixture = Fixture::new("v9.1.0");
        fixture.current().unwrap();
        fixture.release("v9.2.0", &archive_of(&[("yardr", b'0', b"a newer yardr")]));
        fixture.site.put(
            &Fixture::archive_path("v9.2.0"),
            archive_of(&[("yardr", b'0', b"another's yardr")]),
        );
        let (kept, why) = fixture.current().unwrap();
        assert_eq!(kept.version, "v9.1.0");
        assert_eq!(fs::read(&kept.path).unwrap(), BINARY);
        assert!(matches!(why, Some(Refusal::Differs { .. })), "{why:?}");
        // So is it with the site away.
        fixture.site.status(LATEST, 503);
        let (kept, why) = fixture.current().unwrap();
        assert_eq!(kept.version, "v9.1.0");
        assert!(matches!(why, Some(Refusal::Answered { status: 503, .. })));

        // A binary whose version's file is gone is no kept yardr: with the
        // site away there is nothing to run.
        fs::remove_file(fixture.dir().join(VERSION_FILE)).unwrap();
        assert_eq!(stored(&fixture.dir()), None);
        assert!(matches!(
            fixture.current(),
            Err(Refusal::Answered { status: 503, .. })
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
    fn a_newer_release_replaces_the_kept_one_and_a_refused_one_leaves_it() {
        let fixture = Fixture::new("v9.1.0");
        fixture.fetch().unwrap();

        let newer = archive_of(&[("yardr", b'0', b"a newer yardr")]);
        fixture.release("v9.2.0", &newer);
        let stored = fixture.fetch().unwrap();
        assert_eq!(stored.version, "v9.2.0");
        assert_eq!(fs::read(&stored.path).unwrap(), b"a newer yardr");

        fixture
            .site
            .put(&Fixture::archive_path("v9.2.0"), release_archive());
        assert!(matches!(fixture.fetch(), Err(Refusal::Differs { .. })));
        assert_eq!(fs::read(&stored.path).unwrap(), b"a newer yardr");
    }

    #[test]
    fn a_version_below_the_lowest_is_refused_before_anything_is_fetched() {
        let fixture = Fixture::new("v8.9.9");
        let refusal = fixture.fetch().unwrap_err();

        assert!(
            matches!(&refusal, Refusal::TooOld { version, lowest } if version == "v8.9.9" && lowest == "v9.0.0"),
            "{refusal:?}"
        );
        assert_eq!(fixture.site.asked(), [LATEST]);
        assert!(fixture.kept_nothing());
    }

    /// The rollback: the site hands out an old release, signed as it was,
    /// and says it is a new one.
    #[test]
    fn an_older_release_under_a_newer_version_is_refused() {
        let fixture = Fixture::new("v8.0.0");

        // latest.json says v9.5.0 and points at the old archive.
        fixture.latest("v9.5.0", &Fixture::archive_path("v8.0.0"));
        let refusal = fixture.fetch().unwrap_err();
        assert!(matches!(refusal, Refusal::Unreadable { .. }), "{refusal:?}");

        // The old files, checksums and signature under the new version's
        // names: the signed checksums name the old archive alone.
        for (new, old) in [
            (
                Fixture::archive_path("v9.5.0"),
                Fixture::archive_path("v8.0.0"),
            ),
            (
                Fixture::checksums_path("v9.5.0"),
                Fixture::checksums_path("v8.0.0"),
            ),
            (
                format!("{}.minisig", Fixture::checksums_path("v9.5.0")),
                format!("{}.minisig", Fixture::checksums_path("v8.0.0")),
            ),
        ] {
            let bytes = fixture.site.files.lock().unwrap()[&old].clone();
            fixture.site.put(&new, bytes);
        }
        fixture.latest("v9.5.0", &Fixture::archive_path("v9.5.0"));
        let refusal = fixture.fetch().unwrap_err();
        assert!(
            matches!(&refusal, Refusal::NotListed { name } if name == "yardr_9.5.0_linux_amd64.tar.gz"),
            "{refusal:?}"
        );
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_release_with_no_signature_is_refused_and_its_archive_not_fetched() {
        let fixture = Fixture::new("v9.1.0");
        fixture
            .site
            .remove("/releases/v9.1.0/checksums.txt.minisig");
        let refusal = fixture.fetch().unwrap_err();

        assert!(
            matches!(refusal, Refusal::NoSignature { .. }),
            "{refusal:?}"
        );
        assert!(
            !fixture
                .site
                .asked()
                .contains(&Fixture::archive_path("v9.1.0"))
        );
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_signature_redirect_is_not_reported_as_a_missing_signature() {
        let fixture = Fixture::new("v9.1.0");
        let signature = "/releases/v9.1.0/checksums.txt.minisig";
        fixture.site.status(signature, 302);

        let refusal = fixture.fetch().unwrap_err();
        assert!(
            matches!(&refusal, Refusal::Answered { path, status: 302, .. } if path == signature),
            "{refusal:?}"
        );
        assert!(
            !fixture
                .site
                .asked()
                .contains(&Fixture::archive_path("v9.1.0"))
        );
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_signature_of_another_key_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        let other = KeyPair::generate_unencrypted_keypair().unwrap();

        // The site's release is signed with its own key, and footplate was
        // built with another.
        let refusal = fixture.fetch_with(&other.pk.to_base64()).unwrap_err();
        assert!(matches!(refusal, Refusal::BadSignature), "{refusal:?}");
        assert!(
            !fixture
                .site
                .asked()
                .contains(&Fixture::archive_path("v9.1.0"))
        );
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn checksums_changed_after_they_were_signed_are_refused() {
        let fixture = Fixture::new("v9.1.0");
        // Whoever holds the site swaps the archive and writes its checksum
        // into the checksums: the signature is of the old ones.
        let swapped = archive_of(&[("yardr", b'0', b"not yardr's makers'")]);
        let name = Version(9, 1, 0).archive(TARGET);
        fixture.site.put(&Fixture::archive_path("v9.1.0"), &swapped);
        fixture.site.put(
            &Fixture::checksums_path("v9.1.0"),
            format!("{}  {name}\n", sha256(&swapped)),
        );
        let refusal = fixture.fetch().unwrap_err();

        assert!(matches!(refusal, Refusal::BadSignature), "{refusal:?}");
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_signature_that_is_none_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        let path = "/releases/v9.1.0/checksums.txt.minisig";
        for signature in [&b""[..], b"untrusted comment: none\nAAAA\n", b"\xff\xfe"] {
            fixture.site.put(path, signature);
            let refusal = fixture.fetch().unwrap_err();
            assert!(matches!(refusal, Refusal::BadSignature), "{refusal:?}");
        }
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn an_archive_whose_checksum_differs_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        let mut archive = release_archive();
        let last = archive.len() - 1;
        archive[last] ^= 1;
        fixture.site.put(&Fixture::archive_path("v9.1.0"), archive);
        let refusal = fixture.fetch().unwrap_err();

        assert!(
            matches!(&refusal, Refusal::Differs { name } if name == "yardr_9.1.0_linux_amd64.tar.gz"),
            "{refusal:?}"
        );
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_signed_archive_that_holds_more_than_a_release_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        let archive = archive_of(&[
            ("yardr", b'0', BINARY),
            ("install.sh", b'0', b"#!/bin/sh\n"),
        ]);
        fixture.release("v9.1.0", &archive);
        let refusal = fixture.fetch().unwrap_err();

        assert!(
            matches!(&refusal, Refusal::Archive { why, .. } if why.contains("install.sh")),
            "{refusal:?}"
        );
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn checksums_that_do_not_name_the_archive_or_name_it_twice_are_refused() {
        let name = Version(9, 1, 0).archive(TARGET);
        let sum = sha256(b"an archive");

        let none = format!("{sum}  yardr_9.1.0_linux_arm64.tar.gz\n");
        assert!(matches!(
            listed(none.as_bytes(), &name),
            Err(Refusal::NotListed { .. })
        ));
        let twice = format!("{sum}  {name}\n{}  {name}\n", sha256(b"another"));
        assert!(matches!(
            listed(twice.as_bytes(), &name),
            Err(Refusal::Unreadable { .. })
        ));
        let short = format!("abc123  {name}\n");
        assert!(matches!(
            listed(short.as_bytes(), &name),
            Err(Refusal::Unreadable { .. })
        ));
        // As sha256sum writes it, with a star for a binary and in capitals.
        let starred = format!("{}  *{name}\n", sum.to_ascii_uppercase());
        assert_eq!(listed(starred.as_bytes(), &name).unwrap(), sum);
    }

    #[test]
    fn a_site_with_no_release_for_this_system_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        let refusal = Source {
            site: &fixture.site.address,
            key: &fixture.key,
            lowest: Version(9, 0, 0),
            target: "darwin_arm64",
        }
        .yardr(&fixture.dir())
        .unwrap_err();

        assert!(
            matches!(&refusal, Refusal::NoSystem { system } if system == "darwin_arm64"),
            "{refusal:?}"
        );
        assert_eq!(fixture.site.asked(), [LATEST]);
    }

    #[test]
    fn a_footplate_without_the_key_asks_the_site_nothing() {
        let fixture = Fixture::new("v9.1.0");
        for key in ["", "not a key"] {
            let refusal = fixture.fetch_with(key).unwrap_err();
            assert!(matches!(refusal, Refusal::NoKey), "{refusal:?}");
        }
        assert!(fixture.site.asked().is_empty());
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_latest_json_that_cannot_be_read_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        let archive = Fixture::archive_path("v9.1.0");
        let checksums = Fixture::checksums_path("v9.1.0");
        let with = |version: &str, checksums: &str, path: &str| {
            serde_json::json!({
                "version": version,
                "checksums": checksums,
                "targets": { TARGET: { "path": path } },
            })
            .to_string()
        };
        for latest in [
            "<html>a page of a hotel's network</html>".to_string(),
            "{}".to_string(),
            // No version of a release.
            with("9.1.0", &checksums, &archive),
            with("v9.1.0-rc1", &checksums, &archive),
            with("v9.1", &checksums, &archive),
            // Files that are not under /releases/, or not plainly named.
            with("v9.1.0", &checksums, "/yardr_9.1.0_linux_amd64.tar.gz"),
            with(
                "v9.1.0",
                &checksums,
                "/releases/../yardr_9.1.0_linux_amd64.tar.gz",
            ),
            with(
                "v9.1.0",
                &checksums,
                "/releases/v9.1.0/yardr_9.1.0_linux_amd64.tar.gz HTTP/1.1\r\nX: y",
            ),
            with(
                "v9.1.0",
                &checksums,
                "https://elsewhere.example/releases/v9.1.0/yardr_9.1.0_linux_amd64.tar.gz",
            ),
            with("v9.1.0", "/elsewhere/checksums.txt", &archive),
            with("v9.1.0", "/releases/v9.1.0/checksums.txt?x=1", &archive),
        ] {
            fixture.site.put(LATEST, &latest);
            let refusal = fixture.fetch().unwrap_err();
            assert!(
                matches!(refusal, Refusal::Unreadable { .. }),
                "{latest}: {refusal:?}"
            );
        }
        // Nothing but latest.json was asked for, each time.
        assert!(fixture.site.asked().iter().all(|path| path == LATEST));
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_site_that_has_no_latest_json_or_does_not_answer_is_refused() {
        let fixture = Fixture::new("v9.1.0");
        fixture.site.remove(LATEST);
        let refusal = fixture.fetch().unwrap_err();
        assert!(
            matches!(&refusal, Refusal::Answered { path, status: 404, .. } if path == LATEST),
            "{refusal:?}"
        );

        // A port nothing listens on: one that was bound, and is not now.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let nowhere = Address::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let refusal = Source {
            site: &nowhere,
            key: &fixture.key,
            lowest: Version(9, 0, 0),
            target: TARGET,
        }
        .yardr(&fixture.dir())
        .unwrap_err();
        assert!(matches!(refusal, Refusal::Down { .. }), "{refusal:?}");
        assert!(fixture.kept_nothing());
    }

    #[test]
    fn a_file_longer_than_it_may_be_is_not_read_to_its_end() {
        let fixture = Fixture::new("v9.1.0");
        fixture
            .site
            .put(LATEST, vec![b' '; SMALL_MOST as usize + 1]);
        let refusal = fixture.fetch().unwrap_err();
        assert!(matches!(refusal, Refusal::Unreadable { .. }), "{refusal:?}");

        assert_eq!(
            get(&fixture.site.address, LATEST, SMALL_MOST + 1)
                .unwrap()
                .len(),
            SMALL_MOST as usize + 1
        );
    }

    #[test]
    fn a_member_is_taken_from_an_archive_that_holds_what_is_expected() {
        let most = 1024 * 1024;
        assert_eq!(
            member(&release_archive(), "yardr", &BESIDE, most).unwrap(),
            BINARY
        );
        // What may be beside it need not be.
        let alone = archive_of(&[("yardr", b'0', BINARY)]);
        assert_eq!(member(&alone, "yardr", &BESIDE, most).unwrap(), BINARY);
        // A file longer than a block, and one that fills its blocks.
        let long = vec![7u8; 1024];
        let archive = archive_of(&[("README.md", b'0', &[1u8; 700]), ("yardr", b'0', &long)]);
        assert_eq!(member(&archive, "yardr", &BESIDE, most).unwrap(), long);
    }

    #[test]
    fn an_archive_that_holds_anything_else_is_refused() {
        let most = 1024 * 1024;
        let refused = |archive: &[u8]| member(archive, "yardr", &BESIDE, most).unwrap_err();

        let why = refused(&archive_of(&[
            ("yardr", b'0', BINARY),
            ("extra", b'0', b"x"),
        ]));
        assert!(why.contains("\"extra\""), "{why}");
        // A name that leaves the directory, or stands in one.
        for name in ["../yardr", "/usr/bin/yardr", "./yardr", "bin/yardr"] {
            let why = refused(&archive_of(&[(name, b'0', BINARY)]));
            assert!(why.contains("not expected"), "{name}: {why}");
        }
        // A link, a hard link and a directory under an expected name, and
        // the header of an extension, which could rename what follows.
        for kind in *b"215xgL" {
            let why = refused(&archive_of(&[("yardr", kind, b"")]));
            assert!(why.contains("no regular file"), "{}: {why}", kind as char);
        }
        let why = refused(&archive_of(&[
            ("yardr", b'0', BINARY),
            ("yardr", b'0', b"x"),
        ]));
        assert!(why.contains("twice"), "{why}");
        let why = refused(&archive_of(&[("README.md", b'0', b"# yardr\n")]));
        assert!(why.contains("holds no yardr"), "{why}");
        let why = refused(&archive_of(&[("yardr", b'0', b"")]));
        assert!(why.contains("empty"), "{why}");
    }

    #[test]
    fn an_archive_that_is_no_whole_gzipped_tar_is_refused() {
        let most = 1024 * 1024;
        let refused = |archive: &[u8]| member(archive, "yardr", &BESIDE, most).unwrap_err();
        let whole = tar_member("yardr", b'0', BINARY);

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
        // valid tar followed by unrelated bytes is not a whole release.
        let mut compressed_trailing = gzip(&whole);
        compressed_trailing.extend_from_slice(b"extra");
        assert!(refused(&compressed_trailing).contains("follows its gzipped archive"));
        // A header whose sum is wrong.
        let mut wrong = whole.clone();
        wrong[0] = b'Y';
        wrong.extend([0u8; 1024]);
        assert!(refused(&gzip(&wrong)).contains("no tar archive"));
        // More than it may unpack to.
        let big = archive_of(&[("yardr", b'0', &vec![0u8; 4096])]);
        let why = member(&big, "yardr", &BESIDE, 2048).unwrap_err();
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

    #[test]
    fn a_version_is_three_numbers_after_a_v() {
        assert_eq!(Version::parse("v0.10.0"), Some(Version(0, 10, 0)));
        assert_eq!(Version::parse("v0.10.0").unwrap().to_string(), "v0.10.0");
        for none in [
            "",
            "v",
            "0.10.0",
            "v0.10",
            "v0.10.0.1",
            "v0.10.0-rc1",
            "v0.+1.0",
            "v0..0",
            "v0.10.x",
            "v1.2.9999999999",
        ] {
            assert_eq!(Version::parse(none), None, "{none}");
        }
        // By its numbers, not by its letters.
        assert!(Version::parse("v0.9.9").unwrap() < Version::parse("v0.10.0").unwrap());
        assert!(Version::parse("v1.0.0").unwrap() > Version::parse("v0.99.99").unwrap());
        assert_eq!(
            Version(0, 10, 0).archive("darwin_arm64"),
            "yardr_0.10.0_darwin_arm64.tar.gz"
        );
    }

    #[test]
    fn a_system_is_named_as_the_releases_name_it() {
        assert_eq!(target_of("macos", "aarch64").unwrap(), "darwin_arm64");
        assert_eq!(target_of("macos", "x86_64").unwrap(), "darwin_amd64");
        assert_eq!(target_of("linux", "aarch64").unwrap(), "linux_arm64");
        assert_eq!(target_of("linux", "x86_64").unwrap(), "linux_amd64");
        assert_eq!(target_of("windows", "x86_64"), None);
        assert_eq!(target_of("linux", "riscv64"), None);
        // The system the tests run on is one of them.
        assert!(target_of(env::consts::OS, env::consts::ARCH).is_some());
    }

    /// What is built in: the one host, over TLS, and a key that is a key.
    #[test]
    fn the_site_is_yardr_dev_over_tls_and_a_built_in_key_is_a_key() {
        let site = Address::parse(SITE).unwrap();
        assert_eq!(site.to_string(), "https://yardr.dev");
        assert!(PublicKey::from_base64(KEY).is_ok());
        assert!(LOWEST >= Version(0, 10, 0));
    }

    /// The built-in key is the one releases are signed with, and no typo
    /// of it: v0.10.0's checksums and their signature, as that release
    /// published them (src/fixtures/yardr-v0.10.0), are that key's by the
    /// function the fetch asks. The files are read into the test when it
    /// is built, so it asks yardr.dev nothing.
    #[test]
    fn the_built_in_key_signed_the_checksums_of_v0_10_0() {
        let checksums: &[u8] = include_bytes!("fixtures/yardr-v0.10.0/checksums.txt");
        let signature: &[u8] = include_bytes!("fixtures/yardr-v0.10.0/checksums.txt.minisig");
        let key = PublicKey::from_base64(KEY).unwrap();

        signed(&key, checksums, signature).unwrap();
        // The archive for a system is found in them as the fetch finds it.
        let name = Version(0, 10, 0).archive("darwin_arm64");
        assert_eq!(
            listed(checksums, &name).unwrap(),
            "531771e558a7cab5986adc53da6bcc3b9fe48c6e519425d1f595c65b1e05698c"
        );

        // One byte of the checksums changed, their first and then the last
        // of their last name; and one byte of what the signature says of
        // them.
        for at in [0, checksums.len() - 2] {
            let mut changed = checksums.to_vec();
            changed[at] ^= 1;
            assert!(matches!(
                signed(&key, &changed, signature),
                Err(Refusal::BadSignature)
            ));
        }
        let at = signature
            .windows(5)
            .position(|bytes| bytes == b"file:")
            .unwrap();
        let mut changed = signature.to_vec();
        changed[at] ^= 1;
        assert!(matches!(
            signed(&key, checksums, &changed),
            Err(Refusal::BadSignature)
        ));
    }

    /// A signature the minisign program itself made (0.12, `minisign -S`),
    /// with a key that was made for this test and thrown away: releases
    /// are signed by that program, and the tests' own signer is a library.
    #[test]
    fn a_signature_of_the_minisign_program_is_read() {
        let key =
            PublicKey::from_base64("RWQNyPIoV7GL/P97bZk/pPzJ48AcgOPmTiPFicdxIN/gbfZu0x3AEthl")
                .unwrap();
        let checksums = b"aa11  yardr_9.1.0_linux_amd64.tar.gz\n";
        let signature = "untrusted comment: signature from minisign secret key\n\
            RUQNyPIoV7GL/OdyVoirSuSHlzkwfAsVRnn5iQ/ixIjmfqhoj80qVkwZ7Eaeri2KOjawvl1Qomx5HN4be4DkHE7d0Hxx79JmZQk=\n\
            trusted comment: a release of the tests\n\
            +SYLGnW63pmV5VPdwHRfDhiw74wwmz7U46Ali1hS/9BYzsPNYh+vlDRi8mKfxa9OoDxdr+Rx6D3n3F2PxTNXBw==\n";

        signed(&key, checksums, signature.as_bytes()).unwrap();
        // A letter of the checksums changed, and a changed trusted comment.
        let changed = b"aa12  yardr_9.1.0_linux_amd64.tar.gz\n";
        assert!(matches!(
            signed(&key, changed, signature.as_bytes()),
            Err(Refusal::BadSignature)
        ));
        let commented = signature.replace("a release of the tests", "a release of another's");
        assert!(matches!(
            signed(&key, checksums, commented.as_bytes()),
            Err(Refusal::BadSignature)
        ));
    }

    #[test]
    fn every_refusal_is_said_in_words_for_a_person() {
        let io = || io::Error::from(io::ErrorKind::ConnectionRefused);
        let name = || "yardr_0.10.0_darwin_arm64.tar.gz".to_string();
        let site = || SITE.to_string();
        let refusals = [
            Refusal::NoHome,
            Refusal::NoKey,
            Refusal::NoSystem {
                system: "freebsd x86_64".into(),
            },
            Refusal::Down {
                site: site(),
                err: io(),
            },
            Refusal::Answered {
                site: site(),
                path: LATEST.into(),
                status: 404,
            },
            unreadable(LATEST, "\"x\" is no version of a release"),
            Refusal::TooOld {
                version: "v0.9.0".into(),
                lowest: LOWEST.to_string(),
            },
            Refusal::NoSignature { site: site() },
            Refusal::BadSignature,
            Refusal::NotListed { name: name() },
            Refusal::Differs { name: name() },
            Refusal::Archive {
                name: name(),
                why: "it holds \"install.sh\", which is not expected in it".into(),
            },
            Refusal::NotKept {
                path: "/home/jo/.config/footplate/bin/yardr".into(),
                err: io(),
            },
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
        assert_eq!(
            Refusal::TooOld {
                version: "v0.9.0".into(),
                lowest: "v0.10.0".into(),
            }
            .to_string(),
            "The yardr that is offered is v0.9.0, older than v0.10.0, the oldest this footplate takes. It was not fetched."
        );
    }
}
