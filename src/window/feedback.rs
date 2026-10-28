//! The Feedback form over the window: a message, an email if the person
//! would like an answer, the plain line of what is sent, and the button that
//! sends it (feedback.rs) off the window's thread. It stands where the About
//! card does, over whatever shows, and shuts with esc, its Cancel or a click
//! outside it; what is typed in it is kept nowhere, so shutting it loses it.

use gpui::{
    AnyElement, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton,
    Window, div, prelude::*, px, rems,
};

use super::Footplate;
use super::ask::{SEND_WORDS, card_button};
use super::metrics::{RAIL_LABEL, RAIL_TILE};
use super::rail::RailTip;
use super::ui::{card_layer, small_button};
use crate::feedback;
use crate::input::{Line, Submitted};
use crate::theme::Palette;

/// What the form says of its fields, and of what came of a send.
const MESSAGE_WORDS: &str = "Message";
const EMAIL_WORDS: &str = "Email, if you'd like an answer";
const THANKS: &str = "Sent. Thank you.";
const MESSAGE_WANTED: &str = "A message is wanted: write something to send.";

/// The form over the window: its two fields, whether a send is on its way,
/// why the last did not go, whether one did, and the focus to give back
/// when it shuts.
pub(crate) struct Feedback {
    pub(crate) message: Entity<Line>,
    pub(crate) email: Entity<Line>,
    /// A send is on its way: no second one goes after it.
    pub(crate) sending: bool,
    /// Why the last send did not go, in the form's words, with what was
    /// typed still in the fields.
    pub(crate) said: Option<String>,
    /// The message went. The fields are empty and gone from the card, which
    /// says thanks instead.
    pub(crate) sent: bool,
    pub(crate) back: Option<FocusHandle>,
}

impl Footplate {
    /// Opens the form over the window, the focus in its message, where
    /// feedback is offered at all. One that stands already is left as it is.
    pub(crate) fn show_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.feedback.is_some() || self.means.feedback.is_none() {
            return;
        }
        let message = cx.new(Line::new);
        let email = cx.new(|cx| {
            let mut field = Line::new(cx);
            field.set_placeholder("jo@example.net", cx);
            field
        });
        for field in [&message, &email] {
            // Enter in either field sends, as it does in the composer;
            // shift+Enter breaks the message's line.
            cx.subscribe(field, |this, _, _: &Submitted, cx| this.send_feedback(cx))
                .detach();
        }
        // Send is dim while the message is blank: the window is drawn again
        // with what is typed, and a message that is wanted is not said to
        // be once it is typed.
        cx.observe(&message, |this, message, cx| {
            let typed = !message.read(cx).text().trim().is_empty();
            if let Some(form) = this.feedback.as_mut()
                && typed
                && form.said.as_deref() == Some(MESSAGE_WANTED)
            {
                form.said = None;
            }
            cx.notify();
        })
        .detach();
        cx.observe(&email, |_, _, cx| cx.notify()).detach();
        let back = window.focused(cx);
        window.focus(&message.focus_handle(cx));
        self.feedback = Some(Feedback {
            message,
            email,
            sending: false,
            said: None,
            sent: false,
            back,
        });
        cx.notify();
    }

    /// Shuts the form, and the focus goes back where it was before it
    /// opened. What was typed goes with it. A send that is on its way is
    /// not recalled; its answer finds the form gone and tells no one.
    pub(crate) fn shut_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.feedback.take() {
            match form.back {
                Some(back) => window.focus(&back),
                None => window.focus(&self.focus.clone()),
            }
        }
        cx.notify();
    }

    /// Tab moves between the form's two fields, and shift+Tab back: while it
    /// stands the keys step nothing behind it.
    pub(crate) fn feedback_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.feedback.as_ref().filter(|form| !form.sent) else {
            return;
        };
        let to = match form.message.focus_handle(cx).is_focused(window) {
            true => form.email.focus_handle(cx),
            false => form.message.focus_handle(cx),
        };
        window.focus(&to);
    }

    /// Sends what is typed: off the window's thread, and once at a time. A
    /// message that goes closes the fields to say thanks; one that does not
    /// leaves the form as it was, with why. A blank message goes nowhere.
    pub(crate) fn send_feedback(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.means.feedback.clone() else {
            return;
        };
        let Some(form) = self.feedback.as_mut() else {
            return;
        };
        if form.sending || form.sent {
            return;
        }
        let message = form.message.read(cx).text().trim().to_string();
        if message.is_empty() {
            form.said = Some(MESSAGE_WANTED.to_string());
            cx.notify();
            return;
        }
        let email = form.email.read(cx).text().trim().to_string();
        form.sending = true;
        form.said = None;
        let form_id = form.message.entity_id();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_executor()
                .spawn(async move { feedback::send(&project, &message, &email) })
                .await;
            this.update(cx, |this, cx| {
                // A form that was shut meanwhile, or shut and opened again,
                // has nothing to be told.
                let Some(form) = this
                    .feedback
                    .as_mut()
                    .filter(|form| form.message.entity_id() == form_id)
                else {
                    return;
                };
                form.sending = false;
                match answer {
                    Ok(()) => {
                        form.sent = true;
                        // Gone with the card's fields: nothing of it stays.
                        let fields = [form.message.clone(), form.email.clone()];
                        for field in fields {
                            field.update(cx, |line, cx| line.set_text("", cx));
                        }
                        // The fields held the focus and are not drawn
                        // any more: esc is the window's.
                        let (focus, handle) = (this.focus.clone(), this.handle);
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, window, _| window.focus(&focus));
                        });
                    }
                    Err(why) => form.said = Some(feedback::not_sent(&why)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The rail's Feedback, above the settings' gear, where feedback is
    /// offered: its word alone, in the size of the labels under the other
    /// tiles. The rail's glyphs are the railway's (the lamp, the arm, the
    /// track), and none of them says a word to yardr's makers, so this has
    /// none, and takes the height of its label and no more: a tile with a
    /// glyph would not leave the foot room at the narrowest window, under a
    /// Mac's title bar, where the foot has a few pixels to spare with this.
    pub(crate) fn feedback_tile(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("feedback")
            .debug_selector(|| "feedback tile".to_string())
            .w(px(RAIL_TILE))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .py_1()
            .mt_1()
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .when(self.feedback.is_some(), |el| el.bg(palette.open))
            .on_click(cx.listener(|this, _, window, cx| this.show_feedback(window, cx)))
            .tooltip(|_, cx| {
                cx.new(|_| RailTip("Send feedback to yardr".to_string()))
                    .into()
            })
            .child(
                div()
                    .debug_selector(|| "feedback tile label".to_string())
                    .flex_none()
                    .text_size(px(RAIL_LABEL))
                    .text_color(palette.faint)
                    .child("Feedback"),
            )
    }

    /// The form over the window: the message, the email, what is sent said
    /// plainly under them, and Cancel and Send. A send that did not go says
    /// why in red under that, with the fields as they were; one that went
    /// leaves the card to say thanks, with Close.
    pub(crate) fn feedback_layer(
        &self,
        form: &Feedback,
        top: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let head = div().font_weight(FontWeight::SEMIBOLD).child("Feedback");
        let mut rows = vec![head.into_any_element()];
        if form.sent {
            rows.push(
                div()
                    .debug_selector(|| "feedback thanks".to_string())
                    .child(THANKS)
                    .into_any_element(),
            );
            rows.push(
                div()
                    .flex()
                    .justify_end()
                    .child(
                        small_button("Close (esc)".to_string(), palette)
                            .id("feedback close")
                            .debug_selector(|| "feedback close".to_string())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.shut_feedback(window, cx)),
                            ),
                    )
                    .into_any_element(),
            );
            return card_layer(
                "feedback",
                top,
                palette,
                cx.listener(|this: &mut Footplate, _: &ClickEvent, window, cx| {
                    this.shut_feedback(window, cx)
                }),
                rows,
            )
            .into_any_element();
        }
        let blank = form.message.read(cx).text().trim().is_empty();
        let send = match (form.sending, blank) {
            (true, _) => "Sending…".to_string(),
            _ => SEND_WORDS.to_string(),
        };
        let click =
            (!form.sending && !blank).then(|| cx.listener(|this, _, _, cx| this.send_feedback(cx)));
        rows.extend([
            feedback_field(
                MESSAGE_WORDS,
                "feedback message",
                &form.message,
                true,
                palette,
                cx,
            )
            .into_any_element(),
            feedback_field(
                EMAIL_WORDS,
                "feedback email",
                &form.email,
                false,
                palette,
                cx,
            )
            .into_any_element(),
            div()
                .debug_selector(|| "feedback plain".to_string())
                .pr_4()
                .text_sm()
                .text_color(palette.faint)
                .child(feedback::PLAIN)
                .into_any_element(),
        ]);
        if let Some(said) = &form.said {
            rows.push(
                div()
                    .debug_selector(|| "feedback said".to_string())
                    .text_sm()
                    .text_color(palette.warn)
                    .child(said.clone())
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    small_button("Cancel (esc)".to_string(), palette)
                        .id("feedback cancel")
                        .debug_selector(|| "feedback cancel".to_string())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.shut_feedback(window, cx)),
                        ),
                )
                .child(card_button(
                    "feedback send".to_string(),
                    send,
                    palette,
                    click,
                ))
                .into_any_element(),
        );
        card_layer(
            "feedback",
            top,
            palette,
            cx.listener(|this: &mut Footplate, _: &ClickEvent, window, cx| {
                this.shut_feedback(window, cx)
            }),
            rows,
        )
        .into_any_element()
    }
}

/// One field of the form with its words over it. A press anywhere in its
/// frame is for the line, as the composer's is; the message's frame is tall
/// enough for several rows, and the line grows past that.
fn feedback_field(
    words: &'static str,
    selector: &'static str,
    line: &Entity<Line>,
    tall: bool,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> impl IntoElement {
    let focus = line.focus_handle(cx);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().text_color(palette.faint).child(words))
        .child(
            div()
                .debug_selector(move || selector.to_string())
                .px_3()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(palette.line)
                .bg(palette.ground)
                .when(tall, |el| el.min_h(rems(6.)))
                .on_mouse_down(MouseButton::Left, move |_, window, _| {
                    window.focus(&focus);
                    window.prevent_default();
                })
                .child(line.clone()),
        )
}
