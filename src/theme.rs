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

    /// Whether the choice is none: what the system has, as it is now and
    /// as it changes. The settings say nothing while it is.
    pub fn is_auto(&self) -> bool {
        matches!(*self, Look::Auto)
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

/// A yard's colour, by its name: the chip at the rail's head, the dot
/// in a card's header and in a switcher row are this, so the open yard
/// is unmistakable. It is stable: the same name is the same colour on
/// every run and every machine, from a hash and not from the yard's
/// place in the settings, which moving a yard would change. The name
/// always stands beside it, so two yards that share a colour still
/// read apart.
pub fn yard_color(name: &str, dark: bool) -> Rgba {
    let [dark_ground, light_ground] = YARD_COLORS[slot(name) % YARD_COLORS.len()];
    rgb(match dark {
        true => dark_ground,
        false => light_ground,
    })
}

/// A yard's colour as two grounds: on a dark ground first, on a light
/// one after. Each reads with [`Palette::on_accent`] letters on it, as
/// the rail's chip has them; the rest are dots, which carry no letters.
/// None is the running green nor the error red, which are statuses and
/// not yards.
const YARD_COLORS: [[u32; 2]; 8] = [
    [0xe2794a, 0xb54e22],
    [0xe8a33d, 0xa8600a],
    [0xd9b45c, 0x8a6d1f],
    [0x5fb8a6, 0x2e7d6e],
    [0x6fb3d8, 0x2f6e9e],
    [0x7e93d6, 0x3d4fa3],
    [0xd67e8c, 0xa83a52],
    [0xc08a5e, 0x7a5226],
];

/// The colour's place in [`YARD_COLORS`]: FNV-1a over the name's bytes.
/// The standard hasher will not do: it is seeded anew every run.
fn slot(name: &str) -> usize {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as usize
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Whether the grounds are dark: a yard's colour is the one that
    /// reads on them.
    pub dark: bool,
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
    /// The status row's dot where the yard runs.
    pub ok: Rgba,
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
                dark: true,
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
                ok: rgb(0x7fb069),
                selection: rgba(0xe8a33d55),
                code: rgba(0xffffff14),
            }
        } else {
            Palette {
                dark: false,
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
                ok: rgb(0x3d8b4f),
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

    /// The contrast of `letters` over `ground`, as WCAG reads it: 1 is
    /// none, 21 is black over white.
    fn contrast(letters: Rgba, ground: Rgba) -> f32 {
        let luminance = |color: Rgba| {
            let linear = |channel: f32| match channel <= 0.04045 {
                true => channel / 12.92,
                false => ((channel + 0.055) / 1.055).powf(2.4),
            };
            0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
        };
        let (lighter, darker) = match luminance(letters) > luminance(ground) {
            true => (luminance(letters), luminance(ground)),
            false => (luminance(ground), luminance(letters)),
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn yards_keep_their_colours_and_read_with_letters_on_them() {
        // The same name is the same colour, and a dark ground's is not
        // a light one's.
        assert_eq!(
            yard_color("adventuretime", true),
            yard_color("adventuretime", true)
        );
        assert_eq!(yard_color("plantime", false), yard_color("plantime", false));
        assert_ne!(
            yard_color("adventuretime", true),
            yard_color("adventuretime", false)
        );
        // The two yards of the bead's Mac read apart, on either ground.
        assert_ne!(
            yard_color("adventuretime", true),
            yard_color("plantime", true)
        );
        assert_ne!(
            yard_color("adventuretime", false),
            yard_color("plantime", false)
        );
        // Eight colours a side, each its own.
        let darks: Vec<Rgba> = YARD_COLORS.iter().map(|[dark, _]| rgb(*dark)).collect();
        let lights: Vec<Rgba> = YARD_COLORS.iter().map(|[_, light]| rgb(*light)).collect();
        for pair in [darks.as_slice(), lights.as_slice()] {
            for (one, another) in pair
                .iter()
                .enumerate()
                .flat_map(|(n, one)| pair.iter().skip(n + 1).map(move |another| (one, another)))
            {
                assert_ne!(one, another, "{one:?} {another:?}");
            }
        }
        // The chip's letters over each: three to one, as non-text wants.
        // None is the running green nor the error red.
        for look in [Look::Dark, Look::Light] {
            let palette = Palette::of(look, WindowAppearance::Dark);
            for [dark, light] in YARD_COLORS {
                let ground = rgb(match palette.dark {
                    true => dark,
                    false => light,
                });
                assert!(
                    contrast(palette.on_accent, ground) >= 3.,
                    "{ground:?} under {:?}",
                    palette.on_accent
                );
                assert_ne!(ground, palette.ok, "{ground:?}");
                assert_ne!(ground, palette.warn, "{ground:?}");
            }
        }
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
