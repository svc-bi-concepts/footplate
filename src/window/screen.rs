//! One session's screen over the window: what the session's pane
//! shows, drawn like the terminal — monospace, dark, the styling as
//! printed — from the `SessionScreen` answer, parsed by [`crate::ansi`].

use gpui::{
    Context, Div, FocusHandle, FontStyle, FontWeight, MouseButton, Rgba, SharedString, Stateful,
    StyledText, TextRun, TextStyle, UnderlineStyle, Window, div, prelude::*, px, rgb, rgba,
};

use super::Footplate;
use super::metrics::IMAGE_MARGIN;
use super::rail::RailTip;
use super::ui::terminal_glyph;
use crate::ansi::{self, Color, Span};
use crate::model;
use crate::select;
use crate::theme::{Look, Palette};

/// The screen open over the window: the session it reads, what the read
/// has of it, and the focus to give back when it shuts.
pub(crate) struct ScreenOpen {
    pub(crate) session: String,
    pub(crate) shown: Shown,
    pub(crate) back: Option<FocusHandle>,
    pub(super) request: u64,
}

impl ScreenOpen {
    /// Whether this answer is still the newest read of this open screen.
    fn takes(&self, session: &str, request: u64) -> bool {
        self.session == session && self.request == request
    }
}

/// What the open screen has: the read is underway, the parsed screen, that
/// the yard reads it on its machine alone, or why there is none.
pub(crate) enum Shown {
    /// The yard is asked for it.
    Asked,
    Here(ansi::Screen),
    /// The yard reads no screen down the link from its edge: it is not
    /// asked, or it refused as much. Nothing is wrong, so nothing is red.
    OnTheYard,
    /// Why there is none to show, in a line.
    Not(String),
}

/// What the entries' hover says: what opening the screen shows.
pub(crate) const SCREEN_TIP: &str = "screen: what the session's pane shows";

/// A monospaced character's width, in ems: the advance of the faces
/// [`select::mono`] names.
const CHAR_ADVANCE: f32 = 0.6;
/// How narrow the card never is, in rems: an empty screen still stands
/// with its head and its refresh.
const CARD_MIN: f32 = 20.;

/// The control that opens `session`'s screen: link-styled "Screen" text
/// with the hover, small wherever it stands. A click opens the screen
/// instead of what the control stands on.
pub(crate) fn screen_control(
    session: String,
    id: String,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .flex_none()
        .text_xs()
        .text_color(palette.link)
        .cursor_pointer()
        .tooltip(|_, cx| cx.new(|_| RailTip(SCREEN_TIP.to_string())).into())
        // The press takes no focus: the window holds the keys once the
        // screen shows, and the focus goes back where the click found
        // it when it shuts.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.open_screen(session.clone(), window, cx);
        }))
        .child("Screen")
}

/// The control that opens `session`'s screen from the conversation
/// header: the terminal glyph at the header's right edge, quiet until
/// the pointer is over it. The hover, the press that takes no focus
/// and the click are the text control's.
pub(crate) fn screen_icon(
    session: String,
    id: String,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .flex_none()
        .p_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(palette.hover))
        .tooltip(|_, cx| cx.new(|_| RailTip(SCREEN_TIP.to_string())).into())
        // The press takes no focus, as the text control's: the window
        // holds the keys once the screen shows, and the focus goes back
        // where the click found it when it shuts.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.open_screen(session.clone(), window, cx);
        }))
        .child(terminal_glyph(palette.faint))
}

/// The triple draws as an Rgba.
fn tint(color: Color) -> Rgba {
    let (r, g, b) = color.rgb();
    rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
}

/// The colour dimmed: halfway to the ground it stands on.
fn dimmed(color: Rgba, ground: Rgba) -> Rgba {
    Rgba {
        r: (color.r + ground.r) / 2.,
        g: (color.g + ground.g) / 2.,
        b: (color.b + ground.b) / 2.,
        a: color.a,
    }
}

/// The colours `span` draws in: its foreground and ground, inverse
/// with the card's ground for the missing one, and dim halfway there.
/// The text's own colour is the dark palette's, on the card's ground.
fn tints(span: &Span, dark: &Palette) -> (Rgba, Option<Rgba>) {
    let mut fg = span.style.fg.map(tint).unwrap_or(dark.text);
    let mut bg = span.style.bg.map(tint);
    if span.style.inverse {
        (fg, bg) = (bg.unwrap_or(dark.ground), Some(fg));
    }
    if span.style.dim {
        fg = dimmed(fg, bg.unwrap_or(dark.ground));
    }
    (fg, bg)
}

/// The run `span` is drawn in, in the mono `face`: its weight, slant,
/// underline in its own colour, and the foreground [`tints`] resolves.
/// One span draws in one run, in an element of its own: gpui walks
/// glyphs in the order the shaper returns them, and CoreText returns
/// cascade-fallback glyphs (the panes' ornaments) out of string order,
/// so a run that shares its element with another styling can draw in
/// the wrong one. The span's ground rides on its div below, for the
/// same reason: a run's background starts at its first-visited glyph,
/// which is past a leading ornament.
fn run(span: &Span, face: &TextStyle, dark: &Palette, mono: Option<&SharedString>) -> TextRun {
    let (fg, _) = tints(span, dark);
    let mut run = face.to_run(span.text.len());
    if let Some(mono) = mono {
        run.font.family = mono.clone();
    }
    if span.style.bold {
        run.font.weight = FontWeight::BOLD;
    }
    if span.style.italic {
        run.font.style = FontStyle::Italic;
    }
    run.color = fg.into();
    if span.style.underline {
        run.underline = Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(fg.into()),
            wavy: false,
        });
    }
    run
}

impl Footplate {
    /// Opens the session's screen over the window: the read goes out,
    /// and the screen shows when it answers. Over one already shown the
    /// second shows instead, and the focus still goes back where the
    /// first found it. While one shows, the window holds the keys:
    /// nothing typed goes to a line that is not in view.
    pub(crate) fn open_screen(
        &mut self,
        session: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let back = self
            .screen_open
            .take()
            .and_then(|open| open.back)
            .or_else(|| window.focused(cx));
        self.screen_open = Some(ScreenOpen {
            session,
            shown: Shown::Asked,
            back,
            request: 0,
        });
        // A pane is read from its left edge, where its lines begin, and
        // from its bottom, which the answer stands it at: the handle
        // outlives the card, and how far the last one was read sideways
        // is not for this one. A refresh leaves the reader where they are.
        self.screen_scroll.set_offset(gpui::Point::default());
        window.focus(&self.focus);
        self.read_screen(cx);
        cx.notify();
    }

    /// Reads the open screen again: the refresh, and the open. An answer
    /// for a read that no longer stands current is dropped: a shut
    /// screen stays shut, and a refresh keeps its own answer. A yard that
    /// reads no screen down the link from its edge is not asked, and one
    /// that refuses as much is not asked again: the card says where the
    /// screen is read.
    pub(crate) fn read_screen(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.screen_open.as_mut() else {
            return;
        };
        let session = open.session.clone();
        self.screen_read = self.screen_read.wrapping_add(1);
        let request = self.screen_read;
        open.request = request;
        if self.model.as_ref().is_some_and(|m| m.screen_on_the_yard()) {
            open.shown = Shown::OnTheYard;
            cx.notify();
            return;
        }
        open.shown = Shown::Asked;
        self.ask(
            cx,
            {
                let session = session.clone();
                move |client, _| client.session_screen(&session)
            },
            move |this, answer, cx| {
                if !this
                    .screen_open
                    .as_ref()
                    .is_some_and(|open| open.takes(&session, request))
                {
                    return;
                }
                let shown = match answer {
                    Ok(screen) => {
                        // The pane's now is at its bottom, where the agent's
                        // input line is, for the open and for each refresh.
                        // How far that is, only the layout knows: gpui's flag
                        // is taken in the prepaint of the frame that first
                        // draws these lines, from its own measure.
                        this.screen_scroll.scroll_to_bottom();
                        Shown::Here(ansi::parse(&screen.screen))
                    }
                    // However the client came by the yard, it came down
                    // the link from the edge: no Screen is offered from
                    // here, and the refusal's advice is not shown, since
                    // the device did nothing wrong.
                    Err(err) if model::screen_over_the_link(&err) => {
                        if let Some(model) = this.model.as_mut() {
                            model.screen_refused_down_the_link();
                        }
                        Shown::OnTheYard
                    }
                    Err(err) => Shown::Not(format!("the screen could not be read: {err}")),
                };
                this.screen_open
                    .as_mut()
                    .expect("the screen stands open")
                    .shown = shown;
                cx.notify();
            },
        );
    }

    /// Shuts the screen shown over the window, and the focus goes back
    /// where it was before it opened. Shutting none shuts nothing: the
    /// × stands on the backdrop, and its click shuts through both.
    pub(crate) fn shut_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.screen_open.take() {
            match open.back {
                Some(back) => window.focus(&back),
                None => window.focus(&self.focus.clone()),
            }
        }
        cx.notify();
    }

    /// The screen open over the window: the dark backdrop, the card with
    /// the session's name, the refresh and the ×, and the screen's lines
    /// as the terminal draws them — monospace, dark, no wrapping. A
    /// click outside the card or on the × shuts it, as Esc does; a
    /// click on the card shuts nothing.
    pub(crate) fn screen_layer(
        &self,
        open: &ScreenOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The dark palette explicitly, whatever the window's look: the
        // screen looks like the terminal, dark always.
        let dark = Palette::of(Look::Dark, window.appearance());
        let rem = window.rem_size();
        let mut face = window.text_style();
        let mono = select::mono(cx);
        if let Some(mono) = mono.as_ref() {
            face.font_family = mono.clone();
        }
        face.color = dark.text.into();
        // The card fits the longest line, with its padding: the pane's
        // width, as the call omits it and the parse measures it. A
        // screen not there yet sizes to its line; a narrow one never
        // stands under the minimum, and none outgrows the window.
        let margin = f32::from(rem) * IMAGE_MARGIN;
        let avail = window.viewport_size();
        let card_w = match &open.shown {
            Shown::Here(screen) => {
                let em = f32::from(face.font_size.to_pixels(rem));
                let w = f32::from(rem) * 2. + screen.width as f32 * CHAR_ADVANCE * em;
                w.clamp(
                    f32::from(rem) * CARD_MIN,
                    f32::from(avail.width) - 2. * margin,
                )
            }
            Shown::Asked | Shown::OnTheYard | Shown::Not(_) => f32::from(rem) * CARD_MIN,
        };
        let body = match &open.shown {
            Shown::Asked => div()
                .text_color(dark.faint)
                .child("Reading the screen.")
                .into_any_element(),
            Shown::OnTheYard => div()
                .debug_selector(|| "screen on the yard".to_string())
                .text_color(dark.faint)
                .child(model::SCREEN_ON_THE_YARD)
                .into_any_element(),
            Shown::Not(why) => {
                let why = why.clone();
                // gpui lays a text out unbroken when asked for its min or
                // max content, so as a flex item the line would be its own
                // minimum width and run out of the card: free of it, the
                // item takes the card's width and the line wraps there.
                // Where the card is shorter than the wrapped line (a short
                // window), the line scrolls, as the terminal's screen does.
                div()
                    .id("screen refusal")
                    .debug_selector(|| "screen refusal".to_string())
                    .min_w_0()
                    .overflow_y_scroll()
                    .text_color(dark.warn)
                    .child(
                        div()
                            .debug_selector(|| "screen refusal text".to_string())
                            .child(why),
                    )
                    .into_any_element()
            }
            Shown::Here(screen) => {
                // The lines do not shrink, as the cells in their rows do
                // not: they stand as wide as their longest row, whatever
                // the card is (see the scroll below).
                let mut lines = div().flex().flex_col().flex_none();
                for line in &screen.lines {
                    let mut row = div().flex().flex_row();
                    if line.is_empty() {
                        // An empty line stands as a space: an empty text
                        // draws no height, and the terminal's blank lines
                        // stand.
                        row = row.child(StyledText::new(" ").with_runs(vec![face.to_run(1)]));
                    }
                    for span in line {
                        let (_, bg) = tints(span, &dark);
                        let mut cell = div().flex_none();
                        if let Some(bg) = bg {
                            cell = cell.bg(bg);
                        }
                        row = row.child(cell.child(
                            StyledText::new(span.text.clone()).with_runs(vec![run(
                                span,
                                &face,
                                &dark,
                                mono.as_ref(),
                            )]),
                        ));
                    }
                    lines = lines.child(row);
                }
                // gpui takes how far a scroll goes from the bounds of its
                // direct children. A block child would be as wide as the
                // card, and the cells that run out of it would not count:
                // the height would scroll, the width never. A flex item
                // that does not shrink is as wide as its content, which the
                // layout measures as the text is drawn, so a wide pane
                // scrolls to its last character and not past it, and a
                // narrow one has nowhere to go. The card's width is no
                // measure to scroll by: it is an estimate, 0.6 em a char
                // at the size the text has before the root sets its own,
                // and a scroll by it would end in blank room or short of
                // the line. Items at the start keep the lines their own
                // height, for the scroll down.
                div()
                    .id("screen scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .overflow_scroll()
                    .track_scroll(&self.screen_scroll)
                    .child(lines)
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .into_any_element()
            }
        };
        let session = open.session.clone();
        div()
            .id("screen overlay")
            .debug_selector(|| "screen overlay".to_string())
            .block_mouse_except_scroll()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .right(px(0.))
            .bottom(px(0.))
            .bg(rgba(0x000000cc))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, window, cx| this.shut_screen(window, cx)))
            .child(
                div()
                    .id(SharedString::from(format!("screen card {session}")))
                    .debug_selector({
                        let session = session.clone();
                        move || format!("screen card {session}")
                    })
                    .w(px(card_w))
                    .max_h(px(f32::from(avail.height) - 2. * margin))
                    .rounded_lg()
                    .border_1()
                    .border_color(dark.line)
                    .bg(dark.ground)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(dark.text)
                                    .debug_selector({
                                        let session = session.clone();
                                        move || format!("screen head {session}")
                                    })
                                    .child(session.clone()),
                            )
                            .child(
                                div()
                                    .id("screen refresh")
                                    .debug_selector(|| "screen refresh".to_string())
                                    .flex_none()
                                    .text_sm()
                                    .text_color(dark.link)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.read_screen(cx)))
                                    .child("Refresh"),
                            )
                            .child(
                                div()
                                    .id("screen close")
                                    .debug_selector(|| "screen close".to_string())
                                    .flex_none()
                                    .text_xl()
                                    .text_color(dark.faint)
                                    .cursor_pointer()
                                    .child("×")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.shut_screen(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .flex()
                            .debug_selector({
                                let session = session.clone();
                                move || format!("screen body {session}")
                            })
                            .child(body),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_latest_read_of_the_open_session_is_taken() {
        let open = ScreenOpen {
            session: "assembly".into(),
            shown: Shown::Asked,
            back: None,
            request: 2,
        };

        assert!(!open.takes("assembly", 1));
        assert!(!open.takes("yardmaster", 2));
        assert!(open.takes("assembly", 2));
    }
}
