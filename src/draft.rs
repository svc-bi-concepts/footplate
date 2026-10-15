//! What is written and not sent yet: for each conversation of each yard
//! the files attached to it, and the text typed in it while another is
//! open. A file that is pasted or dropped is attached here and goes with
//! Enter, in one mail with the text, each with its base name.
//!
//! Nothing here calls the yard or draws: the window (window.rs) does both.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::model;

/// What a file from the clipboard is called: it has no name of its own.
const PASTED: &str = "what was pasted";

/// Something pasted or dropped, before it is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// A file's name, or what stands for one.
    pub name: String,
    /// Its bytes. Empty for a file that was refused before it was read.
    pub bytes: Vec<u8>,
    pub size: u64,
    /// Why it cannot go, if it cannot.
    pub refused: Option<String>,
}

/// A file attached to a draft, or what was refused as one: that stays,
/// marked, until the person removes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// Its own among all the drafts of the window.
    pub id: u64,
    pub name: String,
    pub bytes: Vec<u8>,
    pub size: u64,
    /// Why it is marked, if it is.
    pub refused: Option<String>,
}

/// How many bytes a file's name may hold on a letter: the yard's rule.
const NAME_BYTES: usize = 255;

/// How much of a dropped file its kind is told by: the yard reads the type
/// by its first 512 bytes.
const HEAD: usize = 512;

/// The most bytes one drop unpacks to on this machine, whatever the yard's
/// cap and count allow.
const UNPACK_BYTES: u64 = 256 * 1024 * 1024;

/// The first bytes of an entry's data that say it is itself an archive:
/// told by its bytes, not its suffix.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// What a drop goes on: a mail, or a note on a card. The count guard says
/// which.
#[derive(Clone, Copy)]
enum Letter {
    Mail,
    Note,
}

impl Letter {
    fn carries(&self) -> &'static str {
        match self {
            Letter::Mail => "mail carries",
            Letter::Note => "a note carries",
        }
    }
}

/// Whether `c` is a bidi control: what the yard's `CheckBlobName` refuses
/// beside the control characters, with U+2028 and U+2029.
fn is_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Whether `name` is one the yard takes on a letter: no `/`, `\` or
/// control character, and neither `.` nor `..`. As the yard's
/// `CheckBlobName` holds it, so a name refused here is one the yard would
/// refuse at send, and the send is never refused late for a name.
fn name_valid(name: &str) -> bool {
    if name == "." || name == ".." {
        return false;
    }
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    !name
        .chars()
        .any(|c| c.is_control() || is_bidi(c) || c == '\u{2028}' || c == '\u{2029}')
}

/// The stem and the extension of a base name: the extension is what
/// follows its last dot, the dot with it, and nothing when it has none or
/// is nothing but it.
fn split_name(base: &str) -> (&str, &str) {
    match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => base.split_at(stem.len()),
        _ => (base, ""),
    }
}

/// The name `stem` with `suffix` (the ` (2)` numbering, or nothing) and
/// `ext` goes under: shortened at a character boundary to the 255 bytes
/// the yard takes, the suffix and the extension kept whole. A name the
/// yard takes none of, or one that shortens past nothing, is why it
/// cannot go instead.
fn fit_name(stem: &str, suffix: &str, ext: &str) -> Result<String, String> {
    let candidate = format!("{stem}{suffix}{ext}");
    if !name_valid(&candidate) {
        return Err(format!(
            "{candidate} is no name the yard takes (a name holds no / \\ or control character, and is never . or ..)"
        ));
    }
    if candidate.len() <= NAME_BYTES {
        return Ok(candidate);
    }
    let keep = NAME_BYTES.saturating_sub(suffix.len() + ext.len());
    let mut short = String::new();
    for c in stem.chars() {
        if short.len() + c.len_utf8() > keep {
            break;
        }
        short.push(c);
    }
    if short.is_empty() {
        return Err(format!(
            "{candidate} is longer than the {NAME_BYTES} bytes a name may hold, and shortens to nothing"
        ));
    }
    // A shorter stem of a valid name is valid: the checks above hold for
    // it, and it cannot be `.` or `..` shortened from longer.
    Ok(format!("{short}{suffix}{ext}"))
}

/// Why `name` with `head` of `size` bytes is not attached, if it is not:
/// no name the yard takes, or over the yard's cap when it is known. The
/// yard takes any bytes; whether a link carries one to a linked yard is
/// the yard's to say, when the mail is sent.
pub fn unattachable(name: &str, head: &[u8], size: u64, cap: Option<u64>) -> Option<String> {
    let (stem, ext) = split_name(name);
    if let Err(why) = fit_name(stem, "", ext) {
        return Some(why);
    }
    cap.and_then(|cap| model::over_cap_for(head, size as usize, cap))
}

/// Why `name` is not attached when `existing` rows already stand on the
/// draft: one file too many for a mail.
pub fn one_too_many(name: &str, existing: usize, max: u64) -> String {
    single_over_count(Letter::Mail, name, existing, max)
}

/// Why `name` is not attached when `existing` rows already stand on the
/// draft, on a mail or a note: it is one file too many.
fn single_over_count(letter: Letter, name: &str, existing: usize, max: u64) -> String {
    format!(
        "{name} is one too many: with {existing} attached {} {max} at most (yardr settings: files_max)",
        letter.carries()
    )
}

/// Why a drop of `adding` files is not unpacked when `existing` rows
/// already stand on the draft, on a mail or a note: more than it carries.
/// One marked row for the whole drop, and none of its files attaches.
fn batch_over_count(letter: Letter, adding: usize, existing: usize, max: u64) -> String {
    let held = match adding {
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    };
    match existing {
        0 => format!(
            "the drop holds {held}, and {} {max} at most (yardr settings: files_max)",
            letter.carries()
        ),
        n => format!(
            "the drop holds {held}, and with {n} already attached {} {max} at most (yardr settings: files_max)",
            letter.carries()
        ),
    }
}

/// Why a draft of `n` files is not sent: more than a mail carries. No one
/// file is at fault, so none is marked: the draft is back as it was.
pub fn over_count(n: usize, max: u64) -> String {
    format!("the draft holds {n} files, and mail carries {max} at most (yardr settings: files_max)")
}

/// Why a note of `n` files is not sent: more than a note carries. No one
/// file is at fault, so none is marked: the words and the files are back
/// where they were.
pub fn note_over_count(n: usize, max: u64) -> String {
    format!(
        "the note holds {n} files, and a note carries {max} at most (yardr settings: files_max)"
    )
}

/// A file from the clipboard. The yard's cap and count are not known here:
/// the window asks for them and marks the attachment then.
pub fn pasted(bytes: Vec<u8>) -> Found {
    let size = bytes.len() as u64;
    Found {
        name: PASTED.to_string(),
        refused: None,
        bytes,
        size,
    }
}

/// What is dropped on the window for a mail: the file, or the files a zip
/// or a folder unpacks to on this machine, each with its base name. One
/// that cannot go is refused by the yard's cap `cap` when it is known and
/// by the letter's count `max`, counting the `existing` rows already on
/// the draft: the rest of it is not read, and nothing of it leaves the
/// window. A zip or a folder whose files are more than the letter carries
/// is one marked row, and none of its files attaches.
pub fn dropped(path: &Path, cap: Option<u64>, max: u64, existing: usize) -> Vec<Found> {
    dropped_as(path, cap, max, existing, Letter::Mail)
}

/// What is dropped on an open card for its note, as for a mail: a note
/// takes files as a conversation does, not pages alone.
pub fn dropped_note(path: &Path, cap: Option<u64>, max: u64, existing: usize) -> Vec<Found> {
    dropped_as(path, cap, max, existing, Letter::Note)
}

/// What is dropped on the window, for a mail or a note: a folder is walked
/// one level deep, a zip unpacked, anything else read whole. A symlink is
/// read as the file it names.
fn dropped_as(
    path: &Path,
    cap: Option<u64>,
    max: u64,
    existing: usize,
    letter: Letter,
) -> Vec<Found> {
    let name = match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    };
    if path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_dir())
    {
        return dropped_folder(path, &name, cap, max, existing, letter);
    }
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        return dropped_zip(path, &name, cap, max, existing, letter);
    }
    vec![dropped_file(path, &name, cap, max, existing, letter)]
}

/// A file that could not be read, in words.
fn unreadable(name: &str, size: u64, err: io::Error) -> Found {
    Found {
        name: name.to_string(),
        bytes: Vec::new(),
        size,
        refused: Some(format!("{name} could not be read: {err}")),
    }
}

/// A file dropped on the window: refused by the letter's count, its name
/// and the yard's cap, in that order, and read whole when it can go. One
/// that is refused is not read past its first bytes, and nothing of it
/// leaves the window.
fn dropped_file(
    path: &Path,
    name: &str,
    cap: Option<u64>,
    max: u64,
    existing: usize,
    letter: Letter,
) -> Found {
    if existing as u64 >= max {
        let size = path.metadata().map(|meta| meta.len()).unwrap_or(0);
        return Found {
            name: name.to_string(),
            bytes: Vec::new(),
            size,
            refused: Some(single_over_count(letter, name, existing, max)),
        };
    }
    let read = || -> io::Result<(Vec<u8>, u64, Option<String>)> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        let mut head = Vec::new();
        file.by_ref().take(HEAD as u64).read_to_end(&mut head)?;
        if let Some(refused) = unattachable(name, &head, size, cap) {
            return Ok((Vec::new(), size, Some(refused)));
        }
        let mut bytes = head;
        file.read_to_end(&mut bytes)?;
        Ok((bytes, size, None))
    };
    let (bytes, size, refused) = read().unwrap_or_else(|err| {
        let why = format!("{name} could not be read: {err}");
        (Vec::new(), 0, Some(why))
    });
    // The name it goes under: fitted, since it can go. Unrefused, the
    // fit cannot fail: it is the same name the check fitted.
    let (stem, ext) = split_name(name);
    let name = match refused.is_some() {
        true => name.to_string(),
        false => fit_name(stem, "", ext).unwrap_or_else(|_| name.to_string()),
    };
    Found {
        name,
        bytes,
        size,
        refused,
    }
}

/// A folder dropped on the window: its files one level deep, in the order
/// of their names, each as a file dropped on its own. A folder within, a
/// symlink and what is neither is skipped silently, and a folder that
/// holds no files is one marked row. More files than the letter carries is
/// one marked row too, and none attaches.
fn dropped_folder(
    path: &Path,
    name: &str,
    cap: Option<u64>,
    max: u64,
    existing: usize,
    letter: Letter,
) -> Vec<Found> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) => return vec![unreadable(name, 0, err)],
    };
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let is_file = entry
            .path()
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_file());
        if !is_file {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        files.push((file_name, entry.path()));
    }
    files.sort();
    if files.is_empty() {
        return vec![Found {
            name: name.to_string(),
            bytes: Vec::new(),
            size: 0,
            refused: Some(format!("{name} holds no files")),
        }];
    }
    if existing.saturating_add(files.len()) as u64 > max {
        return vec![Found {
            name: name.to_string(),
            bytes: Vec::new(),
            size: 0,
            refused: Some(batch_over_count(letter, files.len(), existing, max)),
        }];
    }
    files
        .iter()
        .enumerate()
        .map(|(at, (file_name, file_path))| {
            dropped_file(file_path, file_name, cap, max, existing + at, letter)
        })
        .collect()
}

/// An entry of a zip to unpack: its index in the archive, and its base
/// name, the paths inside the archive dropped.
struct Entry {
    index: usize,
    base: String,
}

/// A zip dropped on the window: each entry an attachment with its base
/// name only, unpacked here, nothing written to disk. A directory, a
/// symlink, an empty name, `.` and `..` are skipped silently; two entries
/// with one base name are numbered. More entries than the letter carries
/// is one marked row, and none attaches. An entry over the yard's cap, one
/// the read refuses, a nested archive and one that takes the drop past its
/// ceiling are each a marked row; past the ceiling the rest is not
/// unpacked.
fn dropped_zip(
    path: &Path,
    name: &str,
    cap: Option<u64>,
    max: u64,
    existing: usize,
    letter: Letter,
) -> Vec<Found> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return vec![unreadable(name, 0, err)],
    };
    let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(zip) => zip,
        Err(err) => {
            return vec![Found {
                name: name.to_string(),
                bytes: Vec::new(),
                size,
                refused: Some(format!("{name} could not be unpacked: {err}")),
            }];
        }
    };
    let mut planned: Vec<Entry> = Vec::new();
    for index in 0..zip.len() {
        let entry = match zip.by_index(index) {
            Ok(entry) => entry,
            Err(err) => {
                return vec![Found {
                    name: name.to_string(),
                    bytes: Vec::new(),
                    size,
                    refused: Some(format!("{name} could not be unpacked: {err}")),
                }];
            }
        };
        // The name as the crate reads it, UTF-8 or cp437; lossy where it
        // reads none, so an entry is never lost to its name.
        let decoded = match entry.name() {
            Ok(decoded) => decoded.into_owned(),
            Err(_) => String::from_utf8_lossy(entry.name_raw()).into_owned(),
        };
        if let Some(base) = unpackable(&decoded, entry.is_dir(), entry.unix_mode()) {
            planned.push(Entry { index, base });
        }
    }
    if planned.is_empty() {
        return vec![Found {
            name: name.to_string(),
            bytes: Vec::new(),
            size,
            refused: Some(format!("{name} holds no files")),
        }];
    }
    if existing.saturating_add(planned.len()) as u64 > max {
        return vec![Found {
            name: name.to_string(),
            bytes: Vec::new(),
            size,
            refused: Some(batch_over_count(letter, planned.len(), existing, max)),
        }];
    }
    let ceiling = unpack_ceiling(cap, max);
    let mut total: u64 = 0;
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut out: Vec<Found> = Vec::with_capacity(planned.len());
    for entry in &planned {
        let (stem, ext) = split_name(&entry.base);
        let count = seen.entry(entry.base.clone()).or_insert(0);
        *count += 1;
        let mut numbered = *count;
        let fitted = loop {
            let suffix = match numbered {
                1 => String::new(),
                n => format!(" ({n})"),
            };
            match fit_name(stem, &suffix, ext) {
                Err(why) => {
                    out.push(Found {
                        name: format!("{stem}{suffix}{ext}"),
                        bytes: Vec::new(),
                        size: 0,
                        refused: Some(why),
                    });
                    break None;
                }
                Ok(fitted) if used.insert(fitted.clone()) => break Some(fitted),
                Ok(_) => numbered += 1,
            }
        };
        let Some(fitted) = fitted else {
            continue;
        };
        let mut file = match zip.by_index(entry.index) {
            Ok(file) => file,
            Err(err) => {
                out.push(Found {
                    name: fitted.clone(),
                    bytes: Vec::new(),
                    size: 0,
                    refused: Some(format!("{fitted} could not be unpacked: {err}")),
                });
                continue;
            }
        };
        let declared = file.size();
        let mut head = Vec::new();
        if let Err(err) = file.by_ref().take(HEAD as u64).read_to_end(&mut head) {
            out.push(Found {
                name: fitted.clone(),
                bytes: Vec::new(),
                size: declared,
                refused: Some(format!("{fitted} could not be unpacked: {err}")),
            });
            continue;
        }
        if let Some(refused) =
            cap.and_then(|cap| model::over_cap_for(&head, declared as usize, cap))
        {
            out.push(Found {
                name: fitted,
                bytes: Vec::new(),
                size: declared,
                refused: Some(refused),
            });
            continue;
        }
        let rest = read_rest(&mut file, &head, declared, ceiling.saturating_sub(total));
        let mut bytes = head;
        let rest = match rest {
            Ok(rest) => rest,
            Err(err) => {
                out.push(Found {
                    name: fitted.clone(),
                    bytes: Vec::new(),
                    size: declared,
                    refused: Some(format!("{fitted} could not be unpacked: {err}")),
                });
                continue;
            }
        };
        bytes.extend_from_slice(&rest);
        if bytes.len() as u64 > ceiling.saturating_sub(total) {
            out.push(Found {
                name: fitted.clone(),
                bytes: Vec::new(),
                size: declared,
                refused: Some(format!(
                    "{fitted} takes the drop past {}: one drop unpacks no more",
                    model::size(ceiling)
                )),
            });
            break;
        }
        if let Err(held) = check_size(bytes.len() as u64, declared) {
            out.push(Found {
                name: fitted.clone(),
                bytes: Vec::new(),
                size: declared,
                refused: Some(format!(
                    "{fitted} claims {}, and the zip holds {held}",
                    model::size(declared)
                )),
            });
            continue;
        }
        total += bytes.len() as u64;
        if bytes.starts_with(ZIP_MAGIC) {
            out.push(Found {
                name: fitted.clone(),
                bytes: Vec::new(),
                size: declared,
                refused: Some(format!(
                    "{fitted} is a zip in a zip, and the window unpacks one level only"
                )),
            });
            continue;
        }
        out.push(Found {
            name: fitted,
            size: declared,
            bytes,
            refused: None,
        });
    }
    out
}

/// The ceiling on one drop's unpacked bytes: 256 MB, or the cap times
/// the count where the yard's cap is known.
fn unpack_ceiling(cap: Option<u64>, max: u64) -> u64 {
    match cap {
        Some(cap) => UNPACK_BYTES.min(cap.saturating_mul(max)),
        None => UNPACK_BYTES,
    }
}

/// The base name of a zip entry to unpack, if it unpacks: a directory,
/// a symlink, an empty name, `.` and `..` are skipped silently. The base
/// name only, never joined to anything on disk: an entry named `../x.pdf`
/// arrives as `x.pdf`.
fn unpackable(name: &str, is_dir: bool, unix_mode: Option<u32>) -> Option<String> {
    if is_dir || name.is_empty() {
        return None;
    }
    let base = name.rsplit('/').next().unwrap_or("");
    if base.is_empty() || base == "." || base == ".." {
        return None;
    }
    let symlink = unix_mode.is_some_and(|mode| mode & 0o170000 == 0o120000);
    (!symlink).then(|| base.to_string())
}

/// Whether an entry read to `len` bytes is its `declared` size: what
/// holds more or less is refused, never unpacked.
fn check_size(len: u64, declared: u64) -> Result<(), &'static str> {
    match len.cmp(&declared) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Greater => Err("more"),
        std::cmp::Ordering::Less => Err("less"),
    }
}

/// The rest of a zip entry past its `head`: `declared` bytes at most, one
/// past what `left` of the ceiling allows. One byte past either is read so
/// that the entry is refused for it, and no more. The byte past the
/// declared size is also what asks the reader past the entry's end, so its
/// own bound and checksum run: without it an entry read to exactly its
/// declared size is never verified.
fn read_rest(file: &mut impl Read, head: &[u8], declared: u64, left: u64) -> io::Result<Vec<u8>> {
    let limit = declared
        .min(left)
        .saturating_add(1)
        .saturating_sub(head.len() as u64);
    let mut rest = Vec::new();
    file.take(limit).read_to_end(&mut rest)?;
    Ok(rest)
}

/// Why a marked attachment holds its send back, if one does: a mail or
/// a note goes as its draft stands, never less one.
fn marked(attachments: &[Attachment]) -> Option<String> {
    let marked = attachments.iter().find(|a| a.refused.is_some())?;
    let n = attachments.iter().filter(|a| a.refused.is_some()).count();
    Some(match n {
        1 => format!(
            "{} is marked and cannot go: remove it, then send",
            marked.name
        ),
        n => format!("{n} attachments are marked and cannot go: remove them, then send"),
    })
}

/// Why Enter sends nothing of a draft, if it does not: empty when there is
/// nothing to send. A file is a mail without text; a marked attachment
/// holds the whole draft back, so that what goes is what the draft shows.
pub fn unsendable(text: &str, attachments: &[Attachment]) -> Option<String> {
    if let Some(why) = marked(attachments) {
        return Some(why);
    }
    match model::unsendable(text) {
        Some(why) if why.is_empty() && !attachments.is_empty() => None,
        other => other,
    }
}

/// Why attachments cannot go, each reason once: for the line their send
/// is typed in.
pub fn reasons(attachments: &[Attachment]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for why in attachments.iter().filter_map(|a| a.refused.as_ref()) {
        if !out.contains(why) {
            out.push(why.clone());
        }
    }
    out
}

/// Why Enter sends no note of the words with the card's files, if it does
/// not: a marked file holds the note back, and a file without words goes
/// nowhere, since the yard wants to know what it shows.
pub fn note_unsendable(words: &str, attachments: &[Attachment]) -> Option<String> {
    if let Some(why) = marked(attachments) {
        return Some(why);
    }
    if !attachments.is_empty() && words.trim().is_empty() {
        return Some("a note with a file needs words: say what it shows".to_string());
    }
    None
}

/// How long the question in a card's reference may be, before it is cut
/// short with "...".
const REFERENCE_QUESTION: usize = 64;

/// The first line a card puts in its asker's draft: the bead's handle as
/// the card shows it, whole, and the question's first line.
pub fn ask_reference(bead: &str, question: &str) -> String {
    let first = question.lines().next().unwrap_or_default();
    let mut chars = first.chars();
    let short: String = chars.by_ref().take(REFERENCE_QUESTION).collect();
    let question = match chars.next() {
        None => short,
        Some(_) => format!("{short}..."),
    };
    format!("re {bead} \"{question}\"")
}

/// Puts the card's reference before what the draft holds, with an empty
/// line between for the caret; a draft that names the card already is
/// left as it is. Answers the draft and where the caret goes: the empty
/// line after the reference.
pub fn prepend_reference(existing: &str, reference: &str) -> (String, usize) {
    let caret = reference.len() + 1;
    if existing.lines().next() == Some(reference) {
        return (existing.to_string(), caret.min(existing.len()));
    }
    (format!("{reference}\n\n{existing}"), caret)
}

/// A text that did not go, before what was typed since.
pub fn rejoin(unsent: String, typed: String) -> String {
    match (unsent.is_empty(), typed.is_empty()) {
        (_, true) => unsent,
        (true, false) => typed,
        (false, false) => format!("{unsent} {typed}"),
    }
}

#[derive(Debug, Default)]
struct Draft {
    text: String,
    attachments: Vec<Attachment>,
}

/// The drafts of the window, by the yard and by whom the conversation is
/// with there. They speak of one yard at a time, the one that is open: a
/// name is not the same one in another yard, and what was written for one
/// yard cannot be named from another, so it is never sent there.
#[derive(Debug, Default)]
pub struct Drafts {
    yards: HashMap<String, HashMap<String, Draft>>,
    /// The yard whose drafts these are now.
    open: String,
    /// The id of the attachment made last.
    last: u64,
}

impl Drafts {
    /// From here on these are the drafts of `yard`. Those of the one
    /// before stay as they are, for when it is open again; which that was
    /// is answered.
    pub fn turn_to(&mut self, yard: &str) -> String {
        std::mem::replace(&mut self.open, yard.to_string())
    }

    /// Nothing is kept of `yard`: it is no longer among the window's. The
    /// attachment ids let the window release their drawn image previews too.
    pub fn forget(&mut self, yard: &str) -> Vec<u64> {
        self.yards
            .remove(yard)
            .into_iter()
            .flat_map(HashMap::into_values)
            .flat_map(|draft| {
                draft
                    .attachments
                    .into_iter()
                    .map(|attachment| attachment.id)
            })
            .collect()
    }

    fn of(&self, with: &str) -> Option<&Draft> {
        self.yards.get(&self.open)?.get(with)
    }

    fn of_mut(&mut self, with: &str) -> Option<&mut Draft> {
        self.yards.get_mut(&self.open)?.get_mut(with)
    }

    fn entry(&mut self, with: &str) -> &mut Draft {
        let yard = self.yards.entry(self.open.clone()).or_default();
        yard.entry(with.to_string()).or_default()
    }

    fn all_mut(&mut self) -> impl Iterator<Item = &mut Draft> {
        self.yards.values_mut().flat_map(HashMap::values_mut)
    }

    /// What is attached to the draft for `with`, in the order it came.
    pub fn attachments(&self, with: &str) -> &[Attachment] {
        self.of(with).map_or(&[], |d| &d.attachments)
    }

    /// Why attachments of the draft for `with` are marked, each reason once.
    pub fn reasons(&self, with: &str) -> Vec<String> {
        reasons(self.attachments(with))
    }

    /// The id of the next attachment, of a draft or of a card's note: an
    /// id is its attachment's alone, in whichever yard, so one counter
    /// serves both.
    pub fn next_id(&mut self) -> u64 {
        self.last += 1;
        self.last
    }

    /// Attaches what was found to the draft for `with`, after what is there.
    pub fn attach(&mut self, with: &str, found: Found) -> u64 {
        let id = self.next_id();
        let draft = self.entry(with);
        draft.attachments.push(Attachment {
            id,
            name: found.name,
            bytes: found.bytes,
            size: found.size,
            refused: found.refused,
        });
        id
    }

    /// Takes the attachment `id` off whichever draft has it: an id is its
    /// attachment's alone, in whichever yard.
    pub fn remove(&mut self, id: u64) -> bool {
        self.all_mut().any(|d| {
            let before = d.attachments.len();
            d.attachments.retain(|a| a.id != id);
            d.attachments.len() < before
        })
    }

    /// Takes the last attachment off the draft for `with`, and says which.
    pub fn remove_last(&mut self, with: &str) -> Option<u64> {
        let draft = self.of_mut(with)?;
        draft.attachments.pop().map(|a| a.id)
    }

    /// Marks the attachment `id`: it cannot go, and why.
    pub fn refuse(&mut self, id: u64, why: &str) {
        let all = self.all_mut().flat_map(|d| &mut d.attachments);
        for a in all.filter(|a| a.id == id) {
            a.refused = Some(why.to_string());
        }
    }

    /// The attachments of the draft for `with`, to be sent in this order.
    /// The draft has none from then on.
    pub fn take(&mut self, with: &str) -> Vec<Attachment> {
        self.of_mut(with)
            .map(|d| std::mem::take(&mut d.attachments))
            .unwrap_or_default()
    }

    /// Attachments that did not go are back in the draft for `with`, before
    /// what was attached since.
    pub fn restore(&mut self, with: &str, mut unsent: Vec<Attachment>) {
        let draft = self.entry(with);
        unsent.append(&mut draft.attachments);
        draft.attachments = unsent;
    }

    /// Keeps what is typed to `with` while another conversation is open.
    pub fn put_text(&mut self, with: &str, text: String) {
        self.entry(with).text = text;
    }

    /// What was typed to `with`, for the line: the conversation is open.
    pub fn take_text(&mut self, with: &str) -> String {
        self.of_mut(with)
            .map(|d| std::mem::take(&mut d.text))
            .unwrap_or_default()
    }

    /// A text to `with` that did not go while another conversation is open:
    /// it is back in the draft, before what was typed there since.
    pub fn unsend(&mut self, with: &str, text: String) {
        let draft = self.entry(with);
        draft.text = rejoin(text, std::mem::take(&mut draft.text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe rest";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 16];

    fn image(bytes: &[u8]) -> Found {
        pasted(bytes.to_vec())
    }

    fn ids(drafts: &Drafts, with: &str) -> Vec<u64> {
        drafts.attachments(with).iter().map(|a| a.id).collect()
    }

    #[test]
    fn what_is_attached_stays_in_the_draft_in_the_order_it_came() {
        let mut drafts = Drafts::default();
        assert_eq!(drafts.attachments("yardmaster"), []);
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("yardmaster", image(JPEG));
        assert_eq!(ids(&drafts, "yardmaster"), [one, two]);
        let first = &drafts.attachments("yardmaster")[0];
        assert_eq!(
            (first.name.as_str(), first.bytes.as_slice(), first.size),
            ("what was pasted", PNG, 16)
        );
        assert_eq!(first.refused, None);
        assert_eq!(drafts.reasons("yardmaster"), Vec::<String>::new());
    }

    #[test]
    fn an_attachment_is_removed_by_its_mark_or_as_the_last() {
        let mut drafts = Drafts::default();
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("yardmaster", image(PNG));
        let three = drafts.attach("yardmaster", image(PNG));
        assert!(drafts.remove(two));
        assert_eq!(ids(&drafts, "yardmaster"), [one, three]);
        // One that is gone is not removed again.
        assert!(!drafts.remove(two));
        assert_eq!(drafts.remove_last("yardmaster"), Some(three));
        assert_eq!(drafts.remove_last("yardmaster"), Some(one));
        assert_eq!(drafts.remove_last("yardmaster"), None);
        assert_eq!(drafts.remove_last("nobody"), None);
        // An id is never given twice, whatever was removed.
        assert!(drafts.attach("yardmaster", image(PNG)) > three);
    }

    #[test]
    fn a_card_names_itself_in_its_asker_s_draft() {
        assert_eq!(
            ask_reference("bead-a", "Move them now?"),
            "re bead-a \"Move them now?\""
        );
        // A foreign card names the bead whole, with its peer.
        assert_eq!(
            ask_reference("yardr-ndv0@mac", "Parallel reviews?"),
            "re yardr-ndv0@mac \"Parallel reviews?\""
        );
        // The question's first line, cut short with "..." when long.
        assert_eq!(ask_reference("bead-a", "one\ntwo"), "re bead-a \"one\"");
        let long =
            "Parallel reviews (quality, security, UI): which of them land tonight, and who signs?";
        let short: String = long.chars().take(64).collect();
        assert_eq!(
            ask_reference("yardr-ndv0@mac", long),
            format!("re yardr-ndv0@mac \"{short}...\"")
        );
    }

    #[test]
    fn the_reference_goes_before_what_the_draft_holds_once_with_the_caret_after_it() {
        let reference = "re bead-a \"Move them now?\"";
        // An empty draft holds the reference and an empty line, the caret
        // on it.
        let (text, caret) = prepend_reference("", reference);
        assert_eq!(text, "re bead-a \"Move them now?\"\n\n");
        assert_eq!(&text[..caret], "re bead-a \"Move them now?\"\n");
        // What the draft held is kept after it.
        let (text, caret) = prepend_reference("half a thought", reference);
        assert_eq!(text, "re bead-a \"Move them now?\"\n\nhalf a thought");
        assert_eq!(&text[..caret], "re bead-a \"Move them now?\"\n");
        // A second ask names the card once: the draft is left as it is.
        let (again, caret) = prepend_reference(&text, reference);
        assert_eq!(again, text);
        assert_eq!(&again[..caret], "re bead-a \"Move them now?\"\n");
    }

    #[test]
    fn a_text_kept_for_a_conversation_keeps_its_line_breaks() {
        let mut drafts = Drafts::default();
        drafts.put_text("assembly", "one\ntwo\n\nfour".to_string());
        assert_eq!(drafts.take_text("assembly"), "one\ntwo\n\nfour");
        // So does one that did not go, before what was typed since.
        drafts.put_text("assembly", "typed\nsince".to_string());
        drafts.unsend("assembly", "not\nsent".to_string());
        assert_eq!(drafts.take_text("assembly"), "not\nsent typed\nsince");
    }

    #[test]
    fn each_conversation_has_its_own_draft() {
        let mut drafts = Drafts::default();
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("assembly", image(JPEG));
        drafts.put_text("yardmaster", "Like this.".into());
        assert_eq!(ids(&drafts, "yardmaster"), [one]);
        assert_eq!(ids(&drafts, "assembly"), [two]);
        assert_eq!(drafts.remove_last("assembly"), Some(two));
        assert_eq!(ids(&drafts, "yardmaster"), [one]);
        // The text is the line's once the conversation is open again.
        assert_eq!(drafts.take_text("assembly"), "");
        assert_eq!(drafts.take_text("yardmaster"), "Like this.");
        assert_eq!(drafts.take_text("yardmaster"), "");
        // Sending one leaves the other.
        drafts.attach("assembly", image(JPEG));
        assert_eq!(drafts.take("yardmaster").len(), 1);
        assert_eq!(drafts.attachments("assembly").len(), 1);
    }

    #[test]
    fn each_yard_has_its_own_drafts_and_none_is_named_from_another() {
        let mut drafts = Drafts::default();
        assert_eq!(drafts.turn_to("mac"), "");
        let one = drafts.attach("yardmaster", image(PNG));
        drafts.put_text("yardmaster", "For the mac.".into());
        // Another yard has a yardmaster too, and nothing written to it.
        assert_eq!(drafts.turn_to("plantime"), "mac");
        assert_eq!(drafts.attachments("yardmaster"), []);
        assert_eq!(drafts.take_text("yardmaster"), "");
        assert_eq!(drafts.take("yardmaster"), []);
        assert_eq!(drafts.remove_last("yardmaster"), None);
        // What is written here stays here, and an id is given once in the
        // whole window: a mark removes its own attachment wherever it is.
        let two = drafts.attach("yardmaster", image(JPEG));
        assert!(two > one);
        drafts.put_text("yardmaster", "For plantime.".into());
        drafts.refuse(one, "too large");
        assert_eq!(drafts.reasons("yardmaster"), Vec::<String>::new());
        // Back in the first, all of it is as it was left.
        assert_eq!(drafts.turn_to("mac"), "plantime");
        assert_eq!(ids(&drafts, "yardmaster"), [one]);
        assert_eq!(drafts.reasons("yardmaster"), ["too large"]);
        assert_eq!(drafts.take_text("yardmaster"), "For the mac.");
        // A yard that is removed from the list takes its drafts with it.
        assert_eq!(drafts.forget("plantime"), [two]);
        drafts.turn_to("plantime");
        assert_eq!(drafts.attachments("yardmaster"), []);
        assert_eq!(drafts.take_text("yardmaster"), "");
        assert!(!drafts.remove(two));
    }

    #[test]
    fn a_draft_is_sent_in_the_order_attached_and_what_did_not_go_is_back_first() {
        let mut drafts = Drafts::default();
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("yardmaster", image(JPEG));
        let sending = drafts.take("yardmaster");
        assert_eq!(sending.iter().map(|a| a.id).collect::<Vec<_>>(), [one, two]);
        assert_eq!(drafts.attachments("yardmaster"), []);
        assert_eq!(drafts.take("yardmaster"), []);

        // It did not go, and another was attached meanwhile.
        let three = drafts.attach("yardmaster", image(PNG));
        drafts.restore("yardmaster", sending);
        assert_eq!(ids(&drafts, "yardmaster"), [one, two, three]);
        // The yard refused the second: it is marked, with the yard's words.
        drafts.refuse(two, "an image is a PNG or a JPEG");
        assert_eq!(
            drafts.reasons("yardmaster"),
            ["an image is a PNG or a JPEG"]
        );
        assert_eq!(
            unsendable("Like this.", drafts.attachments("yardmaster")).as_deref(),
            Some("what was pasted is marked and cannot go: remove it, then send")
        );
        assert!(drafts.remove(two));
        assert_eq!(unsendable("", drafts.attachments("yardmaster")), None);

        // The text of a conversation that is not open is back before what
        // was typed there since.
        drafts.put_text("assembly", "And this.".into());
        drafts.unsend("assembly", "First this.".into());
        assert_eq!(drafts.take_text("assembly"), "First this. And this.");
        assert_eq!(rejoin("a".into(), String::new()), "a");
        assert_eq!(rejoin(String::new(), "b".into()), "b");
    }

    #[test]
    fn enter_sends_text_files_or_both_and_nothing_of_a_marked_draft() {
        let mut drafts = Drafts::default();
        assert_eq!(unsendable("  ", &[]), Some(String::new()));
        assert_eq!(unsendable("hello", &[]), None);
        drafts.attach("yardmaster", image(PNG));
        let attached = drafts.attachments("yardmaster");
        // A file is a mail without text.
        assert_eq!(unsendable("", attached), None);
        assert_eq!(unsendable("Like this.", attached), None);
        let long = unsendable(&"x".repeat(4097), attached).unwrap();
        assert!(long.contains("this is 4097"), "{long}");

        // What is pasted attaches whatever its bytes: a TIFF goes as a
        // file, under what it is called.
        let tiff = image(b"II*\0");
        assert_eq!(tiff.refused, None);
        assert_eq!(tiff.name, "what was pasted");
        let one = drafts.attach("yardmaster", tiff);
        let two = drafts.attach("yardmaster", image(b"GIF89a"));
        drafts.refuse(one, "too large");
        drafts.refuse(two, "too large");
        assert_eq!(
            unsendable("Like this.", drafts.attachments("yardmaster")).as_deref(),
            Some("2 attachments are marked and cannot go: remove them, then send")
        );
        // Each reason is said once.
        assert_eq!(drafts.reasons("yardmaster"), ["too large"]);
    }

    #[test]
    fn any_bytes_attach_and_only_the_cap_and_the_count_refuse_them() {
        assert_eq!(unattachable("a.png", PNG, 16, None), None);
        assert_eq!(unattachable("a.jpg", JPEG, 6, Some(6)), None);
        // A PDF, a TIFF, whatever its name: the yard takes any bytes.
        assert_eq!(
            unattachable("notes.pdf", b"%PDF-1.7", 900, Some(1024)),
            None
        );
        assert_eq!(unattachable("shot.tiff", b"II*\0", 4, None), None);
        assert_eq!(
            unattachable("a.png", PNG, 2048, Some(1024)).as_deref(),
            Some(
                "the image is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
        // The refusal names what the bytes are.
        assert_eq!(
            unattachable("notes.pdf", b"%PDF-1.7", 2048, Some(1024)).as_deref(),
            Some(
                "the file is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
        assert_eq!(
            unattachable("mockup.html", HTML, 2048, Some(1024)).as_deref(),
            Some(
                "the page is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
    }

    #[test]
    fn a_name_the_yard_takes_none_of_is_refused_in_words() {
        for name in [
            ".",
            "..",
            "a/b.pdf",
            "a\\b.pdf",
            "a\nb.pdf",
            "a\tb.pdf",
            "a\x7fb.pdf",
            "report\u{202e}fdp.exe",
            "a\u{2028}b.pdf",
            "a\u{2029}b.pdf",
        ] {
            let why = unattachable(name, PNG, 16, Some(1024)).unwrap();
            assert!(
                why.starts_with(&format!("{name} is no name the yard takes")),
                "{name:?}: {why}"
            );
        }
        // And what the yard takes attaches: a space, a quote, parentheses,
        // non-ASCII.
        for name in [
            "the report (2).pdf",
            "a\"b.pdf",
            "café.pdf",
            "what was pasted",
        ] {
            assert_eq!(unattachable(name, PNG, 16, Some(1024)), None, "{name:?}");
        }
    }

    #[test]
    fn a_dropped_file_is_read_when_it_can_go_and_refused_in_words_when_not() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };

        let found = dropped(&at("fence.png", PNG), Some(1024), 64, 0);
        assert_eq!(
            found,
            vec![Found {
                name: "fence.png".into(),
                bytes: PNG.to_vec(),
                size: 16,
                refused: None
            }]
        );
        // A PDF goes as a file, under its base name.
        let found = dropped(&at("notes.pdf", b"%PDF-1.7 and more"), Some(1024), 64, 0);
        assert_eq!(
            found,
            vec![Found {
                name: "notes.pdf".into(),
                bytes: b"%PDF-1.7 and more".to_vec(),
                size: 17,
                refused: None
            }]
        );
        // The cap could not be asked for: Enter asks again.
        assert_eq!(dropped(&at("fence.jpg", JPEG), None, 64, 0)[0].bytes, JPEG);

        // Over the cap: refused, and not read past its first bytes.
        let found = dropped(&at("big.png", PNG), Some(15), 64, 0);
        assert!(
            found[0]
                .refused
                .as_deref()
                .unwrap()
                .starts_with("the image is 16 bytes"),
            "{found:?}"
        );
        assert!(found[0].bytes.is_empty());

        // One too many for the letter: refused, and not read.
        let found = dropped(&at("one-more.png", PNG), Some(1024), 64, 64);
        assert_eq!(
            found[0].refused.as_deref(),
            Some(
                "one-more.png is one too many: with 64 attached mail carries 64 at most (yardr settings: files_max)"
            )
        );
        assert!(found[0].bytes.is_empty());

        // No name the yard takes: refused, and not read.
        let found = dropped(&at("a\nb.pdf", PNG), Some(1024), 64, 0);
        assert!(
            found[0]
                .refused
                .as_deref()
                .unwrap()
                .starts_with("a\nb.pdf is no name the yard takes"),
            "{found:?}"
        );
        assert!(found[0].bytes.is_empty());

        // A name past 255 bytes cannot exist on disk: shortening to
        // the yard's 255 is a zip entry's, and tested there.

        let found = dropped(&dir.path().join("gone.png"), None, 64, 0);
        let why = found[0].refused.clone().unwrap();
        assert!(why.starts_with("gone.png could not be read: "), "{why}");
    }

    const HTML: &[u8] = b"<!DOCTYPE html><html><body>the mockup</body></html>";
    const PDF: &[u8] = b"%PDF-1.7 and more";

    #[test]
    fn a_note_takes_files_as_a_conversation_does_not_pages_alone() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };

        // A page as before, and a PDF as a file.
        let found = dropped_note(&at("mockup.html", HTML), Some(1024), 64, 0);
        assert_eq!(
            found,
            vec![Found {
                name: "mockup.html".into(),
                bytes: HTML.to_vec(),
                size: 51,
                refused: None
            }]
        );
        let found = dropped_note(&at("notes.pdf", PDF), Some(1024), 64, 0);
        assert_eq!(found[0].refused, None);
        assert_eq!(found[0].bytes, PDF);

        // The guards say a note: the cap by what the bytes are, the count
        // by what carries them.
        let found = dropped_note(&at("big.html", HTML), Some(15), 64, 0);
        assert!(
            found[0]
                .refused
                .as_deref()
                .unwrap()
                .starts_with("the page is 51 bytes"),
            "{found:?}"
        );
        let found = dropped_note(&at("one-more.pdf", PDF), Some(1024), 2, 2);
        assert_eq!(
            found[0].refused.as_deref(),
            Some(
                "one-more.pdf is one too many: with 2 attached a note carries 2 at most (yardr settings: files_max)"
            )
        );

        let found = dropped_note(&dir.path().join("gone.html"), None, 64, 0);
        let why = found[0].refused.clone().unwrap();
        assert!(why.starts_with("gone.html could not be read: "), "{why}");
    }

    #[test]
    fn a_dropped_folder_walks_one_level_deep_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("papers");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("b.pdf"), PDF).unwrap();
        std::fs::write(root.join("a.pdf"), PDF).unwrap();
        // A folder within is not walked into, and a symlink is not
        // followed: neither attaches, silently.
        std::fs::create_dir(root.join("more")).unwrap();
        std::fs::write(root.join("more").join("c.pdf"), PDF).unwrap();
        std::os::unix::fs::symlink(root.join("a.pdf"), root.join("link.pdf")).unwrap();
        std::os::unix::fs::symlink(root.join("more"), root.join("morer")).unwrap();

        // In the order of their names.
        let found = dropped(&root, Some(1024), 64, 0);
        assert_eq!(
            found
                .iter()
                .map(|f| (f.name.as_str(), f.refused.clone()))
                .collect::<Vec<_>>(),
            [("a.pdf", None), ("b.pdf", None)]
        );
        assert_eq!(found[0].bytes, PDF);

        // A folder that holds no files is one marked row.
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        std::fs::create_dir(empty.join("more")).unwrap();
        let found = dropped(&empty, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].refused.as_deref(), Some("empty holds no files"));

        // More than the letter carries is one marked row too, and none
        // attaches: the rows already on the draft count.
        let found = dropped(&root, Some(1024), 3, 2);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].refused.as_deref(),
            Some(
                "the drop holds 2 files, and with 2 already attached mail carries 3 at most (yardr settings: files_max)"
            )
        );
        assert!(found[0].bytes.is_empty());
    }

    #[test]
    fn names_split_stem_from_extension_and_fit_the_yard_s_255() {
        assert_eq!(split_name("report.pdf"), ("report", ".pdf"));
        assert_eq!(split_name("a.tar.gz"), ("a.tar", ".gz"));
        assert_eq!(split_name("README"), ("README", ""));
        assert_eq!(split_name(".gitignore"), (".gitignore", ""));
        assert_eq!(fit_name("report", "", ".pdf").as_deref(), Ok("report.pdf"));
        // Numbered, kept whole.
        assert_eq!(
            fit_name("report", " (2)", ".pdf").as_deref(),
            Ok("report (2).pdf")
        );
        // Shortened at a character boundary, the suffix and the extension
        // kept: 251 é's and ` (2).pdf` would be 506 bytes.
        let fitted = fit_name(&"é".repeat(251), " (2)", ".pdf").unwrap();
        assert_eq!(fitted.len(), 254);
        assert!(fitted.ends_with(" (2).pdf"), "{fitted}");
        assert_eq!(fitted.chars().count(), 123 + 8);
        // An extension and a suffix past 255 on their own shorten to
        // nothing: a marked row, never a late refusal.
        let ext = format!(".{}", "x".repeat(250));
        let why = fit_name("report", " (2)", &ext).unwrap_err();
        assert!(why.ends_with("and shortens to nothing"), "{why}");
    }

    #[test]
    fn an_entry_skips_when_it_is_no_file_and_arrives_by_its_base_name() {
        // A directory, a symlink, an empty name, `.` and `..` are skipped
        // silently.
        assert_eq!(unpackable("papers/", true, None), None);
        assert_eq!(unpackable("papers", true, None), None);
        assert_eq!(unpackable("", false, None), None);
        assert_eq!(unpackable(".", false, None), None);
        assert_eq!(unpackable("..", false, None), None);
        assert_eq!(unpackable("papers/..", false, None), None);
        assert_eq!(unpackable("link.pdf", false, Some(0o120777)), None);
        // The base name only, never joined to anything on disk.
        assert_eq!(
            unpackable("../x.pdf", false, None).as_deref(),
            Some("x.pdf")
        );
        assert_eq!(
            unpackable("a/b/c.pdf", false, None).as_deref(),
            Some("c.pdf")
        );
        assert_eq!(
            unpackable("report.pdf", false, Some(0o100644)).as_deref(),
            Some("report.pdf")
        );
    }

    #[test]
    fn the_rest_of_an_entry_is_read_one_byte_past_either_bound() {
        use std::io::Cursor;
        // Exact: all of it.
        let mut data = Cursor::new(vec![7u8; 100]);
        assert_eq!(read_rest(&mut data, &[], 100, 1000).unwrap().len(), 100);
        // Past its head: the rest, no more.
        let mut data = Cursor::new(vec![7u8; 100]);
        assert_eq!(
            read_rest(&mut data, &[7u8; 40], 100, 1000).unwrap().len(),
            61
        );
        // One byte past its declared size, so the entry is refused for it.
        let mut data = Cursor::new(vec![7u8; 200]);
        assert_eq!(read_rest(&mut data, &[], 100, 1000).unwrap().len(), 101);
        // One byte past what the ceiling allows, for the same reason.
        let mut data = Cursor::new(vec![7u8; 200]);
        assert_eq!(read_rest(&mut data, &[], 1000, 100).unwrap().len(), 101);
    }

    #[test]
    fn the_count_reasons_say_what_holds_what() {
        assert_eq!(
            one_too_many("one-more.png", 64, 64),
            "one-more.png is one too many: with 64 attached mail carries 64 at most (yardr settings: files_max)"
        );
        assert_eq!(
            over_count(65, 64),
            "the draft holds 65 files, and mail carries 64 at most (yardr settings: files_max)"
        );
        assert_eq!(
            note_over_count(65, 64),
            "the note holds 65 files, and a note carries 64 at most (yardr settings: files_max)"
        );
    }

    #[test]
    fn a_note_with_a_file_needs_words_and_no_marked_file() {
        let file = |refused: Option<&str>| Attachment {
            id: 1,
            name: "report.pdf".into(),
            bytes: PDF.to_vec(),
            size: 17,
            refused: refused.map(str::to_string),
        };
        let files = [file(None)];
        assert_eq!(note_unsendable("the figures, third pass", &files), None);
        assert_eq!(
            note_unsendable("  \n ", &files).as_deref(),
            Some("a note with a file needs words: say what it shows")
        );
        assert_eq!(
            note_unsendable("the figures", &[file(Some("too large")), file(None)]).as_deref(),
            Some("report.pdf is marked and cannot go: remove it, then send")
        );
        assert_eq!(
            note_unsendable(
                "the figures",
                &[file(Some("too large")), file(Some("too large"))]
            )
            .as_deref(),
            Some("2 attachments are marked and cannot go: remove them, then send")
        );
    }

    /// A zip of stored files, written the usual way.
    fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A zip of deflated files, written the usual way.
    fn zip_deflated(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// An entry of a hand-built zip: `data` as stored, `declared` sizes
    /// where the entry lies about them, `attrs` for the symlink mode.
    struct RawEntry<'a> {
        name: &'a str,
        data: &'a [u8],
        declared: Option<u32>,
        attrs: u32,
    }

    /// A zip of stored entries, byte for byte: for the names no writer
    /// writes, the modes none sets, and the sizes none declares.
    fn raw_zip(entries: &[RawEntry]) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut table = [0u32; 256];
            for (i, slot) in table.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xedb88320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                *slot = c;
            }
            let mut crc = 0xffff_ffffu32;
            for b in data {
                crc = table[((crc ^ (*b as u32)) & 0xff) as usize] ^ (crc >> 8);
            }
            !crc
        }
        let mut out = Vec::new();
        let mut central = Vec::new();
        for entry in entries {
            let offset = out.len() as u32;
            let declared = entry.declared.unwrap_or(entry.data.len() as u32);
            let crc = crc32(entry.data);
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0x0800u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&declared.to_le_bytes());
            out.extend_from_slice(&declared.to_le_bytes());
            out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(entry.name.as_bytes());
            out.extend_from_slice(entry.data);
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&0x031eu16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0x0800u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&declared.to_le_bytes());
            central.extend_from_slice(&declared.to_le_bytes());
            central.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&entry.attrs.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(entry.name.as_bytes());
        }
        let offset = out.len() as u32;
        out.extend_from_slice(&central);
        let size = central.len() as u32;
        let n = entries.len() as u16;
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// A PDF of `n` bytes exactly, no two alike.
    fn pdf(n: usize) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        out.extend((0..n).map(|i| (i % 251) as u8));
        out.truncate(n);
        out
    }

    #[test]
    fn a_dropped_zip_unpacks_to_its_files_one_by_one() {
        let dir = tempfile::tempdir().unwrap();
        let held: Vec<(String, Vec<u8>)> = (0..30)
            .map(|n| (format!("papers/report-{n:02}.pdf"), pdf(100 + n)))
            .collect();
        let refs: Vec<(&str, &[u8])> = held
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_slice()))
            .collect();
        let path = dir.path().join("thirty.zip");
        std::fs::write(&path, zip_stored(&refs)).unwrap();

        // Thirty rows, each with its base name and its bytes; the zip
        // itself never travels.
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 30);
        for (n, row) in found.iter().enumerate() {
            assert_eq!(row.name, format!("report-{n:02}.pdf"));
            assert_eq!(row.refused, None);
            assert_eq!(row.bytes, pdf(100 + n));
            assert_eq!(row.size, 100 + n as u64);
        }

        // Deflated entries unpack too.
        let path = dir.path().join("deflated.zip");
        std::fs::write(&path, zip_deflated(&refs[..2])).unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "report-00.pdf");
        assert_eq!(found[0].bytes, pdf(100));
        assert_eq!(found[1].bytes, pdf(101));
    }

    #[test]
    fn a_zip_of_more_than_the_letter_carries_marks_the_drop() {
        let dir = tempfile::tempdir().unwrap();
        let held: Vec<(String, Vec<u8>)> = (0..70)
            .map(|n| (format!("report-{n:02}.pdf"), pdf(100)))
            .collect();
        let refs: Vec<(&str, &[u8])> = held
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_slice()))
            .collect();
        let path = dir.path().join("seventy.zip");
        std::fs::write(&path, zip_stored(&refs)).unwrap();

        // One marked row for the whole drop, and none of its files
        // attaches: it sends nothing.
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "seventy.zip");
        assert_eq!(
            found[0].refused.as_deref(),
            Some(
                "the drop holds 70 files, and mail carries 64 at most (yardr settings: files_max)"
            )
        );
        assert!(found[0].bytes.is_empty());

        // The rows already on the draft count: sixty files with ten
        // attached is more than the letter carries.
        let path = dir.path().join("sixty.zip");
        std::fs::write(&path, zip_stored(&refs[..60])).unwrap();
        let found = dropped(&path, Some(1024), 64, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].refused.as_deref(),
            Some(
                "the drop holds 60 files, and with 10 already attached mail carries 64 at most (yardr settings: files_max)"
            )
        );

        // Sixty-four files on an empty draft is exactly the letter: all
        // attach.
        let path = dir.path().join("sixty-four.zip");
        std::fs::write(&path, zip_stored(&refs[..64])).unwrap();
        assert_eq!(dropped(&path, Some(1024), 64, 0).len(), 64);

        // On a card the guard says a note.
        let found = dropped_note(&dir.path().join("seventy.zip"), Some(1024), 64, 0);
        assert_eq!(
            found[0].refused.as_deref(),
            Some(
                "the drop holds 70 files, and a note carries 64 at most (yardr settings: files_max)"
            )
        );
    }

    #[test]
    fn a_zip_entry_over_the_cap_is_marked_and_not_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mixed.zip");
        std::fs::write(
            &path,
            zip_stored(&[("small.pdf", &pdf(100)), ("big.pdf", &pdf(2048))]),
        )
        .unwrap();

        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].refused, None);
        assert_eq!(found[0].bytes, pdf(100));
        assert_eq!(
            found[1].refused.as_deref(),
            Some(
                "the file is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
        assert!(found[1].bytes.is_empty());
    }

    #[test]
    fn a_zip_whose_entry_lies_about_its_size_is_refused_not_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        // An entry that claims 2 KiB and holds 1 KiB: refused, not
        // unpacked, and the rest attaches. The read past its declared size
        // is what refuses it, in the reader's words.
        let lying = RawEntry {
            name: "lying.pdf",
            data: &pdf(1024),
            declared: Some(2048),
            attrs: 0o100644 << 16,
        };
        let straight = RawEntry {
            name: "straight.pdf",
            data: &pdf(100),
            declared: None,
            attrs: 0o100644 << 16,
        };
        let path = dir.path().join("lying.zip");
        std::fs::write(&path, raw_zip(&[lying, straight])).unwrap();
        let found = dropped(&path, Some(4096), 64, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].refused.as_deref(),
            Some("lying.pdf could not be unpacked: Invalid checksum")
        );
        assert!(found[0].bytes.is_empty());
        assert_eq!(found[1].refused, None);

        // An entry that claims 1 KiB and holds 100 MB: refused once the
        // read passes what it claims, long before the rest is read.
        let greedy = RawEntry {
            name: "greedy.pdf",
            data: &pdf(100 * 1024 * 1024),
            declared: Some(1024),
            attrs: 0o100644 << 16,
        };
        let path = dir.path().join("greedy.zip");
        std::fs::write(&path, raw_zip(&[greedy])).unwrap();
        let found = dropped(&path, Some(200 * 1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].refused.as_deref(),
            Some("greedy.pdf could not be unpacked: Invalid checksum")
        );
        assert!(found[0].bytes.is_empty());

        // An entry of the size it claims, but none of its bytes the
        // checksum answers for: refused, since the read past its end
        // verifies it.
        let mut rot = raw_zip(&[RawEntry {
            name: "rot.pdf",
            data: &pdf(1024),
            declared: None,
            attrs: 0o100644 << 16,
        }]);
        rot[30 + "rot.pdf".len() + 3] ^= 0xff;
        let path = dir.path().join("rot.zip");
        std::fs::write(&path, rot).unwrap();
        let found = dropped(&path, Some(4096), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].refused.as_deref(),
            Some("rot.pdf could not be unpacked: Invalid checksum")
        );
        assert!(found[0].bytes.is_empty());
    }

    #[test]
    fn what_holds_more_or_less_than_it_claims_is_refused() {
        assert_eq!(check_size(100, 100), Ok(()));
        assert_eq!(check_size(101, 100), Err("more"));
        assert_eq!(check_size(99, 100), Err("less"));
    }

    #[test]
    fn a_zip_in_a_zip_is_marked_and_the_rest_attaches() {
        let dir = tempfile::tempdir().unwrap();
        let nested = [b"PK\x03\x04".as_slice(), b"not really a zip".as_slice()].concat();
        let path = dir.path().join("nested.zip");
        std::fs::write(
            &path,
            zip_stored(&[("inner.zip", &nested), ("outer.pdf", PDF)]),
        )
        .unwrap();

        // Told by its first bytes, not its suffix: the nested one is
        // marked, and the rest attaches, to send once it is removed.
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].refused.as_deref(),
            Some("inner.zip is a zip in a zip, and the window unpacks one level only")
        );
        assert!(found[0].bytes.is_empty());
        assert_eq!(found[1].refused, None);
        assert_eq!(found[1].bytes, PDF);
    }

    #[test]
    fn entries_arrive_by_their_base_names_numbered() {
        let plain = RawEntry {
            name: "../x.pdf",
            data: PDF,
            declared: None,
            attrs: 0o100644 << 16,
        };
        let deep = RawEntry {
            name: "a/b/c.pdf",
            data: PDF,
            declared: None,
            attrs: 0o100644 << 16,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("names.zip");
        std::fs::write(&path, raw_zip(&[plain, deep])).unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(
            found
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            ["x.pdf", "c.pdf"]
        );

        // Two entries with one base name are numbered.
        let path = dir.path().join("numbered.zip");
        std::fs::write(
            &path,
            raw_zip(&[
                RawEntry {
                    name: "a/report.pdf",
                    data: PDF,
                    declared: None,
                    attrs: 0o100644 << 16,
                },
                RawEntry {
                    name: "b/report.pdf",
                    data: PDF,
                    declared: None,
                    attrs: 0o100644 << 16,
                },
                RawEntry {
                    name: "c/report.pdf",
                    data: PDF,
                    declared: None,
                    attrs: 0o100644 << 16,
                },
                RawEntry {
                    name: "a/README",
                    data: b"read me",
                    declared: None,
                    attrs: 0o100644 << 16,
                },
                RawEntry {
                    name: "b/README",
                    data: b"read me",
                    declared: None,
                    attrs: 0o100644 << 16,
                },
            ]),
        )
        .unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(
            found
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            [
                "report.pdf",
                "report (2).pdf",
                "report (3).pdf",
                "README",
                "README (2)"
            ]
        );
    }

    #[test]
    fn the_ceiling_is_256_mb_or_the_cap_times_the_count() {
        assert_eq!(unpack_ceiling(None, 64), 256 * 1024 * 1024);
        assert_eq!(unpack_ceiling(Some(8 * 1024 * 1024), 64), 256 * 1024 * 1024);
        assert_eq!(unpack_ceiling(Some(1024), 2), 2048);
        assert_eq!(unpack_ceiling(Some(u64::MAX), u64::MAX), 256 * 1024 * 1024);
    }

    #[test]
    fn a_zip_that_unpacks_past_its_ceiling_stops_there() {
        // The cap unknown, the ceiling is 256 MB: past it the entry is
        // marked where the drop passes it, and the rest is not unpacked.
        // Two entries of zeros, deflated small on disk, unpacked large:
        // some 250 MB passes through here, in a second or two.
        let big = vec![0u8; 200 * 1024 * 1024];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ceiling.zip");
        std::fs::write(
            &path,
            zip_deflated(&[
                ("big-1.bin", &big),
                ("big-2.bin", &big[..100 * 1024 * 1024]),
            ]),
        )
        .unwrap();

        let found = dropped(&path, None, 64, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].refused, None);
        assert_eq!(found[0].bytes.len(), 200 * 1024 * 1024);
        assert_eq!(
            found[1].refused.as_deref(),
            Some("big-2.bin takes the drop past 256.0 MiB: one drop unpacks no more")
        );
        assert!(found[1].bytes.is_empty());
    }

    #[test]
    fn a_zip_entry_with_a_long_name_shortens_and_attaches() {
        // No name on disk is past 255 bytes, but an entry's may be: it
        // shortens, the extension kept, and attaches under the shorter.
        let long = format!("{}.pdf", "r".repeat(300));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.zip");
        std::fs::write(&path, zip_stored(&[(long.as_str(), PDF)])).unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].refused, None);
        assert_eq!(found[0].name.len(), 255);
        assert!(found[0].name.ends_with(".pdf"), "{}", found[0].name);
        assert_eq!(found[0].bytes, PDF);
    }

    #[test]
    fn a_zip_of_nothing_unpacks_to_one_marked_row() {
        let dir = tempfile::tempdir().unwrap();
        // No entries at all.
        let path = dir.path().join("empty.zip");
        std::fs::write(&path, zip_stored(&[])).unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].refused.as_deref(),
            Some("empty.zip holds no files")
        );

        // Directories and symlinks alone: skipped silently, and the zip
        // holds no files.
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.add_directory("papers/", options).unwrap();
        let dirs = zip.finish().unwrap().into_inner();
        let path = dir.path().join("dirs.zip");
        std::fs::write(&path, dirs).unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].refused.as_deref(), Some("dirs.zip holds no files"));

        let link = RawEntry {
            name: "link.pdf",
            data: b"target.pdf",
            declared: None,
            attrs: 0o120777 << 16,
        };
        let file = RawEntry {
            name: "target.pdf",
            data: PDF,
            declared: None,
            attrs: 0o100644 << 16,
        };
        let path = dir.path().join("link.zip");
        std::fs::write(&path, raw_zip(&[link, file])).unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "target.pdf");

        // No zip by its bytes: one marked row, in words.
        let path = dir.path().join("junk.zip");
        std::fs::write(&path, b"not a zip at all").unwrap();
        let found = dropped(&path, Some(1024), 64, 0);
        assert_eq!(found.len(), 1);
        let why = found[0].refused.as_deref().unwrap();
        assert!(why.starts_with("junk.zip could not be unpacked: "), "{why}");
    }
}
