//! The text of a message, which can be selected with the mouse: gpui draws
//! text and wraps it, and keeps no selection. The talk has one selection at
//! a time, so it is kept beside the texts and not in each of them; a text
//! draws its part of it and changes it with the mouse.

use std::ops::Range;

use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, Element, ElementId, Entity, GlobalElementId,
    HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyledText,
    TextLayout, Window,
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
    /// wherever it was. A second press selects the word there.
    fn press(
        &mut self,
        key: u64,
        text: SharedString,
        index: usize,
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
            cx.notify();
        }
    }

    fn release(&mut self) {
        if let Some(at) = self.at.as_mut() {
            at.selecting = false;
        }
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
    Text {
        key,
        text: text.into(),
        selection: selection.clone(),
    }
}

pub struct Text {
    key: u64,
    text: SharedString,
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
        let mut styled = StyledText::new(self.text.clone());
        if let Some(range) = self.selection.read(cx).range_in(self.key, &self.text) {
            let selected = HighlightStyle {
                background_color: Some(Palette::of(window.appearance()).selection.into()),
                ..Default::default()
            };
            styled = styled.with_highlights([(range, selected)]);
        }
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
        window.set_cursor_style(CursorStyle::IBeam, hitbox);

        let key = self.key;
        window.on_mouse_event({
            let (selection, text) = (self.selection.clone(), self.text.clone());
            let (layout, hitbox) = (styled.layout().clone(), hitbox.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                if hitbox.is_hovered(window) {
                    let index = index_at(&layout, &text, event.position);
                    selection.update(cx, |selection, cx| {
                        selection.press(key, text.clone(), index, event.click_count, cx)
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
            let layout = styled.layout().clone();
            move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || !selection.read(cx).selecting(key) {
                    return;
                }
                // The button came up where the window did not see it.
                if event.pressed_button != Some(MouseButton::Left) {
                    return selection.update(cx, |selection, _| selection.release());
                }
                let index = index_at(&layout, &text, event.position);
                selection.update(cx, |selection, cx| selection.drag(index, cx));
            }
        });
        window.on_mouse_event({
            let selection = self.selection.clone();
            move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && selection.read(cx).selecting(key)
                {
                    selection.update(cx, |selection, _| selection.release());
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
}
