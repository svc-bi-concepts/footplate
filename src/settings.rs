//! What a person set and the window keeps between runs: the text size, the
//! look, and which yard it shows. One small JSON file where the platform has a program's own
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

use crate::theme::Look;

/// The text size the window was drawn with before there was a setting, in
/// pixels, and the ends of what can be set, a pixel a step. Below the
/// smallest the small lines (three quarters of it) are not read any more;
/// above the largest the left column leaves the conversation too little of
/// the smallest window.
pub const TEXT: u8 = 14;
pub const TEXT_MIN: u8 = 11;
pub const TEXT_MAX: u8 = 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The size of the window's text, in pixels.
    pub text: u8,
    pub look: Look,
    /// The window shows the yard at `address`, not this machine's. Both
    /// are left out of the file while they say nothing, so the file of a
    /// person who never chose is as it was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remote: bool,
    /// The address of the yard at an address that was last paired with. It
    /// is kept when this machine's yard is chosen again, to go back by.
    /// The device's token is never here: it has a file of its own
    /// (yard.rs).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub address: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            text: TEXT,
            look: Look::Auto,
            remote: false,
            address: String::new(),
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

    /// The text `by` steps larger, or smaller, within the ends.
    /// The address of the yard at an address the settings choose, if they
    /// choose one.
    pub fn chosen(&self) -> Option<&str> {
        Some(self.address.as_str()).filter(|address| self.remote && !address.is_empty())
    }

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

/// Where the settings are kept, or nowhere for a person without a home.
/// An `$XDG_CONFIG_HOME` that is no absolute path is ignored, as the XDG
/// base directory specification has it.
pub fn file_under(
    macos: bool,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    let dir = if macos {
        home.and_then(absolute)?.join("Library/Application Support")
    } else {
        xdg_config_home
            .and_then(absolute)
            .or_else(|| Some(home.and_then(absolute)?.join(".config")))?
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
    fn the_yard_that_was_chosen_is_kept_and_never_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let set = Settings {
            remote: true,
            address: "https://mac.example.net".into(),
            ..Settings::default()
        };
        assert_eq!(set.chosen(), Some("https://mac.example.net"));
        set.write(&file).unwrap();
        assert_eq!(Settings::read(&file), set);
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"text\": 14,\n  \"look\": \"auto\",\n  \"remote\": true,\n  \"address\": \"https://mac.example.net\"\n}\n"
        );
        // This machine's yard again: the address stays, to go back by.
        let near = Settings {
            remote: false,
            ..set
        };
        assert_eq!(near.chosen(), None);
        near.write(&file).unwrap();
        assert_eq!(Settings::read(&file).address, "https://mac.example.net");
        // Remote with no address chooses nothing.
        let none = Settings {
            remote: true,
            ..Settings::default()
        };
        assert_eq!(none.chosen(), None);
    }

    #[test]
    fn the_file_is_where_the_platform_has_a_program_s_configuration() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            file_under(true, os("/elsewhere"), os("/Users/jo")),
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
        assert_eq!(file_under(true, os("/elsewhere"), None), None);
    }
}
