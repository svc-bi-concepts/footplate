//! What a person set and the window keeps between runs: the text size, the
//! look, the yards it can show and which of them is open, what the system
//! tells of, and whether footplate stays up in the menu bar. One small JSON file where the platform has a program's own
//! configuration: `~/Library/Application Support/footplate/settings.json`
//! on macOS, `$XDG_CONFIG_HOME/footplate/settings.json` elsewhere
//! (`~/.config` when that is not set). It is footplate's to write, on every
//! change; a file that is missing or that it cannot read is today's look.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gpui::{Pixels, px};
use serde::{Deserialize, Serialize};

use crate::api::Address;
use crate::model::Kind;
use crate::theme::Look;
use crate::yard;

/// The text size the window was drawn with before there was a setting, in
/// pixels, and the ends of what can be set, a pixel a step. Below the
/// smallest the small lines (three quarters of it) are not read any more;
/// above the largest the left column leaves the conversation too little of
/// the smallest window.
pub const TEXT: u8 = 14;
pub const TEXT_MIN: u8 = 11;
pub const TEXT_MAX: u8 = 20;

/// What the one yard is called that a file names none for: this machine's,
/// by this run's socket.
pub const HERE: &str = "this machine";

/// The web view's base address where none is set: its default, as `yardr
/// web start` prints it. A note's pages open in the browser through it.
pub const WEB_VIEW: &str = "http://127.0.0.1:8791/";

/// A yard the window can show: its name in the switcher, and how it is
/// reached. Near, by its socket on this machine, or far, at an address, as
/// a device of its user with a token in a file of its own. What says
/// nothing is left out of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Yard {
    /// What the switcher calls it: no two of the list share one.
    pub name: String,
    /// Of a near yard: its socket's path. Empty is this run's socket, a
    /// path names another home's yard on this machine.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub socket: String,
    /// Of a far yard: its address. A yard with one is far.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub address: String,
    /// Of a far yard: the file its device's token is in, a bare name
    /// beside the settings or a whole path. The token is never here.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token_file: String,
}

impl Yard {
    /// A yard of this machine, by `socket`, or by this run's where that
    /// is empty.
    pub fn near(name: &str, socket: &str) -> Yard {
        Yard {
            name: name.to_string(),
            socket: socket.to_string(),
            ..Yard::default()
        }
    }

    /// A yard at `address`, whose device's token is in `token_file`.
    pub fn far(name: &str, address: &str, token_file: &str) -> Yard {
        Yard {
            name: name.to_string(),
            address: address.to_string(),
            token_file: token_file.to_string(),
            ..Yard::default()
        }
    }

    pub fn is_far(&self) -> bool {
        !self.address.is_empty()
    }

    /// What kind of yard it is, as the switcher and the settings say it.
    pub fn kind(&self) -> &'static str {
        match self.is_far() {
            true => "paired",
            false => "on this machine",
        }
    }
}

/// The name a far yard gets where none is typed, and the one yard of a
/// file from before there were several: its address's host.
pub fn host_of(address: &str) -> String {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host),
    };
    host.to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "Kept")]
pub struct Settings {
    /// The size of the window's text, in pixels.
    pub text: u8,
    pub look: Look,
    /// The yards the window can show, in the order the switcher lists
    /// them: never none. They are left out of the file while they are the
    /// one yard nobody chose, so the file of a person who never chose is
    /// as it was.
    #[serde(skip_serializing_if = "nobody_chose")]
    pub yards: Vec<Yard>,
    /// The name of the yard the window had open last, which the next start
    /// opens. Empty, or the name of none, is the first of the list.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub open: String,
    /// What the system tells of. Left out of the file while nothing was
    /// chosen, as the yards are.
    #[serde(skip_serializing_if = "Notify::is_usual")]
    pub notifications: Notify,
    /// Whether footplate has its item in the Mac's menu bar or a Linux
    /// desktop's tray. On until a person switches it off, and left out of
    /// the file while it is on. On a Mac it governs what closing the
    /// window does ([`Settings::closing_hides`]).
    #[serde(skip_serializing_if = "is_on")]
    pub menu_bar: bool,
    /// The web view's base address, through which a note's pages open in
    /// the browser. The view's default until one is set, and left out of
    /// the file while it is that. A yard reached by pairing opens its
    /// pages once this names the address that yard's view is reached by.
    #[serde(skip_serializing_if = "is_usual_view")]
    pub web_view: String,
}

fn is_on(on: &bool) -> bool {
    *on
}

fn is_usual_view(view: &str) -> bool {
    view == WEB_VIEW
}

fn nobody_chose(yards: &[Yard]) -> bool {
    yards == [Yard::near(HERE, "")]
}

/// A settings file as it is read: today's, and the one from before there
/// were several yards, which had `remote` and `address` for its one.
#[derive(Deserialize)]
#[serde(default)]
struct Kept {
    text: u8,
    look: Look,
    yards: Vec<Yard>,
    open: String,
    remote: bool,
    address: String,
    notifications: Notify,
    menu_bar: bool,
    web_view: String,
}

impl Default for Kept {
    fn default() -> Self {
        Kept {
            text: TEXT,
            look: Look::Auto,
            yards: Vec::new(),
            open: String::new(),
            remote: false,
            address: String::new(),
            notifications: Notify::default(),
            menu_bar: true,
            web_view: String::new(),
        }
    }
}

impl From<Kept> for Settings {
    /// A file that lists no yards is one from before the list. Where it
    /// chose the yard at its address, that is the first, with the token
    /// file a pairing wrote then, and so the one that opens; this machine's
    /// follows it, which the window offered beside it then. Else it has
    /// this machine's alone. The next write keeps it as a list.
    fn from(kept: Kept) -> Settings {
        let moved_remote = kept.yards.is_empty() && kept.remote && !kept.address.is_empty();
        let yards = match kept.yards.is_empty() {
            false => kept.yards,
            true if kept.remote && !kept.address.is_empty() => vec![
                Yard::far(&host_of(&kept.address), &kept.address, yard::TOKEN_FILE),
                Yard::near(HERE, ""),
            ],
            true => vec![Yard::near(HERE, "")],
        };
        let yards = told_apart(yards);
        // Before the list, an unmatched `open` fell back to the paired
        // yard. Keep that behavior if it happens to match the new near yard.
        let open = if moved_remote
            && yards
                .iter()
                .any(|yard| yard.name == kept.open && !yard.is_far())
        {
            String::new()
        } else {
            kept.open
        };
        Settings {
            yards,
            text: kept.text,
            look: kept.look,
            open,
            notifications: kept.notifications,
            menu_bar: kept.menu_bar,
            web_view: match kept.web_view.trim() {
                "" => WEB_VIEW.to_string(),
                _ => kept.web_view,
            },
        }
    }
}

/// The yards of a file with a name each that no other has. Where they are
/// typed the settings refuse a name that is taken or empty; a list edited
/// by hand may have both, and a yard is opened, paired and removed by its
/// name: two of one name would be the first of them whichever was meant,
/// with the other's address beside it in the list. The later one is called
/// by its name and its place among them.
fn told_apart(mut yards: Vec<Yard>) -> Vec<Yard> {
    for place in 0..yards.len() {
        let name = match yards[place].name.trim() {
            "" => "yard",
            name => name,
        };
        let taken = |name: &str| yards[..place].iter().any(|other| other.name == name);
        let mut told = name.to_string();
        let mut n = 1;
        while taken(&told) {
            n += 1;
            told = format!("{name} {n}");
        }
        yards[place].name = told;
    }
    yards
}

/// Whether the system shows a notification for what arrives while the
/// window does not show it, and for which kinds. Off until a person
/// switches it on: nothing surprises one who did not ask. The kinds are
/// kept while it is off, for the next time it is on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    pub on: bool,
    pub mail: bool,
    pub decisions: bool,
    /// The yard's notices tell of every bead that moves, so they are not
    /// among what tells until they are chosen.
    pub notices: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Notify {
            on: false,
            mail: true,
            decisions: true,
            notices: false,
        }
    }
}

impl Notify {
    fn is_usual(&self) -> bool {
        *self == Notify::default()
    }

    /// Whether `kind` is among what tells; whether anything tells is `on`.
    pub fn chosen(&self, kind: Kind) -> bool {
        match kind {
            Kind::Mail => self.mail,
            Kind::Decision => self.decisions,
            Kind::Notice => self.notices,
        }
    }

    /// Whether the system tells of news of `kind`.
    pub fn tells(&self, kind: Kind) -> bool {
        self.on && self.chosen(kind)
    }

    /// With `kind` among what tells, or not.
    pub fn with(mut self, kind: Kind, tells: bool) -> Notify {
        match kind {
            Kind::Mail => self.mail = tells,
            Kind::Decision => self.decisions = tells,
            Kind::Notice => self.notices = tells,
        }
        self
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            text: TEXT,
            look: Look::Auto,
            yards: vec![Yard::near(HERE, "")],
            open: String::new(),
            notifications: Notify::default(),
            menu_bar: true,
            web_view: WEB_VIEW.to_string(),
        }
    }
}

impl Settings {
    /// What `file` keeps, or today's look where there is no file or it does
    /// not say: nothing here is worth a word to the person at the start.
    pub fn read(file: &Path) -> Settings {
        let kept = fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default();
        Settings {
            text: kept.text.clamp(TEXT_MIN, TEXT_MAX),
            ..kept
        }
    }

    /// Keeps them in `file`, whose directory is made when it is not there.
    pub fn write(&self, file: &Path) -> io::Result<()> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        text.push('\n');
        fs::write(file, text)
    }

    /// Whether closing the window hides footplate, which then goes on
    /// following its yards, where it would else quit: on a Mac (`mac`),
    /// while the menu bar item is on. The item is the way back that is
    /// seen, so without it nothing is ever hidden: a person who switched
    /// it off, or whose system has none, quits by closing.
    pub fn closing_hides(&self, mac: bool) -> bool {
        mac && self.menu_bar
    }

    /// The yard the window opens: the one it had open last, else the first
    /// of the list.
    pub fn opened(&self) -> &Yard {
        let named = self.yards.iter().find(|yard| yard.name == self.open);
        named
            .or(self.yards.first())
            .expect("the settings always list a yard")
    }

    /// The yard called `name`, if the list has one.
    pub fn yard(&self, name: &str) -> Option<&Yard> {
        self.yards.iter().find(|yard| yard.name == name)
    }

    /// With the yard called `name` the one to open.
    pub fn opening(self, name: &str) -> Settings {
        Settings {
            open: name.to_string(),
            ..self
        }
    }

    /// The far yard other than `name` whose token is in the same file as
    /// that one's, and the file: only a list edited by hand has such a
    /// pair, and neither of them is opened or paired until their files
    /// differ. `file` is where the settings are kept.
    pub fn shares(&self, name: &str, file: Option<&Path>) -> Option<(&Yard, PathBuf)> {
        let at = yard::token_path(
            &self.yard(name).filter(|yard| yard.is_far())?.token_file,
            file,
        )
        .ok()?;
        let others = self.yards.iter().filter(|other| other.is_far());
        let other = others.filter(|other| other.name != name).find(|other| {
            yard::token_path(&other.token_file, file).is_ok_and(|path| yard::same_file(&path, &at))
        })?;
        Some((other, at))
    }

    /// With `yard` in place of the one called `at`, or after the last of
    /// the list: a yard added, renamed or edited. What cannot stand in the
    /// list is refused with its reason before anything is paired or
    /// written: a name another yard has, an address or a token file that
    /// is none, and a token file another yard's token is in. A yard that is
    /// renamed keeps its token file, and stays the open one if it was.
    pub fn with_yard(
        mut self,
        at: Option<&str>,
        yard: Yard,
        file: Option<&Path>,
    ) -> Result<Settings, String> {
        let name = yard.name.trim().to_string();
        if name.is_empty() {
            return Err("A yard wants a name, to tell it from the others.".to_string());
        }
        let others = || {
            self.yards
                .iter()
                .filter(|other| Some(other.name.as_str()) != at)
        };
        if others().any(|other| other.name == name) {
            return Err(format!(
                "There is a yard called {name} already, and two may not share a name. Give this one another."
            ));
        }
        let yard = match yard.is_far() {
            true => {
                Address::parse(&yard.address)?;
                let path = yard::token_path(&yard.token_file, file)?;
                let taken = others().filter(|other| other.is_far()).find(|other| {
                    yard::token_path(&other.token_file, file)
                        .is_ok_and(|theirs| yard::same_file(&theirs, &path))
                });
                if let Some(other) = taken {
                    return Err(format!(
                        "The token file {} is the yard {}'s, and two yards may not share one: whoever holds a token calls its yard as you. Name another file.",
                        path.display(),
                        other.name
                    ));
                }
                Yard::far(&name, yard.address.trim(), yard.token_file.trim())
            }
            false => {
                let socket = yard.socket.trim();
                if !socket.is_empty() && !Path::new(socket).is_absolute() {
                    return Err(format!(
                        "{socket:?} is no path of a socket: give it whole, from /, or leave it empty for this machine's own yard."
                    ));
                }
                Yard::near(&name, socket)
            }
        };
        match at.and_then(|at| self.yards.iter().position(|other| other.name == at)) {
            Some(place) => {
                if self.open == self.yards[place].name {
                    self.open = name;
                }
                self.yards[place] = yard;
            }
            None => self.yards.push(yard),
        }
        Ok(self)
    }

    /// With the web view's base address `address`: what opens a note's
    /// pages in the browser. What is no address is refused with why, and
    /// nothing changed; empty is the view's default again.
    pub fn with_web_view(mut self, address: &str) -> Result<Settings, String> {
        let address = address.trim();
        if address.is_empty() {
            self.web_view = WEB_VIEW.to_string();
            return Ok(self);
        }
        Address::parse_view(address)?;
        self.web_view = address.to_string();
        Ok(self)
    }

    /// The token file a new far yard called `name` gets: `yard-<name>.token`
    /// beside the settings, the name in small letters with `-` for what is
    /// neither a letter nor a digit, and with `-2`, `-3` and so on while
    /// another yard's token is in a file of that name or such a file is
    /// there already.
    pub fn token_name(&self, name: &str, file: Option<&Path>) -> String {
        let mut slug = String::new();
        for c in name.trim().chars().flat_map(char::to_lowercase) {
            match c.is_alphanumeric() {
                true => slug.push(c),
                false if !slug.ends_with('-') => slug.push('-'),
                false => {}
            }
        }
        let slug = match slug.trim_matches('-') {
            "" => "yard",
            slug => slug,
        };
        let taken = |token_file: &str| {
            let path = yard::token_path(token_file, file).ok();
            let theirs = self
                .yards
                .iter()
                .filter(|other| other.is_far())
                .any(|other| {
                    other.token_file == token_file
                        || yard::token_path(&other.token_file, file)
                            .ok()
                            .zip(path.as_ref())
                            .is_some_and(|(theirs, path)| yard::same_file(&theirs, path))
                });
            theirs || path.is_some_and(|path| path.exists())
        };
        let mut token_file = format!("yard-{slug}.token");
        let mut n = 1;
        while taken(&token_file) {
            n += 1;
            token_file = format!("yard-{slug}-{n}.token");
        }
        token_file
    }

    /// Without the yard called `name`, and what became of its token's
    /// file. That is deleted only where it is a bare name beside the
    /// settings, footplate wrote it for a pairing, and no other yard's
    /// token is in it; other files are left. A list is never empty: the
    /// last yard removed leaves this machine's.
    pub fn without(mut self, name: &str, file: Option<&Path>) -> (Settings, TokenLeft) {
        let Some(yard) = self.yard(name).cloned() else {
            return (self, TokenLeft::None);
        };
        let left = match yard::token_path(&yard.token_file, file) {
            _ if !yard.is_far() => TokenLeft::None,
            Err(_) => TokenLeft::None,
            Ok(path) => match self.shares(name, file) {
                Some((other, _)) => TokenLeft::Shared(path, other.name.clone()),
                None if yard::is_bare(&yard.token_file) => match fs::symlink_metadata(&path) {
                    Err(err) if err.kind() == io::ErrorKind::NotFound => TokenLeft::Deleted(path),
                    Err(err) => TokenLeft::DeleteFailed(path, err.to_string()),
                    Ok(meta)
                        if meta.file_type().is_file() && yard::is_managed_token_file(&path) =>
                    {
                        match fs::remove_file(&path) {
                            Ok(()) => TokenLeft::Deleted(path),
                            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                                TokenLeft::Deleted(path)
                            }
                            Err(err) => TokenLeft::DeleteFailed(path, err.to_string()),
                        }
                    }
                    Ok(_) => TokenLeft::Unmanaged(path),
                },
                None => TokenLeft::Elsewhere(path),
            },
        };
        self.yards.retain(|other| other.name != name);
        if self.yards.is_empty() {
            self.yards.push(Yard::near(HERE, ""));
        }
        if self.open == name {
            self.open.clear();
        }
        (self, left)
    }

    /// The text `by` steps larger, or smaller, within the ends.
    pub fn sized(self, by: i8) -> Settings {
        let text = self.text.saturating_add_signed(by);
        Settings {
            text: text.clamp(TEXT_MIN, TEXT_MAX),
            ..self
        }
    }

    /// The window's rem for this text size. Every size of the window that
    /// goes with its text is in rems, the text itself too (seven eighths of
    /// one), so this one number scales all of it; at the usual size it is
    /// gpui's own 16 pixels.
    pub fn rem(&self) -> Pixels {
        px(16. * f32::from(self.text) / f32::from(TEXT))
    }
}

/// What became of the token's file of a yard that was removed.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenLeft {
    /// It had none: a yard of this machine, or one whose file was no file.
    None,
    Deleted(PathBuf),
    /// It was a bare name, but the file could not be removed.
    DeleteFailed(PathBuf, String),
    /// It was a bare name, but footplate did not write this file.
    Unmanaged(PathBuf),
    /// Named by a whole path, and left where the person keeps it.
    Elsewhere(PathBuf),
    /// Left: the yard of this name keeps its token in it.
    Shared(PathBuf, String),
}

impl TokenLeft {
    /// What the settings say of a yard called `name` that was removed, in
    /// a line, with the command that removes the device at the yard:
    /// removing it here does not. `device` is its user and its name there,
    /// where the yard said them.
    pub fn said(&self, name: &str, device: Option<(&str, &str)>) -> String {
        let (user, device) = device.unwrap_or(("<user>", "<device>"));
        let stays = format!(
            "The device itself stays at the yard until its operator removes it: yardr user device rm {user} {device}"
        );
        match self {
            TokenLeft::None => format!("{name} is removed from the list. Its yard is as it was."),
            TokenLeft::Deleted(path) => format!(
                "{name} is removed, and its token's file {} with it. {stays}",
                path.display()
            ),
            TokenLeft::DeleteFailed(path, why) => format!(
                "{name} is removed from the list, but its token's file {} could not be removed ({why}). {stays}",
                path.display()
            ),
            TokenLeft::Unmanaged(path) => format!(
                "{name} is removed. Its file {} is left alone: footplate did not write it as a token file. {stays}",
                path.display()
            ),
            TokenLeft::Elsewhere(path) => format!(
                "{name} is removed. Its token's file {} is not beside the settings, and is left alone. {stays}",
                path.display()
            ),
            TokenLeft::Shared(path, other) => format!(
                "{name} is removed. Its token's file {} is left: the yard {other} keeps its token in it. {stays}",
                path.display()
            ),
        }
    }
}

/// Where the settings are kept, or nowhere for a person without a home.
/// An absolute `$XDG_CONFIG_HOME` wins on either platform; a relative one
/// is ignored, as the XDG base directory specification has it.
pub fn file_under(
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
    Some(dir.join(crate::NAME).join("settings.json"))
}

/// Where this run keeps them.
pub fn file() -> Option<PathBuf> {
    file_under(
        cfg!(target_os = "macos"),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        // The directory is not there before the first change.
        let file = dir.path().join("footplate/settings.json");
        let set = Settings {
            text: 17,
            look: Look::Dark,
            ..Settings::default()
        };
        set.write(&file).unwrap();
        assert_eq!(Settings::read(&file), set);
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"text\": 17,\n  \"look\": \"dark\"\n}\n"
        );
    }

    #[test]
    fn a_missing_or_damaged_file_is_today_s_look() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let today = Settings {
            text: 14,
            look: Look::Auto,
            ..Settings::default()
        };
        assert_eq!(Settings::read(&file), today);
        for damaged in [
            "",
            "{\"text\": 17, \"look\": \"da",
            "[]",
            "{\"text\": \"large\"}",
            "{\"text\": 17, \"look\": \"sepia\"}",
            "{\"text\": 1000}",
        ] {
            fs::write(&file, damaged).unwrap();
            assert_eq!(Settings::read(&file), today, "{damaged}");
        }
        fs::write(&file, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(Settings::read(&file), today);
    }

    #[test]
    fn a_file_with_one_value_keeps_it_and_a_size_out_of_range_is_the_nearest_end() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(&file, "{\"look\": \"light\"}").unwrap();
        assert_eq!(
            Settings::read(&file),
            Settings {
                text: 14,
                look: Look::Light,
                ..Settings::default()
            }
        );
        fs::write(&file, "{\"text\": 3, \"later\": true}").unwrap();
        assert_eq!(Settings::read(&file).text, TEXT_MIN);
        fs::write(&file, "{\"text\": 200}").unwrap();
        assert_eq!(Settings::read(&file).text, TEXT_MAX);
    }

    #[test]
    fn the_size_steps_by_a_pixel_and_stops_at_the_ends() {
        let set = Settings::default();
        assert_eq!(set.clone().sized(1).text, 15);
        assert_eq!(set.clone().sized(-1).text, 13);
        assert_eq!(set.clone().sized(1).look, set.look);
        let mut large = set.clone();
        let mut small = set.clone();
        for _ in 0..40 {
            large = large.sized(1);
            small = small.sized(-1);
        }
        assert_eq!((small.text, large.text), (TEXT_MIN, TEXT_MAX));
        // The usual size is gpui's own rem, so nothing moved for those who
        // set nothing.
        assert_eq!(set.rem(), px(16.));
        assert!(small.rem() < set.rem() && set.rem() < large.rem());
    }

    #[test]
    fn the_yards_are_kept_in_their_order_with_the_open_one_and_never_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let set = Settings::default()
            .with_yard(
                None,
                Yard::far(
                    "plantime",
                    "https://plantime.example.net",
                    "yard-plantime.token",
                ),
                Some(&file),
            )
            .unwrap()
            .with_yard(
                None,
                Yard::near("shop", "/home/shop/.yardr/yard.sock"),
                None,
            )
            .unwrap()
            .opening("plantime");
        assert_eq!(set.opened().name, "plantime");
        assert_eq!(set.opened().kind(), "paired");
        assert_eq!(set.yards[0].kind(), "on this machine");
        set.write(&file).unwrap();
        assert_eq!(Settings::read(&file), set);
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            r#"{
  "text": 14,
  "look": "auto",
  "yards": [
    {
      "name": "this machine"
    },
    {
      "name": "plantime",
      "address": "https://plantime.example.net",
      "token_file": "yard-plantime.token"
    },
    {
      "name": "shop",
      "socket": "/home/shop/.yardr/yard.sock"
    }
  ],
  "open": "plantime"
}
"#
        );
        // The yard that was open last is opened, else the first: a name
        // of none, as of a yard removed by hand, is the first too.
        assert_eq!(set.clone().opening("").opened().name, HERE);
        assert_eq!(set.clone().opening("gone").opened().name, HERE);
        // A file that lists none has this machine's.
        fs::write(&file, r#"{"yards": []}"#).unwrap();
        assert_eq!(Settings::read(&file), Settings::default());
    }

    #[test]
    fn a_file_of_one_yard_from_before_there_were_several_opens_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        // Paired with the yard at an address: that far yard first, called
        // by the address's host, with the token file the pairing wrote
        // then, and this machine's after it, which the window offered
        // beside it before there was a list.
        fs::write(
            &file,
            "{\n  \"text\": 15,\n  \"look\": \"dark\",\n  \"remote\": true,\n  \"address\": \"https://mac.example.net:8443/yard\"\n}\n",
        )
        .unwrap();
        let set = Settings::read(&file);
        assert_eq!(
            set.yards,
            [
                Yard::far(
                    "mac.example.net",
                    "https://mac.example.net:8443/yard",
                    "yard.token"
                ),
                Yard::near(HERE, "")
            ]
        );
        assert_eq!(
            [set.yards[0].kind(), set.yards[1].kind()],
            ["paired", "on this machine"]
        );
        assert_eq!((set.text, set.look), (15, Look::Dark));
        // The paired one opens, as it did.
        assert_eq!(set.opened().name, "mac.example.net");
        assert_eq!(set.opened().kind(), "paired");
        assert_eq!(
            yard::token_path(&set.opened().token_file, Some(&file)).unwrap(),
            dir.path().join("yard.token")
        );

        // An old unmatched open name also fell back to the only yard. Once
        // the near yard is added, it must not start matching that name.
        fs::write(
            &file,
            "{\"remote\": true, \"address\": \"https://mac.example.net:8443/yard\", \"open\": \"this machine\"}",
        )
        .unwrap();
        let moved = Settings::read(&file);
        assert_eq!(moved.opened().name, "mac.example.net");

        // The next write keeps it as a list, and reads back the same.
        set.write(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("remote") && text.contains("\"yards\""),
            "{text}"
        );
        assert_eq!(Settings::read(&file), set);

        // An existing list is respected even if legacy fields remain: a
        // person who removed this machine's yard does not get it back.
        fs::write(
            &file,
            r#"{"remote": true, "address": "https://mac.example.net:8443/yard", "yards": [{"name": "mac.example.net", "address": "https://mac.example.net:8443/yard", "token_file": "yard.token"}]}"#,
        )
        .unwrap();
        assert_eq!(
            Settings::read(&file).yards,
            [Yard::far(
                "mac.example.net",
                "https://mac.example.net:8443/yard",
                "yard.token"
            )]
        );

        // This machine's yard was chosen, or nothing ever was: the one
        // yard is this machine's, and the file says nothing of it.
        for near in [
            "{\"text\": 15}",
            "{\"remote\": false, \"address\": \"https://mac.example.net\"}",
            "{\"remote\": true}",
        ] {
            fs::write(&file, near).unwrap();
            let set = Settings::read(&file);
            assert_eq!(set.yards, [Yard::near(HERE, "")], "{near}");
            assert!(!set.opened().is_far());
            set.write(&file).unwrap();
            assert!(!fs::read_to_string(&file).unwrap().contains("yards"));
        }
        assert_eq!(host_of("http://[::1]:8790"), "::1");
        assert_eq!(host_of("http://127.0.0.1:8790/"), "127.0.0.1");
        assert_eq!(host_of("https://mac.example.net"), "mac.example.net");
    }

    #[test]
    fn yards_of_one_name_or_of_none_in_a_file_are_told_apart_when_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        // Only a list edited by hand comes to this.
        fs::write(
            &file,
            r#"{"yards": [{"name": "mac", "address": "https://mac.example.net", "token_file": "yard.token"},
                          {"name": " mac ", "address": "https://twin.example.net", "token_file": "yard.token"},
                          {"name": "mac", "socket": "/home/mac/.yardr/yard.sock"},
                          {"name": "mac 2"},
                          {"address": "https://shop.example.net", "token_file": "shop.token"},
                          {"name": "  "}],
                "open": "mac"}"#,
        )
        .unwrap();
        let set = Settings::read(&file);
        let names: Vec<&str> = set.yards.iter().map(|yard| yard.name.as_str()).collect();
        assert_eq!(
            names,
            ["mac", "mac 2", "mac 3", "mac 2 2", "yard", "yard 2"]
        );
        // Each is itself by its name: the one the switcher's row opens,
        // and the one whose address stands beside it.
        for yard in &set.yards {
            assert_eq!(set.clone().opening(&yard.name).opened(), yard);
            assert_eq!(set.yard(&yard.name), Some(yard));
        }
        assert_eq!(set.opened().address, "https://mac.example.net");
        // Two of them on one token file are found from either, as they
        // are refused to be opened and paired.
        assert_eq!(set.shares("mac", Some(&file)).unwrap().0.name, "mac 2");
        assert_eq!(set.shares("mac 2", Some(&file)).unwrap().0.name, "mac");
        // Removing one leaves the other, and the file both name.
        fs::write(dir.path().join("yard.token"), "yardr-dev-kept\n").unwrap();
        let (less, left) = set.clone().without("mac 2", Some(&file));
        assert_eq!(
            left,
            TokenLeft::Shared(dir.path().join("yard.token"), "mac".into())
        );
        assert_eq!(less.yard("mac"), set.yard("mac"));
        assert!(dir.path().join("yard.token").exists());
    }

    fn two_far(file: &Path) -> Settings {
        Settings::default()
            .with_yard(
                None,
                Yard::far("mac", "https://mac.example.net", "yard.token"),
                Some(file),
            )
            .unwrap()
            .with_yard(
                None,
                Yard::far(
                    "plantime",
                    "https://plantime.example.net",
                    "yard-plantime.token",
                ),
                Some(file),
            )
            .unwrap()
    }

    #[test]
    fn no_two_yards_share_a_name_or_a_token_file_by_adding_renaming_or_editing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let set = two_far(&file);
        let refused = |at: Option<&str>, yard: Yard| {
            set.clone().with_yard(at, yard, Some(&file)).unwrap_err()
        };

        // Adding: a name that is taken, no name, an address that is none.
        let why = refused(None, Yard::near("mac", ""));
        assert!(
            why.starts_with("There is a yard called mac already"),
            "{why}"
        );
        let why = refused(None, Yard::near("  ", ""));
        assert!(why.starts_with("A yard wants a name"), "{why}");
        let why = refused(
            None,
            Yard::far("shop", "http://shop.example.net", "shop.token"),
        );
        assert!(why.contains("plain http"), "{why}");
        let why = refused(None, Yard::near("shop", "run/yard.sock"));
        assert!(why.contains("is no path of a socket"), "{why}");

        // Adding with the token file of another: by its bare name, and by
        // the whole path of the same file.
        let shared = |why: String, of: &str| {
            assert!(
                why.starts_with(&format!(
                    "The token file {} is the yard {of}'s",
                    dir.path()
                        .join(format!(
                            "{}.token",
                            match of {
                                "mac" => "yard",
                                _ => "yard-plantime",
                            }
                        ))
                        .display()
                )),
                "{why}"
            );
        };
        shared(
            refused(
                None,
                Yard::far("shop", "https://shop.example.net", "yard.token"),
            ),
            "mac",
        );
        let whole = dir.path().join("yard-plantime.token");
        shared(
            refused(
                None,
                Yard::far("shop", "https://shop.example.net", whole.to_str().unwrap()),
            ),
            "plantime",
        );
        // A token file that is neither a bare name nor a whole path.
        for none in ["", "tokens/shop.token", "../shop.token", "..", "/"] {
            let why = refused(None, Yard::far("shop", "https://shop.example.net", none));
            assert!(why.contains("names no token file"), "{none:?}: {why}");
        }

        // Editing a yard onto another's file is refused the same; onto
        // its own, as with another address, it is taken.
        shared(
            refused(
                Some("plantime"),
                Yard::far("plantime", "https://plantime.example.net", "yard.token"),
            ),
            "mac",
        );
        let moved = set
            .clone()
            .with_yard(
                Some("plantime"),
                Yard::far("plantime", "https://new.example.net", "yard-plantime.token"),
                Some(&file),
            )
            .unwrap();
        assert_eq!(moved.yards[2].address, "https://new.example.net");
        assert_eq!(moved.yards.len(), 3);

        // Renaming keeps the token file, so it cannot come to another's;
        // only the name may not be one that is taken. The open yard stays
        // the open one under its new name, in its place in the list.
        let why = refused(
            Some("plantime"),
            Yard {
                name: "mac".into(),
                ..set.yards[2].clone()
            },
        );
        assert!(
            why.starts_with("There is a yard called mac already"),
            "{why}"
        );
        let renamed = set
            .clone()
            .opening("mac")
            .with_yard(
                Some("mac"),
                Yard {
                    name: "plantime 2".into(),
                    ..set.yards[1].clone()
                },
                Some(&file),
            )
            .unwrap();
        assert_eq!(
            renamed.yards[1],
            Yard::far("plantime 2", "https://mac.example.net", "yard.token")
        );
        assert_eq!(renamed.opened().name, "plantime 2");
        assert_eq!(renamed.shares("plantime 2", Some(&file)), None);

        // A list edited by hand may still have two on one file: it is
        // found, from either of them.
        let mut edited = set.clone();
        edited.yards[2].token_file = "yard.token".into();
        let (other, at) = edited.shares("plantime", Some(&file)).unwrap();
        assert_eq!(
            (other.name.as_str(), at),
            ("mac", dir.path().join("yard.token"))
        );
        assert_eq!(
            edited.shares("mac", Some(&file)).unwrap().0.name,
            "plantime"
        );
        assert_eq!(edited.shares(HERE, Some(&file)), None);
    }

    #[test]
    fn a_new_yard_s_token_file_is_named_for_it_and_skips_what_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let set = two_far(&file);
        assert_eq!(set.token_name("Shop", Some(&file)), "yard-shop.token");
        assert_eq!(
            set.token_name("  The Shop, north (2) ", Some(&file)),
            "yard-the-shop-north-2.token"
        );
        assert_eq!(set.token_name("...", Some(&file)), "yard-yard.token");
        // Another yard's token is in a file of that name.
        assert_eq!(
            set.token_name("Plantime", Some(&file)),
            "yard-plantime-2.token"
        );
        // A file of that name is there already, whoever's it is.
        fs::write(dir.path().join("yard-plantime-2.token"), "").unwrap();
        fs::write(dir.path().join("yard-shop.token"), "").unwrap();
        assert_eq!(
            set.token_name("plantime", Some(&file)),
            "yard-plantime-3.token"
        );
        assert_eq!(set.token_name("shop", Some(&file)), "yard-shop-2.token");
        // What it names is taken as a new yard's.
        let name = set.token_name("plantime", Some(&file));
        let more = set
            .clone()
            .with_yard(
                None,
                Yard::far("plantime north", "https://north.example.net", &name),
                Some(&file),
            )
            .unwrap();
        assert_eq!(
            more.token_name("plantime", Some(&file)),
            "yard-plantime-4.token"
        );
    }

    #[test]
    fn removing_a_yard_deletes_its_token_file_only_where_it_is_footplate_s_alone() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let own = other.path().join("shop.token");
        let set = two_far(&file)
            .with_yard(
                None,
                Yard::far("shop", "https://shop.example.net", own.to_str().unwrap()),
                Some(&file),
            )
            .unwrap()
            .opening("plantime");
        let beside = |name: &str| dir.path().join(name);
        for token in [
            beside("yard.token"),
            beside("yard-plantime.token"),
            own.clone(),
        ] {
            crate::yard::keep_token(&token, "yardr-dev-kept").unwrap();
        }

        // A bare name beside the settings, and no other yard's: deleted.
        // The open yard removed leaves the first to open.
        let (less, left) = set.clone().without("plantime", Some(&file));
        assert_eq!(left, TokenLeft::Deleted(beside("yard-plantime.token")));
        assert!(!beside("yard-plantime.token").exists() && beside("yard.token").exists());
        assert_eq!(less.yard("plantime"), None);
        assert_eq!((less.yards.len(), less.opened().name.as_str()), (3, HERE));
        let said = left.said("plantime", Some(("jo", "laptop")));
        assert!(
            said.starts_with("plantime is removed, and its token's file"),
            "{said}"
        );
        assert!(said.ends_with("yardr user device rm jo laptop"), "{said}");

        // A whole path is the person's own file: left, and said so.
        let (less, left) = less.without("shop", Some(&file));
        assert_eq!(left, TokenLeft::Elsewhere(own.clone()));
        assert!(own.exists());
        let said = left.said("shop", None);
        assert!(said.contains("is left alone"), "{said}");
        assert!(
            said.ends_with("yardr user device rm <user> <device>"),
            "{said}"
        );

        // A file another yard's token is in, as in a list edited by hand:
        // left, and the settings say which yard keeps it.
        let mut edited = less.clone();
        edited
            .yards
            .push(Yard::far("twin", "https://twin.example.net", "yard.token"));
        let (edited, left) = edited.without("twin", Some(&file));
        assert_eq!(left, TokenLeft::Shared(beside("yard.token"), "mac".into()));
        assert!(beside("yard.token").exists());
        assert!(
            left.said("twin", None)
                .contains("the yard mac keeps its token in it")
        );
        assert_eq!(edited, less);

        // A yard of this machine has no token, and one that is not listed
        // changes nothing.
        let (less, left) = less.without(HERE, Some(&file));
        assert_eq!(left, TokenLeft::None);
        assert_eq!(
            left.said(HERE, None),
            "this machine is removed from the list. Its yard is as it was."
        );
        assert_eq!(
            less.clone().without("nobody", Some(&file)),
            (less.clone(), TokenLeft::None)
        );
        // The last one removed leaves this machine's: a list is never
        // empty.
        let (none, left) = less.without("mac", Some(&file));
        assert_eq!(left, TokenLeft::Deleted(beside("yard.token")));
        assert_eq!(none.yards, [Yard::near(HERE, "")]);

        // A path that cannot be removed is reported honestly; the settings
        // must not say it deleted a token file that is still there.
        let blocked = beside("yard-blocked.token");
        fs::create_dir(&blocked).unwrap();
        let blocked_set = Settings::default()
            .with_yard(
                None,
                Yard::far(
                    "blocked",
                    "https://blocked.example.net",
                    "yard-blocked.token",
                ),
                Some(&file),
            )
            .unwrap();
        let (_, left) = blocked_set.without("blocked", Some(&file));
        assert_eq!(left, TokenLeft::Unmanaged(blocked.clone()));
        assert!(left.said("blocked", None).contains("left alone"));
        assert!(blocked.is_dir());
    }

    #[test]
    fn removing_a_yard_leaves_a_bare_file_it_did_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let token = dir.path().join("device.token");
        // A valid token on one line may have been kept by another program.
        fs::write(&token, format!("{}\n", crate::fake::TOKEN)).unwrap();
        let set = Settings::default()
            .with_yard(
                None,
                Yard::far("external", "https://external.example.net", "device.token"),
                Some(&file),
            )
            .unwrap();

        let (less, left) = set.without("external", Some(&file));
        assert_eq!(left, TokenLeft::Unmanaged(token.clone()));
        assert_eq!(
            fs::read_to_string(token).unwrap(),
            format!("{}\n", crate::fake::TOKEN)
        );
        assert!(less.yard("external").is_none());
    }

    #[test]
    fn removing_an_unpaired_symlink_leaves_it_and_its_target_alone() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let target = dir.path().join("external.token");
        let link = dir.path().join("yard-link.token");
        fs::write(&target, format!("{}\n", crate::fake::TOKEN)).unwrap();
        symlink(&target, &link).unwrap();
        let set = Settings::default()
            .with_yard(
                None,
                Yard::far("linked", "https://linked.example.net", "yard-link.token"),
                Some(&file),
            )
            .unwrap();

        let (_, left) = set.without("linked", Some(&file));
        assert_eq!(left, TokenLeft::Unmanaged(link.clone()));
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            format!("{}\n", crate::fake::TOKEN)
        );
    }

    #[test]
    fn notifications_are_off_until_they_are_chosen_and_the_choice_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        // Nobody chose: nothing tells, whatever the kind, and the file of
        // a person who changes something else says nothing of it.
        let usual = Settings::default();
        for kind in [Kind::Mail, Kind::Decision, Kind::Notice] {
            assert!(!usual.notifications.tells(kind), "{kind:?}");
        }
        fs::write(&file, "{\"text\": 15}").unwrap();
        assert_eq!(Settings::read(&file).notifications, Notify::default());
        usual.write(&file).unwrap();
        assert!(!fs::read_to_string(&file).unwrap().contains("notifications"));

        // Switched on, mail and decisions tell, and the yard's notices
        // only once they are chosen.
        let on = Notify {
            on: true,
            ..Notify::default()
        };
        assert!(on.tells(Kind::Mail) && on.tells(Kind::Decision) && !on.tells(Kind::Notice));
        let chosen = on.with(Kind::Notice, true).with(Kind::Mail, false);
        assert!(!chosen.tells(Kind::Mail) && chosen.tells(Kind::Notice));
        let set = Settings {
            notifications: chosen,
            ..Settings::default()
        };
        set.write(&file).unwrap();
        assert_eq!(Settings::read(&file), set);
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"text\": 14,\n  \"look\": \"auto\",\n  \"notifications\": {\n    \"on\": true,\n    \"mail\": false,\n    \"decisions\": true,\n    \"notices\": true\n  }\n}\n"
        );
        // Off again, the kinds stay for the next time, and nothing tells.
        let off = Notify {
            on: false,
            ..chosen
        };
        assert!(!off.tells(Kind::Notice) && off.chosen(Kind::Notice));
        // A file that names one of them keeps the others as they are usual.
        fs::write(&file, "{\"notifications\": {\"on\": true}}").unwrap();
        assert_eq!(Settings::read(&file).notifications, on);
    }

    #[test]
    fn the_menu_bar_item_is_on_until_it_is_switched_off_and_closing_hides_with_it_on_a_mac() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        // Nobody chose: it is on, a file from before it says nothing of it
        // and has it on, and the file of a person who changes something
        // else still says nothing.
        let usual = Settings::default();
        assert!(usual.menu_bar);
        fs::write(&file, "{\"text\": 15}").unwrap();
        assert!(Settings::read(&file).menu_bar);
        usual.write(&file).unwrap();
        assert!(!fs::read_to_string(&file).unwrap().contains("menu_bar"));
        // Closing hides on a Mac alone, and only while the item is on.
        assert!(usual.closing_hides(true));
        assert!(!usual.closing_hides(false));

        // Switched off, it is kept, and closing quits wherever it is.
        let off = Settings {
            menu_bar: false,
            ..Settings::default()
        };
        off.write(&file).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"text\": 14,\n  \"look\": \"auto\",\n  \"menu_bar\": false\n}\n"
        );
        assert_eq!(Settings::read(&file), off);
        assert!(!off.closing_hides(true));
        assert!(!off.closing_hides(false));
    }

    #[test]
    fn the_web_view_s_address_is_its_default_until_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        // Nobody chose: the view's default, and a file from before it says
        // nothing of it, as a file that keeps something else still says
        // nothing.
        let usual = Settings::default();
        assert_eq!(usual.web_view, WEB_VIEW);
        fs::write(&file, "{\"text\": 15}").unwrap();
        assert_eq!(Settings::read(&file).web_view, WEB_VIEW);
        usual.write(&file).unwrap();
        assert!(!fs::read_to_string(&file).unwrap().contains("web_view"));
        // An empty one is the default again.
        fs::write(&file, "{\"web_view\": \"\"}").unwrap();
        assert_eq!(Settings::read(&file).web_view, WEB_VIEW);
        // Set, it is kept and read back.
        let set = usual
            .with_web_view("https://plantime.example.net:8443/yard/")
            .unwrap();
        set.write(&file).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"text\": 14,\n  \"look\": \"auto\",\n  \"web_view\": \"https://plantime.example.net:8443/yard/\"\n}\n"
        );
        assert_eq!(Settings::read(&file), set);
        // What is no address is refused with why, in the view's words.
        let why = Settings::default().with_web_view("plantime").unwrap_err();
        assert_eq!(
            why,
            "\"plantime\" is no address of the web view: one starts with https://, or with http:// for the web view on this machine"
        );
        let why = Settings::default()
            .with_web_view("http://plantime:8791")
            .unwrap_err();
        assert!(
            why.starts_with(
                "http://plantime:8791 is refused: plain http is for the web view on this machine alone"
            ),
            "{why}"
        );
    }

    #[test]
    fn the_file_is_where_the_platform_has_a_program_s_configuration() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            file_under(true, os("/elsewhere"), os("/Users/jo")),
            Some(PathBuf::from("/elsewhere/footplate/settings.json"))
        );
        assert_eq!(
            file_under(true, None, os("/Users/jo")),
            Some(PathBuf::from(
                "/Users/jo/Library/Application Support/footplate/settings.json"
            ))
        );
        assert_eq!(
            file_under(true, os("relative"), os("/Users/jo")),
            Some(PathBuf::from(
                "/Users/jo/Library/Application Support/footplate/settings.json"
            ))
        );
        assert_eq!(
            file_under(false, os("/home/jo/.cfg"), os("/home/jo")),
            Some(PathBuf::from("/home/jo/.cfg/footplate/settings.json"))
        );
        assert_eq!(
            file_under(false, os("relative"), os("/home/jo")),
            Some(PathBuf::from("/home/jo/.config/footplate/settings.json"))
        );
        assert_eq!(
            file_under(false, None, os("/home/jo")),
            Some(PathBuf::from("/home/jo/.config/footplate/settings.json"))
        );
        assert_eq!(file_under(false, None, None), None);
        assert_eq!(
            file_under(true, os("/elsewhere"), None),
            Some(PathBuf::from("/elsewhere/footplate/settings.json"))
        );
    }
}
