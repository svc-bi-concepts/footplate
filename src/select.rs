//! The text of a message, which can be selected with the mouse: gpui draws
//! text and wraps it, and keeps no selection. The talk has one selection at
//! a time, so it is kept beside the texts and not in each of them; a text
//! draws its part of it and changes it with the mouse. A link in the text is
//! drawn as one, and a click on it opens it, as a click on an ask or bead
//! id the model resolved opens the card or the bead.
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
    AnyTooltip, AnyView, App, AppContext, Bounds, ClipboardItem, Context, CursorStyle,
    DispatchPhase, Element, ElementId, Entity, FontStyle, FontWeight, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString, Style,
    Styled, StyledText, TextLayout, TextRun, TextStyle, TextStyleRefinement, UnderlineStyle,
    Window, div, fill, point, px, rems, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::input::next_boundary;
use crate::markdown::{self, Block, Kind, Marks};
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

/// What a press was on that a release on the same opens: a link, or an
/// ask or bead id.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Clicked {
    Link(SharedString),
    Ref(RefTarget),
}

struct Selected {
    /// The text it is in, and that text as it was when it was pressed on.
    key: u64,
    text: SharedString,
    /// What the press was on: a word, or a place. A place is empty, or the
    /// marks that stand there and are not drawn. A drag selects from it.
    anchor: Range<usize>,
    word: bool,
    range: Range<usize>,
    /// What the press was on that a release on the same opens, until the
    /// pointer selects something: a drag is no click.
    clicked: Option<Clicked>,
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

    /// A posed selection, for the screenshots: `range` of the text `key`
    /// stands selected, as a drag over it would. A debug build's alone.
    #[cfg(debug_assertions)]
    pub fn pose(
        &mut self,
        key: u64,
        text: SharedString,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.at = Some(Selected {
            key,
            text,
            anchor: range.clone(),
            word: false,
            range,
            clicked: None,
            selecting: false,
        });
        cx.notify();
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
    /// wherever it was. A second press selects the word there. `clicked`
    /// is the link or reference under the pointer.
    fn press(
        &mut self,
        key: u64,
        text: SharedString,
        place: Range<usize>,
        clicked: Option<Clicked>,
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
            clicked: clicked.filter(|_| clicks == 1),
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
            at.clicked = None;
            cx.notify();
        }
    }

    /// The button came up: what was pressed on, if nothing was dragged
    /// since.
    fn release(&mut self) -> Option<Clicked> {
        let at = self.at.as_mut()?;
        at.selecting = false;
        at.clicked.take()
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

    /// How many copy controls are drawn, in every text.
    #[cfg(test)]
    pub fn chip_count(&self) -> usize {
        self.chips.iter().map(|(_, chips)| chips.len()).sum()
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

/// A link as it is drawn: `from` is where the full link stands in the
/// block's text, `shown` where its form stands in what gpui lays out,
/// and `url` what a click opens. A long link's form is its short one,
/// and the tooltip names it whole; a short link draws as written.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ShownLink {
    from: Range<usize>,
    shown: Range<usize>,
    url: SharedString,
    /// Whether the link draws in its short form: only then does the
    /// tooltip stand over it, since a short link already shows its
    /// whole URL, and only then do its bounds round out to it.
    short: bool,
}

/// What gpui lays out for the block's `text`, whose links are `links`:
/// the text with every long link in its short form, and each link's
/// spans in both. The spans stand in order, and never overlap.
fn shrink(text: &str, links: &[Range<usize>]) -> (String, Vec<ShownLink>) {
    let mut display = String::with_capacity(text.len());
    let mut shown = Vec::with_capacity(links.len());
    let mut at = 0;
    for link in links {
        display.push_str(&text[at..link.start]);
        let url = &text[link.clone()];
        let short = markdown::short_link(url);
        let start = display.len();
        display.push_str(&short);
        shown.push(ShownLink {
            from: link.clone(),
            shown: start..display.len(),
            short: short != url,
            url: url.to_string().into(),
        });
        at = link.end;
    }
    display.push_str(&text[at..]);
    (display, shown)
}

/// Where the display boundary `at` stands in the block's text. Inside
/// a shortened link at its start, and at its end where the boundary is
/// the link's end, so that selecting the short form selects the whole
/// link. Everywhere else the shift the shorter links before it made.
fn to_src(links: &[ShownLink], at: usize) -> usize {
    for link in links {
        // Ahead of the link, or in one that draws whole: the shift the
        // shorter links before it made.
        if at <= link.shown.start || !link.short && at < link.shown.end {
            return at + (link.from.start - link.shown.start);
        }
        // In a shortened link at its start, and at its end on its end.
        if at < link.shown.end {
            return link.from.start;
        }
        if at == link.shown.end {
            return link.from.end;
        }
    }
    match links.last() {
        Some(link) => at + (link.from.end - link.shown.end),
        None => at,
    }
}

/// Where the block-text range stands in what is drawn. A bound inside
/// a shortened link rounds out to the link's short form, so that what
/// is selected of a link highlights it whole. References never stand
/// inside a link, so theirs only shift.
fn to_display(links: &[ShownLink], range: Range<usize>) -> Range<usize> {
    // The shift before the link `at` stands ahead of: what the
    // shorter links before it took.
    let shift = |at: usize| {
        links.iter().find(|link| at < link.from.end).map_or_else(
            || {
                links
                    .last()
                    .map_or(0, |link| link.from.end - link.shown.end)
            },
            |link| link.from.start - link.shown.start,
        )
    };
    let start = match links
        .iter()
        .find(|link| link.short && link.from.start <= range.start && range.start < link.from.end)
    {
        Some(link) => link.shown.start,
        None => range.start - shift(range.start),
    };
    let end = match links
        .iter()
        .find(|link| link.short && link.from.start < range.end && range.end <= link.from.end)
    {
        Some(link) => link.shown.end,
        None => range.end - shift(range.end),
    };
    start..end
}

/// What opening a resolved reference shows: an ask's card, or a bead
/// whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefTarget {
    /// An ask's card: an open one opens in Decide, a dealt-with one where
    /// it stands.
    Ask { bead: String, ask: u64, open: bool },
    /// A bead whole, as the timetable shows it.
    Bead { bead: String },
}

/// A resolved reference in one of a text's blocks: where it stands in the
/// block's drawn text, and what opening it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefSpan {
    /// The block it stands in, as `markdown::blocks` numbers them.
    pub block: usize,
    pub range: Range<usize>,
    pub target: RefTarget,
}

/// What a click on a resolved reference does: the window opens the card
/// or the bead.
pub type RefClick = Rc<dyn Fn(RefTarget, &mut Window, &mut App)>;

/// An ask or bead id named in drawn text, before the model resolves it
/// against what the yard said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    /// Where it stands, a qualified ask's bead included.
    pub range: Range<usize>,
    pub kind: RefKind,
}

/// What the text names: an ask's number, with its bead where the text
/// says which bead's, or a bead id alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// `ask 6926`: the number alone.
    Ask(u64),
    /// `ask 18 on yd-crm`: the number with its bead, and where the bead
    /// stands, so the bead alone still opens where the ask is none the
    /// yard said.
    AskOn {
        ask: u64,
        bead: String,
        bead_range: Range<usize>,
    },
    /// `yardr-e9f0`: a bead id alone.
    Bead(String),
}

/// Whether the byte may stand inside a word an ask reference abuts: the
/// ask in `re-ask 5` is none, and neither is one in `asked 5`.
fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Whether the byte may stand inside a bead id: an id's own characters
/// with the `@` of another yard's.
fn bead_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@')
}

/// A bead id at `start`: its own characters with one `@peer` at most,
/// without what closes the sentence after it. None where no bead id
/// starts here: one has a dash, and neither it nor its peer is empty.
fn bead_token(text: &str, start: usize) -> Option<(String, usize)> {
    if !text
        .as_bytes()
        .get(start)
        .is_some_and(|b| b.is_ascii_alphanumeric())
    {
        return None;
    }
    let mut end = start;
    while end < text.len() && bead_byte(text.as_bytes()[end]) {
        end += 1;
    }
    let token = text[start..end].trim_end_matches(['.', '-', '@']);
    if token.is_empty() {
        return None;
    }
    let end = start + token.len();
    let (head, peer) = match token.split_once('@') {
        None => (token, None),
        Some((head, peer)) => (head, Some(peer)),
    };
    if peer.is_some_and(|peer| peer.is_empty() || peer.contains('@')) {
        return None;
    }
    if !head.contains('-') || head.starts_with('-') || head.starts_with('.') {
        return None;
    }
    Some((token.to_string(), end))
}

/// An ask reference at `start`: `ask` in any case, then whitespace, then
/// the number, then `on` with a bead id where one follows. None where no
/// reference starts here.
fn ask_at(text: &str, start: usize) -> Option<Ref> {
    let bytes = text.as_bytes();
    if start > 0 && word_byte(bytes[start - 1]) {
        return None;
    }
    if !text[start..]
        .get(..3)
        .is_some_and(|w| w.eq_ignore_ascii_case("ask"))
    {
        return None;
    }
    let mut at = start + 3;
    let spaces = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_whitespace())
        .count();
    if spaces == 0 {
        return None;
    }
    at += spaces;
    let digits = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    if bytes
        .get(at + digits)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return None;
    }
    let ask: u64 = text[at..at + digits].parse().ok()?;
    let end = at + digits;
    // `on` with a bead id where one follows; else the number alone.
    let mut after = end;
    after += bytes[after..]
        .iter()
        .take_while(|b| b.is_ascii_whitespace())
        .count();
    if text[after..]
        .get(..2)
        .is_some_and(|w| w.eq_ignore_ascii_case("on"))
        && bytes
            .get(after + 2)
            .is_some_and(|b| b.is_ascii_whitespace())
    {
        after += 2;
        after += bytes[after..]
            .iter()
            .take_while(|b| b.is_ascii_whitespace())
            .count();
        if let Some((bead, bead_end)) = bead_token(text, after) {
            return Some(Ref {
                range: start..bead_end,
                kind: RefKind::AskOn {
                    ask,
                    bead,
                    bead_range: after..bead_end,
                },
            });
        }
    }
    Some(Ref {
        range: start..end,
        kind: RefKind::Ask(ask),
    })
}

/// Whether `range` stands inside one of `ranges` at any character.
fn overlaps(ranges: &[Range<usize>], range: &Range<usize>) -> bool {
    ranges
        .iter()
        .any(|other| range.start < other.end && other.start < range.end)
}

/// The asks and bead ids `text` names: `ask 6926`, `ask 18 on yd-crm`,
/// and bead ids alone, each where they stand. A qualified ask's bead is
/// no bead id of its own, and a link's words are none of these: the link
/// opens whole.
pub fn refs(text: &str) -> Vec<Ref> {
    let links = links(text);
    // Past the character at `at`: every match ends on a boundary, and a
    // step past a miss must too.
    let step = |at: usize| text[at..].chars().next().map_or(1, |c| c.len_utf8());
    let mut out = Vec::new();
    let mut at = 0;
    while at + 3 <= text.len() {
        match ask_at(text, at) {
            Some(found) if !overlaps(&links, &found.range) => {
                at = found.range.end;
                out.push(found);
            }
            _ => at += step(at),
        }
    }
    let asked: Vec<Range<usize>> = out.iter().map(|found| found.range.clone()).collect();
    let mut at = 0;
    while at < text.len() {
        let starts = at == 0 || !bead_byte(text.as_bytes()[at - 1]);
        match starts.then(|| bead_token(text, at)).flatten() {
            Some((bead, end)) => {
                let range = at..end;
                if !overlaps(&links, &range) && !overlaps(&asked, &range) {
                    out.push(Ref {
                        range,
                        kind: RefKind::Bead(bead),
                    });
                }
                at = end;
            }
            None => at += step(at),
        }
    }
    out.sort_by_key(|found| found.range.start);
    out
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
    /// What gpui lays out: the block's text with every long link in its
    /// short form.
    display: SharedString,
    /// The links in both, each with the URL a click opens.
    links: Vec<ShownLink>,
    /// The resolved references in what is drawn, each with what opening
    /// it shows.
    refs: Vec<(Range<usize>, RefTarget)>,
    /// An exact copy payload for a code block whose displayed form was
    /// sanitized before it reached the markdown reader.
    copy_override: Option<SharedString>,
}

/// The text `text` of the talk, which `selection` may be in. `key` tells it
/// from the other texts.
pub fn text(key: u64, text: impl Into<SharedString>, selection: &Entity<Selection>) -> Text {
    let text = text.into();
    let blocks = markdown::blocks(&text);
    text_inner(key, text, blocks, None, Vec::new(), None, selection)
}

/// Like [`text`], but read as no Markdown: no fence, list, heading
/// or mark in it draws as one. An asker's marked words, which keep no
/// block.
pub fn verbatim(key: u64, text: impl Into<SharedString>, selection: &Entity<Selection>) -> Text {
    let text = text.into();
    let blocks = vec![markdown::verbatim(&text)];
    text_inner(key, text, blocks, None, Vec::new(), None, selection)
}

/// Like [`text`], but the first code block copies `copy_text` exactly. The
/// displayed text may have had control characters removed before it was
/// fenced; copying still returns the bytes the caller was asked to show.
pub fn text_with_code_copy(
    key: u64,
    text: impl Into<SharedString>,
    copy_text: impl Into<SharedString>,
    selection: &Entity<Selection>,
) -> Text {
    let text = text.into();
    let blocks = markdown::blocks(&text);
    text_inner(
        key,
        text,
        blocks,
        Some(copy_text.into()),
        Vec::new(),
        None,
        selection,
    )
}

/// Like [`text`], with resolved references: each stands in its block as a
/// chip, and a click on one calls `on_ref` with what it opens.
pub fn text_with_refs(
    key: u64,
    text: impl Into<SharedString>,
    refs: Vec<RefSpan>,
    on_ref: RefClick,
    selection: &Entity<Selection>,
) -> Text {
    let text = text.into();
    let blocks = markdown::blocks(&text);
    text_inner(key, text, blocks, None, refs, Some(on_ref), selection)
}

fn text_inner(
    key: u64,
    text: SharedString,
    blocks: Vec<Block>,
    mut code_copy: Option<SharedString>,
    refs: Vec<RefSpan>,
    on_ref: Option<RefClick>,
    selection: &Entity<Selection>,
) -> Text {
    let blocks = blocks
        .into_iter()
        .enumerate()
        .map(|(ix, block)| {
            let (display, links) = shrink(&block.text, &links(&block.text));
            Shown {
                copy_override: if block.kind == Kind::Code {
                    code_copy.take()
                } else {
                    None
                },
                display: display.into(),
                refs: refs
                    .iter()
                    .filter(|span| span.block == ix)
                    .map(|span| (to_display(&links, span.range.clone()), span.target.clone()))
                    .collect(),
                links,
                block,
            }
        })
        .collect();
    Text {
        key,
        text,
        blocks,
        on_ref,
        selection: selection.clone(),
        tip: None,
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
/// and its ground alone tells it from the text. The arrivals board's
/// times are drawn in it too.
pub(crate) fn mono(cx: &App) -> Option<SharedString> {
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

/// The runs `shown` is drawn in, in the face `face` of its kind:
/// bold, italic and inline code as its marks say, with a quiet ground
/// under the code; a link as one, in the link blue; a reference as a
/// chip, in the code's face on its ground but in the link blue, with
/// no underline; and `selected` of it on the selection's ground.
/// `selected` stands in
/// the text as it was written, as the selection keeps it; every other
/// range stands in what gpui lays out, the long links shortened. A
/// selection may begin or end inside a link or a mark: the runs do not
/// overlap.
fn runs(
    shown: &Shown,
    face: &TextStyle,
    palette: &Palette,
    mono: Option<&SharedString>,
    refs: &[Range<usize>],
    selected: Option<Range<usize>>,
) -> Vec<TextRun> {
    let block = &shown.block;
    let selected = selected
        .map(|selected| to_display(&shown.links, block.shown(selected)))
        .filter(|selected| !selected.is_empty());
    let marks: Vec<(Range<usize>, Marks)> = block
        .marks
        .iter()
        .map(|(range, marks)| (to_display(&shown.links, range.clone()), *marks))
        .collect();
    let links: Vec<Range<usize>> = shown.links.iter().map(|link| link.shown.clone()).collect();
    let mut cuts = vec![0, shown.display.len()];
    let marked = marks.iter().map(|(range, _)| range);
    for range in marked.chain(&links).chain(refs).chain(&selected) {
        cuts.extend([range.start, range.end]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let within = |range: &Range<usize>, at: usize| range.start <= at && at < range.end;
    let mut runs = Vec::new();
    for cut in cuts.windows(2) {
        let (start, end) = (cut[0], cut[1]);
        let mut run = face.to_run(end - start);
        if let Some((_, marks)) = marks.iter().find(|(range, _)| within(range, start)) {
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
            run.color = palette.link.into();
            run.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(palette.link.into()),
                wavy: false,
            });
        }
        if refs.iter().any(|reference| within(reference, start)) {
            if let Some(mono) = mono {
                run.font.family = mono.clone();
            }
            run.color = palette.link.into();
            run.background_color = Some(palette.code.into());
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
    /// the marks that stand there and are not drawn. A shortened link's
    /// short form stands for the whole link.
    fn place_at(&self, position: Point<Pixels>) -> Range<usize> {
        let Some(ix) = self.block_at(position) else {
            return 0..0;
        };
        let shown = &self.blocks[ix];
        let index = index_at(&self.layouts[ix], &shown.display, position);
        shown.block.source_at(to_src(&shown.links, index))
    }

    /// The full link under the pointer, where it draws shortened:
    /// what the tooltip names. The character under the pointer counts
    /// here, not the closest boundary.
    fn short_link_at(&self, position: Point<Pixels>) -> Option<SharedString> {
        let ix = self.block_at(position)?;
        let shown = &self.blocks[ix];
        let index = self.layouts[ix].index_for_position(position).ok()?;
        shown
            .links
            .iter()
            .find(|link| link.short && link.shown.contains(&index))
            .map(|link| link.url.clone())
    }

    /// The link or reference the pointer is over: the character under
    /// it counts here, not the closest boundary.
    fn clicked_at(&self, position: Point<Pixels>) -> Option<Clicked> {
        let ix = self.block_at(position)?;
        let shown = &self.blocks[ix];
        let index = self.layouts[ix].index_for_position(position).ok()?;
        if let Some(link) = shown.links.iter().find(|link| link.shown.contains(&index)) {
            return Some(Clicked::Link(link.url.clone()));
        }
        shown
            .refs
            .iter()
            .find(|(range, _)| range.contains(&index))
            .map(|(_, target)| Clicked::Ref(target.clone()))
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

/// The full link under the pointer, where it draws shortened: one
/// line in the tooltip's own style, wrapped where it runs longer than
/// the tooltip may.
struct LinkTip(SharedString);

impl Render for LinkTip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::now(window, cx);
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(palette.line)
            .bg(palette.ground)
            .text_sm()
            .text_color(palette.text)
            .max_w(rems(30.))
            .child(self.0.clone())
    }
}

pub struct Text {
    key: u64,
    /// The text as it was written: what is selected is a stretch of this.
    text: SharedString,
    blocks: Rc<[Shown]>,
    on_ref: Option<RefClick>,
    selection: Entity<Selection>,
    /// The tooltip of the link under the pointer, by its URL: made once,
    /// and shown while the pointer stays over that link.
    tip: Option<(SharedString, AnyView)>,
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

impl Text {
    /// The blocks where they are drawn: what the mouse goes by.
    fn drawn(&self, laid: &[Laid], rem: Pixels) -> Drawn {
        Drawn {
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
        }
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
            let refs: Vec<Range<usize>> =
                shown.refs.iter().map(|(range, _)| range.clone()).collect();
            let runs = runs(
                shown,
                &face,
                &palette,
                mono.as_ref(),
                &refs,
                selected.clone(),
            );
            let mut styled = StyledText::new(shown.display.clone()).with_runs(runs);
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
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        // The full link under the pointer, where it draws shortened.
        // Asked here and not when painting: the window takes tooltips
        // in prepaint alone.
        let drawn = self.drawn(laid, window.rem_size());
        let selecting = self.selection.read(cx).selecting(self.key);
        if !selecting
            && let Some(url) = hitbox
                .is_hovered(window)
                .then(|| drawn.short_link_at(window.mouse_position()))
                .flatten()
        {
            let view = match &self.tip {
                Some((at, view)) if at == &url => view.clone(),
                _ => {
                    let view: AnyView = cx.new(|_| LinkTip(url.clone())).into();
                    self.tip = Some((url.clone(), view.clone()));
                    view
                }
            };
            let (drawn, hitbox) = (drawn.clone(), hitbox.clone());
            window.set_tooltip(AnyTooltip {
                view,
                mouse_position: window.mouse_position(),
                check_visible_and_update: Rc::new(move |_, window, _| {
                    hitbox.is_hovered(window)
                        && drawn
                            .short_link_at(window.mouse_position())
                            .is_some_and(|at| at == url)
                }),
            });
        }
        hitbox
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
        let drawn = self.drawn(laid, rem);
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
                    let rows = wrapped_rows(&layout, &shown.display);
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
            // Said, not waiting: the text colour, never the accent.
            let color = if just_copied {
                palette.text
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

        // The link or reference under the pointer, where the text is
        // not covered.
        let over = {
            let (drawn, hitbox) = (drawn.clone(), hitbox.clone());
            move |position: Point<Pixels>, window: &Window| {
                hitbox
                    .is_hovered(window)
                    .then(|| drawn.clicked_at(position))
                    .flatten()
            }
        };
        let on_open = over(window.mouse_position(), window).is_some();
        let on_copy = drawn.copy_at(window.mouse_position()).is_some();
        let cursor = if on_open || on_copy {
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
                        let shown = &drawn.blocks[block];
                        let copy = shown
                            .copy_override
                            .as_ref()
                            .map(ToString::to_string)
                            .or_else(|| shown.block.copy_text().map(str::to_string));
                        if let Some(text) = copy {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            selection
                                .update(cx, |selection, cx| selection.copied_block(key, block, cx));
                        }
                        return;
                    }
                    let place = drawn.place_at(event.position);
                    let clicked = over(event.position, window);
                    selection.update(cx, |selection, cx| {
                        selection.press(key, text.clone(), place, clicked, event.click_count, cx)
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
                // The pointer came onto a link, a reference, a code
                // block or a copy control, or left one: the cursor and the
                // controls are set when the text is drawn.
                let hovered_now = hitbox
                    .is_hovered(window)
                    .then(|| drawn.hovered_code(event.position))
                    .flatten();
                if over(event.position, window).is_some() != on_open
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
            let on_ref = self.on_ref.clone();
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !selection.read(cx).selecting(key)
                {
                    return;
                }
                // A click on a link or a reference: pressed and released
                // on it, and nothing dragged between the two.
                let pressed = selection.update(cx, |selection, _| selection.release());
                let released = over(event.position, window);
                match (pressed, released) {
                    (Some(clicked), Some(at)) if clicked == at => match clicked {
                        Clicked::Link(url) => cx.open_url(&url),
                        Clicked::Ref(target) => {
                            if let Some(on_ref) = &on_ref {
                                on_ref(target, window, cx);
                            }
                        }
                    },
                    _ => {}
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
        drawn_refs(text, selected, &[])
    }

    /// The block as it is drawn, with no resolved references.
    fn shown_block(block: Block) -> Shown {
        let (display, links) = shrink(&block.text, &links(&block.text));
        Shown {
            block,
            display: display.into(),
            links,
            refs: Vec::new(),
            copy_override: None,
        }
    }

    /// The one block of `text` as it is drawn, with no resolved
    /// references.
    fn shown_of(text: &str) -> Shown {
        shown_block(one(text))
    }

    /// The runs of the one block of `text` with resolved references at
    /// `refs`, as (what is drawn, the run). `selected` stands in the
    /// text as it was written, as the selection keeps it.
    fn drawn_refs(
        text: &str,
        selected: Option<Range<usize>>,
        refs: &[Range<usize>],
    ) -> Vec<(String, TextRun)> {
        let shown = shown_of(text);
        let (base, mono) = (base(), mono_face());
        let face = face(&shown.block.kind, &base, Some(&mono), px(16.));
        let mut at = 0;
        runs(&shown, &face, &dark(), Some(&mono), refs, selected)
            .into_iter()
            .map(|run| {
                let words = shown.display[at..at + run.len].to_string();
                at += run.len;
                (words, run)
            })
            .collect()
    }

    #[test]
    fn a_text_without_a_mark_has_the_runs_it_had_before_there_were_marks() {
        let text = "see https://example.org/a now";
        let (base, palette) = (base(), dark());
        let shown = shown_of(text);
        let block = &shown.block;
        assert_eq!((&block.kind, block.text.as_str()), (&Kind::Text, text));
        let face = face(&block.kind, &base, Some(&mono_face()), px(16.));
        assert_eq!(face, base);

        // What gpui made of the link and the selection, when the text was
        // handed to it with the two as highlights.
        let link = gpui::HighlightStyle {
            color: Some(palette.link.into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(palette.link.into()),
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
        let now = |selected| runs(&shown, &face, &palette, None, &[], selected);

        assert_eq!(now(None), before(vec![(4..25, link)]));
        assert_eq!(now(None).len(), 3);
        // A selection that begins before the link and ends in it.
        assert_eq!(
            now(Some(2..10)),
            before(vec![(4..25, link), (2..10, selection)])
        );
        assert_eq!(now(Some(2..10)).len(), 5);
        // Nothing in it: one run, the text's own.
        let plain = shown_of("Land it today.");
        assert_eq!(
            runs(&plain, &base, &palette, None, &[], None),
            [base.to_run(14)]
        );
        let empty = shown_of("");
        assert_eq!(
            runs(&empty, &base, &palette, None, &[], None),
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
        let shown = shown_block(blocks.remove(1));
        let code = &shown.block;
        assert_eq!(
            (&code.kind, code.text.as_str()),
            (&Kind::Code, "  ls -la\n1. kept")
        );
        let face = face(&code.kind, &base, Some(&mono), px(16.));
        assert_eq!(face.font_family, mono);
        assert_eq!(face.font_size, base.font_size);
        assert_eq!(face.font_weight, base.font_weight);
        let all = runs(&shown, &face, &palette, Some(&mono), &[], None);
        assert_eq!(all, [face.to_run(code.text.len())]);
        assert_eq!(
            (&all[0].font.family, all[0].background_color),
            (&mono, None)
        );
        let inside = frame(code, Some(&Kind::Text)).padding;
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
        let shown = shown_of("run `yardr prime` now");
        let all = runs(&shown, &base, &palette, None, &[], None);
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
            let shown = shown_of(text);
            let block = &shown.block;
            assert_eq!(block.kind, Kind::Heading(level));
            let face = face(&block.kind, &base, Some(&mono), px(16.));
            assert_eq!(face.font_weight, FontWeight::BOLD);
            assert_eq!(face.font_family, base.font_family);
            let all = runs(&shown, &face, &dark(), Some(&mono), &[], None);
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
        assert_eq!(link.color, palette.link.into());
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

    /// A long link for the shortening: past the length that draws whole.
    const LONG: &str =
        "https://teams.microsoft.com/l/meetup-join/19%3ameeting_N2Q4YzQefBDwL/0?context=chat";

    #[test]
    fn a_long_link_draws_short_and_maps_to_the_whole_link() {
        let text = format!("call: {LONG} now");
        let (display, spans) = shrink(&text, &links(&text));
        assert_eq!(spans.len(), 1);
        let link = &spans[0];
        assert_eq!(&text[link.from.clone()], LONG);
        assert_eq!(&display[link.shown.clone()], markdown::short_link(LONG));
        assert_eq!(link.url.as_ref(), LONG);
        assert!(link.short);
        assert_eq!(display, format!("call: {} now", markdown::short_link(LONG)));
        // A short link draws as written, and carries no tooltip.
        let text = "see https://example.org/a now";
        let (display, spans) = shrink(text, &links(text));
        assert_eq!(display, text);
        assert_eq!(spans.len(), 1);
        assert!(!spans[0].short);
        // None at all: the text draws as it is.
        let (display, spans) = shrink("no link in this", &[]);
        assert_eq!(display, "no link in this");
        assert!(spans.is_empty());
    }

    #[test]
    fn display_and_text_boundaries_map_past_shortened_links() {
        let text = format!("call: {LONG} now, and https://example.org/a too");
        let (display, spans) = shrink(&text, &links(&text));
        assert_eq!(spans.len(), 2);
        let short = &spans[0];
        assert!(short.short);
        let plain = &spans[1];
        assert!(!plain.short);
        // Before the short link the two stand alike.
        assert_eq!(to_src(&spans, 0), 0);
        assert_eq!(to_src(&spans, short.shown.start), short.from.start);
        // Inside its short form at its start, and at its end on its end:
        // selecting the short form selects the whole link.
        assert_eq!(to_src(&spans, short.shown.start + 1), short.from.start);
        assert_eq!(to_src(&spans, short.shown.end - 1), short.from.start);
        assert_eq!(to_src(&spans, short.shown.end), short.from.end);
        // Past it the shift it made, and the short link after it stands
        // whole in both.
        let after = short.shown.end + 1;
        assert_eq!(
            &display[after..after + 1],
            &text[to_src(&spans, after)..][..1]
        );
        assert_eq!(to_src(&spans, plain.shown.start), plain.from.start);
        assert_eq!(to_src(&spans, plain.shown.end), plain.from.end);
        assert_eq!(to_src(&spans, display.len()), text.len());
        // Inside a link that draws whole the same character, shifted
        // past the shortened one.
        assert_eq!(to_src(&spans, plain.shown.start + 1), plain.from.start + 1);
        assert_eq!(
            to_display(&spans, plain.from.start + 1..plain.from.end - 1),
            plain.shown.start + 1..plain.shown.end - 1
        );
        // The other way: a bound inside the long link rounds out to its
        // short form, and every other bound only shifts.
        assert_eq!(to_display(&spans, short.from.clone()), short.shown.clone());
        assert_eq!(
            to_display(&spans, short.from.start + 1..short.from.end - 1),
            short.shown.clone()
        );
        assert_eq!(
            to_display(&spans, 0..short.from.start),
            0..short.shown.start
        );
        assert_eq!(to_display(&spans, plain.from.clone()), plain.shown.clone());
        assert_eq!(
            to_display(&spans, text.len()..text.len()),
            display.len()..display.len()
        );
        // A round trip past the links stands where it began.
        for at in [
            0,
            short.from.start,
            short.from.end,
            plain.from.start,
            text.len(),
        ] {
            let there = to_display(&spans, at..at);
            assert_eq!(there.start, there.end);
            assert_eq!(to_src(&spans, there.start), at, "{at}");
        }
    }

    /// What `refs` names in `text`: each where it stands, with what it is.
    fn reffed(text: &str) -> Vec<(&str, RefKind)> {
        refs(text)
            .into_iter()
            .map(|found| (&text[found.range], found.kind))
            .collect()
    }

    #[test]
    fn an_ask_is_its_number_after_ask() {
        assert_eq!(reffed("ask 6926"), [("ask 6926", RefKind::Ask(6926))]);
        assert_eq!(
            reffed("its card (ask 6926)."),
            [("ask 6926", RefKind::Ask(6926))]
        );
        // In any case, after any space, with leading zeroes.
        assert_eq!(reffed("Ask 007?"), [("Ask 007", RefKind::Ask(7))]);
        assert_eq!(reffed("ASK\t42"), [("ASK\t42", RefKind::Ask(42))]);
        // Two asks, in order.
        assert_eq!(
            reffed("ask 6893, not ask 6891."),
            [
                ("ask 6893", RefKind::Ask(6893)),
                ("ask 6891", RefKind::Ask(6891)),
            ]
        );
    }

    #[test]
    fn an_ask_with_its_bead_is_one_reference() {
        assert_eq!(
            reffed("ask 18 on yd-crm is declined"),
            [(
                "ask 18 on yd-crm",
                RefKind::AskOn {
                    ask: 18,
                    bead: "yd-crm".to_string(),
                    bead_range: 10..16,
                }
            )]
        );
        // The bead alone still opens where the ask is none the yard said.
        let found = &refs("ask 18 on yd-crm")[0];
        assert_eq!(
            &"ask 18 on yd-crm"[match &found.kind {
                RefKind::AskOn { bead_range, .. } => bead_range.clone(),
                _ => panic!("{found:?}"),
            }],
            "yd-crm"
        );
    }

    #[test]
    fn ask_without_a_number_is_no_ask() {
        assert!(refs("").is_empty());
        assert!(refs("ask yardmaster, ask about this").is_empty());
        assert!(refs("asked 5, asks 5, ask5, task 5").is_empty());
        assert!(refs("ask five, ask 5x, ask 99999999999999999999999").is_empty());
        // `on` without a bead id leaves the number alone.
        assert_eq!(reffed("ask 5 on Friday"), [("ask 5", RefKind::Ask(5))]);
        // `re-ask` is shaped as a bead id is: the model, which knows the
        // yard's beads, leaves it plain.
        assert_eq!(
            reffed("re-ask 5"),
            [("re-ask", RefKind::Bead("re-ask".to_string()))]
        );
    }

    #[test]
    fn a_bead_id_is_an_id_with_a_dash() {
        assert_eq!(
            reffed("over the limit (yardr-e9f0), so run:"),
            [("yardr-e9f0", RefKind::Bead("yardr-e9f0".to_string()))]
        );
        assert_eq!(
            reffed("see footplate-ky46 and yd-zulu."),
            [
                (
                    "footplate-ky46",
                    RefKind::Bead("footplate-ky46".to_string())
                ),
                ("yd-zulu", RefKind::Bead("yd-zulu".to_string())),
            ]
        );
        // Another yard's, with its peer; a short test id; what the
        // sentence puts after it is no part of it.
        assert_eq!(
            reffed("ask 3 on bead-a@north, bead-s, yd-zulu's turn."),
            [
                (
                    "ask 3 on bead-a@north",
                    RefKind::AskOn {
                        ask: 3,
                        bead: "bead-a@north".to_string(),
                        bead_range: 9..21,
                    }
                ),
                ("bead-s", RefKind::Bead("bead-s".to_string())),
                ("yd-zulu", RefKind::Bead("yd-zulu".to_string())),
            ]
        );
    }

    #[test]
    fn what_has_no_dash_is_no_bead_id() {
        assert!(refs("no bead here").is_empty());
        assert!(refs("clerk@north, jo, yardmaster@droplet").is_empty());
        assert!(refs("a@b@c-d, -x, up- -to- -date").is_empty());
        // Whole words: no bead id hides in a longer one.
        assert_eq!(
            reffed("xfootplate-ky46 stands whole"),
            [(
                "xfootplate-ky46",
                RefKind::Bead("xfootplate-ky46".to_string())
            )]
        );
    }

    #[test]
    fn a_link_s_words_are_no_reference() {
        assert!(refs("see https://example.org/ask 5 now").is_empty());
        assert_eq!(
            linked("see https://example.org/a-b now"),
            ["https://example.org/a-b"]
        );
        assert!(refs("see https://example.org/a-b now").is_empty());
        // Beside a link, a reference still stands.
        assert_eq!(
            reffed("ask 5: https://example.org/a"),
            [("ask 5", RefKind::Ask(5))]
        );
    }

    #[test]
    fn a_reference_is_a_chip_in_the_link_blue_on_the_code_s_ground() {
        let (palette, mono) = (dark(), mono_face());
        let text = "its card (ask 6926).";
        let span = [Range { start: 10, end: 18 }];
        let all = drawn_refs(text, None, &span);
        assert_eq!(all.len(), 3);
        let chip = &all[1];
        assert_eq!(chip.0, "ask 6926");
        assert_eq!(chip.1.color, palette.link.into());
        assert_eq!(chip.1.background_color, Some(palette.code.into()));
        assert_eq!(chip.1.font.family, mono);
        assert!(chip.1.underline.is_none());
        // A selection over it keeps the link blue on its own ground.
        let all = drawn_refs(text, Some(12..14), &span);
        let words: Vec<&str> = all.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(words, ["its card (", "as", "k ", "6926", ")."]);
        assert_eq!(all[2].1.color, palette.link.into());
        assert_eq!(all[2].1.background_color, Some(palette.selection.into()));
        // And a bold reference keeps its weight.
        let all = drawn_refs("its card (**ask 6926**).", None, &span);
        assert_eq!(all[1].0, "ask 6926");
        assert_eq!(all[1].1.font.weight, FontWeight::BOLD);
        assert_eq!(all[1].1.color, palette.link.into());
    }
}
