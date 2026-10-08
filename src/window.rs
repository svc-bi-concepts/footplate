//! The window: left, whom there is to talk with; right, the open
//! conversation and the line to write in. What it shows is the model's
//! (model.rs); this file draws it and makes the calls the model asks for.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gpui::{
    App, Context, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, Image, ImageFormat,
    KeyBinding, MouseButton, ObjectFit, ScrollHandle, SharedString, Window, actions, div, img,
    prelude::*, px,
};

use crate::api::{self, BlobRef, Client};
use crate::draft::{self, Attachment, Drafts, Found};
use crate::input::{Emptied, Line, Pasted, Submitted};
use crate::model::{self, Effect, Entry, Model, Row, Target};
use crate::session::{self, Update};
use crate::theme::Palette;

/// The application's icon, from the 1024px master in assets/icon.png.
pub const ICON: &[u8] = include_bytes!("../assets/icon.png");

actions!(footplate, [Next, Previous]);

const CONTEXT: &str = "Footplate";

/// How high an image of a mail stands in the conversation, whatever its own
/// size: the room is there before the image is, so nothing moves under the
/// reader when it comes.
const IMAGE_HEIGHT: f32 = 240.;

/// The side of an attachment of the draft, above the line.
const ATTACHMENT: f32 = 56.;

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

/// A draft that did not go: all of it, to be put back, with the attachment
/// that kept it, if it was one, and why.
struct Unsent {
    text: String,
    images: Vec<Attachment>,
    at: Option<u64>,
    why: String,
}

/// Sends a draft as one mail from `me`: `PutBlob` for each image in the
/// order attached, then `Mail` with the text and the hashes they answered,
/// `MailPeer` to one of a linked yard. The cap is asked for first, so that
/// an image over it is refused in footplate's words and none travels.
/// Whether the link carries an image is the yard's to say: it refuses the
/// mail when not, in its words. Answers the images as the yard has them, or
/// the attachment that kept the mail, if it was one, and why.
fn post(
    client: &Client,
    me: &str,
    target: &Target,
    text: &str,
    images: &[Attachment],
) -> Result<Vec<BlobRef>, (Option<u64>, String)> {
    let said = |err: api::Error| (None, err.to_string());
    let mut blobs = Vec::with_capacity(images.len());
    if !images.is_empty() {
        let cap = client.settings().map_err(said)?.blob_cap();
        if let Some((image, why)) = images
            .iter()
            .find_map(|a| Some((a, model::over_cap(a.bytes.len(), cap)?)))
        {
            return Err((Some(image.id), why));
        }
        for image in images {
            let blob = client
                .put_blob(&image.bytes)
                .map_err(|err| (Some(image.id), err.to_string()))?;
            blobs.push(blob);
        }
    }
    let hashes: Vec<String> = blobs.iter().map(|blob| blob.hash.clone()).collect();
    match target {
        Target::Crew(name) => client.mail(name, text, me, &hashes).map_err(said)?,
        Target::Peer { name, peer } => {
            client
                .mail_peer(name, peer, text, me, &hashes)
                .map_err(said)?;
        }
    }
    Ok(blobs)
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
    /// What is attached and not sent, for each conversation, and the text
    /// typed in those that are not open; the open one's is in the line.
    drafts: Drafts,
    /// The attachments that are images, by id, as they are drawn.
    thumbs: HashMap<u64, Arc<Image>>,
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
        cx.subscribe(&line, |this, _, _: &Emptied, cx| this.remove_last(cx))
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
            drafts: Drafts::default(),
            thumbs: HashMap::new(),
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
                self.turned(open, cx);
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
        self.turned(before.0, cx);
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

    /// The conversation `from` was open. When another is now, what is typed
    /// stays with the one it was typed in, and the line has what was typed
    /// in the one that is open.
    fn turned(&mut self, from: Option<String>, cx: &mut Context<Self>) {
        let to = self
            .model
            .as_ref()
            .and_then(|m| m.selected().map(str::to_string));
        let Some(from) = from.filter(|from| Some(from) != to.as_ref()) else {
            return;
        };
        let typed = self.line.read(cx).text().to_string();
        self.drafts.put_text(&from, typed);
        let text = to.map(|to| self.drafts.take_text(&to)).unwrap_or_default();
        self.line.update(cx, |line, cx| line.set_text(&text, cx));
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

    /// Whom the open conversation is with, and where what is typed goes.
    fn open(&self) -> Option<(String, Target)> {
        let model = self.model.as_ref()?;
        Some((model.selected()?.to_string(), model.target()?))
    }

    /// Sends the draft of the open conversation as one mail: what is typed,
    /// and what is attached. A draft with a marked attachment is not sent.
    fn send(&mut self, cx: &mut Context<Self>) {
        let Some((to, target)) = self.open() else {
            return;
        };
        let text = self.line.read(cx).text().trim().to_string();
        match draft::unsendable(&text, self.drafts.attachments(&to)) {
            Some(why) if why.is_empty() => return,
            Some(why) => return self.notice(why, cx),
            None => {}
        }
        // The draft is empty while the mail is on its way, so that Enter
        // again does not send it again.
        let images = self.drafts.take(&to);
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.notice(String::new(), cx);
        self.ask(
            cx,
            move |client, me| match post(client, me, &target, &text, &images) {
                Ok(blobs) => Ok(images.iter().map(|a| a.id).zip(blobs).collect::<Vec<_>>()),
                Err((at, why)) => Err(Unsent {
                    text,
                    images,
                    at,
                    why,
                }),
            },
            move |this, sent, cx| match sent {
                Ok(sent) => {
                    // The bytes are here already: the yard is not asked.
                    for (id, blob) in sent {
                        if let Some(image) = this.thumbs.remove(&id) {
                            this.images.insert(blob.hash, Picture::Here(image));
                        }
                    }
                    this.change(cx, |m| m.sent(&to))
                }
                Err(unsent) => this.unsent(&to, unsent, cx),
            },
        );
    }

    /// Nothing of a draft is lost to a failed send: what did not go is back
    /// where it was, before what was typed and attached since, the
    /// attachment that kept it is marked, and the window says why.
    fn unsent(&mut self, to: &str, unsent: Unsent, cx: &mut Context<Self>) {
        self.drafts.restore(to, unsent.images);
        if let Some(id) = unsent.at {
            self.drafts.refuse(id, &unsent.why);
        }
        let open = self.model.as_ref().and_then(|m| m.selected()) == Some(to);
        if open {
            self.line.update(cx, |line, cx| {
                let back = draft::rejoin(unsent.text, line.text().to_string());
                line.set_text(&back, cx);
            });
        } else {
            self.drafts.unsend(to, unsent.text);
        }
        self.notice(format!("not sent to {to}: {}", unsent.why), cx);
    }

    /// Attaches what was pasted or dropped to the draft for `with`. An
    /// image among it is drawn from the bytes that came.
    fn attach(&mut self, with: &str, found: Found, cx: &mut Context<Self>) -> u64 {
        let image = model::image_type(&found.bytes)
            .and_then(ImageFormat::from_mime_type)
            .map(|format| Arc::new(Image::from_bytes(format, found.bytes.clone())));
        let id = self.drafts.attach(with, found);
        if let Some(image) = image {
            self.thumbs.insert(id, image);
        }
        cx.notify();
        id
    }

    /// Attaches an image that was pasted to the draft of the open
    /// conversation: Enter sends it. One that cannot go is marked at once,
    /// and one over the yard's cap as soon as the yard said its cap.
    fn paste(&mut self, image: &Image, cx: &mut Context<Self>) {
        let Some((with, _)) = self.open() else {
            return;
        };
        let found = draft::pasted(image.bytes().to_vec());
        let (bytes, marked) = (found.bytes.len(), found.refused.is_some());
        let id = self.attach(&with, found, cx);
        if marked {
            return;
        }
        self.ask(
            cx,
            |client, _| client.settings().map(|settings| settings.blob_cap()),
            move |this, cap, cx| {
                // A yard that does not say its cap now is asked at Enter.
                if let Ok(cap) = cap
                    && let Some(why) = model::over_cap(bytes, cap)
                {
                    this.drafts.refuse(id, &why);
                    cx.notify();
                }
            },
        );
    }

    /// Attaches the files that were dropped on the window to the draft of
    /// the conversation that was open then. They are read off the window's
    /// thread, and one that is no image the yard takes only as far as it
    /// takes to say so.
    fn dropped(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some((with, _)) = self.open() else {
            return;
        };
        let paths = paths.to_vec();
        self.ask(
            cx,
            move |client, _| {
                let cap = client.settings().ok().map(|settings| settings.blob_cap());
                let found = paths.iter().map(|path| draft::dropped(path, cap));
                found.collect::<Vec<Found>>()
            },
            move |this, found, cx| {
                for found in found {
                    this.attach(&with, found, cx);
                }
            },
        );
    }

    /// Takes an attachment off its draft.
    fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.drafts.remove(id);
        self.thumbs.remove(&id);
        cx.notify();
    }

    /// Backspace in the empty line takes the last attachment off the draft.
    fn remove_last(&mut self, cx: &mut Context<Self>) {
        let Some((with, _)) = self.open() else {
            return;
        };
        if let Some(id) = self.drafts.remove_last(&with) {
            self.thumbs.remove(&id);
            cx.notify();
        }
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
        if !file.not_taken.is_empty() {
            return line(model::not_taken(file)).into_any_element();
        }
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

    /// An attachment of the draft: the image small, or its name when it is
    /// none footplate draws, with the mark that removes it. One that cannot
    /// go is pale in a frame of the colour its reason is said in.
    fn attachment(
        &self,
        attachment: &Attachment,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = attachment.id;
        let marked = attachment.refused.is_some();
        let name = attachment.name.clone();
        let named = move || {
            div()
                .size_full()
                .p_1()
                .text_xs()
                .text_color(palette.faint)
                .child(name.clone())
        };
        let shown = match self.thumbs.get(&id) {
            Some(image) => img(image.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(move || named().into_any_element())
                .into_any_element(),
            None => named().into_any_element(),
        };
        div()
            .relative()
            .flex_none()
            .size(px(ATTACHMENT))
            .child(
                div()
                    .size_full()
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(if marked { palette.warn } else { palette.line })
                    .when(marked, |el| el.opacity(0.5))
                    .child(shown),
            )
            .child(
                div()
                    .id(("remove", id))
                    .debug_selector(|| format!("remove {id}"))
                    .absolute()
                    .top(px(-6.))
                    .right(px(-6.))
                    .size(px(16.))
                    .rounded_full()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.ground)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .cursor_pointer()
                    .child("×")
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(id, cx))),
            )
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
        let attached = self.drafts.attachments(with);
        let attached = (!attached.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .pt_1()
                .children(attached.iter().map(|a| self.attachment(a, palette, cx)))
        });
        let reasons = self.drafts.reasons(with);
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
                    .flex()
                    .flex_col()
                    .gap_2()
                    // A press anywhere in the frame is for the line. The
                    // window itself takes the focus of a press no one
                    // claimed, so this one is claimed.
                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                        window.focus(&focus);
                        window.prevent_default();
                    })
                    .children(attached)
                    // Why a marked attachment cannot go, by the line it
                    // keeps from being sent.
                    .children(reasons.into_iter().map(|why| {
                        div()
                            .text_sm()
                            .text_color(palette.warn)
                            .child(format!("! {why}"))
                    }))
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
            // Files dropped anywhere on the window are for the draft of the
            // open conversation.
            .on_drop(
                cx.listener(|this, files: &ExternalPaths, _, cx| this.dropped(files.paths(), cx)),
            )
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

    #[test]
    fn the_window_icon_is_the_1024px_master_png() {
        let number = |at: usize| u32::from_be_bytes(ICON[at..at + 4].try_into().unwrap());
        assert_eq!(&ICON[12..16], b"IHDR");
        assert_eq!((number(16), number(20)), (1024, 1024));
    }

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

    /// What is attached to the draft of the open conversation: for each
    /// its id, its name and why it is marked.
    fn attached(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
    ) -> Vec<(u64, String, Option<String>)> {
        view.read_with(cx, |v, _| {
            let with = v.model.as_ref().unwrap().selected().unwrap();
            let all = v.drafts.attachments(with).iter();
            all.map(|a| (a.id, a.name.clone(), a.refused.clone()))
                .collect()
        })
    }

    fn typed(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, cx| v.line.read(cx).text().to_string())
    }

    /// Clicks the mark that removes the attachment `id`, where it is drawn.
    fn click_remove(cx: &mut VisualTestContext, id: u64) {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let mark: &'static str = Box::leak(format!("remove {id}").into_boxed_str());
        let bounds = cx
            .debug_bounds(mark)
            .unwrap_or_else(|| panic!("no mark is drawn that removes attachment {id}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    const PASTED: &str = "what was pasted";

    /// What the yard says of mail with an image over a link without files.
    const NO_FILES: &str = "mail to clerk@north: peer north: the link does not carry file out, so mail to it carries no image: the operator allows it with yardr peer allow north --send file";

    fn clerk() -> Target {
        Target::Peer {
            name: "clerk".into(),
            peer: "north".into(),
        }
    }

    fn pasted(id: u64) -> Attachment {
        Attachment {
            id,
            name: PASTED.to_string(),
            size: png().len() as u64,
            bytes: png(),
            refused: None,
        }
    }

    #[test]
    fn an_image_for_a_linked_yard_is_put_and_then_mailed_with_its_hash() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "PutBlob",
            &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
        );
        let blobs = post(&yard.client(), "jo", &clerk(), "Like this.", &[pasted(1)]).unwrap();
        assert_eq!(blobs[0].hash, HASH);
        assert_eq!(yard.calls(), ["Settings", "PutBlob", "MailPeer"]);
        assert_eq!(
            yard.asked("MailPeer"),
            [
                json!({"to": "clerk", "peer": "north", "text": "Like this.", "actor": "jo", "files": [HASH]})
            ]
        );
    }

    #[test]
    fn a_link_that_carries_no_files_refuses_the_mail_in_the_yard_s_words() {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "PutBlob",
            &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
        );
        yard.answer(
            "MailPeer",
            &json!({"error": {"code": "error", "message": NO_FILES}}).to_string(),
        );
        // The reason is the yard's, and no attachment is the one it is
        // about: the link is.
        assert_eq!(
            post(&yard.client(), "jo", &clerk(), "Like this.", &[pasted(1)]),
            Err((None, NO_FILES.to_string()))
        );
        // The mail was asked for once and nothing else went: none to the
        // crew, and the text not on its own.
        assert_eq!(yard.calls(), ["Settings", "PutBlob", "MailPeer"]);
    }

    #[gpui::test]
    fn images_are_attached_to_the_draft_and_sent_with_enter(cx: &mut TestAppContext) {
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

        // An image that is pasted is attached to the draft, and nothing is
        // sent. One over the yard's cap is marked as soon as the yard said
        // its cap, with the reason.
        yard.answer(
            "Settings",
            r#"{"result": {"operator": "jo", "blob_max_bytes": "69"}}"#,
        );
        cx.simulate_input("Paint the fence like this.");
        let image = Image::from_bytes(ImageFormat::Png, png());
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v");
        let over = "the image is 70 bytes, and this yard takes one of 69 bytes at most (yardr settings: blob_max_bytes)";
        wait(cx, "that the image is over the cap", |cx| {
            attached(&view, cx) == [(1, PASTED.to_string(), Some(over.to_string()))]
        });
        // Enter sends nothing of a draft with a marked attachment, and says
        // so; what is typed stays.
        cx.simulate_keystrokes("enter");
        assert_eq!(
            notice(&view, cx),
            "what was pasted is marked and cannot go: remove it, then send"
        );
        assert_eq!(typed(&view, cx), "Paint the fence like this.");
        cx.run_until_parked();
        assert_eq!(yard.asked("PutBlob"), Vec::<serde_json::Value>::new());
        assert_eq!(yard.asked("Mail"), Vec::<serde_json::Value>::new());
        // Its mark removes it.
        click_remove(cx, 1);
        assert_eq!(attached(&view, cx), []);
        view.read_with(cx, |v, _| assert!(v.thumbs.is_empty()));

        // Under the cap: two images wait in the draft, with what is typed,
        // until Enter.
        yard.answer("Settings", r#"{"result": {"operator": "jo"}}"#);
        cx.simulate_keystrokes("cmd-v");
        let jpeg = Image::from_bytes(ImageFormat::Jpeg, vec![0xff, 0xd8, 0xff, 0xe0, 0, 16]);
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&jpeg));
        cx.simulate_keystrokes("cmd-v");
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(
            attached(&view, cx),
            [(2, PASTED.to_string(), None), (3, PASTED.to_string(), None)]
        );
        assert_eq!(yard.asked("PutBlob"), Vec::<serde_json::Value>::new());
        assert_eq!(yard.asked("Mail"), Vec::<serde_json::Value>::new());

        // Enter: each image is put, in the order attached, and one mail
        // sent with their hashes and with what is typed.
        cx.simulate_keystrokes("enter");
        wait(cx, "the mail with the images", |_| {
            yard.asked("Mail")
                == [
                    json!({"to": "yardmaster", "text": "Paint the fence like this.", "actor": "jo", "files": [HASH, HASH]}),
                ]
        });
        let put = yard.asked("PutBlob");
        assert_eq!(put.len(), 2);
        assert_eq!(
            put[0]["data"],
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
        );
        assert_eq!(put[1]["data"], "/9j/4AAQ");
        wait(cx, "the draft empty again", |cx| {
            typed(&view, cx).is_empty() && attached(&view, cx).is_empty()
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
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v enter");
        wait(cx, "the mail that is an image only", |_| {
            yard.asked("Mail").last()
                == Some(&json!({"to": "yardmaster", "text": "", "actor": "jo", "files": [HASH]}))
        });
        wait(cx, "the draft empty again", |cx| {
            attached(&view, cx).is_empty()
        });

        // Backspace takes text while there is text, and then the last
        // attachment.
        cx.simulate_keystrokes("cmd-v cmd-v");
        cx.simulate_input("x");
        cx.simulate_keystrokes("backspace");
        assert_eq!(attached(&view, cx).len(), 2);
        cx.simulate_keystrokes("backspace");
        assert_eq!(attached(&view, cx), [(5, PASTED.to_string(), None)]);

        // What the yard refuses is said: the text is back in the line and
        // the image in the draft, marked with the yard's words, and no mail
        // was sent.
        let mails = yard.asked("Mail").len();
        yard.answer(
            "PutBlob",
            r#"{"error": {"code": "error", "message": "an image is a PNG or a JPEG"}}"#,
        );
        cx.simulate_input("And this.");
        cx.simulate_keystrokes("enter");
        wait(cx, "why the image was not sent", |cx| {
            notice(&view, cx) == "not sent to yardmaster: an image is a PNG or a JPEG"
        });
        assert_eq!(typed(&view, cx), "And this.");
        assert_eq!(
            attached(&view, cx),
            [(
                5,
                PASTED.to_string(),
                Some("an image is a PNG or a JPEG".to_string())
            )]
        );
        assert_eq!(yard.asked("Mail").len(), mails);
        click_remove(cx, 5);

        // Another kind of image is marked as it is pasted, and never leaves
        // the window.
        let tiff = Image::from_bytes(ImageFormat::Tiff, b"II*\0".to_vec());
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&tiff));
        cx.simulate_keystrokes("cmd-v");
        let neither = "mail carries a PNG or a JPEG, and what was pasted is neither";
        assert_eq!(
            attached(&view, cx),
            [(7, PASTED.to_string(), Some(neither.to_string()))]
        );
        click_remove(cx, 7);

        // Files that are dropped are attached as well, and one that is no
        // image the yard takes is refused in words, before anything is put.
        let puts = yard.asked("PutBlob").len();
        let dir = tempfile::tempdir().unwrap();
        let (fence, notes) = (dir.path().join("fence.png"), dir.path().join("notes.pdf"));
        std::fs::write(&fence, png()).unwrap();
        std::fs::write(&notes, b"%PDF-1.7 and more").unwrap();
        view.update(cx, |v, cx| v.dropped(&[fence, notes], cx));
        wait(cx, "the dropped files", |cx| attached(&view, cx).len() == 2);
        assert_eq!(
            attached(&view, cx),
            [
                (8, "fence.png".to_string(), None),
                (
                    9,
                    "notes.pdf".to_string(),
                    Some("mail carries a PNG or a JPEG, and notes.pdf is neither".to_string())
                )
            ]
        );
        view.read_with(cx, |v, _| {
            let reasons = v.drafts.reasons("yardmaster");
            assert_eq!(
                reasons,
                ["mail carries a PNG or a JPEG, and notes.pdf is neither"]
            );
            assert!(v.thumbs.contains_key(&8) && !v.thumbs.contains_key(&9));
        });
        // The window draws both, and nothing went to the yard.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(yard.asked("PutBlob").len(), puts);
        // A drop on the window reaches the same place. gpui hands out no
        // way to make the paths of one, so this one carries none: the yard
        // is asked its cap for it, and nothing is attached.
        let asked = yard.asked("Settings").len();
        let position = gpui::point(px(400.), px(300.));
        cx.simulate_event(gpui::FileDropEvent::Entered {
            position,
            paths: ExternalPaths::default(),
        });
        cx.simulate_event(gpui::FileDropEvent::Submit { position });
        wait(cx, "the drop", |_| yard.asked("Settings").len() > asked);
        cx.run_until_parked();
        assert_eq!(attached(&view, cx).len(), 2);

        // Each conversation keeps its draft: the text and the attachments
        // stay with the yardmaster's when another is opened.
        let mails = yard.asked("Mail").len();
        yard.answer("Conversation", r#"{"result": []}"#);
        cx.simulate_keystrokes("tab");
        wait(cx, "the conversation with the linked yard", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().unwrap().selected() == Some("clerk@north")
            })
        });
        assert_eq!(typed(&view, cx), "");
        assert_eq!(attached(&view, cx), []);

        // To one of a linked yard an image is attached as to the crew:
        // whether the link carries it is the yard's to say. This one's does
        // not, so the mail is refused in the yard's words, and the draft is
        // back as it was.
        yard.answer(
            "PutBlob",
            &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
        );
        yard.answer_once_status(
            "MailPeer",
            200,
            &json!({"error": {"code": "error", "message": NO_FILES}}).to_string(),
        );
        cx.simulate_input("Hello north.");
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v enter");
        wait(cx, "the yard's refusal", |cx| {
            notice(&view, cx) == format!("not sent to clerk@north: {NO_FILES}")
        });
        assert_eq!(attached(&view, cx), [(10, PASTED.to_string(), None)]);
        assert_eq!(typed(&view, cx), "Hello north.");
        assert_eq!(
            yard.asked("MailPeer"),
            [
                json!({"to": "clerk", "peer": "north", "text": "Hello north.", "actor": "jo", "files": [HASH]})
            ]
        );
        assert_eq!(yard.asked("Mail").len(), mails);

        // Back with the yardmaster the draft is as it was left, and with
        // the linked yard again as well.
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(typed(&view, cx), "And this.");
        assert_eq!(attached(&view, cx).len(), 2);
        cx.simulate_keystrokes("tab");
        assert_eq!(typed(&view, cx), "Hello north.");
        assert_eq!(attached(&view, cx).len(), 1);

        // Without the image the text goes over the link.
        cx.simulate_keystrokes("end");
        click_remove(cx, 10);
        cx.simulate_keystrokes("enter");
        wait(cx, "the mail queued for the peer", |_| {
            yard.asked("MailPeer").last()
                == Some(
                    &json!({"to": "clerk", "peer": "north", "text": "Hello north.", "actor": "jo"}),
                )
        });

        // Once the link carries files the image goes with the mail, and is
        // shown from the bytes that were pasted.
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v enter");
        wait(cx, "the image queued for the peer", |_| {
            yard.asked("MailPeer").last()
                == Some(
                    &json!({"to": "clerk", "peer": "north", "text": "", "actor": "jo", "files": [HASH]}),
                )
        });
        wait(cx, "the draft empty again", |cx| {
            attached(&view, cx).is_empty()
        });
        assert_eq!(notice(&view, cx), "");
        assert_eq!(picture(&view, cx, HASH), "here");

        // Text on the clipboard is pasted into the line, as before.
        cx.simulate_keystrokes("shift-tab");
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(" Two\nlines.".into()));
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(typed(&view, cx), "And this. Two lines.");
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
