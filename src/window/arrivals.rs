//! The timetable's views: the arrivals pane with its panel of
//! tracks and departures board, the lines' flows, and the beads that
//! travel with no line.

use gpui::{
    AnyElement, Context, FontWeight, MouseButton, SharedString, div, prelude::*, px, relative,
    rems, rgba,
};

use super::Footplate;
use super::ask::section_head;
use super::metrics::{
    BACKLOG_TITLES, BOARD_BEAD, BOARD_EXPECTED, BOARD_LINE, BOARD_PLATFORM, BOARD_REMARK,
    LINE_COLUMN, LINE_HEAD, PANEL_MIN, grown,
};
use super::panel::{
    buffer_stop, column_rule, lines_too_old, panel_card, panel_columns, panel_meta,
    panel_quiet_head, panel_stage_name, panel_track_line, quiet_head, waits_head,
};
use super::rail::RailTip;
use super::screen::screen_control;
use super::words::{
    bead_words, flow_words, headcode_color, lamp_dot, landed_age, remark_color, stage_group_words,
    stage_lamp, waiting_words, waits_for_seat, worker_words, working_tip, working_words,
};
use crate::api::{Arrival, Flow, FlowBead, FlowDepot, FlowLanding, FlowStage};
use crate::model::{self, Mode, Model, Selected, backlog_count, short_bead, stage_age, you_first};
use crate::select;
use crate::theme::Palette;

impl Footplate {
    pub(crate) fn toggle_quiet(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_quiet();
            Vec::new()
        });
    }

    /// The arrivals head's toggle sets which view stands: the
    /// departures board where true, the panel where false.
    pub(crate) fn set_departures(&mut self, departures: bool, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.set_departures(departures);
            Vec::new()
        });
    }

    /// The timetable's list: the arrivals row with the yard-wide
    /// travelling total, the active lines in the served order with
    /// their own, No line while beads travel with none, and the quiet
    /// lines folded under Quiet.
    pub(crate) fn list_timetable(
        &self,
        model: &Model,
        palette: Palette,
        top: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut body = self.list_body();
        let arrivals = model.arrivals_row();
        body = body.child(section_head("Timetable", arrivals.line.clone(), palette));
        body = body.child(self.row(arrivals, Selected::Arrivals, palette, cx));
        for row in model.active_line_rows() {
            let sel = Selected::Line(row.name.clone());
            body = body.child(self.row(row, sel, palette, cx));
        }
        if let Some(row) = model.no_line_row() {
            body = body.child(self.row(row, Selected::NoLine, palette, cx));
        }
        let quiet = model.quiet_rows();
        if !quiet.is_empty() {
            let open = model.quiet_expanded();
            body = body.child(
                div()
                    .id("quiet")
                    .mx_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_quiet(cx)))
                    .child(
                        div()
                            .debug_selector(|| "quiet".to_string())
                            .text_sm()
                            .text_color(palette.faint)
                            .child(quiet_head(open, quiet.len())),
                    ),
            );
            if open {
                for row in quiet {
                    let sel = Selected::Line(row.name.clone());
                    body = body.child(self.row(row, sel, palette, cx));
                }
            }
        }
        self.column(body, palette, top, cx)
    }

    /// One row of the departures board: the expected arrival in
    /// mono, the bead's title with its headcode faint after it, the
    /// platform as a plain word, the line, and the remark last in its
    /// tone. The rows stand flat in the served arrival order, a late
    /// mark as words among the remarks. A click opens the details.
    /// Hovering a worked row says which session works it, from the
    /// bead's flows row, which is where the yard says so. Where the
    /// window is the operator's a worked row carries Screen, which opens
    /// that session's screen over the window instead of the details.
    pub(crate) fn arrival_row(
        &self,
        model: &Model,
        row: &Arrival,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bead = row.bead.clone();
        let sel = Selected::Bead(row.bead.clone());
        let remark = you_first(&row.remark, model.me());
        let color = remark_color(&row.remark, &row.tone, &row.held_by, model.me(), palette);
        let tip = model
            .flow_bead(&row.bead)
            .and_then(|(found, _)| worker_words(found).map(|(tip, _)| tip));
        let mut expected = grown(BOARD_EXPECTED)
            .debug_selector({
                let bead = bead.clone();
                move || format!("arrival expected {bead}")
            })
            .flex_none()
            .truncate()
            .text_xs()
            .text_color(match row.expected.as_str() {
                "—" => palette.faint,
                _ => palette.text,
            })
            .child(row.expected.clone());
        if let Some(mono) = mono.clone() {
            expected = expected.font_family(mono);
        }
        let mut headcode = div()
            .flex_none()
            .text_xs()
            .text_color(palette.faint)
            .child(short_bead(&row.bead));
        if let Some(mono) = mono {
            headcode = headcode.font_family(mono);
        }
        div()
            .id(SharedString::from(format!("arrival {bead}")))
            .debug_selector({
                let bead = bead.clone();
                move || format!("arrival {bead}")
            })
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .py_1p5()
            .border_b_1()
            .border_color(palette.line)
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .when_some(tip, |el, tip| {
                el.tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
            })
            .child(expected)
            .child(
                grown(BOARD_BEAD)
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("arrival bead {bead}")
                    })
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap_1()
                    .child(
                        div()
                            .flex_shrink()
                            .min_w_0()
                            .truncate()
                            .debug_selector({
                                let bead = bead.clone();
                                move || format!("arrival title {bead}")
                            })
                            .child(row.title.clone()),
                    )
                    .child(headcode),
            )
            .child(
                grown(BOARD_PLATFORM)
                    .flex_none()
                    .truncate()
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("arrival platform {bead}")
                    })
                    .child(row.platform.clone()),
            )
            .child(
                grown(BOARD_LINE)
                    .flex_none()
                    .truncate()
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("arrival line {bead}")
                    })
                    .child(match row.line.is_empty() {
                        true => "no line".to_string(),
                        false => row.line.clone(),
                    }),
            )
            .child(
                grown(BOARD_REMARK)
                    .flex_none()
                    .truncate()
                    .text_color(color)
                    .when(!remark.is_empty(), |el| {
                        el.debug_selector({
                            let bead = bead.clone();
                            move || format!("arrival remark {bead}")
                        })
                    })
                    .child(remark),
            )
            .children(model.screen_for_bead(&row.bead).map(|session| {
                screen_control(session, format!("arrival screen {bead}"), palette, cx)
            }))
    }

    /// What the arrivals pane's tooltip says, in one line.
    pub(crate) const ARRIVALS_TIP: &str = "arrivals: the panel of beads still travelling";

    /// The arrivals pane: the panel of tracks, or the departures
    /// board under the head's toggle. The head carries the travelling
    /// words, the panel's summary where it stands, and the toggle.
    pub(crate) fn talk_arrivals(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let head = self.arrivals_head(model, palette, cx);
        let body = match model.departures() {
            true => self.departures_board(model, palette, cx).into_any_element(),
            false => self.panel(model, palette, cx).into_any_element(),
        };
        pane.children(self.reconnect_strip(palette))
            .child(head)
            .child(body)
            .when(!model.notice.is_empty(), |el| {
                el.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(palette.warn)
                        .child(format!("! {}", model.notice)),
                )
            })
    }

    /// The arrivals pane's head: the title with the Panel/Departures
    /// toggle at its right, then the travelling words and the panel's
    /// summary where it stands. The words stand full-width under the
    /// title's row, so the summary holds one line down to the floor.
    pub(crate) fn arrivals_head(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .id("arrivals head")
                            .debug_selector(|| "arrivals head".to_string())
                            .line_height(rems(1.25))
                            .font_weight(FontWeight::SEMIBOLD)
                            .tooltip(|_, cx| {
                                cx.new(|_| RailTip(Self::ARRIVALS_TIP.to_string())).into()
                            })
                            .child("arrivals"),
                    )
                    .child(self.arrivals_toggle(model, palette, cx)),
            )
            .child(
                div()
                    .text_sm()
                    .line_height(rems(1.25))
                    .text_color(palette.faint)
                    .child(format!("{} · all lines", model.arrivals_row().line)),
            )
            .children(self.panel_summary(model, palette, cx))
    }

    /// The head's Panel/Departures toggle: two quiet words, the open
    /// view in the text colour and the other faint, each opening its
    /// view.
    pub(crate) fn arrivals_toggle(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let departures = model.departures();
        let mut toggle = div().flex().flex_row().items_start().gap_3().flex_none();
        toggle = toggle.child(self.toggle_word("Panel", false, !departures, palette, cx));
        toggle = toggle.child(self.toggle_word("Departures", true, departures, palette, cx));
        toggle
    }

    /// One word of the arrivals toggle: in the text colour where its
    /// view stands, faint otherwise, opening its view on a click.
    pub(crate) fn toggle_word(
        &self,
        word: &'static str,
        departures: bool,
        open: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let what = format!("arrivals {}", word.to_lowercase());
        div()
            .id(SharedString::from(what.clone()))
            .debug_selector(move || what.clone())
            .text_sm()
            .line_height(rems(1.25))
            .text_color(match open {
                true => palette.text,
                false => palette.faint,
            })
            .when(open, |el| el.font_weight(FontWeight::SEMIBOLD))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.set_departures(departures, cx)))
            .child(word)
    }

    /// The panel's summary words: how many beads are being worked, wait
    /// on the reader, and are held, each where any are, in quiet words.
    /// The worked words split by served kind where two or more kinds
    /// work, and hovering them lists each working group with its seats.
    /// The waiting words open the decisions mode, as the board's banner
    /// count does. None where nothing is worked, waited or held, and
    /// never on the departures board, whose banner counts the waits.
    pub(crate) fn panel_summary(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if model.departures() {
            return None;
        }
        let working = model.working_count();
        let waiting = model.waiting_on_you().len();
        let held = model.held_count();
        if working == 0 && waiting == 0 && held == 0 {
            return None;
        }
        let mut parts: Vec<AnyElement> = Vec::new();
        if working > 0 {
            let split = model.working_split();
            parts.push(
                div()
                    .id("panel working")
                    .debug_selector(|| "panel working".to_string())
                    .text_color(palette.text)
                    .when_some(working_tip(&split.groups), |el, tip| {
                        el.tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
                    })
                    .child(working_words(working, &split.kinds))
                    .into_any_element(),
            );
        }
        if waiting > 0 {
            parts.push(
                div()
                    .id("panel waits")
                    .debug_selector(|| "panel waits".to_string())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.text_accent)
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.set_mode(Mode::Decisions, window, cx)
                        }),
                    )
                    .child(format!("{waiting} waiting on you"))
                    .into_any_element(),
            );
        }
        if held > 0 {
            parts.push(
                div()
                    .text_color(palette.faint)
                    .child(format!("{held} held"))
                    .into_any_element(),
            );
        }
        let mut line = panel_meta()
            .debug_selector(|| "panel summary".to_string())
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1();
        for (n, part) in parts.into_iter().enumerate() {
            if n > 0 {
                line = line.child(div().text_color(palette.faint).child("·"));
            }
            line = line.child(part);
        }
        Some(line)
    }

    /// The departures board under the toggle: the served rows flat
    /// in the served arrival order under their column head, with the
    /// waits banner and the cap's honest line.
    pub(crate) fn departures_board(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mono = select::mono(cx);
        let mut list = div()
            .id("arrivals")
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
            .overflow_y_scroll()
            .track_scroll(&self.decision)
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        let waiting = model.waiting_on_you().len();
        if waiting > 0 {
            list = list.child(self.waits_banner(waiting, palette, cx));
        }
        let rows = model.arrival_rows();
        if rows.is_empty() {
            list = list.child(match model.arrivals_too_old() {
                true => lines_too_old(palette),
                false => div().text_color(palette.faint).child("None in transit."),
            });
        } else {
            list = list.child(self.departures_head(palette));
            for row in rows {
                list = list.child(self.arrival_row(model, row, mono.clone(), palette, cx));
            }
        }
        // The board caps at 15 rows; the total is complete.
        let travelling = model.travelling();
        let shown = model.arrival_rows().len();
        if travelling > shown {
            list = list.child(
                div()
                    .debug_selector(|| "arrivals capped".to_string())
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("{shown} of {travelling} shown")),
            );
        }
        list
    }

    /// The departures board's column head: Expected, Bead, Platform,
    /// Line and Remark over the rows' columns.
    pub(crate) fn departures_head(&self, palette: Palette) -> impl IntoElement + use<> {
        let cell = |words: &'static str, width: f32| {
            grown(width)
                .flex_none()
                .truncate()
                .text_xs()
                .text_color(palette.faint)
                .child(words)
        };
        div()
            .debug_selector(|| "departures head".to_string())
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .pb_1p5()
            .border_b_1()
            .border_color(palette.line)
            .child(cell("Expected", BOARD_EXPECTED))
            .child(cell("Bead", BOARD_BEAD))
            .child(cell("Platform", BOARD_PLATFORM))
            .child(cell("Line", BOARD_LINE))
            .child(cell("Remark", BOARD_REMARK))
    }

    /// The arrivals panel: every travelling flow one track — its name
    /// above its stages, a cell per stage but the backlog's in travel
    /// order — each track a card of its own, the Elsewhere stubs, the
    /// beads with no line as their own track, and the quiet tracks
    /// folded to one row. Every track fits the pane's width: the cells
    /// share it, wrapping where they cannot, and the panel scrolls only
    /// down over the tracks.
    pub(crate) fn panel(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mono = select::mono(cx);
        // Every track shares the widest flow's columns, the quiet
        // flows' too, so folding the quiet tracks moves none of them.
        let columns = panel_columns(model.depots());
        let mut body = div()
            .id("panel")
            .debug_selector(|| "panel".to_string())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.decision)
            .px_3()
            .pt_2()
            .pb_3()
            .flex()
            .flex_col()
            .gap_2();
        let mut empty = true;
        for depot in model
            .depots()
            .iter()
            .filter(|depot| depot.travelling > 0 || !depot.error.is_empty())
        {
            empty = false;
            body = body.child(self.panel_depot(depot, columns, mono.clone(), palette, cx));
        }
        if !model.unassigned().is_empty() {
            empty = false;
            body = body.child(self.panel_no_line(model, mono.clone(), palette, cx));
        }
        if model
            .depots()
            .iter()
            .any(|depot| depot.travelling == 0 && depot.error.is_empty())
        {
            empty = false;
            body = body.child(self.panel_quiet(model, columns, mono.clone(), palette, cx));
        }
        if empty {
            body = body.child(match model.flows_too_old() {
                true => lines_too_old(palette),
                false => div().text_color(palette.faint).child("None in transit."),
            });
        }
        body
    }

    /// One depot's rows on the panel: its error where its flows could
    /// not be read, one track per flow in the served order, and the
    /// Elsewhere stub where beads stand there. The rows of one depot
    /// stand close, so the depots read apart. The rows never shrink:
    /// squeezed, their cards would spill over the next row, so the
    /// panel scrolls instead.
    pub(crate) fn panel_depot(
        &self,
        depot: &FlowDepot,
        columns: usize,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut rows = div().flex().flex_col().flex_none().gap_1();
        if !depot.error.is_empty() {
            rows = rows.child(self.panel_words(depot, depot.error.clone(), palette, cx));
        }
        for flow in &depot.flows {
            rows = rows.child(self.panel_track(depot, flow, columns, mono.clone(), palette, cx));
        }
        if depot.flows.is_empty() && depot.error.is_empty() && depot.elsewhere.is_empty() {
            rows = rows.child(self.panel_words(depot, "None in transit.".into(), palette, cx));
        }
        if !depot.elsewhere.is_empty() {
            rows = rows.child(self.panel_elsewhere(depot, mono, palette, cx));
        }
        rows
    }

    /// One depot's row where its flows draw no track: its name above
    /// faint words — the error where its flows could not be read, else
    /// that none is in transit.
    pub(crate) fn panel_words(
        &self,
        depot: &FlowDepot,
        words: String,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let name = depot.name.clone();
        panel_card(palette)
            .debug_selector({
                let name = name.clone();
                move || format!("panel words {name}")
            })
            .child(self.panel_name(
                name.clone(),
                String::new(),
                None,
                Selected::Line(name.clone()),
                format!("panel name {name}"),
                palette,
                cx,
            ))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .py_2()
                    .px_2()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(words),
            )
    }

    /// One track, a card of its own: its name above a cell per stage
    /// but the backlog's, left to right in travel order, wrapping where
    /// the stages cannot fit the card's width.
    pub(crate) fn panel_track(
        &self,
        depot: &FlowDepot,
        flow: &Flow,
        columns: usize,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let second = match flow.flow_type.is_empty() {
            true => flow.name.clone(),
            false => format!("{} · {}", flow.name, flow.flow_type),
        };
        let count = backlog_count(flow);
        let mut stages = div().flex().flex_row().flex_wrap().w_full().min_w_0();
        for stage in flow.stages.iter().filter(|stage| stage.stage != "backlog") {
            stages = stages.child(self.panel_cell(
                depot,
                flow,
                stage,
                columns,
                mono.clone(),
                palette,
                cx,
            ));
        }
        panel_card(palette)
            .debug_selector({
                let (depot, flow) = (depot.name.clone(), flow.name.clone());
                move || format!("panel track {depot} {flow}")
            })
            .min_h(rems(4.))
            .child(self.panel_name(
                depot.name.clone(),
                second,
                Some(count),
                Selected::Line(depot.name.clone()),
                format!("panel name {} {}", depot.name, flow.name),
                palette,
                cx,
            ))
            .child(stages)
    }

    /// One track's name above its stages: the depot's name with the
    /// flow's name and type and the backlog count on one line, opening
    /// the line. The depot stands in the label type, the flow and the
    /// count in the meta type; where one line cannot hold them they wrap
    /// whole, never cut.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn panel_name(
        &self,
        first: String,
        second: String,
        backlog: Option<usize>,
        to: Selected,
        what: String,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut row = div()
            .id(SharedString::from(what.clone()))
            .debug_selector({
                let what = what.clone();
                move || what.clone()
            })
            .w_full()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_baseline()
            .gap_2()
            .py_1p5()
            .pr_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(move |this, _, window, cx| this.select(to.clone(), window, cx)))
            .child(
                div()
                    .debug_selector({
                        let what = what.clone();
                        move || format!("{what} depot")
                    })
                    .line_height(rems(1.25))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(first),
            );
        if !second.is_empty() {
            row = row.child(
                panel_meta()
                    .debug_selector({
                        let what = what.clone();
                        move || format!("{what} flow")
                    })
                    .text_color(palette.faint)
                    .child(second),
            );
        }
        if let Some(count) = backlog {
            row = row.child(
                panel_meta()
                    .debug_selector(move || format!("{what} backlog"))
                    .text_color(match count {
                        0 => palette.faint,
                        _ => palette.text,
                    })
                    .child(format!("{count} backlog")),
            );
        }
        row
    }

    /// One stage's cell: its name with the lamp where one burns and
    /// the buffer stop where the stage is terminal, the track through
    /// it, and two headcodes, the second reading "+N" where a third bead
    /// stands on it. Lit only with a bead on it: an empty stage stands
    /// on its track's card, its name faint, and an occupied one steps up
    /// to the raised surface. The cells of a track share its width, each
    /// one of `columns` to the card and never narrower than a headcode;
    /// a head longer than its cell wraps whole onto a second line. Every
    /// head stands two lines high with its words on the rule, so a head
    /// of one line and a head of two put the rule at one place: the
    /// rules of a row are one straight track, and of every track alike,
    /// with no row measured for its tallest head. A name that wants a
    /// third line makes its own cell taller rather than run over the row
    /// above.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn panel_cell(
        &self,
        depot: &FlowDepot,
        flow: &Flow,
        stage: &FlowStage,
        columns: usize,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let lit = !stage.beads.is_empty();
        let lamp = lamp_dot(
            stage_lamp(stage),
            format!("panel lamp {} {} {}", depot.name, flow.name, stage.stage),
            palette,
        );
        let name = panel_stage_name(&stage.stage, stage.human);
        let stop = stage.terminal.then(|| {
            buffer_stop(
                format!("panel stop {} {} {}", depot.name, flow.name, stage.stage),
                palette,
            )
        });
        let mut cell = div()
            .debug_selector({
                let (depot, flow, stage) =
                    (depot.name.clone(), flow.name.clone(), stage.stage.clone());
                move || format!("panel stage {depot} {flow} {stage}")
            })
            .flex_none()
            .flex_basis(relative(1. / columns as f32))
            .min_w(rems(PANEL_MIN))
            .pt_1()
            .rounded_md()
            .when(lit, |el| el.bg(palette.ground))
            .child(
                // Two of `panel_meta`'s lines, 1 rem each, bottom-aligned.
                // The cell holds the top padding, so it stays out of
                // this height. A minimum, not a fixed height: a head of
                // three lines would spill upward out of a fixed box.
                div()
                    .min_h(rems(2.))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .px_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .children(lamp)
                            .child(
                                panel_meta()
                                    .debug_selector({
                                        let (depot, flow, stage) = (
                                            depot.name.clone(),
                                            flow.name.clone(),
                                            stage.stage.clone(),
                                        );
                                        move || format!("panel head {depot} {flow} {stage}")
                                    })
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(match lit {
                                        true => palette.text,
                                        false => palette.faint,
                                    })
                                    .child(name),
                            )
                            .children(stop),
                    ),
            )
            .child(panel_track_line(stage.human, lit, palette).debug_selector({
                let (depot, flow, stage) =
                    (depot.name.clone(), flow.name.clone(), stage.stage.clone());
                move || format!("panel rule {depot} {flow} {stage}")
            }));
        let mut codes = div().flex().flex_col().px_2().pb_1p5();
        match stage.beads.as_slice() {
            [] => {}
            [one] => {
                codes = codes.child(self.panel_headcode(one, mono, palette, cx));
            }
            [one, two] => {
                codes = codes.child(self.panel_headcode(one, mono.clone(), palette, cx));
                codes = codes.child(self.panel_headcode(two, mono, palette, cx));
            }
            [one, ..] => {
                codes = codes.child(self.panel_headcode(one, mono, palette, cx));
                codes = codes.child(
                    panel_meta()
                        .debug_selector({
                            let (depot, flow, stage) =
                                (depot.name.clone(), flow.name.clone(), stage.stage.clone());
                            move || format!("panel more {depot} {flow} {stage}")
                        })
                        .text_color(palette.faint)
                        .child(format!("+{}", stage.beads.len() - 1)),
                );
            }
        }
        cell = cell.child(codes);
        cell
    }

    /// One bead's headcode on the panel: the short handle in its
    /// marker drawing, in mono, opening the bead. A bead being worked
    /// carries the running dot at its left, and hovering it says which
    /// session works it, with its group, kind and model.
    pub(crate) fn panel_headcode(
        &self,
        bead: &FlowBead,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let sel = Selected::Bead(bead.bead.clone());
        let name = bead.bead.clone();
        let tip = worker_words(bead).map(|(tip, _)| tip);
        let me = self.model.as_ref().map(Model::me).unwrap_or_default();
        let color = headcode_color(
            bead.working,
            &bead.remark,
            &bead.tone,
            &bead.held_by,
            me,
            palette,
        );
        let mut text = panel_meta()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_color(color)
            .when(bead.working, |el| el.font_weight(FontWeight::SEMIBOLD))
            .child(short_bead(&bead.bead));
        if let Some(mono) = mono {
            text = text.font_family(mono);
        }
        div()
            .id(SharedString::from(format!("panel headcode {name}")))
            .debug_selector({
                let name = name.clone();
                move || format!("panel headcode {name}")
            })
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .when_some(tip, |el, tip| {
                el.tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
            })
            .children(
                bead.working
                    .then(|| div().size(px(6.)).rounded_full().bg(palette.ok).flex_none()),
            )
            .child(text)
    }

    /// The stub under a depot's last track: its Elsewhere beads'
    /// headcodes on a card, only where beads stand there, under their
    /// count and wrapping where they cannot fit one row.
    pub(crate) fn panel_elsewhere(
        &self,
        depot: &FlowDepot,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut codes = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_2()
            .w_full()
            .min_w_0();
        for bead in &depot.elsewhere {
            codes = codes.child(self.panel_headcode(bead, mono.clone(), palette, cx));
        }
        panel_card(palette)
            .debug_selector({
                let depot = depot.name.clone();
                move || format!("panel elsewhere {depot}")
            })
            .gap_2()
            .child(
                panel_meta()
                    .text_color(palette.faint)
                    .child(format!("Elsewhere · {}", depot.elsewhere.len())),
            )
            .child(codes)
    }

    /// The beads travelling with no line: their own track at the foot
    /// of the active ones, its name above their headcodes, because a
    /// bead with no depot travels no flow and has no stages to stand
    /// between.
    pub(crate) fn panel_no_line(
        &self,
        model: &Model,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut codes = div().flex().flex_col().px_2().pb_1p5();
        for bead in model.unassigned() {
            codes = codes.child(self.panel_headcode(bead, mono.clone(), palette, cx));
        }
        panel_card(palette)
            .debug_selector(|| "panel noline".to_string())
            .min_h(rems(4.))
            .child(self.panel_name(
                "No line".into(),
                "no depot".into(),
                None,
                Selected::NoLine,
                "panel noline name".into(),
                palette,
                cx,
            ))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .rounded_md()
                    .bg(palette.ground)
                    .flex()
                    .flex_col()
                    .child(
                        panel_meta()
                            .px_2()
                            .pt_1()
                            .text_color(palette.text)
                            .child("travelling"),
                    )
                    .child(panel_track_line(false, true, palette))
                    .child(codes),
            )
    }

    /// The quiet tracks folded to one row at the panel's foot: the
    /// folded names, unfolding to their tracks with the list's Quiet
    /// fold.
    pub(crate) fn panel_quiet(
        &self,
        model: &Model,
        columns: usize,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let quiet: Vec<&FlowDepot> = model
            .depots()
            .iter()
            .filter(|depot| depot.travelling == 0 && depot.error.is_empty())
            .collect();
        let names: Vec<&str> = quiet.iter().map(|depot| depot.name.as_str()).collect();
        let open = model.quiet_expanded();
        // Never shrunk: squeezed, the folded row or the unfolded
        // tracks would spill over whatever the panel draws last, so
        // the panel scrolls instead.
        let mut section = div().flex().flex_col().flex_none().gap_3().child(
            div()
                .id("panel quiet")
                .debug_selector(|| "panel quiet".to_string())
                .px_2()
                .py_2()
                .rounded_md()
                .flex()
                .flex_row()
                .items_center()
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_quiet(cx)))
                .child(
                    panel_meta()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(palette.faint)
                        .child(panel_quiet_head(open, &names)),
                ),
        );
        if open {
            for depot in quiet {
                section =
                    section.child(self.panel_depot(depot, columns, mono.clone(), palette, cx));
            }
        }
        section
    }

    /// The beads that wait on the person reading, above the table:
    /// how many, as one quiet link. A click opens the decisions mode,
    /// as Enter does with the focus on it.
    pub(crate) fn waits_banner(
        &self,
        waiting: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .id("waits banner")
            .debug_selector(|| "waits banner".to_string())
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.text_accent)
            .cursor_pointer()
            .track_focus(&self.waits_focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| window.focus(&this.waits_focus)),
            )
            .on_click(cx.listener(|this, _, window, cx| this.set_mode(Mode::Decisions, window, cx)))
            .child(waits_head(waiting))
    }

    /// One line's flows: each flow a section with its stages in travel
    /// order as columns — the panel's track straightened — each column
    /// its head with the group words and one card per bead, and an
    /// Elsewhere column under the flows for the line's beads in no
    /// stage of them. A bead's click opens its details.
    pub(crate) fn talk_flow(
        &self,
        model: &Model,
        line: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(depot) = model.depot(line) else {
            return pane
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child("The line is gone.");
        };
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector({
                        let line = line.to_string();
                        move || format!("flow head {line}")
                    })
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(line.to_string()),
            )
            .child(div().text_sm().text_color(palette.faint).child(format!(
                        "{} · {}",
                        model
                            .line_rows()
                            .iter()
                            .find(|row| row.name == line)
                            .map(|row| row.line.clone())
                            .unwrap_or_default(),
                        flow_words(depot.flows.len())
                    )));
        let mono = select::mono(cx);
        let now = chrono::Utc::now();
        let mut list = div()
            .id("flow")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.decision)
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        // The list takes vertical moves alone: gpui hands a wheel move
        // to every scroller under the pointer and spends a move along
        // one axis on a scroller of the other, so the horizontal moves
        // meant for the flows' columns would scroll the list as well.
        list.style().restrict_scroll_to_axis = Some(true);
        if !depot.error.is_empty() {
            list = list.child(
                div()
                    .debug_selector({
                        let line = line.to_string();
                        move || format!("flow error {line}")
                    })
                    .text_sm()
                    .text_color(palette.faint)
                    .child(depot.error.clone()),
            );
        }
        for flow in &depot.flows {
            list = list.child(self.flow_section(flow, &now, mono.clone(), palette, cx));
        }
        if !depot.elsewhere.is_empty() {
            list = list.child(self.elsewhere_column(depot, &now, mono.clone(), palette, cx));
        }
        if depot.flows.is_empty() && depot.elsewhere.is_empty() {
            list = list.child(div().text_color(palette.faint).child("None in transit."));
        }
        pane.children(self.reconnect_strip(palette))
            .child(head)
            .child(list)
            .when(!model.notice.is_empty(), |el| {
                el.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(palette.warn)
                        .child(format!("! {}", model.notice)),
                )
            })
    }

    /// One flow's section: the flow name with its type where typed and
    /// its source, then its stages in travel order as columns — the
    /// panel's track straightened — scrolling sideways past the pane's
    /// edge. Stages never collapse.
    pub(crate) fn flow_section(
        &self,
        flow: &Flow,
        now: &chrono::DateTime<chrono::Utc>,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut columns = div()
            .id(SharedString::from(format!("columns {}", flow.name)))
            .overflow_x_scroll()
            .flex()
            .flex_row();
        // The columns take horizontal moves alone and leave the
        // vertical ones to the list, for the same reason as the list.
        columns.style().restrict_scroll_to_axis = Some(true);
        for stage in &flow.stages {
            columns = columns.child(match stage.stage.as_str() {
                "backlog" => self
                    .backlog_column(flow, stage, palette, cx)
                    .into_any_element(),
                _ => self
                    .stage_column(flow, stage, now, mono.clone(), palette, cx)
                    .into_any_element(),
            });
        }
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .debug_selector({
                                let flow = flow.name.clone();
                                move || format!("flow {flow}")
                            })
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(flow.name.clone()),
                    )
                    .child(div().text_xs().text_color(palette.faint).child(
                        match flow.flow_type.is_empty() {
                            true => flow.source.clone(),
                            false => format!("{} · {}", flow.flow_type, flow.source),
                        },
                    )),
            )
            .child(columns)
    }

    /// One stage's column: its name with the group words under it and
    /// the track as the head's rule, then one card per bead. The
    /// terminal stage reads terminal with the buffer stop, and lists
    /// under it what landed there in the trailing 24 hours; an empty
    /// stage says nothing stands here.
    pub(crate) fn stage_column(
        &self,
        flow: &Flow,
        stage: &FlowStage,
        now: &chrono::DateTime<chrono::Utc>,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut column = grown(LINE_COLUMN)
            .debug_selector({
                let (flow, stage) = (flow.name.clone(), stage.stage.clone());
                move || format!("column {flow} {stage}")
            })
            .flex_none()
            .border_l_1()
            .border_color(palette.line)
            .px_2()
            .flex()
            .flex_col()
            .child(
                div().flex().flex_col().pb_1().child(
                    // The name over two group lines, bottom-aligned:
                    // the rules stand level across columns.
                    div()
                        .min_h(rems(LINE_HEAD))
                        .flex()
                        .flex_col()
                        .justify_end()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .children(lamp_dot(
                                    stage_lamp(stage),
                                    format!("column {} {} lamp", flow.name, stage.stage),
                                    palette,
                                ))
                                .child(
                                    div()
                                        .debug_selector({
                                            let (flow, stage) =
                                                (flow.name.clone(), stage.stage.clone());
                                            move || format!("column {flow} {stage} head")
                                        })
                                        .truncate()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(stage.stage.clone()),
                                ),
                        )
                        .child(match stage.terminal {
                            true => div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(palette.faint)
                                .child("terminal")
                                .child(buffer_stop(
                                    format!("column {} {} stop", flow.name, stage.stage),
                                    palette,
                                ))
                                .into_any_element(),
                            false => div()
                                .debug_selector({
                                    let (flow, stage) = (flow.name.clone(), stage.stage.clone());
                                    move || format!("column {flow} {stage} group")
                                })
                                .text_xs()
                                .text_color(palette.faint)
                                .child(stage_group_words(stage))
                                .into_any_element(),
                        }),
                ),
            )
            .child(
                column_rule(stage_lamp(stage), stage.human, palette).debug_selector({
                    let (flow, stage) = (flow.name.clone(), stage.stage.clone());
                    move || format!("column {flow} {stage} rule")
                }),
            );
        // What landed stands under the landed station alone; any
        // other column draws the stages' beads and nothing else.
        let landings = match stage.terminal {
            true => flow.landings.as_slice(),
            false => &[],
        };
        if stage.beads.is_empty() && landings.is_empty() {
            column = column.child(
                div()
                    .debug_selector({
                        let (flow, stage) = (flow.name.clone(), stage.stage.clone());
                        move || format!("column {flow} {stage} empty")
                    })
                    .pt_2()
                    .text_xs()
                    .text_color(palette.faint)
                    .child("nothing here"),
            );
        }
        for bead in &stage.beads {
            column =
                column.child(self.bead_card(bead, Some(stage), now, mono.clone(), palette, cx));
        }
        for landing in landings {
            column = column.child(self.landing_row(landing, now, palette, cx));
        }
        column
    }

    /// One bead that landed under the flow's landed station: its title
    /// with its headcode and how long ago it landed faint under it, in
    /// the served order, newest first. A tap opens the bead whole, as a
    /// card's tap does; the Bead call serves closed beads.
    pub(crate) fn landing_row(
        &self,
        landing: &FlowLanding,
        now: &chrono::DateTime<chrono::Utc>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bead = landing.bead.clone();
        let sel = Selected::Bead(landing.bead.clone());
        let meta = [
            short_bead(&landing.bead),
            landed_age(&model::waits(&landing.landed_at, now)),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        div()
            .id(SharedString::from(format!("landing {bead}")))
            .debug_selector({
                let bead = bead.clone();
                move || format!("landing {bead}")
            })
            .pt_2()
            .flex()
            .flex_col()
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .child(
                div()
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("landing title {bead}")
                    })
                    .truncate()
                    .text_sm()
                    .child(landing.title.clone()),
            )
            .child(
                div()
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("landing meta {bead}")
                    })
                    .text_xs()
                    .text_color(palette.faint)
                    .child(meta),
            )
    }

    /// One flow's backlog column: its name with the group words under
    /// it and the track as the head's rule, then its count with the
    /// first titles as plain rows and "+N more" past them. A title's
    /// click opens its bead.
    pub(crate) fn backlog_column(
        &self,
        flow: &Flow,
        stage: &FlowStage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut column = grown(LINE_COLUMN)
            .debug_selector({
                let flow = flow.name.clone();
                move || format!("column {flow} backlog")
            })
            .flex_none()
            .border_l_1()
            .border_color(palette.line)
            .px_2()
            .flex()
            .flex_col()
            .child(
                div().flex().flex_col().pb_1().child(
                    // The name over two group lines, bottom-aligned:
                    // the rules stand level across columns.
                    div()
                        .min_h(rems(LINE_HEAD))
                        .flex()
                        .flex_col()
                        .justify_end()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .children(lamp_dot(
                                    stage_lamp(stage),
                                    format!("column {} backlog lamp", flow.name),
                                    palette,
                                ))
                                .child(
                                    div()
                                        .debug_selector({
                                            let flow = flow.name.clone();
                                            move || format!("column {flow} backlog head")
                                        })
                                        .truncate()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(stage.stage.clone()),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector({
                                    let flow = flow.name.clone();
                                    move || format!("column {flow} backlog group")
                                })
                                .text_xs()
                                .text_color(palette.faint)
                                .child(stage_group_words(stage)),
                        ),
                ),
            )
            .child(
                column_rule(stage_lamp(stage), stage.human, palette).debug_selector({
                    let flow = flow.name.clone();
                    move || format!("column {flow} backlog rule")
                }),
            )
            .child(
                div()
                    .debug_selector({
                        let flow = flow.name.clone();
                        move || format!("column {flow} backlog count")
                    })
                    .pt_2()
                    .text_xs()
                    .text_color(palette.faint)
                    .child(waiting_words(stage.beads.len())),
            );
        for bead in stage.beads.iter().take(BACKLOG_TITLES) {
            let sel = Selected::Bead(bead.bead.clone());
            let title = bead.bead.clone();
            column =
                column.child(
                    div()
                        .id(SharedString::from(format!("backlog title {title}")))
                        .debug_selector({
                            let title = title.clone();
                            move || format!("backlog title {title}")
                        })
                        .truncate()
                        .text_xs()
                        .py_0p5()
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select(sel.clone(), window, cx)
                        }))
                        .child(bead.title.clone()),
                );
        }
        let rest = stage.beads.len().saturating_sub(BACKLOG_TITLES);
        if rest > 0 {
            column = column.child(
                div()
                    .debug_selector({
                        let flow = flow.name.clone();
                        move || format!("backlog more {flow}")
                    })
                    .pt_1()
                    .text_xs()
                    .text_color(palette.faint)
                    .child(format!("+{rest} more")),
            );
        }
        column
    }

    /// One bead's card in a stage column or the Elsewhere column: the
    /// headcode and the title, then who works it, how long it has stood
    /// at its stage and that it waits for a seat, each where it holds,
    /// the expected arrival in mono, and the remark in its tone.
    /// `stage` names where it stands; Elsewhere's cards stand in none,
    /// and read `elsewhere card` for the tests where a stage's reads
    /// `card`. A click opens the bead's details, and hovering a worked
    /// card says which session works it, with its group, kind and model.
    pub(crate) fn bead_card(
        &self,
        row: &FlowBead,
        stage: Option<&FlowStage>,
        now: &chrono::DateTime<chrono::Utc>,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let which = match stage {
            Some(_) => "card",
            None => "elsewhere card",
        };
        let bead = row.bead.clone();
        let sel = Selected::Bead(row.bead.clone());
        let tip = worker_words(row).map(|(tip, _)| tip);
        let me = self.model.as_ref().map(Model::me).unwrap_or_default();
        let color = remark_color(&row.remark, &row.tone, &row.held_by, me, palette);
        let mut meta = Vec::new();
        if !row.worker.is_empty() {
            meta.push(format!("worked by {}", row.worker));
        }
        if let Some(stage) = stage
            && !row.entered_stage_at.is_empty()
            && let Some(age) = stage_age(&row.entered_stage_at, &stage.stage, now)
        {
            meta.push(age);
        }
        if stage.is_some_and(|stage| waits_for_seat(row, stage)) {
            meta.push("waits for a seat".to_string());
        }
        let mut card = div()
            .id(SharedString::from(format!("{which} {bead}")))
            .debug_selector({
                let bead = bead.clone();
                move || format!("{which} {bead}")
            })
            .mt_2()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(palette.line)
            .bg(palette.side)
            .flex()
            .flex_col()
            .gap_1()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .when_some(tip, |el, tip| {
                el.tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
            });
        let mut code = div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.text)
            .child(short_bead(&row.bead));
        if let Some(mono) = mono.clone() {
            code = code.font_family(mono);
        }
        card = card.child(code).child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .debug_selector({
                    let bead = bead.clone();
                    move || format!("{which} title {bead}")
                })
                .child(row.title.clone()),
        );
        if !meta.is_empty() {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(palette.faint)
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("{which} meta {bead}")
                    })
                    .child(meta.join(" · ")),
            );
        }
        let mut expected = div()
            .text_xs()
            .text_color(palette.faint)
            .debug_selector({
                let bead = bead.clone();
                move || format!("{which} expected {bead}")
            })
            .child(row.expected.clone());
        if let Some(mono) = mono {
            expected = expected.font_family(mono);
        }
        card = card.child(expected);
        if !row.remark.is_empty() {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(color)
                    .debug_selector({
                        let bead = bead.clone();
                        move || format!("{which} remark {bead}")
                    })
                    .child(you_first(&row.remark, me)),
            );
        }
        card
    }

    /// The line's Elsewhere column: its head with its count, then its
    /// beads as cards, under the flows.
    pub(crate) fn elsewhere_column(
        &self,
        depot: &FlowDepot,
        now: &chrono::DateTime<chrono::Utc>,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut column = grown(LINE_COLUMN).flex_none().flex().flex_col();
        for bead in &depot.elsewhere {
            column = column.child(self.bead_card(bead, None, now, mono.clone(), palette, cx));
        }
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| "elsewhere".to_string())
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Elsewhere"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "elsewhere count".to_string())
                            .text_sm()
                            .text_color(palette.faint)
                            .flex_none()
                            .child(bead_words(depot.elsewhere.len())),
                    ),
            )
            .child(column)
    }

    /// One bead travelling with no line: the title, the expected
    /// arrival in mono at the row's end, and faint the id and remark on
    /// the second line, in the plain list newest change first. A click
    /// opens the details.
    pub(crate) fn no_line_bead_row(
        &self,
        row: &FlowBead,
        mono: Option<SharedString>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bead = row.bead.clone();
        let sel = Selected::Bead(row.bead.clone());
        let el = div()
            .id(SharedString::from(format!("no line bead {bead}")))
            .debug_selector({
                let bead = bead.clone();
                move || format!("no line bead {bead}")
            })
            .mx_1()
            .px_2()
            .py_1p5()
            .rounded_md()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .cursor_pointer()
            .border_1()
            .border_color(rgba(0x00000000))
            .hover(|style| style.border_color(palette.line))
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)));
        let me = self.model.as_ref().map(Model::me).unwrap_or_default();
        let remark = remark_color(&row.remark, &row.tone, &row.held_by, me, palette);
        let mut time = div()
            .flex_none()
            .debug_selector({
                let bead = bead.clone();
                move || format!("no line bead time {bead}")
            })
            .text_color(match row.expected.as_str() {
                "—" => palette.faint,
                _ => palette.text,
            })
            .child(row.expected.clone());
        if let Some(mono) = mono {
            time = time.font_family(mono);
        }
        el.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .debug_selector({
                            let bead = bead.clone();
                            move || format!("no line bead title {bead}")
                        })
                        .child(row.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(palette.faint)
                                .debug_selector({
                                    let bead = bead.clone();
                                    move || format!("no line bead sub {bead}")
                                })
                                .child(bead.clone()),
                        )
                        .children((!row.remark.is_empty()).then(|| {
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(remark)
                                .debug_selector({
                                    let bead = bead.clone();
                                    move || format!("no line bead remark {bead}")
                                })
                                .child(format!("· {}", row.remark))
                        })),
                ),
        )
        .child(time)
    }

    /// The beads travelling with no line: a plain list, newest change
    /// first, each row the title with the expected arrival and remark.
    /// No flows, no stages, no lamps, because a bead with no depot
    /// travels no flow.
    pub(crate) fn talk_no_line(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| "no line head".to_string())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(model::NO_LINE),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(model.no_line_row().map(|row| row.line).unwrap_or_default()),
            );
        let mono = select::mono(cx);
        let mut list = div()
            .id("no line")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.decision)
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        for row in model.unassigned() {
            list = list.child(self.no_line_bead_row(row, mono.clone(), palette, cx));
        }
        if model.unassigned().is_empty() {
            list = list.child(div().text_color(palette.faint).child("None in transit."));
        }
        pane.children(self.reconnect_strip(palette))
            .child(head)
            .child(list)
            .when(!model.notice.is_empty(), |el| {
                el.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(palette.warn)
                        .child(format!("! {}", model.notice)),
                )
            })
    }
}
