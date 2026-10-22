//! The window's measures: its fixed sizes, and the small functions that
//! fit a pane or a drawer beside them. In pixels where the frame stands
//! fixed, in rems where the words in it set the size.

use gpui::{Div, Pixels, div, prelude::*, px, rems};

use crate::titlebar;

/// How high an image of a mail stands in the conversation, whatever its own
/// size: the room is there before the image is, so nothing moves under the
/// reader when it comes.
pub(crate) const IMAGE_HEIGHT: f32 = 240.;

/// The margin an image open over the window leaves out, each side, in
/// rems: what its fit is short of the window by.
pub(crate) const IMAGE_MARGIN: f32 = 2.;

/// The side of an attachment of the draft, above the line, in rems: its
/// name stands in it where it is no image, so it goes with the text size.
pub(crate) const ATTACHMENT: f32 = 3.5;

/// The rail's width in pixels, fixed: room for a label under each
/// icon, as the review has it.
pub(crate) const RAIL: f32 = 64.;

/// The window's narrowest width in pixels, which main.rs sets as its
/// minimum: the 560 the list (270 at the usual text size) and the pane
/// beside it have between them, and the rail beside those, so that no pane
/// gives up room for the rail. The tests of the narrowest window read this
/// number too.
pub(crate) const WINDOW_MIN: f32 = 560. + RAIL;

/// A rail tile's width in pixels: the glyph with its label under it, in
/// the rail with its padding on both sides, 6 + 52 + 6.
pub(crate) const RAIL_TILE: f32 = 52.;

/// A rail label's size in pixels, fixed with its tile.
pub(crate) const RAIL_LABEL: f32 = 10.;

/// How far in from the list's left edge a head's first item stands where
/// the content reaches up into the title bar, in pixels: the list starts
/// at the rail's edge, and the traffic lights run 5 pixels past it, so the
/// item begins where [`titlebar::CLEAR`] says, from the window's left. In
/// pixels as the lights are, whatever the text.
pub(crate) const HEAD_INSET: f32 = titlebar::CLEAR - RAIL;

/// A stage cell's narrowest width in rems: a headcode with its lamp
/// dot and the cell's padding, and the longest stage word whole.
/// Measured as DejaVu Sans draws `● 7mcg` (the widest sans of the
/// desktops here, and the font the Linux shots draw in), 47 pixels at
/// the meta size, with the cell's 16 pixels of padding and the running
/// dot's 10 (6 with its 4-pixel gap), rounded up to 96 pixels to hold
/// the longest stage word `contributor` (67 pixels) with its lamp and
/// padding without cutting, at the usual 16-pixel rem. In rems because
/// the words in it are: a cell in pixels cut its words at the larger
/// text sizes. Stage heads longer than the cell wrap whole onto a
/// second line; headcodes longer than this truncate, as they did on the
/// fixed grid.
pub(crate) const PANEL_MIN: f32 = 6.;

/// The switcher's list in rems, drawn once: its narrowest and its widest.
/// They are in rems because the hint line in it is: a list in pixels cut
/// its words at the larger text sizes. Sized for the footer's hint line
/// `↑↓ move · ↵ open · esc close · + add a yard · ? all keys`, as DejaVu
/// Sans draws it (the widest sans of the desktops here, and the font the
/// Linux shots draw in), plus the footer's padding and the list's border,
/// at the usual 16-pixel rem: the line, 340 pixels, between 12 of padding
/// each side with a 1-pixel border. The widest, 480, holds it on one line
/// at the usual size with every row's ctrl+N whole; the narrowest, 320,
/// leaves room to shrink, the hint line eliding, at a larger text size or
/// a narrow window. Where the window's width less the rail is narrower
/// still, the list is capped at that.
///
/// The test beside the switcher's rows holds the hint and every key
/// inside at the usual size and the largest.
pub(crate) const SWITCHER_MIN: f32 = 20.;
pub(crate) const SWITCHER_MAX: f32 = 30.;

/// A line view's stage column in pixels at the usual text size, and no
/// narrower: the panel's track straightened, one column per stage in
/// travel order. Drawn through [`grown`], so it stands 176 wide at the
/// usual size and grows with the text above it.
pub(crate) const LINE_COLUMN: f32 = 176.;

/// A line view's column head in rems: its name with three lines of group
/// words under it, bottom-aligned, so a column whose group wraps onto a
/// second or third line puts its rule level with a shorter group. The
/// row's rules make one straight track, as the panel's heads have it.
/// A name line with three group lines at gpui's line height,
/// 1.42 with 3 × 1.21, rounded up. A group that wants a fourth line
/// makes its own column taller, as a panel head does.
pub(crate) const LINE_HEAD: f32 = 5.2;

/// The departures board's columns in pixels at the usual text size,
/// drawn once: the expected arrival in mono, the bead's title with its
/// headcode, the platform, the line, and the remark. Drawn through
/// [`grown`], so each stands its width at the usual size and grows with
/// the text above it. Past the pane's edge the board scrolls sideways;
/// the panel fits instead. The line holds this yard's longest depot
/// names: 88 cut `yardr-hosted` even at the usual size, so 92.
pub(crate) const BOARD_EXPECTED: f32 = 80.;
pub(crate) const BOARD_BEAD: f32 = 280.;
pub(crate) const BOARD_PLATFORM: f32 = 120.;
pub(crate) const BOARD_LINE: f32 = 92.;
pub(crate) const BOARD_REMARK: f32 = 220.;

/// A column `usual` pixels wide at the usual text size: that wide at
/// smaller sizes too, and growing with the text above it, since the
/// words in it stand in rems. A column in pixels alone cut its words at
/// the larger text sizes; in rems alone it would narrow past what the
/// design draws. `usual` over the usual 16-pixel rem is its width.
pub(crate) fn grown(usual: f32) -> Div {
    div().w(rems(usual / 16.)).min_w(px(usual))
}

/// The bead drawer's width in pixels at the usual text size, drawn
/// through [`grown`]: fixed, and growing with the text above it, as a
/// line view's columns do.
pub(crate) const DRAWER_WIDTH: f32 = 320.;

/// Whether the window `window` pixels wide holds the bead drawer beside
/// the reading pane: the pane keeps at least the drawer's own width. The
/// rail is [`RAIL`] pixels, the list 270 at the usual text size and never
/// over 48 hundredths of the window, as [`Footplate::column`] draws them,
/// and the drawer its grown width; `rem` is the window's rem in pixels.
pub(crate) fn drawer_fits(window: Pixels, rem: f32) -> bool {
    let drawer = DRAWER_WIDTH.max(DRAWER_WIDTH / 16. * rem);
    let list = (270. * rem / 16.).min(0.48 * f32::from(window));
    f32::from(window) - RAIL - list - drawer >= drawer
}

/// How many backlog titles a line view's backlog column lists before
/// "+N more".
pub(crate) const BACKLOG_TITLES: usize = 6;

/// The conversation's reading width: the mail, the notice and the
/// composer stand full width up to 880 pixels, centred in the pane.
pub(crate) const READING_WIDTH: f32 = 880.;
