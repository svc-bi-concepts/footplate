//! The window: left, whom there is to talk with; right, the open
//! conversation and the line to write in. What it shows is the model's
//! (model.rs); this file draws it and makes the calls the model asks for.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, Image, ImageFormat, KeyBinding,
    MouseButton, ObjectFit, ScrollHandle, SharedString, Window, actions, div, img, prelude::*, px,
};

use crate::api::{self, BlobRef, Client};
use crate::input::{Line, Pasted, Submitted};
use crate::model::{self, Effect, Entry, Model, Row, Target};
use crate::session::{self, Update};
use crate::theme::Palette;

/// The application's icon, drawn in assets/icon.svg.
pub const ICON: &[u8] = include_bytes!("../assets/icon.png");

actions!(footplate, [Next, Previous]);

const CONTEXT: &str = "Footplate";

/// How high an image of a mail stands in the conversation, whatever its own
/// size: the room is there before the image is, so nothing moves under the
/// reader when it comes.
const IMAGE_HEIGHT: f32 = 240.;

/// An image a mail of some conversation names, by its hash.
enum Picture {
    /// The yard is asked for it.
    Asked,
    Here(Arc<Image>),
    /// Why there is none to show, in a line.
    Not(String),
}

impl Picture {
    /// What the yard answered for the image `hash`.
    fn of(hash: &str, answer: api::Result<Option<Vec<u8>>>) -> Picture {
        let short = hash.get(..8).unwrap_or(hash);
        match answer {
            Ok(Some(bytes)) => {
                match model::image_type(&bytes).and_then(ImageFormat::from_mime_type) {
                    Some(format) => Picture::Here(Arc::new(Image::from_bytes(format, bytes))),
                    None => Picture::Not(format!(
                        "image {short}: neither a PNG nor a JPEG, and not shown"
                    )),
                }
            }
            Ok(None) => Picture::Not(format!("image {short}: the yard no longer has it")),
            Err(err) => Picture::Not(format!("image {short}: {err}")),
        }
    }
}

/// Sends an image as mail from `me` to `to`, with the text: `PutBlob`, then
/// `Mail` with the hash it answered. The cap is asked for first, so that an
/// image over it is refused in footplate's words and never travels.
fn mail_image(
    client: &Client,
    me: &str,
    to: &str,
    text: &str,
    image: &[u8],
) -> Result<BlobRef, String> {
    let cap = client.settings().map_err(|err| err.to_string())?.blob_cap();
    if let Some(why) = model::over_cap(image.len(), cap) {
        return Err(why);
    }
    let blob = client.put_blob(image).map_err(|err| err.to_string())?;
    client
        .mail(to, text, me, std::slice::from_ref(&blob.hash))
        .map_err(|err| err.to_string())?;
    Ok(blob)
}

/// The keys of the window, as `yardr chat` has them.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Next, Some(CONTEXT)),
        KeyBinding::new("shift-tab", Previous, Some(CONTEXT)),
    ]);
}

pub struct Footplate {
    client: Client,
    /// What the yard has, once it was read. It is kept while the yard is
    /// away, for the conversation that was open.
    model: Option<Model>,
    /// Why the yard is not there, while it is not.
    down: Option<String>,
    line: Entity<Line>,
    conversation: ScrollHandle,
    /// The images of the conversations that were open, by hash: an image
    /// never changes under its hash, so each is asked for once.
    images: HashMap<String, Picture>,
    focus: FocusHandle,
    icon: Arc<Image>,
}

impl Footplate {
    /// The window on the yard behind `client`, for the user the yard knows
    /// `name` as. It follows the yard from here on.
    pub fn new(client: Client, name: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The title in the window's options reaches a macOS and an X11
        // window; a Wayland one has only what is set on it.
        window.set_window_title(crate::NAME);
        let line = cx.new(Line::new);
        cx.subscribe(&line, |this, _, _: &Submitted, cx| this.send(cx))
            .detach();
        cx.subscribe(&line, |this, _, pasted: &Pasted, cx| {
            this.paste(&pasted.0, cx)
        })
        .detach();

        // The yard is followed on a thread of its own, which blocks on the
        // socket; what it learns comes here.
        let (tell, told) = async_channel::unbounded::<Update>();
        let follower = thread::Builder::new().name("yard".into()).spawn({
            let client = client.clone();
            move || {
                session::follow(&client, &name, session::RETRY, |update| {
                    tell.send_blocking(update).is_ok()
                })
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(update) = told.recv().await {
                if this.update(cx, |this, cx| this.told(update, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Footplate {
            client,
            model: None,
            down: Some(match follower {
                Ok(_) => "Looking for the yard.".to_string(),
                Err(err) => format!("footplate could not start following the yard: {err}"),
            }),
            line,
            conversation: ScrollHandle::new(),
            images: HashMap::new(),
            focus: cx.focus_handle(),
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec())),
        }
    }

    pub fn line(&self) -> &Entity<Line> {
        &self.line
    }

    /// What the thread that follows the yard learned.
    fn told(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Up(state) => {
                let open = self
                    .model
                    .as_ref()
                    .and_then(|m| m.selected().map(str::to_string));
                let (model, effects) = Model::new(*state, open.as_deref());
                self.model = Some(model);
                self.down = None;
                self.conversation.scroll_to_bottom();
                // What the yard did not give while it was away is asked for
                // again.
                self.images
                    .retain(|_, picture| matches!(picture, Picture::Here(_)));
                self.shown(cx);
                self.pictures(cx);
                self.run(effects, cx);
            }
            Update::Event(event) => self.change(cx, |m| m.event(&event)),
            Update::Down(why) => self.down = Some(why),
        }
        cx.notify();
    }

    /// Changes the model and does what the change asks for. When the open
    /// conversation is another or has more mail, its newest comes in view.
    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Model) -> Vec<Effect>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let before = (model.selected().map(str::to_string), model.entries().len());
        let effects = f(model);
        if before != (model.selected().map(str::to_string), model.entries().len()) {
            self.conversation.scroll_to_bottom();
        }
        self.shown(cx);
        self.pictures(cx);
        self.run(effects, cx);
        cx.notify();
    }

    /// Asks the yard for the images of the open conversation it was not
    /// asked for yet.
    fn pictures(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let new: Vec<String> = model
            .images()
            .into_iter()
            .filter(|hash| !self.images.contains_key(*hash))
            .map(str::to_string)
            .collect();
        for hash in new {
            self.images.insert(hash.clone(), Picture::Asked);
            self.ask(
                cx,
                {
                    let hash = hash.clone();
                    move |client, _| client.blob(&hash)
                },
                move |this, answer, cx| {
                    let picture = Picture::of(&hash, answer);
                    this.images.insert(hash, picture);
                    cx.notify();
                },
            );
        }
    }

    /// The line says whom it writes to.
    fn shown(&mut self, cx: &mut Context<Self>) {
        let to = self
            .model
            .as_ref()
            .and_then(|m| m.selected())
            .map(|s| format!("Write to {s}"))
            .unwrap_or_default();
        self.line
            .update(cx, |line, cx| line.set_placeholder(&to, cx));
    }

    /// Makes a call off the window's thread, as the user the window is for,
    /// and hands its answer back to the window.
    fn ask<R: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&Client, &str) -> R + Send + 'static,
        then: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let client = self.client.clone();
        let me = self
            .model
            .as_ref()
            .map(|m| m.me().to_string())
            .unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_executor()
                .spawn(async move { call(&client, &me) })
                .await;
            this.update(cx, |this, cx| then(this, answer, cx)).ok();
        })
        .detach();
    }

    fn notice(&mut self, notice: String, cx: &mut Context<Self>) {
        if let Some(model) = self.model.as_mut() {
            model.notice = notice;
        }
        cx.notify();
    }

    fn run(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Read { name, after } => self.ask(
                    cx,
                    {
                        let name = name.clone();
                        move |client, me| client.conversation(me, &name, after)
                    },
                    move |this, letters, cx| match letters {
                        Ok(letters) => this.change(cx, |m| m.conversation(&name, letters)),
                        Err(err) => this.notice(
                            format!("the conversation with {name} could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Ack { through } => self.ask(
                    cx,
                    move |client, me| client.ack_user_inbox(me, through),
                    move |this, acked, cx| {
                        this.change(cx, |m| {
                            match acked {
                                Ok(()) => m.acked(through),
                                Err(err) => m.ack_failed(&err.to_string()),
                            }
                            Vec::new()
                        })
                    },
                ),
                Effect::Crew => self.ask(
                    cx,
                    |client, _| Ok::<_, api::Error>((client.head()?, client.crew()?)),
                    |this, crew, cx| match crew {
                        Ok((head, crew)) => this.change(cx, |m| m.listed(head, crew)),
                        Err(err) => this.notice(format!("the crew could not be read: {err}"), cx),
                    },
                ),
                Effect::Peers => self.ask(
                    cx,
                    |client, _| client.peers(),
                    |this, peers, cx| match peers {
                        Ok(peers) => this.change(cx, |m| m.linked(&peers)),
                        Err(err) => {
                            this.notice(format!("the linked yards could not be read: {err}"), cx)
                        }
                    },
                ),
            }
        }
    }

    /// Sends what is typed as mail to whom the open conversation is with.
    fn send(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let (Some(to), Some(target)) = (model.selected().map(str::to_string), model.target())
        else {
            return;
        };
        let text = self.line.read(cx).text().trim().to_string();
        match model::unsendable(&text) {
            Some(why) if why.is_empty() => return,
            Some(why) => return self.notice(why, cx),
            None => {}
        }
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.notice(String::new(), cx);
        self.ask(
            cx,
            move |client, me| {
                let sent = match &target {
                    Target::Crew(name) => client.mail(name, &text, me, &[]),
                    Target::Peer { name, peer } => {
                        client.mail_peer(name, peer, &text, me).map(drop)
                    }
                };
                sent.map_err(|err| (err, text))
            },
            move |this, sent, cx| match sent {
                Ok(()) => this.change(cx, |m| m.sent(&to)),
                Err((err, text)) => this.unsent(&to, text, &err.to_string(), cx),
            },
        );
    }

    /// Nothing typed is lost to a failed send: what did not go is back in
    /// the line, before what was typed since, and the window says why.
    fn unsent(&mut self, to: &str, text: String, why: &str, cx: &mut Context<Self>) {
        self.line.update(cx, |line, cx| {
            let typed = line.text().to_string();
            let back = match (text.is_empty(), typed.is_empty()) {
                (_, true) => text,
                (true, false) => typed,
                (false, false) => format!("{text} {typed}"),
            };
            line.set_text(&back, cx);
        });
        self.notice(format!("not sent to {to}: {why}"), cx);
    }

    /// Sends an image that was pasted as mail to whom the open conversation
    /// is with, with what is typed. One that cannot go is refused in words,
    /// and what is typed stays.
    fn paste(&mut self, image: &Image, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let (Some(to), Some(target)) = (model.selected().map(str::to_string), model.target())
        else {
            return;
        };
        let text = self.line.read(cx).text().trim().to_string();
        if let Some(why) = model::unpastable(&target, image.bytes(), &text) {
            return self.notice(why, cx);
        }
        let Target::Crew(name) = target else {
            return;
        };
        let image = image.clone();
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.notice(String::new(), cx);
        self.ask(
            cx,
            move |client, me| match mail_image(client, me, &name, &text, image.bytes()) {
                Ok(blob) => Ok((blob, image)),
                Err(why) => Err((why, text)),
            },
            move |this, sent, cx| match sent {
                Ok((blob, image)) => {
                    // The bytes are here already: the yard is not asked.
                    let picture = Picture::of(&blob.hash, Ok(Some(image.bytes)));
                    this.images.insert(blob.hash, picture);
                    this.change(cx, |m| m.sent(&to))
                }
                Err((why, text)) => this.unsent(&to, text, &why, cx),
            },
        );
    }

    fn select(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.change(cx, |m| m.select(name));
        window.focus(&self.line.focus_handle(cx));
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        self.change(cx, |m| m.step(by));
    }

    /// No yard: why, in words, and that it is tried again.
    fn away(&self, why: &str, palette: Palette) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .child(img(self.icon.clone()).size(px(72.)))
            .child(
                div()
                    .max_w(px(520.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().child(why.to_string()))
                    .child(
                        div()
                            .text_color(palette.faint)
                            .child(format!("The socket: {}", self.client.socket().display())),
                    )
                    .child(div().text_color(palette.faint).child(format!(
                        "footplate tries again every {} seconds.",
                        session::RETRY.as_secs()
                    ))),
            )
    }

    fn list(&self, model: &Model, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = model.rows();
        let head = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .child(img(self.icon.clone()).size(px(32.)).flex_none())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("footplate"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.accent)
                            .truncate()
                            .child(model.me().to_string()),
                    ),
            );
        let body = div()
            .id("rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1();
        let body =
            if rows.is_empty() {
                body.child(
                    div().p_3().text_sm().text_color(palette.faint).child(
                        "This yard has no crew yet, and no linked yard exposes anyone to it.",
                    ),
                )
            } else {
                body.children(rows.into_iter().map(|row| self.row(row, palette, cx)))
            };
        div()
            .w(px(260.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.side)
            .border_r_1()
            .border_color(palette.line)
            .child(head)
            .child(body)
    }

    fn row(&self, row: Row, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = row.name.clone();
        div()
            .id(SharedString::from(format!("row {}", row.name)))
            .mx_1()
            .px_2()
            .py_1p5()
            .rounded_md()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .cursor_pointer()
            .when(row.selected, |el| el.bg(palette.open))
            .when(!row.selected, |el| {
                el.hover(|style| style.bg(palette.hover))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.select(&name, window, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .when(row.unread > 0, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .child(row.name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.faint)
                            .truncate()
                            .child(row.line),
                    ),
            )
            .when(row.unread > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .px_2()
                        .rounded_full()
                        .bg(palette.accent)
                        .text_color(palette.on_accent)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(row.unread.to_string()),
                )
            })
    }

    /// An image of a mail, or the line that says why there is none.
    fn picture(&self, file: &BlobRef, palette: Palette) -> impl IntoElement + use<> {
        let line = move |text: String| div().text_sm().text_color(palette.faint).child(text);
        match self.images.get(&file.hash) {
            Some(Picture::Here(image)) => img(image.clone())
                .h(px(IMAGE_HEIGHT))
                .max_w_full()
                .object_fit(ObjectFit::ScaleDown)
                .with_fallback(move || {
                    line("an image footplate cannot draw".to_string()).into_any_element()
                })
                .into_any_element(),
            Some(Picture::Not(why)) => line(why.clone()).into_any_element(),
            Some(Picture::Asked) | None => div()
                .h(px(IMAGE_HEIGHT))
                .child(line(format!("image, {}…", model::size(file.bytes))))
                .into_any_element(),
        }
    }

    fn entry(&self, entry: Entry, palette: Palette) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .text_color(palette.faint)
                            .child(model::clock(&entry.at, &chrono::Local)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(entry.mine, |el| el.text_color(palette.accent))
                            .child(entry.from),
                    ),
            )
            .when(!entry.text.is_empty(), |el| {
                el.child(div().pl_4().child(entry.text))
            })
            .children(
                entry
                    .files
                    .iter()
                    .map(|file| div().pl_4().pt_1().child(self.picture(file, palette))),
            )
            .when_some(entry.standing, |el, standing| {
                el.child(
                    div()
                        .pl_4()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(format!("· {standing}")),
                )
            })
    }

    fn talk(&self, model: &Model, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(with) = model.selected() else {
            return pane
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child("Nobody to write to yet.");
        };

        let about = model.about();
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(with.to_string()),
            )
            .children(about.into_iter().enumerate().map(|(i, line)| {
                // The first says who that is; what follows keeps mail from
                // arriving.
                div()
                    .text_sm()
                    .text_color(if i == 0 { palette.faint } else { palette.warn })
                    .child(line)
            }));

        let entries = model.entries();
        let mail = div()
            .id("conversation")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.conversation)
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        let mail = if entries.is_empty() {
            mail.child(div().text_color(palette.faint).child(format!(
                "No mail between you and {with} yet. What you write here is mail."
            )))
        } else {
            mail.children(entries.into_iter().map(|entry| self.entry(entry, palette)))
        };

        let (others, from) = model.others();
        let focus = self.line.focus_handle(cx);
        pane.child(head)
            .child(mail)
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
            .when(others > 0, |el| {
                let line = format!(
                    "{others} unread from {}, who have no row here: yardr inbox reads it",
                    from.join(", ")
                );
                el.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(line),
                )
            })
            .child(
                div()
                    .m_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.side)
                    .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                    .child(self.line.clone()),
            )
    }
}

impl Focusable for Footplate {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Footplate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::of(window.appearance());
        let root = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .size_full()
            .flex()
            .bg(palette.ground)
            .text_color(palette.text)
            .text_size(px(14.));
        match (&self.down, &self.model) {
            (Some(why), _) => root.child(self.away(why, palette)),
            (None, Some(model)) => root
                .child(self.list(model, palette, cx))
                .child(self.talk(model, palette, cx)),
            (None, None) => root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Yard;
    use crate::input;
    use gpui::{TestAppContext, VisualTestContext};
    use serde_json::json;
    use std::time::Duration;

    /// The yard is followed on a thread of its own, so the test waits for
    /// what it expects of the window, and no longer than ten seconds.
    fn wait(
        cx: &mut VisualTestContext,
        what: &str,
        mut is: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        for _ in 0..2000 {
            cx.run_until_parked();
            if is(cx) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("the window never showed: {what}");
    }

    fn a_yard_of_jo(yard: &Yard) {
        yard.answer("Head", r#"{"result": 20}"#);
        yard.answer(
            "Crew",
            r#"{"result": [{"name": "assembly", "enabled": true, "state": "stopped"},
                           {"name": "yardmaster", "enabled": true, "state": "running", "status": "idle"}]}"#,
        );
        yard.answer(
            "Peers",
            r#"{"result": [{"name": "north", "send": ["mail"], "receive": ["mail"],
                "said": {"roster": [{"name": "clerk", "kind": "crew", "role": "Keeps the books", "mail": true}], "at": "2026-10-08T09:30:00Z"}}]}"#,
        );
        // The login name is no user of the yard; its operator is.
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "jo", "cursor": 5, "notification_cursor": 0}}"#,
        );
        yard.answer_once_status(
            "GetUser",
            200,
            r#"{"error": {"code": "not-found", "message": "user mac-login: not found"}}"#,
        );
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [{"seq": 12, "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}], "unread": 1, "last": 12, "from": ["yardmaster"]}}"#,
        );
        yard.answer(
            "Conversation",
            r#"{"result": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "Shall I land it?", "read": false}]}"#,
        );
        yard.answer("AckUserInbox", "{}");
        yard.answer("Mail", "{}");
        yard.answer(
            "MailPeer",
            r#"{"result": {"peer": "north", "kind": "mail", "at": "2026-10-08T09:30:00Z"}}"#,
        );
    }

    fn texts(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Vec<String> {
        view.read_with(cx, |v, _| {
            v.model
                .as_ref()
                .map(|m| m.entries().into_iter().map(|e| e.text).collect())
                .unwrap_or_default()
        })
    }

    /// The PNG of docs/api.md of the yardr repository: one pixel, 70 bytes.
    fn png() -> Vec<u8> {
        let mut bytes =
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89"
                .to_vec();
        bytes.extend(b"\0\0\0\rIDATx\xdacd`\xf8_\x0f\0\x02\x87\x01\x80\xebG\xba\x92");
        bytes.extend(b"\0\0\0\0IEND\xaeB`\x82");
        bytes
    }

    const HASH: &str = "497790947d4666760ce38f3c00e852c71fdb66cae849bae8e9ede352719e1581";

    fn notice(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, _| v.model.as_ref().unwrap().notice.clone())
    }

    /// What the window has of the image `hash`: `here`, `asked`, the line
    /// that says why there is none, or nothing.
    fn picture(view: &Entity<Footplate>, cx: &mut VisualTestContext, hash: &str) -> String {
        view.read_with(cx, |v, _| match v.images.get(hash) {
            Some(Picture::Here(_)) => "here".to_string(),
            Some(Picture::Asked) => "asked".to_string(),
            Some(Picture::Not(why)) => why.clone(),
            None => String::new(),
        })
    }

    #[gpui::test]
    fn an_image_is_pasted_as_mail_and_the_images_of_mail_are_shown(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // The yardmaster's mail carries two images, and the yard has one.
        let other = "ab".repeat(32);
        let gone = "cd".repeat(32);
        yard.blob(&other, &png());
        yard.answer(
            "Conversation",
            &json!({"result": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "Like this?", "read": false,
                "files": [{"hash": other, "type": "image/png", "bytes": 70}, {"hash": gone, "type": "image/jpeg", "bytes": 900}]}]})
            .to_string(),
        );
        yard.answer(
            "PutBlob",
            &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
        );
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Like this?"]
        });

        // The image the yard has is there, and the one it has not is a line
        // that says so; the window draws both and stays.
        wait(cx, "the image of the mail", |cx| {
            picture(&view, cx, &other) == "here"
        });
        wait(cx, "that the other image is gone", |cx| {
            picture(&view, cx, &gone) == "image cdcdcdcd: the yard no longer has it"
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(yard.blobs_asked().len(), 2);

        // An image over the yard's cap is refused in words: nothing is put,
        // nothing sent, and what is typed stays.
        yard.answer(
            "Settings",
            r#"{"result": {"operator": "jo", "blob_max_bytes": "69"}}"#,
        );
        cx.simulate_input("Paint the fence like this.");
        let image = Image::from_bytes(ImageFormat::Png, png());
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v");
        wait(cx, "that the image is over the cap", |cx| {
            notice(&view, cx)
                == "not sent to yardmaster: the image is 70 bytes, and this yard takes one of 69 bytes at most (yardr settings: blob_max_bytes)"
        });
        view.read_with(cx, |v, cx| {
            assert_eq!(v.line.read(cx).text(), "Paint the fence like this.")
        });
        assert_eq!(yard.asked("PutBlob"), Vec::<serde_json::Value>::new());

        // Under it: the image is put, and mail sent with its hash and with
        // what is typed.
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        cx.simulate_keystrokes("cmd-v");
        wait(cx, "the mail with the image", |_| {
            yard.asked("Mail")
                == [
                    json!({"to": "yardmaster", "text": "Paint the fence like this.", "actor": "jo", "files": [HASH]}),
                ]
        });
        let put = yard.asked("PutBlob");
        assert_eq!(put.len(), 1);
        assert_eq!(
            put[0]["data"],
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
        );
        wait(cx, "the line empty again", |cx| {
            view.read_with(cx, |v, cx| v.line.read(cx).text().is_empty())
        });
        assert_eq!(notice(&view, cx), "");

        // It comes back from the stream and is shown from the bytes that
        // were pasted: the yard is not asked for them.
        yard.emit(&json!({"seq": 21, "kind": "mail", "actor": "jo", "at": "2026-10-08T09:31:00Z",
            "data": {"to": "yardmaster", "text": "Paint the fence like this.", "files": [{"hash": HASH, "type": "image/png", "bytes": 70}]}}).to_string());
        wait(cx, "the user's own mail", |cx| texts(&view, cx).len() == 2);
        assert_eq!(picture(&view, cx, HASH), "here");
        cx.run_until_parked();
        assert_eq!(yard.blobs_asked().len(), 2);

        // With nothing typed the image is the mail.
        cx.simulate_keystrokes("cmd-v");
        wait(cx, "the mail that is an image only", |_| {
            yard.asked("Mail").last()
                == Some(&json!({"to": "yardmaster", "text": "", "actor": "jo", "files": [HASH]}))
        });

        // What the yard refuses is said, and the text is back in the line.
        yard.answer(
            "PutBlob",
            r#"{"error": {"code": "error", "message": "an image is a PNG or a JPEG"}}"#,
        );
        cx.simulate_input("And this.");
        cx.simulate_keystrokes("cmd-v");
        wait(cx, "why the image was not sent", |cx| {
            notice(&view, cx) == "not sent to yardmaster: an image is a PNG or a JPEG"
        });
        view.read_with(cx, |v, cx| assert_eq!(v.line.read(cx).text(), "And this."));

        // Another kind of image never leaves the window.
        let tiff = Image::from_bytes(ImageFormat::Tiff, b"II*\0".to_vec());
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&tiff));
        cx.simulate_keystrokes("cmd-v");
        assert!(
            notice(&view, cx).contains("a PNG or a JPEG, and what was pasted"),
            "{}",
            notice(&view, cx)
        );

        // To one of a linked yard nothing is sent: the window says that
        // images do not cross the link, and what is typed stays.
        let (puts, mails) = (yard.asked("PutBlob").len(), yard.asked("Mail").len());
        yard.answer("Conversation", r#"{"result": []}"#);
        cx.simulate_keystrokes("tab");
        wait(cx, "the conversation with the linked yard", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().unwrap().selected() == Some("clerk@north")
            })
        });
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(notice(&view, cx), "images do not cross the link yet");
        view.read_with(cx, |v, cx| assert_eq!(v.line.read(cx).text(), "And this."));
        cx.run_until_parked();
        assert_eq!(
            (yard.asked("PutBlob").len(), yard.asked("Mail").len()),
            (puts, mails)
        );
        assert_eq!(yard.asked("MailPeer"), Vec::<serde_json::Value>::new());

        // Text on the clipboard is pasted into the line, as before.
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(" Two\nlines.".into()));
        cx.simulate_keystrokes("cmd-v");
        view.read_with(cx, |v, cx| {
            assert_eq!(v.line.read(cx).text(), "And this. Two lines.")
        });
    }

    #[gpui::test]
    fn the_window_shows_the_yard_writes_to_it_and_says_when_it_is_away(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yard.sock");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(Client::new(&socket), "mac-login".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        // A Wayland window has no title but the one set on it.
        assert_eq!(cx.window_title().as_deref(), Some("footplate"));

        // No yard: the window says so in words, and goes on.
        wait(cx, "that there is no yard", |cx| {
            view.read_with(cx, |v, _| {
                v.down
                    .as_deref()
                    .is_some_and(|why| why.starts_with("No yard"))
            })
        });

        // The yard starts: the window finds it without a restart, as the
        // yard's operator, on the yardmaster, whose mail it acknowledges.
        let yard = Yard::start_at(&socket);
        a_yard_of_jo(&yard);
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            assert_eq!(v.down, None);
            assert_eq!(model.me(), "jo");
            assert_eq!(model.selected(), Some("yardmaster"));
            let rows: Vec<String> = model.rows().into_iter().map(|r| r.name).collect();
            assert_eq!(rows, ["assembly", "yardmaster", "clerk@north"]);
        });
        wait(cx, "the mail acknowledged", |_| {
            yard.asked("AckUserInbox") == [json!({"name": "jo", "through": 12})]
        });
        assert_eq!(
            yard.asked("Conversation")[0],
            json!({"a": "jo", "b": "yardmaster", "after": 0})
        );

        // What is typed is mail to the open conversation, from the user.
        cx.simulate_input("Thanks. Land it.");
        view.read_with(cx, |v, cx| {
            assert_eq!(v.line.read(cx).text(), "Thanks. Land it.")
        });
        cx.simulate_keystrokes("enter");
        wait(cx, "the mail sent", |_| {
            yard.asked("Mail")
                == [json!({"to": "yardmaster", "text": "Thanks. Land it.", "actor": "jo"})]
        });
        view.read_with(cx, |v, cx| assert_eq!(v.line.read(cx).text(), ""));
        // An empty line sends nothing.
        cx.simulate_keystrokes("enter");

        // New mail appears without a restart, and is acknowledged as it is
        // shown; the user's own comes from the stream as well.
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "jo", "data": {"to": "yardmaster", "text": "Thanks. Land it."}, "at": "2026-10-08T09:31:00Z"}"#);
        yard.emit(r#"{"seq": 22, "kind": "mail", "actor": "yardmaster", "data": {"to": "jo", "to_user": true, "text": "Landed."}, "at": "2026-10-08T09:32:00Z"}"#);
        wait(cx, "the new mail", |cx| {
            texts(&view, cx) == ["Shall I land it?", "Thanks. Land it.", "Landed."]
        });
        wait(cx, "the new mail acknowledged", |_| {
            yard.asked("AckUserInbox").last() == Some(&json!({"name": "jo", "through": 22}))
        });
        assert_eq!(yard.asked("Mail").len(), 1);

        // Mail in a conversation that is not open counts on its row.
        yard.emit(r#"{"seq": 23, "kind": "mail", "actor": "clerk@north", "data": {"to": "jo", "to_user": true, "text": "The ledger is closed."}, "at": "2026-10-08T09:33:00Z"}"#);
        wait(cx, "the unread count", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().unwrap().unread("clerk@north") == 1
            })
        });

        // Tab opens the next row: one of a linked yard, written to with
        // MailPeer.
        yard.answer(
            "Conversation",
            r#"{"result": [{"seq": 23, "at": "2026-10-08T09:33:00Z", "from": "clerk@north", "to": "jo", "text": "The ledger is closed.", "read": false}]}"#,
        );
        cx.simulate_keystrokes("tab");
        wait(cx, "the conversation with the linked yard", |cx| {
            texts(&view, cx) == ["The ledger is closed."]
        });
        wait(cx, "its mail acknowledged", |_| {
            yard.asked("AckUserInbox").last() == Some(&json!({"name": "jo", "through": 23}))
        });
        cx.simulate_input("Good.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the mail queued for the peer", |_| {
            yard.asked("MailPeer")
                == [json!({"to": "clerk", "peer": "north", "text": "Good.", "actor": "jo"})]
        });

        // A mail the yard refuses is back in the line, with the yard's words.
        yard.answer(
            "MailPeer",
            r#"{"error": {"code": "error", "message": "the link to north carries no mail"}}"#,
        );
        cx.simulate_input("Again.");
        cx.simulate_keystrokes("enter");
        wait(cx, "why it was not sent", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().unwrap().notice
                    == "not sent to clerk@north: the link to north carries no mail"
            })
        });
        view.read_with(cx, |v, cx| assert_eq!(v.line.read(cx).text(), "Again."));

        // The server goes down: the window says so and keeps running, and
        // a send meanwhile panics nothing.
        yard.stop();
        wait(cx, "that the yard is away", |cx| {
            view.read_with(cx, |v, _| {
                v.down
                    .as_deref()
                    .is_some_and(|why| why.starts_with("No yard"))
            })
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        // It is back: the conversation that was open is open again.
        let yard = Yard::start_at(&socket);
        a_yard_of_jo(&yard);
        wait(cx, "the yard again", |cx| {
            view.read_with(cx, |v, _| v.down.is_none())
        });
        view.read_with(cx, |v, _| {
            assert_eq!(v.model.as_ref().unwrap().selected(), Some("clerk@north"))
        });
    }
}
