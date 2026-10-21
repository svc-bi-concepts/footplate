//! The reading pane's primitives: its column, its day breaks and
//! rows, and the panel's heads, tracks and cards.

use gpui::{AnyElement, Div, div, prelude::*, px, rems};

use super::READING_WIDTH;
use crate::api::FlowDepot;
use crate::model;
use crate::theme::Palette;

/// A row of the reading column: full width, its content centred in
/// the reading width.
///
/// The width stands on two divs, not one: a div that is `w_full` with
/// a `max_w` keeps the height its text has at full width once the max
/// clamps it, so the row below starts inside the text above wherever
/// the text wraps. With the max on the inner div alone the row stands
/// as tall as its text wraps.
pub(crate) fn reading_column(row: AnyElement) -> Div {
    div()
        .w_full()
        .child(div().max_w(px(READING_WIDTH)).mx_auto().child(row))
}

/// The day's separator: its name centred between two hairlines, in the
/// reading column.
pub(crate) fn day_break(day: String, palette: Palette) -> AnyElement {
    reading_column(
        div()
            .debug_selector({
                let day = day.clone();
                move || format!("day {day}")
            })
            .mt_2()
            .flex()
            .items_center()
            .gap_2()
            .child(div().h(px(1.)).flex_1().bg(palette.line))
            .child(div().text_sm().text_color(palette.faint).child(day))
            .child(div().h(px(1.)).flex_1().bg(palette.line))
            .into_any_element(),
    )
    // What the rows' gap carried under a row with no body.
    .mb_1()
    .into_any_element()
}

/// A conversation row in the reading column. Group heads stand farther
/// from the mail before them; bodies in one group stand close together.
pub(crate) fn reading_row(row: AnyElement, apart: bool) -> AnyElement {
    let row = reading_column(row);
    if apart {
        row.mt_2().into_any_element()
    } else {
        row.into_any_element()
    }
}

/// What the board's banner says waits on the person reading: `1 waits
/// on you`, `2 wait on you`.
pub(crate) fn waits_head(waiting: usize) -> String {
    match waiting {
        1 => "1 waits on you".to_string(),
        waiting => format!("{waiting} wait on you"),
    }
}

/// What the timetable's Quiet fold says: folded or open, with how many
/// quiet lines stand under it.
pub(crate) fn quiet_head(open: bool, quiet: usize) -> String {
    format!("{} Quiet · {}", if open { "▾" } else { "▸" }, quiet)
}

/// What the panel's folded Quiet says: folded or open, naming the
/// quiet lines, with nothing in transit under them.
pub(crate) fn panel_quiet_head(open: bool, names: &[&str]) -> String {
    format!(
        "{} Quiet · {} · nothing in transit",
        if open { "▾" } else { "▸" },
        names.join(", ")
    )
}

/// The track through a stage's cell: a 2-pixel rail stopping short of
/// each stage boundary for the insulated joint, dashed where people
/// handle the stage. It runs in the border colour through an empty
/// stage and in the faint text colour through one a bead stands on, so
/// a lit section reads along the track as it does on a panel.
pub(crate) fn panel_track_line(human: bool, lit: bool, palette: Palette) -> Div {
    let color = match lit {
        true => palette.faint,
        false => palette.line,
    };
    match human {
        false => div().h(px(2.)).rounded_full().bg(color).mx(px(2.)).my_1(),
        true => {
            let mut dashes = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .mx(px(2.))
                .my_1();
            for _ in 0..14 {
                dashes = dashes.child(
                    div()
                        .h(px(2.))
                        .w(px(6.))
                        .flex_none()
                        .rounded_full()
                        .bg(color),
                );
            }
            dashes
        }
    }
}

/// How many stage columns the panel's tracks share: the stages of
/// the flow with the most of them, whichever line holds it, the backlog
/// never a column. Every track's cells are this many to the card's
/// width, so columns line up across tracks; a flow with fewer stages
/// leaves the rest empty. At least one, so a yard with no stages still
/// divides.
pub(crate) fn panel_columns(depots: &[FlowDepot]) -> usize {
    depots
        .iter()
        .flat_map(|depot| &depot.flows)
        .map(|flow| {
            flow.stages
                .iter()
                .filter(|stage| stage.stage != "backlog")
                .count()
        })
        .max()
        .unwrap_or(0)
        .max(1)
}

/// The kit's meta type, for the panel's small words: 12 pixels on a
/// 16-pixel line, in rems, so the rows stand close and still grow with
/// the text.
pub(crate) fn panel_meta() -> Div {
    div().text_xs().line_height(rems(1.))
}

/// A stage name in the panel: GPUI can break an overlong hyphenated
/// name between its letters, so give it an invisible break after each
/// hyphen. The hyphenated words then stay whole when the cell wraps.
pub(crate) fn panel_stage_name(stage: &str, human: bool) -> String {
    let name = stage.replace('-', "-\u{200b}");
    match human {
        true => format!("{name} · people"),
        false => name,
    }
}

/// What the Lines pane says of a yard from before the Lines view, in
/// place of the arrivals or the flows it would draw: plain words in the
/// faint colour, never a notice.
pub(crate) fn lines_too_old(palette: Palette) -> Div {
    div()
        .debug_selector(|| "lines too old".to_string())
        .text_color(palette.faint)
        .child(model::LINES_TOO_OLD)
}

/// A track's card: a surface of its own under its name and its cells,
/// in the sunken ground with the border's hairline and the kit's
/// 8-pixel radius, as wide as the pane, so the tracks stand as bands
/// one under another and never past the pane's edge. The card never
/// shrinks: squeezed below its content it would spill its headcodes
/// over the next row, so the panel scrolls instead.
pub(crate) fn panel_card(palette: Palette) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .flex_none()
        .px_2()
        .py_1()
        .rounded_lg()
        .border_1()
        .border_color(palette.line)
        .bg(palette.side)
}

/// The buffer stop that closes a terminal stage's track: a short
/// vertical bar where the yard serves the stage terminal, on the
/// panel's cells and in the line view's terminal column.
pub(crate) fn buffer_stop(selector: String, palette: Palette) -> Div {
    div()
        .debug_selector(move || selector.clone())
        .w(px(3.))
        .h(px(14.))
        .flex_none()
        .bg(palette.faint)
}

/// The rule under a stage column's head: the track, 1 pixel in the
/// border colour, in the lamp's colour where one burns, dashed where
/// people handle the stage.
pub(crate) fn column_rule(lamp: &str, human: bool, palette: Palette) -> Div {
    let color = match lamp {
        "fault" => palette.warn,
        "turn" => palette.accent,
        _ => palette.line,
    };
    match human {
        false => div().h_px().flex_none().bg(color),
        true => {
            let mut dashes = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_none();
            for _ in 0..16 {
                dashes = dashes.child(div().h_px().w(px(6.)).flex_none().bg(color));
            }
            dashes
        }
    }
}
