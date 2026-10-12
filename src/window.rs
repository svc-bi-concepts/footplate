//! The window: left, whom there is to talk with; right, the open
//! conversation and the line to write in. What it shows is the model's
//! (model.rs); this file draws it and makes the calls the model asks for.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gpui::{
    App, ClipboardItem, Context, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, Image,
    ImageFormat, KeyBinding, MouseButton, ObjectFit, ScrollHandle, SharedString, Window, actions,
    div, img, prelude::*, px,
};

use crate::api::{self, BlobRef, Client};
use crate::draft::{self, Attachment, Drafts, Found};
use crate::input::{self, Emptied, Line, Pasted, Submitted};
use crate::model::{self, DecidedRow, DecisionRow, Effect, Entry, Model, Row, Selected, Target};
use crate::select::{self, Selection};
use crate::session::{self, Update};
use crate::theme::Palette;

/// The application's icon, from the 1024px master in assets/icon.png.
pub const ICON: &[u8] = include_bytes!("../assets/icon.png");

actions!(footplate, [Next, Previous, Dismiss]);

const CONTEXT: &str = "Footplate";
/// Where no line is typed in: the yard row and the decisions. `x` marks
/// one notice read here; with a conversation open it is the line's text.
const NO_LINE_CONTEXT: &str = "FootplateNoLine";

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

/// A line between the sections of the left list.
fn divider(palette: Palette) -> impl IntoElement {
    div().mx_3().my_1().h(px(1.)).bg(palette.line)
}

/// The keys of the window, as `yardr chat` has them.
pub fn bind_keys(cx: &mut App) {
    for context in [CONTEXT, NO_LINE_CONTEXT] {
        cx.bind_keys([
            KeyBinding::new("tab", Next, Some(context)),
            KeyBinding::new("shift-tab", Previous, Some(context)),
            // The line's own binding is the deeper one: with the focus in
            // the line it copies the line's selection, as before.
            KeyBinding::new("secondary-c", input::Copy, Some(context)),
        ]);
    }
    // One key marks one notice read, where no line is typed in.
    cx.bind_keys([KeyBinding::new("x", Dismiss, Some(NO_LINE_CONTEXT))]);
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
    /// What is selected of the open conversation's messages.
    selection: Entity<Selection>,
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
    /// Which read of the queue a put-aside return waits for: an older
    /// timer does nothing when the queue was read since.
    ask_generation: u64,
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
        let selection = cx.new(|_| Selection::default());
        cx.observe(&selection, |_, _, cx| cx.notify()).detach();

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
            selection,
            images: HashMap::new(),
            drafts: Drafts::default(),
            thumbs: HashMap::new(),
            focus: cx.focus_handle(),
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec())),
            ask_generation: 0,
        }
    }

    pub fn line(&self) -> &Entity<Line> {
        &self.line
    }

    /// What the thread that follows the yard learned.
    fn told(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Up(state) => {
                let open = self.model.as_ref().and_then(|m| m.selected().cloned());
                let (model, effects) = Model::new(*state, open.clone());
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
                self.schedule_return(cx);
            }
            Update::Event(event) => self.change(cx, |m| m.event(&event)),
            Update::Down(why) => self.down = Some(why),
        }
        cx.notify();
    }

    /// Changes the model and does what the change asks for. When what is
    /// open is another or has more in it, its newest comes in view.
    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Model) -> Vec<Effect>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let before = (
            model.selected().cloned(),
            model.entries().len(),
            model.notices().len(),
        );
        let effects = f(model);
        if before
            != (
                model.selected().cloned(),
                model.entries().len(),
                model.notices().len(),
            )
        {
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

    /// What `from` was open. When another is now, what is typed stays
    /// with the conversation it was typed in, and the line has what was
    /// typed in the one that is open, if one is.
    fn turned(&mut self, from: Option<Selected>, cx: &mut Context<Self>) {
        let to = self.model.as_ref().and_then(|m| m.selected().cloned());
        let Some(from) = from.filter(|from| Some(from) != to.as_ref()) else {
            return;
        };
        // What was selected is of the conversation that is no longer shown.
        self.selection
            .update(cx, |selection, cx| selection.clear(cx));
        let typed = self.line.read(cx).text().to_string();
        if let Selected::Conversation(name) = &from {
            self.drafts.put_text(name, typed);
        }
        let text = match &to {
            Some(Selected::Conversation(name)) => self.drafts.take_text(name),
            _ => String::new(),
        };
        self.line.update(cx, |line, cx| line.set_text(&text, cx));
    }

    /// The line says whom it writes to, when a conversation is open.
    fn shown(&mut self, cx: &mut Context<Self>) {
        let to = self
            .model
            .as_ref()
            .and_then(|m| m.selected_conversation())
            .map(|s| format!("Write to {s}"))
            .unwrap_or_default();
        self.line
            .update(cx, |line, cx| line.set_placeholder(&to, cx));
    }

    /// Reads the queue again when the next put-aside ask comes back, which
    /// arrives with no event. One timer waits at a time; a queue read
    /// since replaces it.
    fn schedule_return(&mut self, cx: &mut Context<Self>) {
        self.ask_generation += 1;
        let generation = self.ask_generation;
        let Some(at) = self.model.as_ref().and_then(|m| m.next_return()) else {
            return;
        };
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&at) else {
            return;
        };
        let wait = at.timestamp() - chrono::Utc::now().timestamp();
        if wait <= 0 {
            self.change(cx, |_| vec![Effect::Asks]);
            return;
        }
        let wait = std::time::Duration::from_secs(wait as u64);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                if this.ask_generation == generation {
                    this.change(cx, |_| vec![Effect::Asks]);
                }
            })
            .ok();
        })
        .detach();
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
                Effect::Notifications => self.ask(
                    cx,
                    |client, me| client.user_notifications(me, 0),
                    |this, notifications, cx| match notifications {
                        Ok(notifications) => this.change(cx, |m| {
                            m.notifications(notifications.notifications);
                            Vec::new()
                        }),
                        Err(err) => {
                            this.notice(format!("the notices could not be read: {err}"), cx)
                        }
                    },
                ),
                Effect::AckNotifications { through } => self.ask(
                    cx,
                    move |client, me| client.ack_user_notifications(me, through),
                    move |this, acked, cx| {
                        this.change(cx, |m| {
                            match acked {
                                Ok(()) => m.notifications_acked(through),
                                Err(err) => m.notifications_ack_failed(&err.to_string()),
                            }
                            Vec::new()
                        })
                    },
                ),
                Effect::Asks => self.ask(
                    cx,
                    |client, me| client.user_asks(me),
                    |this, asks, cx| match asks {
                        Ok(asks) => {
                            this.change(cx, |m| m.asks(asks));
                            this.schedule_return(cx);
                        }
                        Err(err) => {
                            this.notice(format!("the decisions could not be read: {err}"), cx)
                        }
                    },
                ),
            }
        }
    }

    /// Whom the open conversation is with, and where what is typed goes.
    fn open(&self) -> Option<(String, Target)> {
        let model = self.model.as_ref()?;
        Some((model.selected_conversation()?.to_string(), model.target()?))
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
        let open = self.model.as_ref().and_then(|m| m.selected_conversation()) == Some(to);
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

    fn select(&mut self, sel: Selected, window: &mut Window, cx: &mut Context<Self>) {
        self.change(cx, |m| m.select(sel));
        // What is typed goes to a conversation; elsewhere the window holds
        // the keys, and `x` marks a notice read.
        let focus = match self.model.as_ref().and_then(|m| m.selected_conversation()) {
            Some(_) => self.line.focus_handle(cx),
            None => self.focus.clone(),
        };
        window.focus(&focus);
    }

    /// Copies what is selected of the messages. The line copies its own
    /// selection while the focus is in it.
    fn copy(&mut self, _: &input::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selection.read(cx).text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
    }

    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.change(cx, |m| m.step(by));
        let focus = match self.model.as_ref().and_then(|m| m.selected_conversation()) {
            Some(_) => self.line.focus_handle(cx),
            None => self.focus.clone(),
        };
        window.focus(&focus);
    }

    /// One key marks one notice read, on the open yard row.
    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        self.change(cx, |m| m.dismiss());
    }

    fn toggle_decided(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_decided();
            Vec::new()
        });
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
        let now = chrono::Utc::now();
        let mut body = div()
            .id("rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1();
        // The open questions above the conversations, grouped by topic.
        let decisions = model.decisions_count();
        let decided = model.decided_count();
        if decisions > 0 || decided > 0 {
            body = body.child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("decisions ({decisions})")),
            );
            for group in model.decision_groups(&now) {
                body = body.child(
                    div()
                        .px_3()
                        .pt_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.faint)
                        .child(group.topic),
                );
                for row in group.rows {
                    body = body.child(self.decision_row(row, palette, cx));
                }
            }
            if decided > 0 {
                let open = model.decided_expanded();
                body = body.child(
                    div()
                        .id("decided")
                        .mx_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.hover))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_decided(cx)))
                        .child(div().text_sm().text_color(palette.faint).child(format!(
                            "{} Decided ({decided})",
                            if open { "▾" } else { "▸" }
                        ))),
                );
                if open {
                    for row in model.decided_rows() {
                        body = body.child(self.decided_row(row, palette, cx));
                    }
                }
            }
            body = body.child(divider(palette));
        }
        if rows.is_empty() {
            body =
                body.child(
                    div().p_3().text_sm().text_color(palette.faint).child(
                        "This yard has no crew yet, and no linked yard exposes anyone to it.",
                    ),
                )
        } else {
            for row in rows {
                let sel = Selected::Conversation(row.name.clone());
                body = body.child(self.row(row, sel, palette, cx));
            }
        }
        body = body.child(divider(palette));
        let yard = model.yard_row();
        body = body.child(self.row(yard, Selected::Yard, palette, cx));
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

    fn row(
        &self,
        row: Row,
        sel: Selected,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = match &sel {
            Selected::Yard => "row yard".to_string(),
            _ => format!("row {}", row.name),
        };
        div()
            .id(SharedString::from(id))
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
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
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

    /// A bead with open asks: the question in plain words, then whose it
    /// is and how long it waits, the bead's id small at the row's end.
    fn decision_row(
        &self,
        row: DecisionRow,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let sel = Selected::Decision(row.bead.clone());
        div()
            .id(SharedString::from(format!("decision {}", row.bead)))
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
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().child(row.head))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.faint)
                            .truncate()
                            .child(row.sub),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(palette.faint)
                    .child(row.short),
            )
    }

    /// One dealt with, with its pick.
    fn decided_row(
        &self,
        row: DecidedRow,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let sel = Selected::Decided(row.bead.clone(), row.ask);
        div()
            .id(SharedString::from(format!(
                "decided {} {}",
                row.bead, row.ask
            )))
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
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().child(row.head))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.faint)
                            .truncate()
                            .child(row.sub),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(palette.faint)
                    .child(row.short),
            )
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
        let seq = entry.seq;
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
                // The margin is outside the text's bounds, so that these
                // are where the text is.
                el.child(
                    div()
                        .ml_4()
                        .debug_selector(|| format!("text {seq}"))
                        .child(select::text(seq, entry.text, &self.selection)),
                )
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
        match model.selected() {
            Some(Selected::Conversation(with)) => self
                .talk_conversation(model, with, palette, cx)
                .into_any_element(),
            Some(Selected::Yard) => self.talk_yard(model, palette).into_any_element(),
            Some(Selected::Decision(bead)) => {
                self.talk_decision(model, bead, palette).into_any_element()
            }
            Some(Selected::Decided(bead, ask)) => self
                .talk_decided(model, bead, *ask, palette)
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child("Nobody to write to yet.")
                .into_any_element(),
        }
    }

    fn talk_conversation(
        &self,
        model: &Model,
        with: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
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

    /// The yard row open: the notices, oldest first, and no line to type
    /// in. One key marks one read.
    fn talk_yard(&self, model: &Model, palette: Palette) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("yard"))
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("What the yard reports, apart from every conversation."),
            );
        let notices = model.notices();
        let list = div()
            .id("notices")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.conversation)
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        let list = if notices.is_empty() {
            list.child(
                div()
                    .text_color(palette.faint)
                    .child("No notices. The yard reports here what you subscribe to."),
            )
        } else {
            list.children(notices.into_iter().map(|n| {
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
                                    .child(model::clock(&n.at, &chrono::Local)),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(n.bead.clone()),
                            ),
                    )
                    .child(
                        div()
                            .ml_4()
                            .when(n.read, |el| el.text_color(palette.faint))
                            .child(n.text),
                    )
            }))
        };
        pane.child(head)
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
            .child(
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("x marks one read"),
            )
    }

    /// A bead with open asks, as read-only text: the picking is the next
    /// bead's.
    fn talk_decision(&self, model: &Model, bead: &str, palette: Palette) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let now = chrono::Utc::now();
        let Some(detail) = model.decision_detail(bead, &now) else {
            return pane
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child("Its questions are answered.");
        };
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.title.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("{} · {}", detail.bead, detail.topic)),
            );
        let list = div()
            .id("decision")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.conversation)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .children(detail.asks.into_iter().map(|ask| {
                div()
                    .flex()
                    .flex_col()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(ask.question))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .child(format!("{} · {} waits", ask.asker, ask.waits)),
                    )
            }));
        pane.child(head)
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

    /// One dealt with, as read-only text.
    fn talk_decided(
        &self,
        model: &Model,
        bead: &str,
        ask: u64,
        palette: Palette,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(detail) = model.decided_detail(bead, ask) else {
            return pane
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child("It is gone from the decided.");
        };
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.title.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("{} · ask {ask}", detail.bead)),
            );
        let mut body = div()
            .id("decided")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.conversation)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.question.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("{} · {}", detail.asker, detail.state)),
            );
        if !detail.pick.is_empty() {
            body = body.child(div().child(format!("picked: {}", detail.pick)));
        }
        if !detail.words.is_empty() {
            body = body.child(div().child(detail.words.clone()));
        }
        if !detail.return_at.is_empty() {
            body = body.child(div().text_sm().text_color(palette.faint).child(format!(
                "back {}",
                model::clock(&detail.return_at, &chrono::Local)
            )));
        }
        pane.child(head)
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
}

impl Focusable for Footplate {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Footplate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::of(window.appearance());
        let context = match self.model.as_ref().and_then(|m| m.selected_conversation()) {
            Some(_) => CONTEXT,
            None => NO_LINE_CONTEXT,
        };
        let root = div()
            .key_context(context)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &Previous, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::dismiss))
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
        yard.answer(
            "UserNotifications",
            r#"{"result": {"notifications": [], "unread": 0, "last": 0}}"#,
        );
        yard.answer("AckUserNotifications", "{}");
        yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
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
            let with = v.model.as_ref().unwrap().selected_conversation().unwrap();
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
                v.model.as_ref().unwrap().selected_conversation() == Some("clerk@north")
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
            assert_eq!(model.selected_conversation(), Some("yardmaster"));
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
            assert_eq!(
                v.model.as_ref().unwrap().selected_conversation(),
                Some("clerk@north")
            )
        });
    }

    const LINK: &str = "see https://example.org/a now";

    /// The yardmaster's conversation is these mails, from seq 12 on.
    fn mails(yard: &Yard, texts: &[&str]) {
        let mails: Vec<serde_json::Value> = texts
            .iter()
            .zip(12..)
            .map(|(text, seq)| json!({"seq": seq, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": text, "read": false}))
            .collect();
        yard.answer("Conversation", &json!({"result": mails}).to_string());
    }

    /// The window on `yard`, the focus in the line, once it shows `texts`.
    fn opened<'a>(
        yard: &Yard,
        shown: &[&str],
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
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
            texts(&view, cx) == shown
        });
        (view, cx)
    }

    /// Where the text of the mail `seq` is drawn.
    fn text_bounds(cx: &mut VisualTestContext, seq: u64) -> gpui::Bounds<gpui::Pixels> {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let text: &'static str = Box::leak(format!("text {seq}").into_boxed_str());
        cx.debug_bounds(text)
            .unwrap_or_else(|| panic!("the text of mail {seq} is not drawn"))
    }

    /// Where the pointer is `by` pixels after the boundary before byte
    /// `index` of the mail `seq`, whose text `text` is one line: the text is
    /// shaped as the window shapes it.
    fn place(
        cx: &mut VisualTestContext,
        seq: u64,
        text: &str,
        index: usize,
        by: f32,
    ) -> gpui::Point<gpui::Pixels> {
        let bounds = text_bounds(cx, seq);
        let x = cx.update(|window, _| {
            let run = gpui::TextRun {
                len: text.len(),
                font: window.text_style().font(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line =
                window
                    .text_system()
                    .shape_line(text.to_string().into(), px(14.), &[run], None);
            line.x_for_index(index)
        });
        gpui::point(bounds.left() + x + px(by), bounds.center().y)
    }

    fn drag(
        cx: &mut VisualTestContext,
        from: gpui::Point<gpui::Pixels>,
        to: gpui::Point<gpui::Pixels>,
    ) {
        let none = gpui::Modifiers::default();
        cx.simulate_mouse_down(from, MouseButton::Left, none);
        cx.simulate_mouse_move(to, MouseButton::Left, none);
        cx.simulate_mouse_up(to, MouseButton::Left, none);
    }

    /// A press and a release where `place` says.
    fn click(cx: &mut VisualTestContext, seq: u64, text: &str, index: usize, by: f32) {
        let at = place(cx, seq, text, index, by);
        cx.simulate_click(at, gpui::Modifiers::default());
    }

    fn selected(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Option<String> {
        view.read_with(cx, |v, cx| v.selection.read(cx).text().map(str::to_string))
    }

    fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    #[gpui::test]
    fn a_link_in_a_message_is_selected_with_the_mouse_and_copied(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        mails(&yard, &[LINK]);
        let (view, cx) = opened(&yard, &[LINK], cx);
        cx.write_to_clipboard(ClipboardItem::new_string("before".into()));

        // A drag from just inside the link's first character to just inside
        // its last selects the link: the boundary closest to the pointer
        // counts, as in the line.
        let (from, to) = (place(cx, 12, LINK, 4, 1.), place(cx, 12, LINK, 25, -1.));
        drag(cx, from, to);
        assert_eq!(
            selected(&view, cx).as_deref(),
            Some("https://example.org/a")
        );
        // Nothing is copied before the key.
        assert_eq!(clipboard(cx).as_deref(), Some("before"));
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("https://example.org/a"));

        // From right to left it is the same selection.
        drag(cx, to, from);
        assert_eq!(
            selected(&view, cx).as_deref(),
            Some("https://example.org/a")
        );

        // A second press selects the word under the pointer.
        let at = place(cx, 12, LINK, 27, 0.);
        cx.simulate_event(gpui::MouseDownEvent {
            position: at,
            modifiers: gpui::Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_mouse_up(at, MouseButton::Left, gpui::Modifiers::default());
        assert_eq!(selected(&view, cx).as_deref(), Some("now"));
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("now"));

        // A press on the talk where no text is drops it, and the key then
        // copies nothing.
        let name = gpui::point(at.x, text_bounds(cx, 12).top() - px(8.));
        cx.simulate_click(name, gpui::Modifiers::default());
        assert_eq!(selected(&view, cx), None);
        cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("before"));

        // What is selected is of the open conversation: another one opened
        // drops it.
        drag(cx, from, to);
        assert!(selected(&view, cx).is_some());
        yard.answer("Conversation", r#"{"result": []}"#);
        view.update(cx, |v, cx| v.change(cx, |m| m.step(1)));
        assert_eq!(selected(&view, cx), None);
    }

    #[gpui::test]
    fn a_click_on_a_link_in_a_message_opens_it(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let two = "(at https://example.org/b). And ftp://example.org/c";
        mails(&yard, &[LINK, two]);
        let (view, cx) = opened(&yard, &[LINK, two], cx);
        let none = gpui::Modifiers::default();

        // A click on the text beside the link opens nothing.
        click(cx, 12, LINK, 1, 1.);
        assert_eq!(cx.opened_url(), None);

        // A drag over the link selects it, as before, and opens nothing:
        // not from end to end, and not when it ends where it began.
        let (from, to) = (place(cx, 12, LINK, 4, 1.), place(cx, 12, LINK, 25, -1.));
        drag(cx, from, to);
        assert_eq!(
            selected(&view, cx).as_deref(),
            Some("https://example.org/a")
        );
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("https://example.org/a"));
        cx.simulate_mouse_down(from, MouseButton::Left, none);
        cx.simulate_mouse_move(to, MouseButton::Left, none);
        cx.simulate_mouse_move(from, MouseButton::Left, none);
        cx.simulate_mouse_up(from, MouseButton::Left, none);
        assert_eq!(cx.opened_url(), None);
        // Nor does a press beside the link that is released on it.
        let beside = place(cx, 12, LINK, 27, 0.);
        cx.simulate_mouse_down(beside, MouseButton::Left, none);
        cx.simulate_mouse_up(from, MouseButton::Left, none);
        assert_eq!(cx.opened_url(), None);

        // A click on the link opens exactly it, and selects nothing.
        click(cx, 12, LINK, 10, 1.);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.org/a"));
        assert_eq!(selected(&view, cx), None);

        // The bracket and the full stop after a link are no part of it,
        // and a click on them opens nothing.
        click(cx, 13, two, 25, 1.);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.org/a"));
        click(cx, 13, two, 24, -1.);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.org/b"));

        // Only http and https are links.
        click(cx, 13, two, 40, 1.);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.org/b"));
    }

    #[gpui::test]
    fn a_selection_in_a_second_message_drops_the_first(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let lines = "one\ntwo";
        mails(&yard, &[LINK, lines]);
        let (view, cx) = opened(&yard, &[LINK, lines], cx);

        let (from, to) = (place(cx, 12, LINK, 0, 0.), place(cx, 12, LINK, 3, 0.));
        drag(cx, from, to);
        assert_eq!(selected(&view, cx).as_deref(), Some("see"));

        // All of the second message, from its first line to its second: the
        // one selection of the talk is now this, and its line break is one
        // in what is copied.
        let second = text_bounds(cx, 13);
        drag(
            cx,
            gpui::point(second.left(), second.top() + px(2.)),
            gpui::point(second.right() + px(40.), second.bottom() - px(2.)),
        );
        assert_eq!(selected(&view, cx).as_deref(), Some(lines));
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("one\ntwo"));

        // A press in the first again drops that of the second.
        cx.simulate_click(from, gpui::Modifiers::default());
        assert_eq!(selected(&view, cx), None);
    }

    #[gpui::test]
    fn the_line_still_copies_its_own_selection(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        mails(&yard, &[LINK]);
        let (view, cx) = opened(&yard, &[LINK], cx);

        cx.simulate_input("Land it today.");
        cx.simulate_keystrokes("shift-left shift-left shift-left shift-left shift-left shift-left");
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("today."));

        // Also while a message has a selection: the focus is in the line,
        // so the key is the line's.
        let (from, to) = (place(cx, 12, LINK, 0, 0.), place(cx, 12, LINK, 3, 0.));
        drag(cx, from, to);
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("see"));
        cx.update(|window, cx| window.focus(&view.read(cx).line.focus_handle(cx)));
        cx.simulate_keystrokes("cmd-a cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some("Land it today."));
        assert_eq!(selected(&view, cx).as_deref(), Some("see"));
    }
}
