//! The window's colours, for a light and a dark desktop. They say nothing
//! the words do not.

use gpui::{App, Global, Rgba, Window, WindowAppearance, rgb, rgba};
use serde::{Deserialize, Serialize};

/// Which of the two a person wants: the desktop's, as it is now and as it
/// changes, or one of them whatever the desktop has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Look {
    #[default]
    Auto,
    Light,
    Dark,
}

/// The window's look is the application's: the line and a message's text
/// draw themselves and ask for their colours where they do.
impl Global for Look {}

impl Look {
    pub const ALL: [Look; 3] = [Look::Auto, Look::Light, Look::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Look::Auto => "auto",
            Look::Light => "light",
            Look::Dark => "dark",
        }
    }

    /// The one after this, around the end: what the one key steps through.
    pub fn next(self) -> Look {
        match self {
            Look::Auto => Look::Light,
            Look::Light => Look::Dark,
            Look::Dark => Look::Auto,
        }
    }

    /// Whether the window is dark on a desktop that is `desktop`.
    pub fn dark(self, desktop: WindowAppearance) -> bool {
        match self {
            Look::Dark => true,
            Look::Light => false,
            Look::Auto => matches!(
                desktop,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The conversation's ground, and the list's.
    pub ground: Rgba,
    pub side: Rgba,
    pub line: Rgba,
    pub text: Rgba,
    pub faint: Rgba,
    /// The open row, and a row under the pointer.
    pub open: Rgba,
    pub hover: Rgba,
    /// The user's own name, the unread count, the cursor.
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub warn: Rgba,
    pub selection: Rgba,
    /// The ground under code, over whatever the text stands on.
    pub code: Rgba,
}

impl Palette {
    /// The colours `window` is drawn in now: those of the look that was
    /// chosen, auto where none was.
    pub fn now(window: &Window, cx: &App) -> Palette {
        let look = cx.try_global::<Look>().copied().unwrap_or_default();
        Palette::of(look, window.appearance())
    }

    /// The colours for what the person chose, on a desktop that is
    /// `desktop`.
    pub fn of(look: Look, desktop: WindowAppearance) -> Palette {
        if look.dark(desktop) {
            Palette {
                ground: rgb(0x1e2024),
                side: rgb(0x17181b),
                line: rgb(0x33363c),
                text: rgb(0xe3e5e8),
                faint: rgb(0x8d939c),
                open: rgb(0x2c3038),
                hover: rgb(0x23262c),
                accent: rgb(0xe8a33d),
                on_accent: rgb(0x1e2024),
                warn: rgb(0xf0764f),
                selection: rgba(0xe8a33d55),
                code: rgba(0xffffff14),
            }
        } else {
            Palette {
                ground: rgb(0xfbfaf8),
                side: rgb(0xf0eeea),
                line: rgb(0xd9d5ce),
                text: rgb(0x22242a),
                faint: rgb(0x6e727a),
                open: rgb(0xe0dcd4),
                hover: rgb(0xe8e5df),
                accent: rgb(0xa8600a),
                on_accent: rgb(0xffffff),
                warn: rgb(0xb3401c),
                selection: rgba(0xa8600a40),
                code: rgba(0x0000000f),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOPS: [WindowAppearance; 4] = [
        WindowAppearance::Light,
        WindowAppearance::VibrantLight,
        WindowAppearance::Dark,
        WindowAppearance::VibrantDark,
    ];

    #[test]
    fn auto_is_the_desktop_s_and_the_other_two_are_theirs_on_any_desktop() {
        let light = Palette::of(Look::Light, WindowAppearance::Light);
        let dark = Palette::of(Look::Dark, WindowAppearance::Light);
        assert_ne!(light, dark);
        // Dark text on a light ground, and the other way round.
        assert_eq!((light.ground, light.text), (rgb(0xfbfaf8), rgb(0x22242a)));
        assert_eq!((dark.ground, dark.text), (rgb(0x1e2024), rgb(0xe3e5e8)));

        for desktop in DESKTOPS {
            assert_eq!(Palette::of(Look::Light, desktop), light, "{desktop:?}");
            assert_eq!(Palette::of(Look::Dark, desktop), dark, "{desktop:?}");
        }
        // Auto is what the window did before there was a choice.
        assert_eq!(Palette::of(Look::Auto, WindowAppearance::Light), light);
        assert_eq!(
            Palette::of(Look::Auto, WindowAppearance::VibrantLight),
            light
        );
        assert_eq!(Palette::of(Look::Auto, WindowAppearance::Dark), dark);
        assert_eq!(Palette::of(Look::Auto, WindowAppearance::VibrantDark), dark);
    }

    #[test]
    fn the_one_key_steps_through_all_three_and_back() {
        let mut look = Look::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(look);
            look = look.next();
        }
        assert_eq!(seen, Look::ALL);
        assert_eq!(look, Look::Auto);
        let names: Vec<&str> = Look::ALL.into_iter().map(Look::name).collect();
        assert_eq!(names, ["auto", "light", "dark"]);
    }
}
