//! The window's colours, for a light and a dark desktop. They say nothing
//! the words do not.

use gpui::{Rgba, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy)]
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
}

impl Palette {
    pub fn of(appearance: WindowAppearance) -> Palette {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Palette {
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
            },
            WindowAppearance::Light | WindowAppearance::VibrantLight => Palette {
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
            },
        }
    }
}
