//! The text of a message, which can be selected with the mouse: gpui draws
//! text and wraps it, and keeps no selection. The talk has one selection at
//! a time, so it is kept beside the texts and not in each of them; a text
//! draws its part of it and changes it with the mouse. A link in the text is
//! drawn as one, and a click on it opens it.

use std::ops::Range;

use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, Element, ElementId, Entity, GlobalElementId,
    HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyledText,
    TextLayout, UnderlineStyle, Window, combine_highlights, px,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::input::next_boundary;
use crate::theme::Palette;

/// What is selected in the talk, if anything is.
#[derive(Default)]
pub struct Selection {
    at: Option<Selected>,
}

struct Selected {
    /// The text it is in, and that text as it was when it was pressed on.
    key: u64,
    text: SharedString,
    /// What the press selected: a place, or a word. A drag selects from it.
    anchor: Range<usize>,
    range: Range<usize>,
    /// The link the press was on, until the pointer selects something: a
    /// release on it then is a click.
    link: Option<Range<usize>>,
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

    /// A press on the text `key` at `index`: what was selected is dropped,
    /// wherever it was. A second press selects the word there. `link` is
    /// the link under the pointer.
    fn press(
        &mut self,
        key: u64,
        text: SharedString,
        index: usize,
        link: Option<Range<usize>>,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        let anchor = if clicks == 2 {
            word_at(&text, index)
        } else {
            index..index
        };
        self.at = Some(Selected {
            key,
            text,
            // The first press of the two has opened it already.
            link: link.filter(|_| clicks == 1),
            range: anchor.clone(),
            anchor,
            selecting: true,
        });
        cx.notify();
    }

    fn drag(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(at) = self.at.as_mut() else {
            return;
        };
        let range = at.anchor.start.min(index)..at.anchor.end.max(index);
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
        let link = at.link.take()?;
        Some(at.text[link].to_string().into())
    }

    /// A press that was not on the text `key`: what is selected in it is
    /// dropped.
    fn pressed_outside(&mut self, key: u64, cx: &mut Context<Self>) {
        if self.at.as_ref().is_some_and(|at| at.key == key) {
            self.clear(cx);
        }
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

/// The link of `links` that the pointer at `position` is over: the
/// character under it counts here, not the closest boundary.
fn link_at(
    layout: &TextLayout,
    links: &[Range<usize>],
    position: Point<Pixels>,
) -> Option<Range<usize>> {
    let index = layout.index_for_position(position).ok()?;
    links.iter().find(|link| link.contains(&index)).cloned()
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

/// The text `text` of the talk, which `selection` may be in. `key` tells it
/// from the other texts.
pub fn text(key: u64, text: impl Into<SharedString>, selection: &Entity<Selection>) -> Text {
    let text = text.into();
    Text {
        key,
        links: links(&text),
        text,
        selection: selection.clone(),
    }
}

pub struct Text {
    key: u64,
    text: SharedString,
    links: Vec<Range<usize>>,
    selection: Entity<Selection>,
}

impl IntoElement for Text {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Text {
    /// gpui's own text: it shapes, wraps and draws.
    type RequestLayoutState = StyledText;
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
        let link = HighlightStyle {
            color: Some(palette.accent.into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(palette.accent.into()),
                wavy: false,
            }),
            ..Default::default()
        };
        let selected_style = HighlightStyle {
            background_color: Some(palette.selection.into()),
            ..Default::default()
        };
        let selected = self.selection.read(cx).range_in(self.key, &self.text);
        // A selection may begin or end inside a link: the two are merged
        // into runs that do not overlap.
        let mut styled = StyledText::new(self.text.clone()).with_highlights(combine_highlights(
            self.links.iter().cloned().map(|range| (range, link)),
            selected.map(|range| (range, selected_style)),
        ));
        let (layout, ()) = styled.request_layout(None, None, window, cx);
        (layout, styled)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        styled.prepaint(None, None, bounds, &mut (), window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        styled.paint(None, None, bounds, &mut (), &mut (), window, cx);
        // The link under the pointer, where the text is not covered.
        let over = {
            let (layout, hitbox, links) =
                (styled.layout().clone(), hitbox.clone(), self.links.clone());
            move |position: Point<Pixels>, window: &Window| {
                hitbox
                    .is_hovered(window)
                    .then(|| link_at(&layout, &links, position))
                    .flatten()
            }
        };
        let on_link = over(window.mouse_position(), window).is_some();
        let cursor = if on_link {
            CursorStyle::PointingHand
        } else {
            CursorStyle::IBeam
        };
        window.set_cursor_style(cursor, hitbox);

        let key = self.key;
        window.on_mouse_event({
            let (selection, text) = (self.selection.clone(), self.text.clone());
            let (layout, hitbox, over) = (styled.layout().clone(), hitbox.clone(), over.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                if hitbox.is_hovered(window) {
                    let index = index_at(&layout, &text, event.position);
                    let link = over(event.position, window);
                    selection.update(cx, |selection, cx| {
                        selection.press(key, text.clone(), index, link, event.click_count, cx)
                    });
                } else {
                    selection.update(cx, |selection, cx| selection.pressed_outside(key, cx));
                }
            }
        });
        // A drag goes on outside the text, so these are the window's events
        // and not those of the place the text is drawn in.
        window.on_mouse_event({
            let (selection, text) = (self.selection.clone(), self.text.clone());
            let (layout, over) = (styled.layout().clone(), over.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                // The pointer came onto a link or left it: the cursor is
                // set when the text is drawn.
                if over(event.position, window).is_some() != on_link {
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
                let index = index_at(&layout, &text, event.position);
                selection.update(cx, |selection, cx| selection.drag(index, cx));
            }
        });
        window.on_mouse_event({
            let (selection, text) = (self.selection.clone(), self.text.clone());
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
                let released = over(event.position, window).map(|link| &text[link]);
                if let Some(url) = pressed
                    && released == Some(url.as_ref())
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
