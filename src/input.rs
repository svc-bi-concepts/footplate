//! The line that is typed in: a text with a cursor, a selection, the
//! clipboard and the system's input methods. gpui has no text field of its
//! own; this follows the one among its examples (examples/input.rs), and
//! adds that a text wider than the box goes on in the next row, that the
//! box is as high as its rows up to a part of the window and then scrolls
//! with the cursor, that shift+Enter breaks the line and that Enter hands
//! the text on. A masked line is the same line for a secret: it draws a
//! mark for each character, takes what is pasted and gives nothing out.
//! Shown, it draws the text as typed, for a look, and still gives
//! nothing out.

#[cfg(target_os = "macos")]
pub(crate) mod clipboard;

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardEntry, ClipboardItem, ContentMask, Context, CursorStyle,
    Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Image, InspectorElementId, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render,
    ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign, TextRun, TextStyle,
    UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill, point, prelude::*, px,
    relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::Palette;

actions!(
    line,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        Up,
        Down,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Newline,
        Submit
    ]
);

/// The key context the line's keys are bound in.
const CONTEXT: &str = "Line";

/// The keys of the line. `secondary` is cmd on macOS and ctrl elsewhere.
pub fn bind_keys(cx: &mut App) {
    let on = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, on),
        KeyBinding::new("delete", Delete, on),
        KeyBinding::new("left", Left, on),
        KeyBinding::new("right", Right, on),
        KeyBinding::new("shift-left", SelectLeft, on),
        KeyBinding::new("shift-right", SelectRight, on),
        KeyBinding::new("up", Up, on),
        KeyBinding::new("down", Down, on),
        KeyBinding::new("shift-up", SelectUp, on),
        KeyBinding::new("shift-down", SelectDown, on),
        KeyBinding::new("secondary-a", SelectAll, on),
        KeyBinding::new("secondary-v", Paste, on),
        KeyBinding::new("secondary-c", Copy, on),
        KeyBinding::new("secondary-x", Cut, on),
        KeyBinding::new("home", Home, on),
        KeyBinding::new("end", End, on),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, on),
        KeyBinding::new("shift-enter", Newline, on),
        KeyBinding::new("enter", Submit, on),
    ]);
}

/// Enter was pressed in the line.
pub struct Submitted;

pub struct Line {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<Layout>,
    last_bounds: Option<Bounds<Pixels>>,
    /// How far the text is moved up, so that the cursor's row is in view.
    scroll: Pixels,
    /// The cursor moved or the text changed: the next frame scrolls to the
    /// cursor. The wheel scrolls away from it.
    reveal: bool,
    /// Where up and down keep the cursor, from the row they set out in.
    column: Option<Pixels>,
    is_selecting: bool,
    /// The text is a secret: each character is drawn as [`MARK`], it is
    /// not copied or cut out, and the input method is told none of it.
    masked: bool,
    /// A secret drawn as typed, for a look: still a secret, which
    /// nothing is copied or cut out of and the input method is told
    /// none of.
    shown: bool,
    images: bool,
}

/// What a masked line draws in place of each character.
const MARK: &str = "\u{2022}";

/// An image was pasted into the line. The line holds text, so it is the
/// window's to attach.
#[derive(Clone)]
pub enum Pasted {
    Image(Image),
    #[cfg(target_os = "macos")]
    Native(clipboard::Content),
}

/// Backspace was pressed in the empty line: there is no text left to take
/// from, and the window may hold something before the text.
pub struct Emptied;

impl EventEmitter<Submitted> for Line {}
impl EventEmitter<Pasted> for Line {}
impl EventEmitter<Emptied> for Line {}

impl Line {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Line {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll: px(0.),
            reveal: true,
            column: None,
            is_selecting: false,
            masked: false,
            shown: false,
            images: false,
        }
    }

    pub fn composer(cx: &mut Context<Self>) -> Self {
        Self {
            images: true,
            ..Self::new(cx)
        }
    }

    /// A line for a secret. It is one line: Enter hands it on and nothing
    /// breaks it, and a line break that is pasted is a character as any
    /// other, drawn as a mark.
    pub fn masked(cx: &mut Context<Self>) -> Self {
        Line {
            masked: true,
            ..Line::new(cx)
        }
    }

    /// Draws the secret as typed, for a look, or as marks again. What
    /// is typed is unchanged, and still nothing is copied or cut out
    /// of it.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown != shown {
            self.shown = shown;
            self.changed(cx);
        }
    }

    /// What is typed, which the line no longer has afterwards: a secret is
    /// taken once, by what sends it.
    pub fn take_text(&mut self, cx: &mut Context<Self>) -> String {
        let text = self.content.to_string();
        self.set_text("", cx);
        text
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replaces what is typed, the cursor at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = line_breaks(text).into();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.changed(cx);
    }

    /// Puts the cursor at `offset`: the window opens a draft with the
    /// caret on its empty line, not at its end.
    pub fn set_cursor(&mut self, offset: usize, cx: &mut Context<Self>) {
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        self.move_to(offset, cx);
    }

    /// The rows of the text and how many of them the box shows, as last
    /// drawn.
    #[cfg(test)]
    pub fn rows(&self) -> (usize, usize) {
        let (Some(layout), Some(bounds)) = (self.last_layout.as_ref(), self.last_bounds) else {
            return (0, 0);
        };
        let shown = bounds.size.height / layout.line_height;
        (layout.rows.len(), shown.round() as usize)
    }

    /// What the line last drew of its text, and what it says while empty.
    #[cfg(test)]
    pub fn drawn(&self) -> (String, String) {
        let lines = self.last_layout.iter().flat_map(|layout| &layout.lines);
        let lines: Vec<&str> = lines.map(|line| line.text.as_ref()).collect();
        (lines.join("\n"), self.placeholder.to_string())
    }

    pub fn set_placeholder(&mut self, placeholder: &str, cx: &mut Context<Self>) {
        if self.placeholder.as_ref() != placeholder {
            self.placeholder = placeholder.to_string().into();
            cx.notify();
        }
    }

    /// What the line says it writes, as last set.
    #[cfg(test)]
    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(Submitted);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.masked {
            return;
        }
        self.replace_text_in_range(None, "\n", window, cx)
    }

    /// The cursor or the text is another: the next frame shows the cursor,
    /// and up and down set out from where it is now.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.reveal = true;
        self.column = None;
        cx.notify()
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.step_rows(false, false, cx)
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.step_rows(true, false, cx)
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step_rows(false, true, cx)
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step_rows(true, true, cx)
    }

    /// The cursor goes one row up or down and keeps its column. Where there
    /// is no row to go to the key is not the line's: the window may choose
    /// with it (the options of a card).
    fn step_rows(&mut self, down: bool, select: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let to = self.last_layout.as_ref().and_then(|layout| {
            let row = layout.row_of(cursor);
            let to = if down { row + 1 } else { row.checked_sub(1)? };
            (to < layout.rows.len()).then(|| {
                let x = self.column.unwrap_or_else(|| layout.x(row, cursor));
                (layout.index_at(to, x, &self.content), x)
            })
        });
        let Some((offset, x)) = to else {
            return cx.propagate();
        };
        if select {
            self.select_to(offset, cx)
        } else {
            self.move_to(offset, cx)
        }
        self.column = Some(x);
    }

    /// Where the cursor's row starts, and the last place of the cursor in it.
    fn row_ends(&self) -> (usize, usize) {
        match self.last_layout.as_ref() {
            Some(layout) => {
                let row = layout.row_of(self.cursor_offset());
                (
                    layout.rows[row].range.start.min(self.content.len()),
                    layout.last_of(row, &self.content),
                )
            }
            None => (0, self.content.len()),
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_ends().0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_ends().1, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.is_empty() {
            return cx.emit(Emptied);
        }
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    /// The wheel moves a text that is higher than its box. One that is not
    /// leaves the wheel to what is around it.
    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(bounds), Some(layout)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return;
        };
        if layout.height() <= bounds.size.height {
            return;
        }
        // Past either end the next frame holds it at the end.
        self.scroll -= event.delta.pixel_delta(layout.line_height).y;
        self.reveal = false;
        cx.stop_propagation();
        cx.notify();
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(target_os = "macos", not(test)))]
        if let Some(content) = clipboard::image(self.images) {
            cx.emit(Pasted::Native(content));
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let text = item.text();
        if self.images
            && let Some(image) = item.into_entries().find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image),
                ClipboardEntry::String(_) => None,
            })
        {
            cx.emit(Pasted::Image(image));
        } else if let Some(text) = text {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    /// A masked line copies nothing out, and so cuts nothing.
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.changed(cx)
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        // Above the box is the first row and under it the last.
        let row = line.row_at(position.y - bounds.top() + self.scroll);
        line.index_at(row, position.x - bounds.left(), &self.content)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.changed(cx)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        offset_to_utf16(&self.content, range.start)..offset_to_utf16(&self.content, range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        offset_from_utf16(&self.content, range_utf16.start)
            ..offset_from_utf16(&self.content, range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        previous_boundary(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        next_boundary(&self.content, offset)
    }
}

fn previous_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(idx, _)| (idx < offset).then_some(idx))
        .unwrap_or(0)
}

/// Where the character that starts at or before `offset` ends. A message's
/// text asks the same of its own (select.rs).
pub fn next_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(idx, _)| (idx > offset).then_some(idx))
        .unwrap_or(text.len())
}

/// A text as the line holds it: every line break is one `\n`, whatever the
/// system it was copied on wrote.
pub fn line_breaks(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The system's input methods count in UTF-16, the text is UTF-8.
pub fn offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }
    utf8_offset
}

pub fn offset_to_utf16(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for ch in text.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

/// How far a text `text` high is moved up so that the cursor's row, from
/// `top` to `bottom`, is in a view `view` high, coming from `scroll`: no
/// further than needed, and never past the text's end.
pub fn scroll_to(top: f32, bottom: f32, text: f32, view: f32, scroll: f32) -> f32 {
    let mut scroll = scroll;
    if bottom - scroll > view {
        scroll = bottom - view;
    }
    if top < scroll {
        scroll = top;
    }
    scroll_within(text, view, scroll)
}

/// A scroll that leaves no gap after the text's end or before its start.
pub fn scroll_within(text: f32, view: f32, scroll: f32) -> f32 {
    scroll.min((text - view).max(0.)).max(0.)
}

/// The part of the window's height the box may take before it scrolls.
const MOST_OF_WINDOW: f32 = 1. / 3.;

/// How many of the text's `rows` the box shows in a window `window` high:
/// all of them up to a third of the window, and one at the least.
pub fn shown_rows(rows: usize, line_height: f32, window: f32) -> usize {
    let most = (window * MOST_OF_WINDOW / line_height).floor().max(1.) as usize;
    rows.clamp(1, most)
}

/// The box keeps the cursor's two pixels free at its right edge, so that
/// the cursor after a row that fills the box is in view.
fn wrap_width(width: Pixels) -> Pixels {
    (width - px(2.)).max(px(1.))
}

/// One row of the text as it is drawn: a line of it, or the part of a line
/// that fits the box.
#[derive(Clone, Debug)]
struct Row {
    /// Its bytes of the text.
    range: Range<usize>,
    /// The line it is of, among the shaped ones, and where that line starts
    /// in the text as drawn.
    line: usize,
    line_start: usize,
    /// The line goes on in the next row: the row ends where the box does,
    /// at no line break.
    cut: bool,
}

/// The text shaped for a width: its lines, each cut into rows.
#[derive(Clone)]
struct Layout {
    lines: Vec<WrappedLine>,
    rows: Vec<Row>,
    line_height: Pixels,
    /// Of a masked text, where each character starts in the text, and the
    /// text's length last: the lines are of marks, one a character, and a
    /// place in the text is another in what is drawn.
    marks: Option<Vec<usize>>,
}

/// Where the place `offset` of a text is in what is drawn of it: the same
/// place, and in a masked text, whose characters start at `marks`, after as
/// many marks as characters are before it.
fn drawn_at(marks: Option<&[usize]>, offset: usize) -> usize {
    match marks {
        Some(marks) => {
            let before = marks.partition_point(|start| *start < offset);
            before.min(marks.len() - 1) * MARK.len()
        }
        None => offset,
    }
}

/// Where each character of `text` starts, and the text's length last.
fn marks_of(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .chain([text.len()])
        .collect()
}

impl Layout {
    /// `lines` are the text's as drawn: of a masked text they are of marks,
    /// and `marks` says where each mark's character is in the text.
    fn new(
        lines: Vec<WrappedLine>,
        len: usize,
        line_height: Pixels,
        marks: Option<Vec<usize>>,
    ) -> Self {
        let mut rows = Vec::new();
        let mut line_start = 0;
        for (ix, line) in lines.iter().enumerate() {
            let cuts = line
                .wrap_boundaries()
                .iter()
                .map(|cut| (line.runs()[cut.run_ix].glyphs[cut.glyph_ix].index, true));
            let mut start = 0;
            for (end, cut) in cuts.chain([(line.text.len(), false)]) {
                rows.push(Row {
                    range: line_start + start..line_start + end,
                    line: ix,
                    line_start,
                    cut,
                });
                start = end;
            }
            // The line break is of no row.
            line_start += line.text.len() + 1;
        }
        let mut layout = Layout {
            lines,
            rows,
            line_height,
            marks,
        };
        // The rows were cut in what is drawn: their bytes are the text's.
        if layout.marks.is_some() {
            for ix in 0..layout.rows.len() {
                let drawn = layout.rows[ix].range.clone();
                layout.rows[ix].range = layout.typed(drawn.start)..layout.typed(drawn.end);
            }
        }
        // A text that could not be shaped is one row, drawn as nothing.
        if layout.rows.is_empty() {
            layout.rows.push(Row {
                range: 0..len,
                line: 0,
                line_start: 0,
                cut: false,
            });
        }
        layout
    }

    /// Where the place `offset` of the text is in what is drawn.
    fn drawn(&self, offset: usize) -> usize {
        drawn_at(self.marks.as_deref(), offset)
    }

    /// Where the place `index` of what is drawn is in the text.
    fn typed(&self, index: usize) -> usize {
        match &self.marks {
            Some(marks) => marks[(index / MARK.len()).min(marks.len() - 1)],
            None => index,
        }
    }

    fn height(&self) -> Pixels {
        self.line_height * self.rows.len() as f32
    }

    /// The row the cursor at `offset` is drawn in. Where a line was cut the
    /// cursor is at the start of the next row, not after the end of this.
    fn row_of(&self, offset: usize) -> usize {
        self.rows
            .iter()
            .position(|row| offset < row.range.end || (offset == row.range.end && !row.cut))
            .unwrap_or(self.rows.len() - 1)
    }

    /// The row `y` under the text's top is in: the first above it, the last
    /// under it.
    fn row_at(&self, y: Pixels) -> usize {
        let row = (y / self.line_height).floor().max(0.) as usize;
        row.min(self.rows.len() - 1)
    }

    /// How far right of the row's start `offset` is.
    fn x(&self, row: usize, offset: usize) -> Pixels {
        let row = &self.rows[row];
        let Some(line) = self.lines.get(row.line) else {
            return px(0.);
        };
        let at = |offset: usize| {
            line.unwrapped_layout
                .x_for_index(self.drawn(offset).saturating_sub(row.line_start))
        };
        at(offset.clamp(row.range.start, row.range.end)) - at(row.range.start)
    }

    /// The place of the cursor in `row` that is closest to `x` right of the
    /// row's start.
    fn index_at(&self, row: usize, x: Pixels, text: &str) -> usize {
        let last = self.last_of(row, text);
        let row = &self.rows[row];
        let Some(line) = self.lines.get(row.line) else {
            return last;
        };
        let layout = &line.unwrapped_layout;
        let start = layout.x_for_index(self.drawn(row.range.start) - row.line_start);
        let index = row.line_start + layout.closest_index_for_x(start + x.max(px(0.)));
        let mut index = self.typed(index).clamp(row.range.start.min(last), last);
        // A key may come before the frame that shapes a new text: the
        // rows are the old text's then, and the place must be one of the
        // new.
        while !text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    /// The character of `row` that is drawn `x` right of the row's start,
    /// if one is.
    fn character_at(&self, row: usize, x: Pixels) -> Option<usize> {
        let row = &self.rows[row];
        let layout = &self.lines.get(row.line)?.unwrapped_layout;
        let start = layout.x_for_index(self.drawn(row.range.start) - row.line_start);
        let index = self.typed(row.line_start + layout.index_for_x(start + x)?);
        row.range.contains(&index).then_some(index)
    }

    /// The last place of the cursor in `row`: its end, and in a row that
    /// was cut the character before, since the cut itself is the start of
    /// the next row.
    fn last_of(&self, row: usize, text: &str) -> usize {
        let row = &self.rows[row];
        let end = row.range.end.min(text.len());
        if row.cut {
            previous_boundary(text, end).max(row.range.start.min(end))
        } else {
            end
        }
    }

    /// Where the cursor at `offset` is, from the text's top left.
    fn position(&self, offset: usize) -> Point<Pixels> {
        let row = self.row_of(offset);
        point(self.x(row, offset), self.line_height * row as f32)
    }
}

/// What the text is drawn with: the element reads it of the window where
/// it stands, and the box is measured with the same.
#[derive(Clone)]
struct Look {
    style: TextStyle,
    font_size: Pixels,
    line_height: Pixels,
}

impl Look {
    fn now(window: &Window) -> Self {
        let style = window.text_style();
        Look {
            font_size: style.font_size.to_pixels(window.rem_size()),
            line_height: window.line_height(),
            style,
        }
    }
}

impl Line {
    /// The text shaped for a box `width` wide. This is the one place the
    /// text becomes what is drawn: a mode that draws it otherwise (a secret
    /// shown as marks) changes the text here, and the rows follow.
    fn shape(&self, width: Option<Pixels>, look: &Look, window: &Window) -> Layout {
        // A masked text is drawn as a mark for each character, and what is
        // shaped is never the text; shown, it is drawn as typed, for a
        // look.
        let marks = (self.masked && !self.shown).then(|| marks_of(&self.content));
        let drawn: SharedString = match &marks {
            Some(marks) => MARK.repeat(marks.len() - 1).into(),
            None => self.content.clone(),
        };
        let at = |offset: usize| drawn_at(marks.as_deref(), offset);
        let run = TextRun {
            len: drawn.len(),
            font: look.style.font(),
            color: look.style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // What an input method is composing is underlined.
        let runs = match self.marked_range.as_ref() {
            Some(marked_range) => vec![
                TextRun {
                    len: at(marked_range.start),
                    ..run.clone()
                },
                TextRun {
                    len: at(marked_range.end) - at(marked_range.start),
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: drawn.len() - at(marked_range.end),
                    ..run
                },
            ],
            None => vec![run],
        };
        let lines = window
            .text_system()
            .shape_text(drawn, look.font_size, &runs, width.map(wrap_width), None)
            .map(|lines| lines.into_vec())
            .unwrap_or_default();
        Layout::new(lines, self.content.len(), look.line_height, marks)
    }
}

impl EntityInputHandler for Line {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        // The system's input method is handed none of a secret.
        if self.masked {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let new_text = line_breaks(new_text);
        self.content =
            (self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        // The row the range starts in, and of the range what is in that row.
        let row = last_layout.row_of(range.start);
        let top = bounds.top() + last_layout.line_height * row as f32 - self.scroll;
        Some(Bounds::from_corners(
            point(bounds.left() + last_layout.x(row, range.start), top),
            point(
                bounds.left() + last_layout.x(row, range.end),
                top + last_layout.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        let row = last_layout.row_at(line_point.y + self.scroll);
        let utf8_index = last_layout.character_at(row, line_point.x)?;
        Some(offset_to_utf16(&self.content, utf8_index))
    }
}

struct LineElement {
    input: Entity<Line>,
}

struct PrepaintState {
    layout: Layout,
    /// What the empty line says in place of a text.
    placeholder: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    scroll: Pixels,
}

impl IntoElement for LineElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        // The height is the text's, and that is known with the width: the
        // box is measured once the layout knows how wide it is.
        let (input, look) = (self.input.clone(), Look::now(window));
        let layout = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let rows = input.read(cx).shape(width, &look, window).rows.len();
            let shown = shown_rows(
                rows,
                look.line_height.into(),
                window.viewport_size().height.into(),
            );
            size(width.unwrap_or_default(), look.line_height * shown as f32)
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let palette = Palette::now(window, cx);
        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let look = Look::now(window);
        let line_height = look.line_height;
        let layout = input.shape(Some(bounds.size.width), &look, window);

        let placeholder = input.content.is_empty().then(|| {
            let run = TextRun {
                len: input.placeholder.len(),
                font: look.style.font(),
                color: palette.faint.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(input.placeholder.clone(), look.font_size, &[run], None)
        });

        let cursor = layout.position(input.cursor_offset());
        let (text, view) = (layout.height().into(), bounds.size.height.into());
        let scroll = px(if input.reveal {
            scroll_to(
                cursor.y.into(),
                (cursor.y + line_height).into(),
                text,
                view,
                input.scroll.into(),
            )
        } else {
            scroll_within(text, view, input.scroll.into())
        });
        let origin = point(bounds.left(), bounds.top() - scroll);
        let (selection, cursor) = if selected_range.is_empty() {
            let quad = fill(
                Bounds::new(origin + cursor, size(px(2.), line_height)),
                palette.accent,
            );
            (Vec::new(), Some(quad))
        } else {
            // Of each row what is selected, and after a row whose line
            // break is selected a little more, so that it shows.
            let quads = layout
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    selected_range.start <= row.range.end && row.range.start < selected_range.end
                })
                .map(|(ix, row)| {
                    let top = origin.y + line_height * ix as f32;
                    let from = layout.x(ix, selected_range.start);
                    let mut to = layout.x(ix, selected_range.end);
                    if selected_range.end > row.range.end && !row.cut {
                        to += line_height / 4.;
                    }
                    fill(
                        Bounds::from_corners(
                            point(origin.x + from, top),
                            point(origin.x + to, top + line_height),
                        ),
                        palette.selection,
                    )
                })
                .collect();
            (quads, None)
        };
        PrepaintState {
            layout,
            placeholder,
            cursor,
            selection,
            scroll,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let layout = prepaint.layout.clone();
        let line_height = layout.line_height;
        let origin = point(bounds.left(), bounds.top() - prepaint.scroll);
        // What is scrolled out of the box is not drawn over its frame.
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for selection in prepaint.selection.drain(..) {
                window.paint_quad(selection)
            }
            // A line that cannot be drawn is left out, and the window stays.
            if let Some(placeholder) = prepaint.placeholder.take() {
                let _ = placeholder.paint(origin, line_height, window, cx);
            }
            let mut rows = 0;
            for line in &layout.lines {
                let at = point(origin.x, origin.y + line_height * rows as f32);
                let _ = line.paint(at, line_height, TextAlign::Left, None, window, cx);
                rows += line.wrap_boundaries().len() + 1;
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });

        let scroll = prepaint.scroll;
        self.input.update(cx, |input, _| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
            input.scroll = scroll;
            input.reveal = false;
        });
    }
}

impl Render for Line {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::submit))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(LineElement { input: cx.entity() })
    }
}

impl Focusable for Line {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::cell::Cell;
    use std::rc::Rc;

    /// The line alone in a window 200 wide and 300 high, with the focus.
    fn opened(cx: &mut TestAppContext) -> (Entity<Line>, &mut VisualTestContext) {
        cx.update(bind_keys);
        let (line, cx) = cx.add_window_view(|window, cx| {
            let line = Line::new(cx);
            window.focus(&line.focus_handle);
            line
        });
        cx.simulate_resize(size(px(200.), px(300.)));
        drawn(cx);
        (line, cx)
    }

    fn drawn(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// The rows of the text, the height of a row, the height of the box and
    /// how far the text is scrolled, as last drawn.
    fn shown(line: &Entity<Line>, cx: &mut VisualTestContext) -> (usize, f32, f32, f32) {
        drawn(cx);
        line.read_with(cx, |line, _| {
            let layout = line.last_layout.as_ref().unwrap();
            (
                layout.rows.len(),
                layout.line_height.into(),
                line.last_bounds.unwrap().size.height.into(),
                line.scroll.into(),
            )
        })
    }

    fn cursor(line: &Entity<Line>, cx: &mut VisualTestContext) -> usize {
        line.read_with(cx, |line, _| line.cursor_offset())
    }

    fn text(line: &Entity<Line>, cx: &mut VisualTestContext) -> String {
        line.read_with(cx, |line, _| line.text().to_string())
    }

    fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    #[gpui::test]
    fn a_text_wider_than_the_box_wraps_and_the_box_grows_then_scrolls(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        // Empty, and with a text that fits: one row high.
        let (rows, row, high, scroll) = shown(&line, cx);
        assert_eq!((rows, high, scroll), (1, row, 0.));
        cx.simulate_input("Land it.");
        assert_eq!(shown(&line, cx), (1, row, row, 0.));

        // Wider than the box: it goes on in a second row, at a space, and
        // the box is as high as its rows.
        cx.simulate_input(" And then the next one after it.");
        let (rows, _, high, scroll) = shown(&line, cx);
        assert!(rows > 1, "the text is in {rows} row");
        assert_eq!((high, scroll), (rows as f32 * row, 0.));
        line.read_with(cx, |line, _| {
            let layout = line.last_layout.as_ref().unwrap();
            assert!(layout.rows[0].cut);
            let second = layout.rows[1].range.start;
            assert_eq!(&line.text()[second - 1..second], " ");
            // No row is wider than the box.
            let width = line.last_bounds.unwrap().size.width;
            for (ix, row) in layout.rows.iter().enumerate() {
                assert!(layout.x(ix, layout.last_of(ix, line.text())) <= width);
                assert!(!row.range.is_empty());
            }
        });

        // A third of the window, 300 high, is the most the box takes: the
        // rows of 100 pixels. Past that the text scrolls, and the cursor's
        // row, the last, is the lowest in view.
        let most = (100. / row).floor();
        cx.simulate_input(&" and more of it".repeat(20));
        let (rows, _, high, scroll) = shown(&line, cx);
        assert!(rows as f32 > most);
        assert_eq!(high, most * row);
        assert_eq!(scroll, rows as f32 * row - high);

        // Up to the first row: the text scrolls back with the cursor, no
        // further than each row needs.
        cx.simulate_keystrokes("up");
        assert_eq!(shown(&line, cx).3, scroll);
        for _ in 0..rows {
            cx.simulate_keystrokes("up");
        }
        assert_eq!(shown(&line, cx).3, 0.);
        line.read_with(cx, |line, _| {
            let layout = line.last_layout.as_ref().unwrap();
            assert_eq!(layout.row_of(line.cursor_offset()), 0);
        });

        // The wheel moves the text away from the cursor and stays within
        // it; a key brings the cursor's row back.
        let at = line.read_with(cx, |line, _| line.last_bounds.unwrap().center());
        cx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-row))),
            ..Default::default()
        });
        assert_eq!(shown(&line, cx).3, row);
        cx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            ..Default::default()
        });
        assert_eq!(shown(&line, cx).3, scroll);
        cx.simulate_keystrokes("left");
        assert_eq!(shown(&line, cx).3, 0.);

        // Sent, or taken out: one row again.
        line.update(cx, |line, cx| line.set_text("", cx));
        assert_eq!(shown(&line, cx), (1, row, row, 0.));
    }

    #[gpui::test]
    fn shift_return_breaks_the_line_and_return_sends(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        let sent = Rc::new(Cell::new(0));
        let count = sent.clone();
        cx.update(|_, cx| {
            cx.subscribe(&line, move |_, _: &Submitted, _| count.set(count.get() + 1))
                .detach()
        });
        cx.simulate_input("ab");
        cx.simulate_keystrokes("left shift-enter");
        assert_eq!(text(&line, cx), "a\nb");
        assert_eq!(cursor(&line, cx), 2);
        // Two rows, and the box as high.
        let (rows, row, high, _) = shown(&line, cx);
        assert_eq!((rows, high), (2, 2. * row));
        assert_eq!(sent.get(), 0);

        // Return hands the text on as it is, and breaks no line.
        cx.simulate_keystrokes("enter");
        assert_eq!(sent.get(), 1);
        assert_eq!(text(&line, cx), "a\nb");

        // A break at the end is a row of its own, where the cursor is.
        cx.simulate_keystrokes("end shift-enter");
        assert_eq!(text(&line, cx), "a\nb\n");
        assert_eq!(shown(&line, cx).0, 3);
        line.read_with(cx, |line, _| {
            let layout = line.last_layout.as_ref().unwrap();
            assert_eq!(layout.position(4), point(px(0.), px(2. * row)));
        });
    }

    #[gpui::test]
    fn up_and_down_move_between_lines_and_keep_the_column(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        line.update(cx, |line, cx| line.set_text("abcdef\nab\nabcdef", cx));
        drawn(cx);
        assert_eq!(cursor(&line, cx), 16);
        // The middle line is shorter: the cursor is at its end, and back
        // in its column on the line above.
        cx.simulate_keystrokes("up");
        assert_eq!(cursor(&line, cx), 9);
        cx.simulate_keystrokes("up");
        assert_eq!(cursor(&line, cx), 6);
        // No row above: the cursor stays.
        cx.simulate_keystrokes("up");
        assert_eq!(cursor(&line, cx), 6);
        cx.simulate_keystrokes("down");
        assert_eq!(cursor(&line, cx), 9);
        cx.simulate_keystrokes("down");
        assert_eq!(cursor(&line, cx), 16);
        cx.simulate_keystrokes("down");
        assert_eq!(cursor(&line, cx), 16);

        // A step to the side is a new column.
        cx.simulate_keystrokes("up left up");
        assert_eq!(cursor(&line, cx), 1);
        cx.simulate_keystrokes("down down");
        assert_eq!(cursor(&line, cx), 11);

        // Home and end are the row's.
        cx.simulate_keystrokes("home");
        assert_eq!(cursor(&line, cx), 10);
        cx.simulate_keystrokes("end");
        assert_eq!(cursor(&line, cx), 16);
        cx.simulate_keystrokes("up home");
        assert_eq!(cursor(&line, cx), 7);

        // In a line that was cut they are the row's too, and end twice
        // stays in its row.
        line.update(cx, |line, cx| line.set_text(&"word ".repeat(30), cx));
        drawn(cx);
        let rows = line.read_with(cx, |line, _| {
            line.last_layout.as_ref().unwrap().rows.clone()
        });
        assert!(rows.len() > 2);
        cx.simulate_keystrokes("up up home");
        let row = rows.len() - 3;
        assert_eq!(cursor(&line, cx), rows[row].range.start);
        cx.simulate_keystrokes("end end");
        assert_eq!(cursor(&line, cx), rows[row].range.end - 1);
        cx.simulate_keystrokes("down home");
        assert_eq!(cursor(&line, cx), rows[row + 1].range.start);
    }

    #[gpui::test]
    fn a_click_puts_the_cursor_in_the_row_it_is_on(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        line.update(cx, |line, cx| line.set_text("one\ntwo\nthree", cx));
        let (_, row, _, _) = shown(&line, cx);
        let bounds = line.read_with(cx, |line, _| line.last_bounds.unwrap());
        // Left in the second row, and far right in the first.
        let at = point(bounds.left(), bounds.top() + px(1.5 * row));
        cx.simulate_click(at, Modifiers::default());
        assert_eq!(cursor(&line, cx), 4);
        let at = point(bounds.right() - px(3.), bounds.top() + px(0.5 * row));
        cx.simulate_click(at, Modifiers::default());
        assert_eq!(cursor(&line, cx), 3);
    }

    #[gpui::test]
    fn a_selection_across_lines_copies_with_its_line_break(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        cx.simulate_input("one");
        cx.simulate_keystrokes("shift-enter");
        cx.simulate_input("two");
        // Up with shift selects to the column above, and left one more.
        cx.simulate_keystrokes("shift-up");
        cx.simulate_keystrokes("secondary-c");
        assert_eq!(clipboard(cx).as_deref(), Some("\ntwo"));
        cx.simulate_keystrokes("shift-left secondary-c");
        assert_eq!(clipboard(cx).as_deref(), Some("e\ntwo"));
        // Cut takes the break with it.
        cx.simulate_keystrokes("secondary-x");
        assert_eq!(text(&line, cx), "on");
        assert_eq!(shown(&line, cx).0, 1);
    }

    #[gpui::test]
    fn pasting_an_image_emits_it_and_keeps_the_text(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        line.update(cx, |line, _| line.images = true);
        let sent = Rc::new(Cell::new(0));
        let count = sent.clone();
        let bytes = include_bytes!("fixtures/shot-page.png");
        cx.update(|_, cx| {
            cx.subscribe(&line, move |_, pasted: &Pasted, _| {
                let image = match pasted {
                    Pasted::Image(image) => image,
                    #[cfg(target_os = "macos")]
                    Pasted::Native(_) => panic!("expected gpui image"),
                };
                assert_eq!(image.bytes(), bytes);
                count.set(count.get() + 1);
            })
            .detach();
        });
        cx.simulate_input("Like this");
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
            gpui::ImageFormat::Png,
            bytes.to_vec(),
        )));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(sent.get(), 1);
        assert_eq!(text(&line, cx), "Like this");
    }

    #[gpui::test]
    fn text_fields_do_not_emit_image_attachments(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        let image = Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("fixtures/shot-page.png").to_vec(),
        );
        cx.update(|_, cx| {
            cx.subscribe(&line, |_, _: &Pasted, _| {
                panic!("text field emitted an attachment")
            })
            .detach();
        });
        cx.write_to_clipboard(ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&line, cx), "");
        cx.write_to_clipboard(ClipboardItem::new_string("screenshot.png".into()));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&line, cx), "screenshot.png");
    }

    #[gpui::test]
    fn pasting_text_with_line_breaks_keeps_them(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        cx.simulate_input("> ");
        cx.write_to_clipboard(ClipboardItem::new_string("one\r\ntwo\n\nfour".to_string()));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&line, cx), "> one\ntwo\n\nfour");
        assert_eq!(shown(&line, cx).0, 4);
    }

    #[gpui::test]
    fn the_input_method_composes_on_a_second_line(cx: &mut TestAppContext) {
        let (line, cx) = opened(cx);
        cx.simulate_input("one");
        cx.simulate_keystrokes("shift-enter");
        cx.simulate_input("é ");
        // The input method marks what it composes, after "é " of the
        // second line: it counts in UTF-16.
        line.update_in(cx, |line, window, cx| {
            line.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            assert_eq!(line.marked_text_range(window, cx), Some(6..7));
        });
        assert_eq!(text(&line, cx), "one\né に");
        let (rows, row, _, _) = shown(&line, cx);
        assert_eq!(rows, 2);
        // The candidates go under the marked text: it is in the second
        // row, right of what is before it there.
        line.update_in(cx, |line, window, cx| {
            let bounds = line.last_bounds.unwrap();
            let marked = line.bounds_for_range(6..7, bounds, window, cx).unwrap();
            assert_eq!(marked.top(), bounds.top() + px(row));
            assert_eq!(marked.size.height, px(row));
            assert!(marked.left() > bounds.left());
            assert!(marked.size.width > px(0.));
            let before = line.bounds_for_range(4..6, bounds, window, cx).unwrap();
            assert_eq!(before.left(), bounds.left());
            assert_eq!(before.right(), marked.left());
            // And a point there is the marked character.
            assert_eq!(
                line.character_index_for_point(marked.center(), window, cx),
                Some(6)
            );

            // Composing on replaces what is marked, and its end commits.
            line.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
            assert_eq!(line.marked_text_range(window, cx), Some(6..8));
            line.replace_text_in_range(None, "日本語", window, cx);
            assert_eq!(line.marked_text_range(window, cx), None);
        });
        assert_eq!(text(&line, cx), "one\né 日本語");
        assert_eq!(cursor(&line, cx), "one\né 日本語".len());
    }

    /// A masked line alone in a window 200 wide and 300 high, with the
    /// focus.
    fn opened_masked(cx: &mut TestAppContext) -> (Entity<Line>, &mut VisualTestContext) {
        cx.update(bind_keys);
        let (line, cx) = cx.add_window_view(|window, cx| {
            let line = Line::masked(cx);
            window.focus(&line.focus_handle);
            line
        });
        cx.simulate_resize(size(px(200.), px(300.)));
        drawn(cx);
        (line, cx)
    }

    /// What the line last drew of its text.
    fn drawn_text(line: &Entity<Line>, cx: &mut VisualTestContext) -> String {
        drawn(cx);
        line.read_with(cx, |line, _| line.drawn().0)
    }

    #[gpui::test]
    fn a_masked_line_draws_a_mark_for_each_character_and_never_the_text(cx: &mut TestAppContext) {
        let (line, cx) = opened_masked(cx);
        // A character of several bytes, and one of several code points,
        // are a mark each.
        cx.simulate_input("s3cr\u{e9}t-e\u{301}");
        assert_eq!(text(&line, cx), "s3cr\u{e9}t-e\u{301}");
        assert_eq!(drawn_text(&line, cx), MARK.repeat(8));
        // The cursor is after the last mark, and a click on the first mark
        // puts it before the first character.
        let (bounds, row) = line.read_with(cx, |line, _| {
            let layout = line.last_layout.as_ref().unwrap();
            assert_eq!(
                layout.x(0, line.content.len()),
                layout.lines[0].unwrapped_layout.width
            );
            (line.last_bounds.unwrap(), f32::from(layout.line_height))
        });
        let at = point(bounds.left() + px(1.), bounds.top() + px(0.5 * row));
        cx.simulate_click(at, Modifiers::default());
        assert_eq!(cursor(&line, cx), 0);
        // The arrows and backspace go by characters of the text.
        cx.simulate_keystrokes("end left left backspace");
        assert_eq!(text(&line, cx), "s3cr\u{e9}-e\u{301}");
        assert_eq!(drawn_text(&line, cx), MARK.repeat(7));
        // The system's input method is handed none of it.
        line.update_in(cx, |line, window, cx| {
            assert_eq!(line.text_for_range(0..4, &mut None, window, cx), None);
        });
        // Taken, the line has it no longer.
        let taken = line.update(cx, |line, cx| line.take_text(cx));
        assert_eq!(taken, "s3cr\u{e9}-e\u{301}");
        assert_eq!(text(&line, cx), "");
        assert_eq!(drawn_text(&line, cx), "");
    }

    #[gpui::test]
    fn a_masked_line_takes_what_is_pasted_and_copies_nothing_out(cx: &mut TestAppContext) {
        let (line, cx) = opened_masked(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("pasted-k3y".to_string()));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&line, cx), "pasted-k3y");
        assert_eq!(drawn_text(&line, cx), MARK.repeat(10));
        // Copy and cut of all of it leave the clipboard as it was, and cut
        // leaves the text.
        cx.write_to_clipboard(ClipboardItem::new_string("before".to_string()));
        cx.simulate_keystrokes("secondary-a secondary-c");
        assert_eq!(clipboard(cx).as_deref(), Some("before"));
        cx.simulate_keystrokes("secondary-x");
        assert_eq!(clipboard(cx).as_deref(), Some("before"));
        assert_eq!(text(&line, cx), "pasted-k3y");
    }

    /// Shown, a masked line draws its text as typed, for a look, and
    /// still copies nothing out; hidden again, it draws marks.
    #[gpui::test]
    fn a_shown_masked_line_draws_its_text_and_still_copies_nothing_out(cx: &mut TestAppContext) {
        let (line, cx) = opened_masked(cx);
        cx.simulate_input("s3cret");
        assert_eq!(drawn_text(&line, cx), MARK.repeat(6));
        line.update(cx, |line, cx| line.set_shown(true, cx));
        assert_eq!(drawn_text(&line, cx), "s3cret");
        assert_eq!(text(&line, cx), "s3cret");
        // Copy and cut of all of it still leave the clipboard as it was,
        // and cut leaves the text.
        cx.write_to_clipboard(ClipboardItem::new_string("before".to_string()));
        cx.simulate_keystrokes("secondary-a secondary-c secondary-x");
        assert_eq!(clipboard(cx).as_deref(), Some("before"));
        assert_eq!(text(&line, cx), "s3cret");
        line.update(cx, |line, cx| line.set_shown(false, cx));
        assert_eq!(drawn_text(&line, cx), MARK.repeat(6));
    }

    #[gpui::test]
    fn a_masked_line_is_one_line_that_wraps_and_enter_hands_it_on(cx: &mut TestAppContext) {
        let (line, cx) = opened_masked(cx);
        let sent = Rc::new(Cell::new(0));
        cx.update(|_, cx| {
            let sent = sent.clone();
            cx.subscribe(&line, move |_, _: &Submitted, _| sent.set(sent.get() + 1))
                .detach();
        });
        // Shift and Enter break no line here.
        cx.simulate_input("one");
        cx.simulate_keystrokes("shift-enter");
        assert_eq!(text(&line, cx), "one");
        // A line break that is pasted is a mark as any character, in the
        // one line.
        cx.write_to_clipboard(ClipboardItem::new_string("\ntwo".to_string()));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&line, cx), "one\ntwo");
        assert_eq!(drawn_text(&line, cx), MARK.repeat(7));
        assert_eq!(shown(&line, cx).0, 1);
        // A value wider than the box goes on in the next row, every row of
        // marks, and the rows are the text's: the cursor at the end is in
        // the last, and home is that row's.
        let long = "k".repeat(120);
        line.update(cx, |line, cx| line.set_text(&long, cx));
        assert_eq!(drawn_text(&line, cx), MARK.repeat(120));
        let rows = shown(&line, cx).0;
        assert!(rows > 1, "{rows} rows");
        line.read_with(cx, |line, _| {
            let layout = line.last_layout.as_ref().unwrap();
            assert_eq!(layout.rows[0].range.start, 0);
            assert_eq!(layout.rows.last().unwrap().range.end, 120);
            for pair in layout.rows.windows(2) {
                assert_eq!(pair[0].range.end, pair[1].range.start);
            }
            assert_eq!(layout.row_of(120), rows - 1);
        });
        cx.simulate_keystrokes("home");
        let home = cursor(&line, cx);
        assert!(home > 0 && home < 120, "home at {home}");
        cx.simulate_keystrokes("up");
        assert!(cursor(&line, cx) < home);
        cx.simulate_keystrokes("enter");
        assert_eq!(sent.get(), 1);
        assert_eq!(text(&line, cx), long);
    }

    #[test]
    fn line_breaks_of_what_is_pasted_are_kept_as_one_kind() {
        assert_eq!(
            line_breaks("one\ntwo\r\nthree\rfour\n\nfive"),
            "one\ntwo\nthree\nfour\n\nfive"
        );
    }

    #[test]
    fn offsets_between_utf8_and_utf16() {
        // é is 2 bytes and 1 unit, 🚂 is 4 bytes and 2 units.
        let text = "aé🚂b";
        assert_eq!(offset_to_utf16(text, 0), 0);
        assert_eq!(offset_to_utf16(text, 3), 2);
        assert_eq!(offset_to_utf16(text, 7), 4);
        assert_eq!(offset_to_utf16(text, 8), 5);
        assert_eq!(offset_from_utf16(text, 2), 3);
        assert_eq!(offset_from_utf16(text, 4), 7);
        assert_eq!(offset_from_utf16(text, 99), text.len());
    }

    #[test]
    fn a_high_text_scrolls_with_the_cursor() {
        // It fits: nothing is moved.
        assert_eq!(scroll_to(40., 60., 80., 100., 0.), 0.);
        // The cursor's row is under the view: just into view.
        assert_eq!(scroll_to(240., 260., 300., 100., 0.), 160.);
        // In view already: left where it is.
        assert_eq!(scroll_to(200., 220., 300., 100., 160.), 160.);
        // Above the view: to the row's top.
        assert_eq!(scroll_to(120., 140., 300., 100., 160.), 120.);
        // The text got lower: no gap after its end.
        assert_eq!(scroll_to(90., 110., 110., 100., 160.), 10.);
        assert_eq!(scroll_to(0., 20., 20., 100., 160.), 0.);
        // The wheel stays within the text.
        assert_eq!(scroll_within(300., 100., 500.), 200.);
        assert_eq!(scroll_within(300., 100., -20.), 0.);
        assert_eq!(scroll_within(60., 100., 30.), 0.);
    }

    #[test]
    fn the_box_shows_its_rows_up_to_a_third_of_the_window() {
        assert_eq!(shown_rows(1, 20., 600.), 1);
        assert_eq!(shown_rows(7, 20., 600.), 7);
        assert_eq!(shown_rows(10, 20., 600.), 10);
        assert_eq!(shown_rows(11, 20., 600.), 10);
        assert_eq!(shown_rows(400, 20., 600.), 10);
        // A window lower than three rows still shows one.
        assert_eq!(shown_rows(5, 20., 50.), 1);
        assert_eq!(shown_rows(0, 20., 600.), 1);
    }
}
