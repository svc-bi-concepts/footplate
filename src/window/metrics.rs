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

/// How wide the Voice section's labels stand before their fields, in rems:
/// the longest of them (`microphone`) whole, and every field after it at one
/// place. In rems because the words in it are.
pub(crate) const VOICE_LABEL: f32 = 6.;

/// How many chars of a microphone's id its row in the Voice settings' list
/// shows before an ellipsis (`ui::shorten`). Measured as DejaVu Sans draws
/// it (the widest sans of the desktops here, and the font the Linux shots
/// draw in) at the base text size, 14 pixels, with the picker at its widest,
/// 28 rems: the list's border and padding and the row's leave 414 pixels
/// for the row's two texts and the gap between them. The tightest row of
/// the shots' microphones is the Studio Display's, the one chosen and so
/// drawn bold: its name takes 211 of them, which leaves 203 for the id.
/// 18 chars and the ellipsis take 175 of those; 22 would take 211 and cut
/// the ellipsis itself. The Mac's own font is narrower still. Everything in
/// the row is in rems but its hairlines, so this holds at every text size
/// where the picker stands at its widest; below that, the row's
/// `truncate()` cuts where the room ends, as it did.
pub(crate) const INPUT_NOTE: usize = 18;

/// How many chars of a microphone's name its row in the Voice settings' list
/// shows before an ellipsis (`ui::shorten`), measured as [`INPUT_NOTE`] is,
/// in the same 414 pixels of the picker at its widest. The longest name of
/// the shots' microphones, the Studio Display's, is 25 chars and stands
/// whole: drawn bold in the chosen row it takes 211 pixels, which leaves the
/// 203 that [`INPUT_NOTE`] counts on for the id. A longer name is cut there,
/// and its 25 chars and the ellipsis take 226, which leave 188 for the id's
/// 175. Left to the name's own cap, 16 rems or 256 pixels, a long name would
/// leave the id 158, and the id's `truncate()` would clip it mid-glyph, as it
/// did the name. The Mac's own font is narrower still. A name of wider
/// letters than the Studio Display's is cut by its `truncate()` where its
/// room ends, as it was.
pub(crate) const INPUT_NAME: usize = 25;

/// How many chars of a voice's note its row in the Voice settings' list
/// shows before an ellipsis (`ui::shorten`), measured as
/// [`INPUT_NOTE`] is, with the picker at its widest, 22 rems: 318 pixels
/// for the name and the note. Beside the widest name, `meridian`, drawn
/// bold where it is the choice, 248 are left for the note, and 26 chars and
/// the ellipsis take 225 of those. The longest note OpenAI's guide gives,
/// `English, North American`, is 23 chars and stands whole, in the widest
/// row 262 of the 318 pixels, so no note is cut today; a longer one the
/// guide adds is cut where it would not fit, and a test holds the notes
/// to this number.
pub(crate) const VOICE_NOTE: usize = 26;

/// How many chars of its choice a Select's field shows before an ellipsis
/// (`ui::shorten`), measured as [`INPUT_NOTE`] is, in the narrower of the
/// two fields, the voice's, at its widest, 22 rems: its border, padding, gap
/// and chevron leave 302 pixels for the choice, and 32 chars of a long id and
/// the ellipsis take 267 of those. The microphone's field, at 28 rems, has
/// 398 pixels and room over. The choice is a long id where the microphone
/// that was chosen is not plugged in, or before the list is read.
pub(crate) const SELECT_CHOICE: usize = 32;

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
/// dot's 12 (the lamp's 8 with its 4-pixel gap), rounded up to 96 pixels
/// to hold the longest stage word `contributor` (67 pixels) with its lamp
/// and padding without cutting, at the usual 16-pixel rem. The 80 pixels
/// under the padding hold each phrase of a "+N" whole up to two digits:
/// `+10 working` is 77 as DejaVu draws it. In rems because
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

/// How far a letter's body stands in from the reading column's left,
/// in rems: past the head's clock, so the clock's place is a gutter
/// the letter's own time stands in while the pointer is on it. The
/// clock says five characters at the text size; 3.5 rems clears the
/// widest of them with room past it, and the words stand roughly
/// under the sender's name. In rems because the clock in it is: a
/// gutter in pixels cut the time at the larger text sizes.
pub(crate) const LETTER_GUTTER: f32 = 3.5;
