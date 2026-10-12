//! The text of a message, which can be selected with the mouse: gpui draws
//! text and wraps it, and keeps no selection. The talk has one selection at
//! a time, so it is kept beside the texts and not in each of them; a text
//! draws its part of it and changes it with the mouse. A link in the text is
//! drawn as one, and a click on it opens it.
//!
//! The text is drawn as the blocks its light Markdown makes of it, one
//! under the other and without their marks. What is selected is still a
//! stretch of the text as it was written: a copy has the marks.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Element, ElementId, Entity,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, SharedString, Style, StyledText, TextLayout, TextRun, TextStyle, TextStyleRefinement,
    UnderlineStyle, Window, fill, point, px, rems, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::input::next_boundary;
use crate::markdown::{self, Block, Kind};
use crate::theme::Palette;

/// What is selected in the talk, if anything is.
#[derive(Default)]
pub struct Selection {
    at: Option<Selected>,
    /// The block whose copy control reads "copied": the last block copied,
    /// by text key and block, until it reads "copy" again.
    copied: Option<(u64, usize)>,
    /// Which wait for "copied" to end is the current one: an older timer
    /// ends nothing.
    copied_generation: u64,
    /// Where each text's copy controls were last drawn, by text key and
    /// block: clicks there copy and select nothing.
    chips: Vec<(u64, Chips)>,
}

/// Where the copy controls of one text were last drawn, by block.
type Chips = Vec<(usize, Bounds<Pixels>)>;

struct Selected {
    /// The text it is in, and that text as it was when it was pressed on.
    key: u64,
    text: SharedString,
    /// What the press was on: a word, or a place. A place is empty, or the
    /// marks that stand there and are not drawn. A drag selects from it.
    anchor: Range<usize>,
    word: bool,
    range: Range<usize>,
    /// The link the press was on, until the pointer selects something: a
    /// release on it then is a click.
    link: Option<SharedString>,
    /// The button is still down.
    selecting: bool,
}

impl Selection {
    /// The selected text, with the line breaks it has.
    pub fn text(&self) -> Option<&str> {
        let at = self.at.as_ref().filter(|at| !at.range.is_empty())?;
        Some(&at.text[at.range.clone()])
    }

    /// Nothing is selected any more.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.at.take().is_some() {
            cx.notify();
        }
    }

    /// What of the text `key` is selected.
    fn range_in(&self, key: u64, text: &str) -> Option<Range<usize>> {
        let at = self.at.as_ref()?;
        (at.key == key && at.text.as_ref() == text && !at.range.is_empty())
            .then(|| at.range.clone())
    }

    fn selecting(&self, key: u64) -> bool {
        self.at
            .as_ref()
            .is_some_and(|at| at.key == key && at.selecting)
    }

    /// A press on the text `key` at `place`: what was selected is dropped,
    /// wherever it was. A second press selects the word there. `link` is
    /// the link under the pointer.
    fn press(
        &mut self,
        key: u64,
        text: SharedString,
        place: Range<usize>,
        link: Option<SharedString>,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        let word = clicks == 2;
        // The word after the marks that stand there.
        let anchor = if word {
            word_at(&text, place.end)
        } else {
            place
        };
        self.at = Some(Selected {
            key,
            text,
            // The first press of the two has opened it already.
            link: link.filter(|_| clicks == 1),
            range: anchor.start..if word { anchor.end } else { anchor.start },
            anchor,
            word,
            selecting: true,
        });
        cx.notify();
    }

    /// The pointer is at `place` with the button down. What is selected
    /// goes from where the press was to there, and has the marks between
    /// the two and none at its ends: bold words selected alone are copied
    /// as the words, and with what is around them as they were written.
    fn drag(&mut self, place: Range<usize>, cx: &mut Context<Self>) {
        let Some(at) = self.at.as_mut() else {
            return;
        };
        let (from, to) = if at.word {
            (at.anchor.start, at.anchor.end)
        } else {
            (at.anchor.end, at.anchor.start)
        };
        let range = if place.start >= at.anchor.end {
            from..place.start
        } else if place.end <= at.anchor.start {
            place.end..to
        } else if at.word {
            at.anchor.clone()
        } else {
            at.anchor.start..at.anchor.start
        };
        if at.range != range {
            at.range = range;
            // A drag selects, also when it comes back to where it began.
            at.link = None;
            cx.notify();
        }
    }

    /// The button came up: the link that was pressed on, if nothing was
    /// dragged since.
    fn release(&mut self) -> Option<SharedString> {
        let at = self.at.as_mut()?;
        at.selecting = false;
        at.link.take()
    }

    /// A press that was not on the text `key`: what is selected in it is
    /// dropped.
    fn pressed_outside(&mut self, key: u64, cx: &mut Context<Self>) {
        if self.at.as_ref().is_some_and(|at| at.key == key) {
            self.clear(cx);
        }
    }

    /// The block of the text `key` whose copy control reads "copied", if
    /// one still does.
    #[cfg(test)]
    pub fn copied_in(&self, key: u64) -> Option<usize> {
        self.copied
            .filter(|(at, _)| *at == key)
            .map(|(_, block)| block)
    }

    /// Where the copy controls of the text `key` were last drawn, by
    /// block.
    #[cfg(test)]
    pub fn chips_in(&self, key: u64) -> Chips {
        self.chips
            .iter()
            .find(|(at, _)| *at == key)
            .map(|(_, chips)| chips.clone())
            .unwrap_or_default()
    }

    /// The copy control of the block `block` of the text `key` was
    /// clicked: it reads "copied" for a short while, then "copy" again.
    fn copied_block(&mut self, key: u64, block: usize, cx: &mut Context<Self>) {
        self.copied_generation += 1;
        let generation = self.copied_generation;
        self.copied = Some((key, block));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            this.update(cx, |selection, cx| {
                if selection.copied_generation == generation {
                    selection.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The copy controls of the text `key` stand at `chips`: clicks there
    /// copy. Noted while painting, so it notifies nothing.
    fn note_chips(&mut self, key: u64, chips: Chips) {
        match self.chips.iter_mut().find(|(at, _)| *at == key) {
            Some((_, at)) => *at = chips,
            None => self.chips.push((key, chips)),
        }
    }

    /// The text and block whose copy control `position` is on, if any.
    fn chip_at(&self, position: Point<Pixels>) -> Option<(u64, usize)> {
        self.chips.iter().find_map(|(key, chips)| {
            chips
                .iter()
                .find(|(_, chip)| chip.contains(&position))
                .map(|(block, _)| (*key, *block))
        })
    }
}

/// The word of `text` that `index` is in.
pub fn word_at(text: &str, index: usize) -> Range<usize> {
    text.split_word_bound_indices()
        .map(|(start, word)| start..start + word.len())
        .find(|word| word.contains(&index))
        .unwrap_or(index..index)
}

/// What ends a sentence or closes a bracket after a link, and is no part of
/// it.
const AFTER_A_LINK: [char; 11] = ['.', ',', ';', ':', '!', '?', ')', ']', '>', '"', '\''];

/// The links in `text`: each starts with `http://` or `https://` and goes to
/// the next whitespace, without what closes the sentence or the bracket it
/// stands in. Nothing else is a link, so only these two are ever opened.
pub fn links(text: &str) -> Vec<Range<usize>> {
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find("http").map(|at| from + at) {
        let rest = &text[start..];
        let Some(scheme) = ["http://", "https://"]
            .into_iter()
            .find(|scheme| rest.starts_with(scheme))
        else {
            from = start + "http".len();
            continue;
        };
        let run = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let link = rest[..run].trim_end_matches(AFTER_A_LINK);
        // The scheme alone names nothing.
        if link.len() > scheme.len() {
            links.push(start..start + link.len());
        }
        from = start + run;
    }
    links
}

/// Where in `text` a pointer at `position` is: the boundary between two
/// characters that is closest to it, as in the typed line. Above the text
/// that is its start, below it its end.
fn index_at(layout: &TextLayout, text: &str, position: Point<Pixels>) -> usize {
    let (index, under) = match layout.index_for_position(position) {
        Ok(index) => (index, true),
        Err(index) => (index, false),
    };
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    // gpui says which character the pointer is over: past its middle, the
    // boundary after it is the closer one.
    if under {
        let next = next_boundary(text, index);
        if let (Some(here), Some(there)) = (
            layout.position_for_index(index),
            layout.position_for_index(next),
        ) && here.y == there.y
            && position.x > (here.x + there.x) / 2.
        {
            return next;
        }
    }
    index
}

/// A block of a text, as it is drawn.
struct Shown {
    block: Block,
    /// The block's text, as gpui takes it, and the links in it.
    text: SharedString,
    links: Vec<Range<usize>>,
}

/// The text `text` of the talk, which `selection` may be in. `key` tells it
/// from the other texts.
pub fn text(key: u64, text: impl Into<SharedString>, selection: &Entity<Selection>) -> Text {
    let text = text.into();
    let blocks = markdown::blocks(&text)
        .into_iter()
        .map(|block| Shown {
            text: block.text.clone().into(),
            links: links(&block.text),
            block,
        })
        .collect();
    Text {
        key,
        text,
        blocks,
        selection: selection.clone(),
    }
}

/// A key for a text that has no number of its own, from what it is: a
/// mail's is its number in the yard.
pub fn key(of: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    of.hash(&mut hasher);
    hasher.finish()
}

/// How far an item's words stand in from the edge of the text, in rems: the
/// marker is in that space. An item under another stands in twice as far.
pub const ITEM_INDENT: f32 = 1.5;
/// Between a marker and its item's words.
const MARKER_GAP: f32 = 0.4;
/// Around the lines of a code block, inside its ground. The left is wider:
/// the mark of a line that was wrapped stands there.
const CODE_LEFT: f32 = 1.;
const CODE_RIGHT: f32 = 0.625;
const CODE_ABOVE: f32 = 0.375;
/// Above a block that a blank line stood before, and above one that is a
/// heading or beside code.
const APART: f32 = 0.6;
const NEAR: f32 = 0.3;
/// The mark before the rest of a line of code that did not fit the width:
/// code is wrapped and not scrolled, and the mark says the break is not one
/// of the code's own.
const WRAPPED: &str = "↪";
/// How far the copy control of a code block stands in from the top right
/// of its ground, and how large it is, in rems: room for "copied" at any
/// text size.
const COPY_INSET: f32 = 0.25;
const COPY_WIDTH: f32 = 4.;
const COPY_HEIGHT: f32 = 1.5;
/// How long a copy control reads "copied" after it was clicked.
const COPIED_FOR: Duration = Duration::from_secs(2);

/// A monospaced face this machine has, for code: the first of these.
const MONO: [&str; 7] = [
    "Menlo",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
    "Consolas",
    "Courier New",
];

/// The face code is drawn in. Without one of them it is the text's face,
/// and its ground alone tells it from the text.
fn mono(cx: &App) -> Option<SharedString> {
    static FACE: OnceLock<Option<&'static str>> = OnceLock::new();
    FACE.get_or_init(|| {
        let names = cx.text_system().all_font_names();
        MONO.into_iter()
            .find(|face| names.iter().any(|name| name == face))
    })
    .map(SharedString::new_static)
}

/// The face a block of `kind` is drawn in, where the text's is `base`: a
/// heading is bold and a little larger than the text, the first level the
/// most, and code is in `mono`. All of it goes with the text's size.
fn face(kind: &Kind, base: &TextStyle, mono: Option<&SharedString>, rem: Pixels) -> TextStyle {
    let mut face = base.clone();
    match kind {
        Kind::Heading(level) => {
            let larger = match level {
                1 => 1.25,
                2 => 1.15,
                _ => 1.05,
            };
            face.font_size = (base.font_size.to_pixels(rem) * larger).into();
            face.font_weight = FontWeight::BOLD;
        }
        Kind::Code => {
            if let Some(mono) = mono {
                face.font_family = mono.clone();
            }
        }
        Kind::Text | Kind::Item { .. } => {}
    }
    face
}

/// The runs `block` is drawn in, in the face `face` of its kind: bold,
/// italic and inline code as its marks say, with a quiet ground under the
/// code; a link as one; and `selected` of it on the selection's ground. A
/// selection may begin or end inside a link or a mark: the runs do not
/// overlap.
fn runs(
    block: &Block,
    face: &TextStyle,
    palette: &Palette,
    mono: Option<&SharedString>,
    links: &[Range<usize>],
    selected: Option<Range<usize>>,
) -> Vec<TextRun> {
    let selected = selected.filter(|selected| !selected.is_empty());
    let mut cuts = vec![0, block.text.len()];
    let marked = block.marks.iter().map(|(range, _)| range);
    for range in marked.chain(links).chain(&selected) {
        cuts.extend([range.start, range.end]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let within = |range: &Range<usize>, at: usize| range.start <= at && at < range.end;
    let mut runs = Vec::new();
    for cut in cuts.windows(2) {
        let (start, end) = (cut[0], cut[1]);
        let mut run = face.to_run(end - start);
        if let Some((_, marks)) = block.marks.iter().find(|(range, _)| within(range, start)) {
            if marks.bold {
                run.font.weight = FontWeight::BOLD;
            }
            if marks.italic {
                run.font.style = FontStyle::Italic;
            }
            if marks.code {
                if let Some(mono) = mono {
                    run.font.family = mono.clone();
                }
                run.background_color = Some(palette.code.into());
            }
        }
        if links.iter().any(|link| within(link, start)) {
            run.color = palette.accent.into();
            run.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(palette.accent.into()),
                wavy: false,
            });
        }
        if selected.as_ref().is_some_and(|s| within(s, start)) {
            run.background_color = Some(palette.selection.into());
        }
        runs.push(run);
    }
    // An empty text is one empty run, as gpui makes it.
    if runs.is_empty() {
        runs.push(face.to_run(0));
    }
    runs
}

/// The space around `block`, which stands under one of kind `before`: the
/// room above it, an item's indent, and the inside of a code block's
/// ground.
fn frame(block: &Block, before: Option<&Kind>) -> Style {
    let mut style = Style::default();
    if let Some(before) = before {
        let heading = matches!(block.kind, Kind::Heading(_));
        let above = if block.apart || heading {
            APART
        } else if block.kind == Kind::Code || *before == Kind::Code {
            NEAR
        } else {
            0.
        };
        style.margin.top = rems(above).into();
    }
    match &block.kind {
        Kind::Code => {
            style.padding.left = rems(CODE_LEFT).into();
            style.padding.right = rems(CODE_RIGHT).into();
            style.padding.top = rems(CODE_ABOVE).into();
            style.padding.bottom = rems(CODE_ABOVE).into();
        }
        Kind::Item { depth, .. } => {
            style.padding.left = rems(ITEM_INDENT * f32::from(depth + 1)).into();
        }
        Kind::Text | Kind::Heading(_) => {}
    }
    style
}

/// Where the rows begin that a line of the code `text` goes on in, after
/// its first: a line that did not fit the width.
fn wrapped_rows(layout: &TextLayout, text: &str) -> Vec<Point<Pixels>> {
    let mut rows = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        if let (Some(at), Some(wrapped)) = (
            layout.position_for_index(start),
            layout.line_layout_for_index(start),
        ) {
            for row in 1..=wrapped.wrap_boundaries.len() {
                rows.push(point(at.x, at.y + layout.line_height() * row as f32));
            }
        }
        start += line.len() + 1;
    }
    rows
}

/// The copy control of a code block: a fixed-size chip at the top right
/// of its ground. Fixed so a click lands where the control was drawn,
/// whatever it reads.
fn copy_chip(frame: Bounds<Pixels>, rem: Pixels) -> Bounds<Pixels> {
    let chip = size(rem * COPY_WIDTH, rem * COPY_HEIGHT);
    Bounds::new(
        point(
            frame.right() - rem * COPY_INSET - chip.width,
            frame.top() + rem * COPY_INSET,
        ),
        chip,
    )
}

/// The blocks of a text where they are drawn: what the mouse goes by.
#[derive(Clone)]
struct Drawn {
    blocks: Rc<[Shown]>,
    layouts: Rc<[TextLayout]>,
    /// Where each block's ground is drawn.
    frames: Rc<[Bounds<Pixels>]>,
    /// The copy control of each code block, by block.
    chips: Rc<[(usize, Bounds<Pixels>)]>,
}

impl Drawn {
    /// The block the pointer at `position` is in. Between two blocks it is
    /// the lower one, whose start is closest, and under the last the last.
    fn block_at(&self, position: Point<Pixels>) -> Option<usize> {
        let last = self.layouts.len().checked_sub(1)?;
        let under = |layout: &TextLayout| position.y < layout.bounds().bottom();
        Some(self.layouts.iter().position(under).unwrap_or(last))
    }

    /// Where in the text as it was written the pointer is: a place, with
    /// the marks that stand there and are not drawn.
    fn place_at(&self, position: Point<Pixels>) -> Range<usize> {
        let Some(ix) = self.block_at(position) else {
            return 0..0;
        };
        let shown = &self.blocks[ix];
        let index = index_at(&self.layouts[ix], &shown.text, position);
        shown.block.source_at(index)
    }

    /// The link the pointer is over: the character under it counts here,
    /// not the closest boundary.
    fn link_at(&self, position: Point<Pixels>) -> Option<SharedString> {
        let ix = self.block_at(position)?;
        let shown = &self.blocks[ix];
        let index = self.layouts[ix].index_for_position(position).ok()?;
        let link = shown.links.iter().find(|link| link.contains(&index))?;
        Some(shown.text[link.clone()].to_string().into())
    }

    /// The code block the pointer is over, if it is over one's ground:
    /// its copy control is drawn.
    fn hovered_code(&self, position: Point<Pixels>) -> Option<usize> {
        self.chips
            .iter()
            .find_map(|(ix, _)| self.frames[*ix].contains(&position).then_some(*ix))
    }

    /// The code block whose copy control the pointer is on, if any.
    fn copy_at(&self, position: Point<Pixels>) -> Option<usize> {
        self.chips
            .iter()
            .find(|(_, chip)| chip.contains(&position))
            .map(|(ix, _)| *ix)
    }
}

pub struct Text {
    key: u64,
    /// The text as it was written: what is selected is a stretch of this.
    text: SharedString,
    blocks: Rc<[Shown]>,
    selection: Entity<Selection>,
}

/// A block of a text where it is laid out.
pub struct Laid {
    /// gpui's own text: it shapes, wraps and draws.
    styled: StyledText,
    text: LayoutId,
    /// The space around the text, and where it is once that is known.
    frame: LayoutId,
    bounds: Bounds<Pixels>,
}

impl IntoElement for Text {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Text {
    type RequestLayoutState = Vec<Laid>;
    type PrepaintState = Hitbox;

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
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let palette = Palette::now(window, cx);
        let (base, rem, mono) = (window.text_style(), window.rem_size(), mono(cx));
        let selected = self.selection.read(cx).range_in(self.key, &self.text);
        let mut laid = Vec::with_capacity(self.blocks.len());
        let mut before = None;
        for shown in self.blocks.iter() {
            let block = &shown.block;
            let face = face(&block.kind, &base, mono.as_ref(), rem);
            let selected = selected.clone().map(|selected| block.shown(selected));
            let runs = runs(
                block,
                &face,
                &palette,
                mono.as_ref(),
                &shown.links,
                selected,
            );
            let mut styled = StyledText::new(shown.text.clone()).with_runs(runs);
            // gpui takes the size of the lines from the window, not from
            // the runs.
            let size = TextStyleRefinement {
                font_size: Some(face.font_size),
                ..Default::default()
            };
            let (text, ()) = window.with_text_style(Some(size), |window| {
                styled.request_layout(None, None, window, cx)
            });
            let frame = window.request_layout(frame(block, before), [text], cx);
            before = Some(&block.kind);
            laid.push(Laid {
                styled,
                text,
                frame,
                bounds: Bounds::default(),
            });
        }
        let frames: Vec<LayoutId> = laid.iter().map(|laid| laid.frame).collect();
        (window.request_layout(Style::default(), frames, cx), laid)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        laid: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        for laid in laid.iter_mut() {
            let text = window.layout_bounds(laid.text);
            laid.styled.prepaint(None, None, text, &mut (), window, cx);
            laid.bounds = window.layout_bounds(laid.frame);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        laid: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let palette = Palette::now(window, cx);
        let (base, rem) = (window.text_style(), window.rem_size());
        let size = base.font_size.to_pixels(rem);
        let key = self.key;
        // What stands beside a block's text and is no part of it: it is
        // not selected, and a copy has what was written there.
        let beside = |text: &'static str, color: gpui::Rgba, window: &mut Window| {
            let mut run = base.to_run(text.len());
            run.color = color.into();
            window
                .text_system()
                .shape_line(text.into(), size, &[run], None)
        };
        let drawn = Drawn {
            blocks: self.blocks.clone(),
            layouts: laid
                .iter()
                .map(|laid| laid.styled.layout().clone())
                .collect(),
            frames: laid.iter().map(|laid| laid.bounds).collect(),
            chips: laid
                .iter()
                .zip(self.blocks.iter())
                .enumerate()
                .filter(|(_, (_, shown))| shown.block.kind == Kind::Code)
                .map(|(ix, (laid, _))| (ix, copy_chip(laid.bounds, rem)))
                .collect(),
        };
        // Where the copy controls stand, for clicks anywhere in the talk:
        // a click on one copies and selects nothing, in any text.
        self.selection.update(cx, |selection, _| {
            selection.note_chips(key, drawn.chips.to_vec())
        });
        for (laid, shown) in laid.iter_mut().zip(self.blocks.iter()) {
            let layout = laid.styled.layout().clone();
            let text = layout.bounds();
            match &shown.block.kind {
                Kind::Code => {
                    window.paint_quad(fill(laid.bounds, palette.code).corner_radii(rem * 0.25));
                    let rows = wrapped_rows(&layout, &shown.text);
                    if !rows.is_empty() {
                        let mark = beside(WRAPPED, palette.faint, window);
                        let left = text.left() - (rem * CODE_LEFT + mark.width) / 2.;
                        for row in rows {
                            let at = point(left, row.y);
                            let _ = mark.paint(at, layout.line_height(), window, cx);
                        }
                    }
                }
                Kind::Item { marker, .. } => {
                    let mut run = base.to_run(marker.len());
                    run.color = palette.faint.into();
                    let marker =
                        window
                            .text_system()
                            .shape_line(marker.clone().into(), size, &[run], None);
                    let at = point(text.left() - rem * MARKER_GAP - marker.width, text.top());
                    let _ = marker.paint(at, layout.line_height(), window, cx);
                }
                Kind::Text | Kind::Heading(_) => {}
            }
            laid.styled
                .paint(None, None, text, &mut (), &mut (), window, cx);
        }

        // The copy control of a code block shows while the pointer is over
        // the block, and after a click reads "copied" for a short while.
        let copied = self.selection.read(cx).copied;
        let hovered = hitbox
            .is_hovered(window)
            .then(|| drawn.hovered_code(window.mouse_position()))
            .flatten();
        for (ix, chip) in drawn.chips.iter() {
            let just_copied = copied == Some((key, *ix));
            if Some(*ix) != hovered && !just_copied {
                continue;
            }
            let label = if just_copied { "copied" } else { "copy" };
            window.paint_quad(fill(*chip, palette.side).corner_radii(rem * 0.25));
            let mut run = base.to_run(label.len());
            let color = if just_copied {
                palette.accent
            } else {
                palette.faint
            };
            run.color = color.into();
            let shaped = window
                .text_system()
                .shape_line(label.into(), size, &[run], None);
            let height = drawn.layouts[*ix].line_height();
            let at = point(
                chip.left() + (chip.size.width - shaped.width) / 2.,
                chip.top() + (chip.size.height - height) / 2.,
            );
            let _ = shaped.paint(at, height, window, cx);
        }

        // The link under the pointer, where the text is not covered.
        let over = {
            let (drawn, hitbox) = (drawn.clone(), hitbox.clone());
            move |position: Point<Pixels>, window: &Window| {
                hitbox
                    .is_hovered(window)
                    .then(|| drawn.link_at(position))
                    .flatten()
            }
        };
        let on_link = over(window.mouse_position(), window).is_some();
        let on_copy = drawn.copy_at(window.mouse_position()).is_some();
        let cursor = if on_link || on_copy {
            CursorStyle::PointingHand
        } else {
            CursorStyle::IBeam
        };
        window.set_cursor_style(cursor, hitbox);

        window.on_mouse_event({
            let (selection, text) = (self.selection.clone(), self.text.clone());
            let (drawn, hitbox, over) = (drawn.clone(), hitbox.clone(), over.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                if hitbox.is_hovered(window) {
                    // A click on a copy control copies the block exactly
                    // as it was written, and starts no selection.
                    if let Some(block) = drawn.copy_at(event.position) {
                        if let Some(text) = drawn.blocks[block].block.copy_text() {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                            selection
                                .update(cx, |selection, cx| selection.copied_block(key, block, cx));
                        }
                        return;
                    }
                    let place = drawn.place_at(event.position);
                    let link = over(event.position, window);
                    selection.update(cx, |selection, cx| {
                        selection.press(key, text.clone(), place, link, event.click_count, cx)
                    });
                } else if selection.read(cx).chip_at(event.position).is_none() {
                    // A click on another text's copy control is no press
                    // outside either: it changes no selection.
                    selection.update(cx, |selection, cx| selection.pressed_outside(key, cx));
                }
            }
        });
        // A drag goes on outside the text, so these are the window's events
        // and not those of the place the text is drawn in.
        window.on_mouse_event({
            let (selection, drawn, hitbox, over) =
                (self.selection.clone(), drawn, hitbox.clone(), over.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                // The pointer came onto a link, a code block or a copy
                // control, or left one: the cursor and the controls are set
                // when the text is drawn.
                let hovered_now = hitbox
                    .is_hovered(window)
                    .then(|| drawn.hovered_code(event.position))
                    .flatten();
                if over(event.position, window).is_some() != on_link
                    || hovered_now != hovered
                    || drawn.copy_at(event.position).is_some() != on_copy
                {
                    window.refresh();
                }
                if !selection.read(cx).selecting(key) {
                    return;
                }
                // The button came up where the window did not see it, so
                // this is no click.
                if event.pressed_button != Some(MouseButton::Left) {
                    selection.update(cx, |selection, _| selection.release());
                    return;
                }
                let place = drawn.place_at(event.position);
                selection.update(cx, |selection, cx| selection.drag(place, cx));
            }
        });
        window.on_mouse_event({
            let selection = self.selection.clone();
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !selection.read(cx).selecting(key)
                {
                    return;
                }
                // A click on a link: pressed and released on it, and
                // nothing dragged between the two.
                let pressed = selection.update(cx, |selection, _| selection.release());
                if let Some(url) = pressed
                    && over(event.position, window).as_ref() == Some(&url)
                {
                    cx.open_url(&url);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_press_selects_the_word_under_it() {
        let text = "see the fence, José";
        assert_eq!(word_at(text, 0), 0..3);
        assert_eq!(word_at(text, 5), 4..7);
        // The comma is no part of the word before it.
        assert_eq!(&text[word_at(text, 12)], "fence");
        assert_eq!(&text[word_at(text, 13)], ",");
        assert_eq!(&text[word_at(text, 16)], "José");
        // Past the end there is none.
        assert_eq!(word_at(text, text.len()), text.len()..text.len());
    }

    /// The one block `text` is drawn as.
    fn one(text: &str) -> Block {
        let mut blocks = markdown::blocks(text);
        assert_eq!(blocks.len(), 1, "{text:?}");
        blocks.remove(0)
    }

    fn dark() -> Palette {
        Palette::of(crate::theme::Look::Dark, gpui::WindowAppearance::Dark)
    }

    /// The text's own style, as the window sets it: seven eighths of a rem.
    fn base() -> TextStyle {
        TextStyle {
            font_size: rems(0.875).into(),
            ..Default::default()
        }
    }

    fn mono_face() -> SharedString {
        "Menlo".into()
    }

    /// The runs of the one block of `text`, as (what is drawn, the run).
    fn drawn(text: &str, selected: Option<Range<usize>>) -> Vec<(String, TextRun)> {
        let block = one(text);
        let (base, mono) = (base(), mono_face());
        let face = face(&block.kind, &base, Some(&mono), px(16.));
        let links = links(&block.text);
        let selected = selected.map(|selected| block.shown(selected));
        let mut at = 0;
        runs(&block, &face, &dark(), Some(&mono), &links, selected)
            .into_iter()
            .map(|run| {
                let words = block.text[at..at + run.len].to_string();
                at += run.len;
                (words, run)
            })
            .collect()
    }

    #[test]
    fn a_text_without_a_mark_has_the_runs_it_had_before_there_were_marks() {
        let text = "see https://example.org/a now";
        let (block, base, palette) = (one(text), base(), dark());
        assert_eq!((&block.kind, block.text.as_str()), (&Kind::Text, text));
        let face = face(&block.kind, &base, Some(&mono_face()), px(16.));
        assert_eq!(face, base);

        // What gpui made of the link and the selection, when the text was
        // handed to it with the two as highlights.
        let link = gpui::HighlightStyle {
            color: Some(palette.accent.into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(palette.accent.into()),
                wavy: false,
            }),
            ..Default::default()
        };
        let selection = gpui::HighlightStyle {
            background_color: Some(palette.selection.into()),
            ..Default::default()
        };
        let before = |highlights: Vec<(Range<usize>, gpui::HighlightStyle)>| {
            let mut runs = Vec::new();
            let mut at = 0;
            for (range, highlight) in gpui::combine_highlights(highlights, None) {
                if at < range.start {
                    runs.push(base.to_run(range.start - at));
                }
                runs.push(base.clone().highlight(highlight).to_run(range.len()));
                at = range.end;
            }
            if at < text.len() {
                runs.push(base.to_run(text.len() - at));
            }
            runs
        };
        let now = |selected| runs(&block, &face, &palette, None, &links(text), selected);

        assert_eq!(now(None), before(vec![(4..25, link)]));
        assert_eq!(now(None).len(), 3);
        // A selection that begins before the link and ends in it.
        assert_eq!(
            now(Some(2..10)),
            before(vec![(4..25, link), (2..10, selection)])
        );
        assert_eq!(now(Some(2..10)).len(), 5);
        // Nothing in it: one run, the text's own.
        let plain = one("Land it today.");
        assert_eq!(
            runs(&plain, &base, &palette, None, &[], None),
            [base.to_run(14)]
        );
        assert_eq!(
            runs(&one(""), &base, &palette, None, &[], None),
            [base.to_run(0)]
        );
    }

    #[test]
    fn code_is_in_a_monospaced_face_on_a_quiet_ground() {
        let (base, mono, palette) = (base(), mono_face(), dark());
        let quiet = Some(palette.code.into());

        // A block: the face is the block's, and its ground is drawn under
        // all of it, not under each run.
        let mut blocks = markdown::blocks("Run:\n```sh\n  ls -la\n1. kept\n```");
        let code = blocks.remove(1);
        assert_eq!(
            (&code.kind, code.text.as_str()),
            (&Kind::Code, "  ls -la\n1. kept")
        );
        let face = face(&code.kind, &base, Some(&mono), px(16.));
        assert_eq!(face.font_family, mono);
        assert_eq!(face.font_size, base.font_size);
        assert_eq!(face.font_weight, base.font_weight);
        let all = runs(&code, &face, &palette, Some(&mono), &[], None);
        assert_eq!(all, [face.to_run(code.text.len())]);
        assert_eq!(
            (&all[0].font.family, all[0].background_color),
            (&mono, None)
        );
        let inside = frame(&code, Some(&Kind::Text)).padding;
        assert_eq!(inside.left, rems(CODE_LEFT).into());
        assert_eq!(inside.top, rems(CODE_ABOVE).into());

        // Inline: the run alone is in the face and on the ground.
        let all = drawn("run `yardr prime` now", None);
        let words: Vec<&str> = all.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(words, ["run ", "yardr prime", " now"]);
        let mut code = base.to_run(11);
        code.font.family = mono.clone();
        code.background_color = quiet;
        assert_eq!(all[0].1, base.to_run(4));
        assert_eq!(all[1].1, code);
        assert_eq!(all[2].1, base.to_run(4));

        // A machine without one of the faces has the ground alone.
        let block = one("run `yardr prime` now");
        let all = runs(&block, &base, &palette, None, &[], None);
        assert_eq!(all[1].font, base.to_run(0).font);
        assert_eq!(all[1].background_color, quiet);
    }

    #[test]
    fn a_code_block_s_copy_control_is_a_chip_at_its_ground_s_top_right() {
        let frame = Bounds::new(point(px(10.), px(20.)), size(px(300.), px(100.)));
        let chip = copy_chip(frame, px(16.));
        // Fixed size, whatever the control reads, and inside the ground.
        assert_eq!(chip.size, size(px(64.), px(24.)));
        assert_eq!(chip.origin, point(px(10. + 300. - 4. - 64.), px(20. + 4.)));
        assert!(frame.contains(&chip.origin));
        assert!(frame.contains(&chip.bottom_right()));
    }

    #[test]
    fn a_heading_is_bold_and_a_little_larger_than_the_text() {
        let (base, mono) = (base(), mono_face());
        let mut larger = Vec::new();
        for (text, level) in [("# One", 1), ("## Two", 2), ("### Three", 3)] {
            let block = one(text);
            assert_eq!(block.kind, Kind::Heading(level));
            let face = face(&block.kind, &base, Some(&mono), px(16.));
            assert_eq!(face.font_weight, FontWeight::BOLD);
            assert_eq!(face.font_family, base.font_family);
            let all = runs(&block, &face, &dark(), Some(&mono), &[], None);
            assert_eq!(all, [face.to_run(block.text.len())]);
            assert_eq!(all[0].font.weight, FontWeight::BOLD);
            larger.push(face.font_size.to_pixels(px(16.)));
        }
        // The text is 14 pixels at a rem of 16: never "extremely large".
        let text = px(14.);
        assert_eq!(larger, [text * 1.25, text * 1.15, text * 1.05]);
    }

    #[test]
    fn the_text_size_scales_a_heading_with_the_text() {
        let (base, heading) = (base(), Kind::Heading(1));
        for rem in [px(13.), px(16.), px(20.)] {
            let text = base.font_size.to_pixels(rem);
            let face = face(&heading, &base, None, rem);
            assert_eq!(face.font_size.to_pixels(rem), text * 1.25);
        }
        // What is around a block is in rems, and so goes with it too.
        let item = one("- one");
        let indent = gpui::DefiniteLength::from(rems(ITEM_INDENT));
        assert_eq!(frame(&item, None).padding.left, indent);
    }

    #[test]
    fn bold_and_italic_are_runs_of_the_text_s_face() {
        let base = base();
        let all = drawn("a **bold** and an *italic* and ***both***", None);
        let words: Vec<&str> = all.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(words, ["a ", "bold", " and an ", "italic", " and ", "both"]);
        let mut bold = base.to_run(4);
        bold.font.weight = FontWeight::BOLD;
        let mut italic = base.to_run(6);
        italic.font.style = FontStyle::Italic;
        let mut both = bold.clone();
        both.font.style = FontStyle::Italic;
        assert_eq!(all[0].1, base.to_run(2));
        assert_eq!(all[1].1, bold);
        assert_eq!(all[2].1, base.to_run(8));
        assert_eq!(all[3].1, italic);
        assert_eq!(all[5].1, both);
    }

    #[test]
    fn an_item_is_text_set_in_with_its_link_and_its_marks() {
        let (base, palette) = (base(), dark());
        let text = "- see **https://example.org/a** now";
        let block = one(text);
        let marker = "•".to_string();
        assert_eq!(block.kind, Kind::Item { marker, depth: 0 });
        assert_eq!(face(&block.kind, &base, None, px(16.)), base);
        // The link is found in what is drawn, without the stars after it.
        assert_eq!(linked(&block.text), ["https://example.org/a"]);
        let all = drawn(text, None);
        let words: Vec<&str> = all.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(words, ["see ", "https://example.org/a", " now"]);
        let link = &all[1].1;
        assert_eq!(link.font.weight, FontWeight::BOLD);
        assert_eq!(link.color, palette.accent.into());
        assert!(link.underline.is_some());

        // An item under another stands in twice as far, and the words of
        // both hang: the indent is the block's, not the first line's.
        let blocks = markdown::blocks("1. one\n   - under");
        let left = |ix: usize| frame(&blocks[ix], None).padding.left;
        assert_eq!(left(0), rems(ITEM_INDENT).into());
        assert_eq!(left(1), rems(2. * ITEM_INDENT).into());
    }

    #[test]
    fn what_is_selected_is_drawn_without_the_marks_it_has() {
        let palette = dark();
        // All of it, as it was written: the stars are not drawn.
        let text = "a **bold** b";
        let all = drawn(text, Some(0..text.len()));
        assert!(
            all.iter()
                .all(|(_, run)| run.background_color == Some(palette.selection.into()))
        );
        let words: String = all.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(words, "a bold b");
        // From inside the bold word on: the selection's ground is over the
        // code's.
        let all = drawn("`code` b", Some(3..8));
        let grounds: Vec<(&str, Option<gpui::Hsla>)> = all
            .iter()
            .map(|(words, run)| (words.as_str(), run.background_color))
            .collect();
        assert_eq!(
            grounds,
            [
                ("co", Some(palette.code.into())),
                ("de", Some(palette.selection.into())),
                (" b", Some(palette.selection.into())),
            ]
        );
    }

    #[gpui::test]
    fn a_selection_has_the_marks_between_its_ends_and_none_at_them(cx: &mut gpui::TestAppContext) {
        let src = "one\n```sh\nls -la\n```\na **bold** b";
        let blocks = markdown::blocks(src);
        let (code, last) = (&blocks[1], &blocks[2]);
        assert_eq!(last.text, "a bold b");
        use gpui::AppContext as _;
        let selection = cx.new(|_| Selection::default());
        let mut selected = |from: Range<usize>, to: Range<usize>, clicks: usize| {
            selection.update(cx, |selection, cx| {
                selection.press(1, src.into(), from, None, clicks, cx);
                selection.drag(to, cx);
                selection.text().map(str::to_string)
            })
        };

        // The bold word alone, either way: the word.
        let (start, end) = (last.source_at(2), last.source_at(6));
        assert_eq!(
            selected(start.clone(), end.clone(), 1).as_deref(),
            Some("bold")
        );
        assert_eq!(
            selected(end.clone(), start.clone(), 1).as_deref(),
            Some("bold")
        );
        // With what is around it: as it was written.
        let (first, after) = (last.source_at(0), last.source_at(8));
        assert_eq!(
            selected(first.clone(), after.clone(), 1).as_deref(),
            Some("a **bold** b")
        );
        // The code alone is the code, a command as it is typed.
        let (top, bottom) = (code.source_at(0), code.source_at(6));
        assert_eq!(
            selected(top.clone(), bottom.clone(), 1).as_deref(),
            Some("ls -la")
        );
        assert_eq!(selected(bottom, top, 1).as_deref(), Some("ls -la"));
        // Across the code block, from the text before it to the text after
        // it: the fences and the language are in it.
        assert_eq!(selected(0..0, after.clone(), 1).as_deref(), Some(src));
        assert_eq!(selected(after, 0..0, 1).as_deref(), Some(src));
        // A second press on the bold word selects it.
        assert_eq!(selected(start.clone(), start, 2).as_deref(), Some("bold"));
        // A press and no drag selects nothing.
        let none = selection.update(cx, |selection, cx| {
            selection.press(1, src.into(), 2..4, None, 1, cx);
            selection.text().map(str::to_string)
        });
        assert_eq!(none, None);
    }

    fn linked(text: &str) -> Vec<&str> {
        links(text).into_iter().map(|link| &text[link]).collect()
    }

    #[test]
    fn a_link_goes_from_its_scheme_to_the_next_whitespace() {
        assert_eq!(linked("https://a.example/x"), ["https://a.example/x"]);
        assert_eq!(linked("see http://a.example now"), ["http://a.example"]);
        assert_eq!(
            linked("one https://a.example/x?y=1&z=(2)#top\nand https://b.example/, both"),
            ["https://a.example/x?y=1&z=(2)#top", "https://b.example/"]
        );
        // A word before it without a space is no part of it.
        assert_eq!(linked("José:https://a.example"), ["https://a.example"]);
    }

    #[test]
    fn what_closes_the_sentence_or_the_bracket_is_no_part_of_a_link() {
        assert_eq!(
            linked("It is at https://a.example/x."),
            ["https://a.example/x"]
        );
        assert_eq!(
            linked("(see https://a.example/x)."),
            ["https://a.example/x"]
        );
        assert_eq!(linked("<https://a.example/x>"), ["https://a.example/x"]);
        assert_eq!(linked("'https://a.example/x'!?"), ["https://a.example/x"]);
        assert_eq!(
            linked("[\"https://a.example/x\"]; https://b.example:"),
            ["https://a.example/x", "https://b.example"]
        );
    }

    #[test]
    fn nothing_but_http_and_https_is_a_link() {
        assert!(linked("no link in this, José").is_empty());
        assert!(linked("").is_empty());
        assert!(linked("ftp://x and file:///x").is_empty());
        assert!(linked("www.a.example, jo@a.example, http and https").is_empty());
        // The scheme alone names nothing.
        assert!(linked("https:// and (http://).").is_empty());
        assert_eq!(linked("httpx http://a.example"), ["http://a.example"]);
    }
}
