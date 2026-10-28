//! The small pieces the window draws with: its glyphs, the band of its
//! top, its buttons, its segments and its checkboxes.

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, FontWeight, Pixels, Rgba, SharedString, Stateful,
    Window, div, prelude::*, px, relative, rems,
};

use super::Footplate;
use crate::theme::Palette;

/// The switcher's mark beside the yard's line: a chevron drawn with two
/// strokes, in the line's own colour. A character would draw on one
/// desktop and stand in for a missing glyph on the next.
/// A rail glyph's canvas: 28 pixels, drawn from the mock's geometry
/// (docs/design/2026-10-16-footplate-zj5f-rail-mock.html), a 24-grid with
/// its strokes, scaled to the canvas. Fixed pixels, not rems, as the
/// labels under the glyphs are: the rail keeps its size whatever the
/// text.
pub(crate) fn glyph_canvas(
    paint: impl Fn(gpui::Point<Pixels>, f32, &mut Window) + 'static,
) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            paint(bounds.origin, f32::from(bounds.size.width) / 24., window);
        },
    )
    .size(px(28.))
    .flex_none()
}

/// A straight line on `path`, in the mock's grid: `o` is the glyph's
/// origin, `s` the grid's scale into it.
pub(crate) fn glyph_line(
    path: &mut gpui::PathBuilder,
    o: gpui::Point<Pixels>,
    s: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) {
    let at = |x: f32, y: f32| gpui::point(o.x + px(x * s), o.y + px(y * s));
    path.move_to(at(x1, y1));
    path.line_to(at(x2, y2));
}

/// A circle on `path`, stroked or filled as the path is: two half arcs.
pub(crate) fn glyph_circle(
    path: &mut gpui::PathBuilder,
    o: gpui::Point<Pixels>,
    s: f32,
    cx: f32,
    cy: f32,
    r: f32,
) {
    let at = |x: f32, y: f32| gpui::point(o.x + px(x * s), o.y + px(y * s));
    let radii = gpui::point(px(r * s), px(r * s));
    path.move_to(at(cx - r, cy));
    path.arc_to(radii, px(0.), false, true, at(cx + r, cy));
    path.arc_to(radii, px(0.), false, true, at(cx - r, cy));
}

/// The kit's track glyph: two rails with three sleepers, the
/// timetable's. Its strokes are the track-icon asset's (two rails 10
/// apart, three sleepers, 24 by 24).
pub(crate) fn track_glyph(color: Rgba) -> impl IntoElement {
    glyph_canvas(move |o, s, window| {
        let mut rails = gpui::PathBuilder::stroke(px(2. * s));
        glyph_line(&mut rails, o, s, 7., 1.5, 7., 22.5);
        glyph_line(&mut rails, o, s, 17., 1.5, 17., 22.5);
        if let Ok(path) = rails.build() {
            window.paint_path(path, color);
        }
        let mut sleepers = gpui::PathBuilder::stroke(px(3. * s));
        for y in [5.5, 12., 18.5] {
            glyph_line(&mut sleepers, o, s, 4.5, y, 19.5, y);
        }
        if let Ok(path) = sleepers.build() {
            window.paint_path(path, color);
        }
    })
}

/// A signal lamp on its post: chat's glyph, the signal that talks.
pub(crate) fn lamp_glyph(color: Rgba) -> impl IntoElement {
    glyph_canvas(move |o, s, window| {
        let mut path = gpui::PathBuilder::stroke(px(2. * s));
        glyph_line(&mut path, o, s, 12., 21., 12., 13.);
        glyph_circle(&mut path, o, s, 12., 7., 4.);
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    })
}

/// A semaphore arm, diagonal on its post: decisions' glyph, stop or go.
pub(crate) fn semaphore_glyph(color: Rgba) -> impl IntoElement {
    glyph_canvas(move |o, s, window| {
        let mut path = gpui::PathBuilder::stroke(px(2. * s));
        glyph_line(&mut path, o, s, 6., 21., 6., 4.);
        glyph_line(&mut path, o, s, 6., 5., 17., 11.);
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
        let mut pivot = gpui::PathBuilder::fill();
        glyph_circle(&mut pivot, o, s, 6., 4., 1.4);
        pivot.close();
        if let Ok(path) = pivot.build() {
            window.paint_path(path, color);
        }
    })
}

/// The settings' gear: a hub with a dashed ring for teeth, in the mock's
/// stroke.
pub(crate) fn gear_glyph(color: Rgba) -> impl IntoElement {
    glyph_canvas(move |o, s, window| {
        let mut hub = gpui::PathBuilder::stroke(px(2. * s));
        glyph_circle(&mut hub, o, s, 12., 12., 3.2);
        if let Ok(path) = hub.build() {
            window.paint_path(path, color);
        }
        let dashes = [px(3.2 * s), px(2.6 * s)];
        let mut teeth = gpui::PathBuilder::stroke(px(2. * s)).dash_array(&dashes);
        glyph_circle(&mut teeth, o, s, 12., 12., 7.);
        if let Ok(path) = teeth.build() {
            window.paint_path(path, color);
        }
    })
}

/// The terminal: Octicons', a rounded square holding a ">" prompt
/// and an underscore, on the mock's grid. The conversation header's
/// screen control; no icon font is loaded for the one glyph.
pub(crate) fn terminal_glyph(color: Rgba) -> impl IntoElement {
    glyph_canvas(move |o, s, window| {
        let mut path = gpui::PathBuilder::stroke(px(2. * s));
        // The rounded square, drawn clockwise from the top edge: the
        // corner arcs join the straight sides.
        let at = |x: f32, y: f32| gpui::point(o.x + px(x * s), o.y + px(y * s));
        let radii = gpui::point(px(4. * s), px(4. * s));
        path.move_to(at(7., 3.));
        path.line_to(at(17., 3.));
        path.arc_to(radii, px(0.), false, true, at(21., 7.));
        path.line_to(at(21., 17.));
        path.arc_to(radii, px(0.), false, true, at(17., 21.));
        path.line_to(at(7., 21.));
        path.arc_to(radii, px(0.), false, true, at(3., 17.));
        path.line_to(at(3., 7.));
        path.arc_to(radii, px(0.), false, true, at(7., 3.));
        path.close();
        glyph_line(&mut path, o, s, 9., 8.5, 12.5, 12.);
        glyph_line(&mut path, o, s, 12.5, 12., 9., 15.5);
        glyph_line(&mut path, o, s, 8., 17.5, 16., 17.5);
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    })
}

/// The window's top band, where the content reaches up into the title bar
/// and the traffic lights stand over the rail's top: `top` pixels high, with
/// the hairline at its foot. The rail, the list's head and the settings'
/// sheet each start with one, side by side, so the hairline is one line from
/// the window's left edge across all of them, and no line of theirs runs
/// down through the band, under the lights. It is a least height: the words
/// in it, at the larger text sizes, make it taller.
pub(crate) fn band(top: f32, palette: Palette) -> Div {
    div()
        .flex_none()
        .w_full()
        .min_h(px(top))
        .border_b_1()
        .border_color(palette.line)
}

/// A card standing over the window, as the About card and the Feedback form
/// do: `rows` in a card of the kit's popover shadow, centred, over an overlay
/// of the whole window that takes the mouse, so that a click outside the card
/// is `shut`'s and no click on what stands under it. Where the content
/// reaches up into the title bar (`top` pixels) the overlay starts under that
/// strip, which stays the window's handle: a drag there still moves it. The
/// overlay is found by `name`, and the card by `name card`.
pub(crate) fn card_layer(
    name: &'static str,
    top: f32,
    palette: Palette,
    shut: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    rows: Vec<AnyElement>,
) -> impl IntoElement {
    let card = format!("{name} card");
    div()
        .id(SharedString::from(format!("{name} overlay")))
        .debug_selector(move || name.to_string())
        // A click outside the card shuts it and is no click on what
        // stands under it.
        .block_mouse_except_scroll()
        .absolute()
        .top(px(top))
        .left(px(0.))
        .right(px(0.))
        .bottom(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .on_click(shut)
        .child(
            div()
                .id(SharedString::from(card.clone()))
                .debug_selector(move || card.clone())
                .w(rems(28.))
                .max_w(relative(0.9))
                .rounded_lg()
                .border_1()
                .border_color(palette.line)
                .bg(palette.side)
                // The kit's popover shadow, as the bar of reactions
                // has it: the card floats over the conversation's
                // lines, which it does not cover.
                .shadow(vec![gpui::BoxShadow {
                    color: palette.shadow.into(),
                    offset: gpui::point(px(0.), px(6.)),
                    blur_radius: px(20.),
                    spread_radius: px(0.),
                }])
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .on_click(|_, _, cx| cx.stop_propagation())
                .children(rows),
        )
}

/// Words that take a click, without the border: the status row at the
/// list's head ends in one. The hover's ground is what says so.
pub(crate) fn quiet_button(text: String, palette: Palette) -> Div {
    div()
        .px_1()
        .rounded_md()
        .text_size(rems(0.8))
        .text_color(palette.faint)
        .cursor_pointer()
        .hover(|style| style.bg(palette.hover))
        .child(text)
}

/// Why Send does nothing while the yard is reconnecting, beside the
/// control that sends: the draft stays as typed, and goes on Enter once
/// the yard answers.
pub(crate) fn send_held(palette: Palette) -> impl IntoElement {
    div()
        .debug_selector(|| "send held".to_string())
        .text_sm()
        .text_color(palette.faint)
        .child("sends when the yard answers")
}

/// A small button beside the send button, without what it does: Later and
/// Decline, with their keys.
pub(crate) fn small_button(text: String, palette: Palette) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(palette.line)
        .text_sm()
        .text_color(palette.faint)
        .cursor_pointer()
        .hover(|style| style.bg(palette.hover))
        .child(text)
}

/// "? all keys", which opens the cheatsheet of the open view's keys:
/// the hint where keys are listed in prose, and where `?` is the
/// line's text. It takes its size from where it stands. `id` names
/// the place, one per hint, and the test finds it by that name: two
/// hints can stand at once, as the switcher's does over a card's.
pub(crate) fn keys_hint(
    id: &'static str,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .text_color(palette.link)
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| this.toggle_keys(cx)))
        .child("? all keys")
}

/// One segment of a segmented control: the choice `label`, filled with
/// the accent where it is chosen, quiet text otherwise. The caller joins
/// the segments with [`segmented`] and puts the clicks on them.
pub(crate) fn segment(label: String, chosen: bool, palette: Palette) -> Div {
    div()
        .px_3()
        .py_1()
        .text_sm()
        .cursor_pointer()
        .when(chosen, |el| {
            el.bg(palette.accent)
                .text_color(palette.on_accent)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!chosen, |el| {
            el.text_color(palette.faint)
                .hover(|style| style.bg(palette.hover).text_color(palette.text))
        })
        .child(label)
}

/// Joins `segments` into one segmented control: a single bordered row
/// with a hairline between each two, rounded at its ends.
pub(crate) fn segmented(segments: Vec<gpui::Stateful<Div>>, palette: Palette) -> Div {
    let mut row = div()
        .flex()
        .rounded_md()
        .border_1()
        .border_color(palette.line)
        .overflow_hidden();
    for (at, segment) in segments.into_iter().enumerate() {
        row = row.child(segment.when(at > 0, |el| el.border_l_1().border_color(palette.line)));
    }
    row
}

/// A drawn check mark, so the checkbox does not depend on a text glyph.
pub(crate) fn checkmark(color: Rgba) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let scale = f32::from(bounds.size.width) / 16.;
            let at = |x: f32, y: f32| {
                gpui::point(
                    bounds.origin.x + px(x * scale),
                    bounds.origin.y + px(y * scale),
                )
            };
            let mut path = gpui::PathBuilder::stroke(px(1.8 * scale));
            path.move_to(at(3.2, 8.2));
            path.line_to(at(6.5, 11.2));
            path.line_to(at(12.8, 4.8));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(rems(1.))
    .flex_none()
}

/// A checkbox: a box with a tick where `checked`, beside its label. The
/// caller puts the click on.
pub(crate) fn checkbox(label: String, checked: bool, palette: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_h(px(32.))
        .cursor_pointer()
        .child(
            div()
                .size(rems(1.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(palette.line)
                .when(checked, |el| {
                    el.bg(palette.accent)
                        .border_color(palette.accent)
                        .child(checkmark(palette.on_accent))
                }),
        )
        .child(div().text_sm().child(label))
}
