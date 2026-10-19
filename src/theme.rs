//! The window's colours, for a light and a dark desktop. They say nothing
//! the words do not.

use gpui::{App, Global, Rgba, Window, WindowAppearance, rgb, rgba};
use serde::{Deserialize, Serialize};

use crate::model::Dot;

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
/// is unmistakable. The shades come from the yard rust scale in the
/// design system. It is stable: the same name is the same colour on
/// every run and every machine, from a hash and not from the yard's
/// place in the settings, which moving a yard would change. The name
/// always stands beside it, so two yards that share a shade still
/// read apart.
pub fn yard_color(name: &str, dark: bool) -> Rgba {
    let colors = match dark {
        true => &YARD_DARK_COLORS,
        false => &YARD_LIGHT_COLORS,
    };
    rgb(colors[slot(name) % colors.len()])
}

/// Brighter rust shades on a dark ground and deeper shades on a light
/// one, from the design system's `yard-300` through `yard-700` tokens.
const YARD_DARK_COLORS: [u32; 3] = [0xeba17e, 0xe2794a, 0xd3602f];
const YARD_LIGHT_COLORS: [u32; 3] = [0xd3602f, 0xb54e22, 0x8f3c19];

/// A linked name's peer suffix in either look: the dark one is the
/// review's peer colour, and the light one darkened past text contrast
/// on the list's grounds.
const PEER_DARK: u32 = 0xb693f5;
const PEER_LIGHT: u32 = 0x6440b8;

/// The chip's letters, chosen to read against its yard shade at text
/// contrast. Dots in cards and rows carry no letters.
pub fn yard_ink(name: &str, dark: bool) -> Rgba {
    match (dark, slot(name) % YARD_DARK_COLORS.len()) {
        (true, _) | (false, 0) => rgb(0x161718),
        (false, _) => rgb(0xffffff),
    }
}

/// The colour's place in the yard shades: FNV-1a over the name's bytes.
/// The standard hasher will not do: it is seeded anew every run.
fn slot(name: &str) -> usize {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as usize
}

/// The dot a crew member's state draws beside their name: what runs is
/// the running green, what waits yard rust, what failed the
/// error red, and what rests or stopped the quiet grey. Never colour
/// alone: the state stays in words on the row's second line.
pub fn crew_dot(dot: Dot, palette: Palette) -> Rgba {
    match dot {
        Dot::Working => palette.ok,
        Dot::Idle | Dot::Stopped => palette.faint,
        Dot::Blocked => palette.accent,
        Dot::Failed => palette.warn,
    }
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
    /// The yard-rust fill for what waits on the person reading (unread
    /// mail, open asks, a bead held by them) and controls that say yes:
    /// the primary button, chosen segment and checked box. Text on the
    /// fill uses `on_accent`; waiting text uses `text_accent`.
    pub accent: Rgba,
    pub on_accent: Rgba,
    /// Yardr Design's text-accent token: a lighter rust on dark grounds
    /// and a darker one on light grounds, each at text contrast.
    pub text_accent: Rgba,
    pub warn: Rgba,
    /// Links: in messages, ask and bead ids, and the small controls that
    /// open an address or reset one (Open, run, reveal, reset). Blue in
    /// either look, at text contrast on every ground. No design token
    /// names a blue, so these two are the pass's own, held by the test
    /// below.
    pub link: Rgba,
    /// A linked name's peer suffix (`@airy`), in its own colour, so what
    /// crosses a link reads apart from this yard's own names.
    pub peer: Rgba,
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
                accent: rgb(0xd3602f),
                on_accent: rgb(0x161718),
                text_accent: rgb(0xe2794a),
                warn: rgb(0xf0764f),
                link: rgb(0x7aa2f7),
                peer: rgb(PEER_DARK),
                ok: rgb(0x7fb069),
                selection: rgba(0xd3602f55),
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
                accent: rgb(0xd3602f),
                on_accent: rgb(0x161718),
                text_accent: rgb(0xb54e22),
                warn: rgb(0xb3401c),
                link: rgb(0x1a56c8),
                peer: rgb(PEER_LIGHT),
                ok: rgb(0x3d8b4f),
                selection: rgba(0xd3602f40),
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
        // Names are stable, and the two yards of the bead's Mac read
        // apart in either look.
        assert_eq!(yard_color("adventuretime", true), rgb(0xe2794a));
        assert_eq!(yard_color("plantime", true), rgb(0xeba17e));
        assert_eq!(yard_color("adventuretime", false), rgb(0xb54e22));
        assert_eq!(yard_color("plantime", false), rgb(0xd3602f));
        assert_ne!(
            yard_color("adventuretime", true),
            yard_color("plantime", true)
        );
        assert_ne!(
            yard_color("adventuretime", false),
            yard_color("plantime", false)
        );
        // Each shade is a design token, and the three in each look differ.
        assert_eq!(YARD_DARK_COLORS, [0xeba17e, 0xe2794a, 0xd3602f]);
        assert_eq!(YARD_LIGHT_COLORS, [0xd3602f, 0xb54e22, 0x8f3c19]);
        for colors in [&YARD_DARK_COLORS, &YARD_LIGHT_COLORS] {
            let colors: Vec<Rgba> = colors.iter().map(|color| rgb(*color)).collect();
            for (one, another) in colors
                .iter()
                .enumerate()
                .flat_map(|(n, one)| colors.iter().skip(n + 1).map(move |another| (one, another)))
            {
                assert_ne!(one, another, "{one:?} {another:?}");
            }
        }
        // The chip's letters pass 4.5:1, and its colour stands apart
        // from the surface. None is the running green nor the error.
        for look in [Look::Dark, Look::Light] {
            let palette = Palette::of(look, WindowAppearance::Dark);
            for name in ["adventuretime", "plantime", "home", "droplet"] {
                let ground = yard_color(name, palette.dark);
                let ink = yard_ink(name, palette.dark);
                assert!(contrast(ink, ground) >= 4.5, "{ground:?} under {:?}", ink);
                for surface in [palette.side, palette.ground] {
                    assert!(
                        contrast(ground, surface) >= 3.,
                        "{ground:?} against {surface:?}"
                    );
                }
                assert_ne!(ground, palette.ok, "{ground:?}");
                assert_ne!(ground, palette.warn, "{ground:?}");
            }
        }
    }

    #[test]
    fn links_are_blue_at_text_contrast_on_every_ground_in_either_look() {
        // The review's blue for links; no design token names one.
        let dark = Palette::of(Look::Dark, WindowAppearance::Dark);
        let light = Palette::of(Look::Light, WindowAppearance::Light);
        assert_eq!((dark.link, light.link), (rgb(0x7aa2f7), rgb(0x1a56c8)));
        for palette in [dark, light] {
            // The code ground is translucent, so no ratio is computed
            // for it: over any of these it lifts them by a shade, and
            // the link still reads.
            for ground in [palette.side, palette.ground, palette.hover, palette.open] {
                assert!(
                    contrast(palette.link, ground) >= 4.5,
                    "{:?} on {ground:?}",
                    palette.link
                );
            }
            assert_ne!(palette.link, palette.accent);
            assert_ne!(palette.link, palette.warn);
            assert_ne!(palette.link, palette.text);
        }
    }

    #[test]
    fn the_accent_is_yard_rust_with_readable_text_on_it_in_either_look() {
        // --accent is --yard-500 in both looks, and the selection
        // carries its tint.
        for look in [Look::Dark, Look::Light] {
            let palette = Palette::of(look, WindowAppearance::Dark);
            assert_eq!(palette.accent, rgb(0xd3602f));
            assert_eq!(
                (
                    palette.selection.r,
                    palette.selection.g,
                    palette.selection.b
                ),
                (palette.accent.r, palette.accent.g, palette.accent.b)
            );
            assert!(
                contrast(palette.on_accent, palette.accent) >= 4.5,
                "{:?} on {:?}",
                palette.on_accent,
                palette.accent
            );
            assert_ne!(palette.accent, palette.ok);
            assert_ne!(palette.accent, palette.warn);
        }
        // The running dot stays green.
        assert_eq!(
            Palette::of(Look::Dark, WindowAppearance::Dark).ok,
            rgb(0x7fb069)
        );
        assert_eq!(
            Palette::of(Look::Light, WindowAppearance::Light).ok,
            rgb(0x3d8b4f)
        );
    }

    #[test]
    fn text_accent_uses_the_readable_yard_rust_step_in_either_look() {
        let dark = Palette::of(Look::Dark, WindowAppearance::Dark);
        let light = Palette::of(Look::Light, WindowAppearance::Light);
        assert_eq!(dark.text_accent, rgb(0xe2794a));
        assert_eq!(light.text_accent, rgb(0xb54e22));
        for palette in [dark, light] {
            for ground in [palette.side, palette.ground, palette.hover, palette.open] {
                assert!(
                    contrast(palette.text_accent, ground) >= 4.5,
                    "{:?} on {ground:?}",
                    palette.text_accent
                );
            }
            assert_ne!(palette.text_accent, palette.accent);
        }
    }

    #[test]
    fn the_peer_suffix_reads_on_every_list_ground_in_either_look() {
        // The dark peer is the review's own; the light one is darkened
        // past text contrast.
        assert_eq!((PEER_DARK, PEER_LIGHT), (0xb693f5, 0x6440b8));
        for look in [Look::Dark, Look::Light] {
            let palette = Palette::of(look, WindowAppearance::Dark);
            for ground in [palette.side, palette.ground, palette.hover, palette.open] {
                assert!(
                    contrast(palette.peer, ground) >= 4.5,
                    "{:?} on {ground:?}",
                    palette.peer
                );
            }
            assert_ne!(palette.peer, palette.text);
            assert_ne!(palette.peer, palette.faint);
        }
    }

    #[test]
    fn every_crew_state_dots_its_colour() {
        for look in [Look::Dark, Look::Light] {
            let palette = Palette::of(look, WindowAppearance::Dark);
            assert_eq!(crew_dot(Dot::Working, palette), palette.ok);
            assert_eq!(crew_dot(Dot::Idle, palette), palette.faint);
            assert_eq!(crew_dot(Dot::Blocked, palette), palette.accent);
            assert_eq!(crew_dot(Dot::Stopped, palette), palette.faint);
            assert_eq!(crew_dot(Dot::Failed, palette), palette.warn);
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
