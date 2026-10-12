//! The window: left, whom there is to talk with; right, the open
//! conversation and the line to write in. What it shows is the model's
//! (model.rs); this file draws it and makes the calls the model asks for.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gpui::{
    Action, App, ClipboardItem, Context, Div, Entity, ExternalPaths, FocusHandle, Focusable,
    FontWeight, Image, ImageFormat, KeyBinding, MouseButton, ObjectFit, ScrollHandle, SharedString,
    Window, actions, div, img, prelude::*, px, relative, rems,
};

use crate::api::{self, BlobRef, Client};
use crate::draft::{self, Attachment, Drafts, Found};
use crate::input::{self, Emptied, Line, Pasted, Submitted};
use crate::model::{
    self, AnswerAction, DecidedDetail, DecidedRow, DecisionAsk, DecisionDetail, DecisionRow,
    Effect, Entry, Model, Row, Selected, Target,
};
use crate::select::{self, Selection};
use crate::session::{self, Update};
use crate::settings::{self, Settings};
use crate::theme::{Look, Palette};

/// The application's icon, from the 1024px master in assets/icon.png.
pub const ICON: &[u8] = include_bytes!("../assets/icon.png");

actions!(
    footplate,
    [
        Next,
        Previous,
        Dismiss,
        Pick1,
        Pick2,
        Pick3,
        Pick4,
        Pick5,
        Pick6,
        Pick7,
        Pick8,
        Pick9,
        ChoiceUp,
        ChoiceDown,
        Decide,
        Decline,
        Expand,
        FocusWords,
        Leave,
        OpenSettings,
        Larger,
        Smaller,
        UsualSize,
        NextLook
    ]
);

const CONTEXT: &str = "Footplate";
/// Where no line is typed in: the yard row, and a decision with the window
/// focused, where a number key picks and `x` puts aside. With a
/// conversation open, or a decision with the line focused to type words in,
/// those keys are the line's text.
const NO_LINE_CONTEXT: &str = "FootplateNoLine";

/// How high an image of a mail stands in the conversation, whatever its own
/// size: the room is there before the image is, so nothing moves under the
/// reader when it comes.
const IMAGE_HEIGHT: f32 = 240.;

/// The side of an attachment of the draft, above the line, in rems: its
/// name stands in it where it is no image, so it goes with the text size.
const ATTACHMENT: f32 = 3.5;

/// The key that is cmd on macOS and ctrl elsewhere, as the hints name it.
const SECONDARY: &str = if cfg!(target_os = "macos") {
    "cmd"
} else {
    "ctrl"
};

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

/// The keys of an open card. Enter with nothing chosen and a recommendation
/// picks it, and says so. Later and Decline are buttons beside the send
/// button, and expand its own control below, so their keys are not listed
/// here; they still work.
fn decision_keys(ask: &DecisionAsk) -> String {
    if ask.options.is_empty() {
        return "words wanted · i words · enter sends the answer".to_string();
    }
    let n = ask.options.len();
    let enter = if ask.chosen != 0 {
        format!("enter picks {}", ask.chosen)
    } else if ask.recommend != 0 {
        format!("enter picks {}", ask.recommend)
    } else {
        "enter sends words as note".to_string()
    };
    format!("{enter} · 1-{n} pick · ↑↓ choose · i words")
}

/// The keys of a dealt with: nothing is chosen by default and Enter alone
/// sends nothing; a number then Enter replaces the pick.
fn decided_keys(detail: &DecidedDetail) -> String {
    if detail.options.is_empty() {
        return "i words · enter sends".to_string();
    }
    format!("1-{} change your pick", detail.options.len())
}

/// The button that sends the chosen option with the line's words, exactly
/// as Enter does: `Send: 1 · Move them now` on an open ask, `Change to:
/// 2 · …` on a dealt with. The card shows it only once an option is
/// chosen: never a button that sends the recommendation unasked.
fn send_button(
    bead: &str,
    ask: u64,
    text: String,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("send {bead} {ask}")))
        .debug_selector(|| format!("send {bead} {ask}"))
        .px_3()
        .py_1()
        .rounded_md()
        .bg(palette.accent)
        .text_color(palette.on_accent)
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| this.decide(cx)))
        .child(text)
}

/// A small button beside the send button, without what it does: Later and
/// Decline, with their keys.
fn small_button(text: String, palette: Palette) -> Div {
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

/// The keys of the window, as `yardr chat` has them, with the card's:
/// a number key or the arrows choose an option, Enter sends the pick with
/// the line's words, `x` puts aside, `d` declines, `e` expands.
pub fn bind_keys(cx: &mut App) {
    for context in [CONTEXT, NO_LINE_CONTEXT] {
        cx.bind_keys([
            KeyBinding::new("tab", Next, Some(context)),
            KeyBinding::new("shift-tab", Previous, Some(context)),
            // The line's own binding is the deeper one: with the focus in
            // the line it copies the line's selection, as before.
            KeyBinding::new("secondary-c", input::Copy, Some(context)),
            // The arrows choose even while words are typed: the line keeps
            // left and right for its cursor.
            KeyBinding::new("up", ChoiceUp, Some(context)),
            KeyBinding::new("down", ChoiceDown, Some(context)),
            KeyBinding::new("escape", Leave, Some(context)),
        ]);
    }
    // Where no line is typed in: one key marks one notice read, and on a
    // card the same keys pick, put aside, decline and expand. With the
    // line focused those keys are the line's text.
    cx.bind_keys([
        KeyBinding::new("x", Dismiss, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("1", Pick1, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("2", Pick2, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("3", Pick3, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("4", Pick4, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("5", Pick5, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("6", Pick6, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("7", Pick7, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("8", Pick8, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("9", Pick9, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("enter", Decide, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("d", Decline, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("e", Expand, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("i", FocusWords, Some(NO_LINE_CONTEXT)),
    ]);
    // The settings and their own keys, wherever the focus is, the yard
    // there or away. Larger is on `=`, the key `+` is on, with shift or
    // without.
    cx.bind_keys([
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-=", Larger, None),
        KeyBinding::new("secondary-+", Larger, None),
        KeyBinding::new("secondary--", Smaller, None),
        KeyBinding::new("secondary-0", UsualSize, None),
        KeyBinding::new("secondary-shift-l", NextLook, None),
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
    /// The conversation's position when it was left for a decision, so
    /// dealing with that decision can return to the same place.
    back_position: Option<(String, gpui::Point<gpui::Pixels>)>,
    /// The open card's own scroll: mail and notices arriving never move it.
    decision: ScrollHandle,
    /// What is selected of the open conversation's messages.
    selection: Entity<Selection>,
    /// The images of the conversations that were open, by hash: an image
    /// never changes under its hash, so each is asked for once.
    images: HashMap<String, Picture>,
    /// What is attached and not sent, for each conversation, and the text
    /// typed in those that are not open; the open one's is in the line.
    drafts: Drafts,
    /// The words typed for each ask that is not the open one, by bead and
    /// ask; the open one's are in the line.
    ask_words: HashMap<(String, u64), String>,
    /// The ask decline was pressed on with an empty line: the next Enter
    /// sends the typed words as the decline, and esc leaves it.
    declining: Option<(String, u64)>,
    /// The attachments that are images, by id, as they are drawn.
    thumbs: HashMap<u64, Arc<Image>>,
    focus: FocusHandle,
    icon: Arc<Image>,
    /// Which read of the queue a put-aside return waits for: an older
    /// timer does nothing when the queue was read since.
    ask_generation: u64,
    /// The text size and the look, as they are drawn.
    settings: Settings,
    /// The file they are kept in, or none: then they last as long as the
    /// window.
    kept: Option<PathBuf>,
    /// Why the last change was not kept, if it was not.
    unkept: Option<String>,
    /// The settings are shown where the conversation is.
    settings_open: bool,
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
        // Auto is the desktop's look: when that changes, the window is
        // drawn again.
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        // The settings' keys are the application's, as quit is. An action
        // of the window goes by way of what has the focus, and while the
        // yard is away that is a line that is not drawn: it would go
        // nowhere.
        Self::on_app_action::<OpenSettings>(window, cx, Self::toggle_settings);
        Self::on_app_action::<Larger>(window, cx, |this, _, cx| this.resize(1, cx));
        Self::on_app_action::<Smaller>(window, cx, |this, _, cx| this.resize(-1, cx));
        Self::on_app_action::<UsualSize>(window, cx, |this, _, cx| this.usual_size(cx));
        Self::on_app_action::<NextLook>(window, cx, |this, _, cx| {
            this.look(this.settings.look.next(), cx)
        });

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
            back_position: None,
            decision: ScrollHandle::new(),
            selection,
            images: HashMap::new(),
            drafts: Drafts::default(),
            ask_words: HashMap::new(),
            declining: None,
            thumbs: HashMap::new(),
            focus: cx.focus_handle(),
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec())),
            ask_generation: 0,
            settings: Settings::default(),
            kept: None,
            unkept: None,
            settings_open: false,
        }
    }

    /// Does `f` in this window for the action `A`, wherever the focus is.
    fn on_app_action<A: Action>(
        window: &Window,
        cx: &mut Context<Self>,
        f: fn(&mut Self, &mut Window, &mut Context<Self>),
    ) {
        let (this, handle) = (cx.weak_entity(), window.window_handle());
        // The application's own, not the one a context has for its window.
        let app: &mut App = cx;
        app.on_action(move |_: &A, cx| {
            let this = this.clone();
            // Afterwards: the window is not to be had while the key that
            // came through it is still on its way.
            cx.defer(move |cx| {
                // A window that is closed has nothing to set.
                let _ = handle.update(cx, |_, window, cx| {
                    this.update(cx, |this, cx| f(this, window, cx))
                });
            });
        });
    }

    /// The window with what `file` keeps of the text size and the look,
    /// which is where a change of them is kept from here on.
    pub fn kept_in(mut self, file: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        if let Some(file) = &file {
            self.settings = Settings::read(file);
        }
        cx.set_global(self.settings.look);
        self.kept = file;
        self
    }

    pub fn line(&self) -> &Entity<Line> {
        &self.line
    }

    /// What the thread that follows the yard learned.
    fn told(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Up(state) => {
                let open = self.model.as_ref().and_then(|m| m.selected().cloned());
                let (model, effects) = match self.model.as_ref() {
                    Some(before) => before.resumed(*state),
                    None => Model::new(*state, None),
                };
                self.model = Some(model);
                self.down = None;
                self.turned(open, cx);
                self.conversation.scroll_to_bottom();
                self.decision.scroll_to_item(0);
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

    /// Changes the model and does what the change asks for. A conversation
    /// shows its newest, and the yard its notices; the open card never moves
    /// for mail or notices arriving.
    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Model) -> Vec<Effect>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let before_sel = model.selected().cloned();
        let before_position = matches!(&before_sel, Some(Selected::Conversation(_)))
            .then(|| self.conversation.offset());
        let before_entries = model.entries().len();
        let before_notices = model.notices().len();
        let effects = f(model);
        let after_sel = model.selected().cloned();
        if let (Some(Selected::Conversation(name)), Some(position), Some(Selected::Decision(_))) =
            (&before_sel, before_position, &after_sel)
        {
            self.back_position = Some((name.clone(), position));
        }
        let returning_position = match (&before_sel, &after_sel) {
            (
                Some(Selected::Decision(_) | Selected::Decided(_, _)),
                Some(Selected::Conversation(name)),
            ) => self
                .back_position
                .take()
                .filter(|(back, _)| back == name)
                .map(|(_, position)| position),
            _ => None,
        };
        if before_sel != after_sel {
            match &after_sel {
                Some(Selected::Decision(_)) | Some(Selected::Decided(_, _)) => {
                    self.decision.scroll_to_item(0);
                }
                _ if returning_position.is_some() => {}
                _ => {
                    self.back_position = None;
                    self.conversation.scroll_to_bottom();
                }
            }
        } else {
            match &after_sel {
                Some(Selected::Conversation(_)) if model.entries().len() != before_entries => {
                    self.conversation.scroll_to_bottom();
                }
                Some(Selected::Yard) if model.notices().len() != before_notices => {
                    self.conversation.scroll_to_bottom();
                }
                _ => {}
            }
        }
        if let Some(position) = returning_position {
            self.conversation.set_offset(position);
        }
        self.turned(before_sel, cx);
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
    /// with the conversation or the ask it was typed in, and the line has
    /// what was typed in the one that is open, if one is.
    fn turned(&mut self, from: Option<Selected>, cx: &mut Context<Self>) {
        let to = self.model.as_ref().and_then(|m| m.selected().cloned());
        let Some(from) = from.filter(|from| Some(from) != to.as_ref()) else {
            return;
        };
        // What was selected is of the conversation that is no longer shown.
        self.selection
            .update(cx, |selection, cx| selection.clear(cx));
        self.declining = None;
        let typed = self.line.read(cx).text().to_string();
        match &from {
            Selected::Conversation(name) => self.drafts.put_text(name, typed),
            Selected::Decision(bead) => {
                if let Some(ask) = self.model.as_ref().and_then(|m| m.current_for(bead)) {
                    self.ask_words.insert((bead.clone(), ask), typed);
                }
            }
            Selected::Decided(bead, ask) => {
                self.ask_words.insert((bead.clone(), *ask), typed);
            }
            Selected::Yard => {}
        }
        let text = match &to {
            Some(Selected::Conversation(name)) => self.drafts.take_text(name),
            Some(Selected::Decision(_)) => self
                .model
                .as_ref()
                .and_then(|m| m.current_ask())
                .and_then(|(bead, ask)| self.ask_words.remove(&(bead, ask)))
                .unwrap_or_default(),
            Some(Selected::Decided(bead, ask)) => self
                .ask_words
                .remove(&(bead.clone(), *ask))
                .unwrap_or_default(),
            _ => String::new(),
        };
        self.line.update(cx, |line, cx| line.set_text(&text, cx));
    }

    /// The line says whom it writes to, or what words on a card do.
    fn shown(&mut self, cx: &mut Context<Self>) {
        let to = match self.model.as_ref().and_then(|m| m.selected().cloned()) {
            Some(Selected::Conversation(name)) => format!("Write to {name}"),
            Some(Selected::Decision(_)) => {
                let asking = self
                    .model
                    .as_ref()
                    .and_then(|m| m.current_ask())
                    .is_some_and(|current| Some(&current) == self.declining.as_ref());
                if asking {
                    "Why decline? Enter sends, esc leaves".to_string()
                } else if self.current_options_empty() {
                    "Words wanted: Enter sends the answer".to_string()
                } else {
                    "Words with the pick, or alone as a note".to_string()
                }
            }
            Some(Selected::Decided(_, _)) => "Words with the pick".to_string(),
            _ => String::new(),
        };
        self.line
            .update(cx, |line, cx| line.set_placeholder(&to, cx));
    }

    /// Whether the open card's current ask wants words: it has no options.
    fn current_options_empty(&self) -> bool {
        let Some(model) = self.model.as_ref() else {
            return false;
        };
        let now = chrono::Utc::now();
        match model.selected() {
            Some(Selected::Decision(bead)) => model
                .decision_detail(bead, &now)
                .and_then(|d| d.asks.into_iter().find(|a| a.current))
                .is_some_and(|a| a.options.is_empty()),
            Some(Selected::Decided(bead, ask)) => model
                .decided_detail(bead, *ask, &chrono::Local)
                .is_some_and(|d| d.options.is_empty()),
            _ => false,
        }
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
                Effect::Bead { bead } => self.ask(
                    cx,
                    {
                        let bead = bead.clone();
                        move |client, _| client.bead(&bead)
                    },
                    move |this, answered, cx| match answered {
                        Ok(bead) => this.change(cx, |m| {
                            m.bead_loaded(bead);
                            Vec::new()
                        }),
                        Err(err) => this.change(cx, |m| {
                            m.bead_failed(&bead, &err.to_string());
                            Vec::new()
                        }),
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
    /// On a card Enter sends the pick with the line's words, or the words
    /// alone.
    fn send(&mut self, cx: &mut Context<Self>) {
        if self.open().is_none() {
            return self.decide(cx);
        }
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

    /// The open card's ask: the current one of a decision, or the dealt
    /// with itself.
    fn card(&self) -> Option<(String, u64)> {
        // The settings stand where the card does: its keys are not for a
        // card that is not in view.
        if self.settings_open {
            return None;
        }
        match self.model.as_ref()?.selected()? {
            Selected::Decision(_) => self.model.as_ref()?.current_ask(),
            Selected::Decided(bead, ask) => Some((bead.clone(), *ask)),
            _ => None,
        }
    }

    /// Enter on a card: the pick with the line's words in the one answer,
    /// or the words alone. Enter with nothing chosen and a recommendation
    /// picks the recommended option.
    fn decide(&mut self, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        // A decline asked for words: they go now.
        if self.declining.as_ref() == Some(&(bead.clone(), ask)) {
            let words = self.line.read(cx).text().trim().to_string();
            if words.is_empty() {
                return self.notice("decline needs words: why you will not decide it".into(), cx);
            }
            return self.send_decline(bead, ask, words, cx);
        }
        let words = self.line.read(cx).text().to_string();
        let action = self
            .model
            .as_ref()
            .and_then(|m| m.answer_action(&bead, ask, &words));
        match action {
            Some(AnswerAction::Answer { option, words }) => {
                self.send_answer(bead, ask, option, words, cx);
            }
            Some(AnswerAction::Note { words }) => self.send_note(bead, ask, words, cx),
            None => {}
        }
    }

    /// Picks an option with words alongside: the ask moves to "Decided" at
    /// once, and the queue and the bead are read again for what the yard
    /// says. What the yard refuses is back in the line, with its words.
    fn send_answer(
        &mut self,
        bead: String,
        ask: u64,
        option: u64,
        words: String,
        cx: &mut Context<Self>,
    ) {
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.ask_words.remove(&(bead.clone(), ask));
        self.notice(String::new(), cx);
        let (call_bead, call_words) = (bead.clone(), words.clone());
        self.ask(
            cx,
            move |client, me| client.answer_ask(&call_bead, ask, option, &call_words, me),
            move |this, answered, cx| match answered {
                Ok(()) => {
                    this.change(cx, |m| m.picked(&bead, ask, option, &words));
                    this.schedule_return(cx);
                }
                Err(err) => {
                    this.line.update(cx, |line, cx| {
                        let back = draft::rejoin(words.clone(), line.text().to_string());
                        line.set_text(&back, cx);
                    });
                    this.notice(format!("not answered: {err}"), cx);
                }
            },
        );
    }

    /// Words with no option chosen go as a plain note; the ask stays open.
    fn send_note(&mut self, bead: String, ask: u64, words: String, cx: &mut Context<Self>) {
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.ask_words.remove(&(bead.clone(), ask));
        self.notice(String::new(), cx);
        let (call_bead, call_words) = (bead.clone(), words.clone());
        self.ask(
            cx,
            move |client, me| client.add_note(&call_bead, &call_words, me),
            move |this, noted, cx| match noted {
                Ok(()) => {
                    // The note arrives as a `noted` event, which reads the
                    // queue and the bead again.
                }
                Err(err) => {
                    this.line.update(cx, |line, cx| {
                        let back = draft::rejoin(words.clone(), line.text().to_string());
                        line.set_text(&back, cx);
                    });
                    this.notice(format!("not noted: {err}"), cx);
                }
            },
        );
    }

    /// Puts the ask aside with `x`: it leaves the list and returns as the
    /// yard says.
    fn send_later(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.ask_words.remove(&(bead.clone(), ask));
        self.declining = None;
        self.notice(String::new(), cx);
        let call_bead = bead.clone();
        self.ask(
            cx,
            move |client, me| client.defer_ask(&call_bead, ask, me),
            move |this, deferred, cx| match deferred {
                Ok(()) => {
                    this.change(cx, |m| m.deferred(&bead, ask));
                    this.schedule_return(cx);
                }
                Err(err) => this.notice(format!("not put aside: {err}"), cx),
            },
        );
    }

    /// Declines the ask with words: not a pick and not "you decide".
    fn send_decline(&mut self, bead: String, ask: u64, words: String, cx: &mut Context<Self>) {
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.ask_words.remove(&(bead.clone(), ask));
        self.declining = None;
        self.notice(String::new(), cx);
        let (call_bead, call_words) = (bead.clone(), words.clone());
        self.ask(
            cx,
            move |client, me| client.decline_ask(&call_bead, ask, &call_words, me),
            move |this, declined, cx| match declined {
                Ok(()) => {
                    this.change(cx, |m| m.declined(&bead, ask, &words));
                    this.schedule_return(cx);
                }
                Err(err) => {
                    this.line.update(cx, |line, cx| {
                        let back = draft::rejoin(words.clone(), line.text().to_string());
                        line.set_text(&back, cx);
                    });
                    this.notice(format!("not declined: {err}"), cx);
                }
            },
        );
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
        self.settings_open = false;
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
        self.settings_open = false;
        self.change(cx, |m| m.step(by));
        let focus = match self.model.as_ref().and_then(|m| m.selected_conversation()) {
            Some(_) => self.line.focus_handle(cx),
            None => self.focus.clone(),
        };
        window.focus(&focus);
    }

    /// One key marks one notice read, on the open yard row; on a card `x`
    /// puts the ask aside.
    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.model.as_ref().and_then(|m| m.selected()) == Some(&Selected::Yard) {
            self.change(cx, |m| m.dismiss());
            return;
        }
        self.later_now(cx);
    }

    /// Puts the open card's current ask aside: the `x` key and the Later
    /// button. Later takes no words: only an open ask, never one dealt with.
    fn later_now(&mut self, cx: &mut Context<Self>) {
        let current = self.model.as_ref().and_then(|m| m.current_ask());
        if let Some((bead, ask)) = current {
            self.send_later(bead, ask, cx);
        }
    }

    /// A number key chooses an option of the open card.
    fn pick(&mut self, option: u64, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        self.declining = None;
        self.change(cx, |m| {
            m.choose(&bead, ask, option);
            Vec::new()
        });
    }

    /// The arrows choose an option of the open card, around the ends.
    fn choice(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        self.declining = None;
        self.change(cx, |m| {
            m.move_choice(&bead, ask, by);
            Vec::new()
        });
    }

    /// Decline asks for words and sends them: not a pick and not
    /// "you decide". With words already typed they go at once; else the
    /// next Enter sends what is typed, and esc leaves it.
    fn decline(&mut self, _: &Decline, window: &mut Window, cx: &mut Context<Self>) {
        self.decline_now(window, cx);
    }

    /// Declines the open card's current ask: the `d` key and the Decline
    /// button.
    fn decline_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.model.as_ref().and_then(|m| m.current_ask());
        let Some((bead, ask)) = current else {
            return;
        };
        let words = self.line.read(cx).text().trim().to_string();
        if !words.is_empty() {
            return self.send_decline(bead, ask, words, cx);
        }
        self.declining = Some((bead, ask));
        self.shown(cx);
        window.focus(&self.line.focus_handle(cx));
        cx.notify();
    }

    /// One key expands the card in place to the bead's body, notes and
    /// about-beads, or folds it again.
    fn expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        self.change(cx, |m| {
            m.toggle_expand(&bead, ask);
            Vec::new()
        });
    }

    /// The line has the words: `i` types them in, where a number key picks.
    fn focus_words(&mut self, _: &FocusWords, window: &mut Window, cx: &mut Context<Self>) {
        if self.card().is_none() {
            return;
        }
        window.focus(&self.line.focus_handle(cx));
    }

    /// Esc leaves what it can: a decline asked for words, the words for
    /// the card's keys, the pick, then the words.
    fn leave(&mut self, _: &Leave, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            return self.toggle_settings(window, cx);
        }
        if self.declining.take().is_some() {
            self.shown(cx);
            window.focus(&self.focus.clone());
            cx.notify();
            return;
        }
        let Some((bead, ask)) = self.card() else {
            return;
        };
        if self.line.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus.clone());
            return;
        }
        // The window holds the keys: first the pick, then the words.
        let chosen = self
            .model
            .as_ref()
            .and_then(|m| {
                let now = chrono::Utc::now();
                match m.selected() {
                    Some(Selected::Decision(b)) => m
                        .decision_detail(b, &now)
                        .and_then(|d| d.asks.into_iter().find(|a| a.ask == ask))
                        .map(|a| a.chosen),
                    Some(Selected::Decided(b, a)) => {
                        m.decided_detail(b, *a, &chrono::Local).map(|d| d.chosen)
                    }
                    _ => None,
                }
            })
            .unwrap_or(0);
        if chosen != 0 {
            self.change(cx, |m| {
                m.clear_choice(&bead, ask);
                Vec::new()
            });
        } else if !self.line.read(cx).text().is_empty() {
            self.line.update(cx, |line, cx| line.set_text("", cx));
            self.ask_words.remove(&(bead, ask));
        }
    }

    /// Makes the ask the bead's current one: the keys act on it. What is
    /// typed stays with the ask it was typed in.
    fn select_ask(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        let old = self.model.as_ref().and_then(|m| m.current_for(&bead));
        if old == Some(ask) {
            return;
        }
        let typed = self.line.read(cx).text().to_string();
        if let Some(old) = old {
            self.ask_words.insert((bead.clone(), old), typed);
        }
        self.change(cx, |m| {
            m.select_ask(&bead, ask);
            Vec::new()
        });
        let text = self.ask_words.remove(&(bead, ask)).unwrap_or_default();
        self.line.update(cx, |line, cx| line.set_text(&text, cx));
        self.declining = None;
        self.shown(cx);
    }

    /// Clicks an option of any card of the open bead: it becomes current,
    /// and the option is chosen.
    fn pick_for(&mut self, bead: String, ask: u64, option: u64, cx: &mut Context<Self>) {
        if self.model.as_ref().and_then(|m| m.current_for(&bead)) != Some(ask) {
            self.select_ask(bead.clone(), ask, cx);
        }
        self.pick(option, cx);
    }

    /// Clicks a card's expand: the bead's body, notes and about-beads.
    fn expand_for(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_expand(&bead, ask);
            Vec::new()
        });
    }

    /// Shows the settings where the conversation is, or what was open
    /// again. While they show, the window holds the keys: nothing typed
    /// goes to a line that is not in view.
    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        let conversation = self.model.as_ref().and_then(|m| m.selected_conversation());
        let focus = match conversation {
            Some(_) if !self.settings_open => self.line.focus_handle(cx),
            _ => self.focus.clone(),
        };
        window.focus(&focus);
        cx.notify();
    }

    /// Draws the window with `settings` from now on and keeps them. What
    /// keeps them from being kept is said in the settings, and the window
    /// has them all the same until it closes.
    fn set(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        // The line and the messages take their colours from this.
        cx.set_global(settings.look);
        if let Some(file) = &self.kept {
            self.unkept = settings.write(file).err().map(|err| err.to_string());
        }
        cx.notify();
    }

    /// The text a step larger or smaller.
    fn resize(&mut self, by: i8, cx: &mut Context<Self>) {
        self.set(self.settings.sized(by), cx);
    }

    fn usual_size(&mut self, cx: &mut Context<Self>) {
        self.set(
            Settings {
                text: settings::TEXT,
                ..self.settings
            },
            cx,
        );
    }

    fn look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.set(
            Settings {
                look,
                ..self.settings
            },
            cx,
        );
    }

    /// The control that opens the settings, and shuts them: in the head of
    /// the left column, and under the words when the yard is away.
    fn settings_control(
        &self,
        text: String,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        small_button(text, palette)
            .id("settings")
            .debug_selector(|| "settings".to_string())
            .flex_none()
            .when(self.settings_open, |el| el.bg(palette.open))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx)))
    }

    /// The settings: the text size and the look, each with its buttons and
    /// its keys, and where they are kept.
    fn talk_settings(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let kept = match &self.kept {
            Some(file) => format!("Kept in {}", file.display()),
            None => "Not kept: there is no home directory to keep them in.".to_string(),
        };
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("settings"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .truncate()
                            .child(kept),
                    ),
            )
            .child(
                small_button("back (esc)".to_string(), palette)
                    .id("settings back")
                    .debug_selector(|| "settings back".to_string())
                    .flex_none()
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx))),
            );
        // What a setting is called, over its buttons and the keys for it.
        let setting = |name: &'static str, keys: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                .child(div().text_sm().text_color(palette.faint).child(keys))
        };
        let text = self.settings.text;
        let size = setting(
            "text size",
            format!("{SECONDARY}+= larger · {SECONDARY}+- smaller · {SECONDARY}+0 the usual size"),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    small_button("smaller".to_string(), palette)
                        .id("text smaller")
                        .debug_selector(|| "text smaller".to_string())
                        .when(text == settings::TEXT_MIN, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.resize(-1, cx))),
                )
                .child(div().debug_selector(|| "text size".to_string()).child(
                    if text == settings::TEXT {
                        format!("{text} px, the usual size")
                    } else {
                        format!("{text} px")
                    },
                ))
                .child(
                    small_button("larger".to_string(), palette)
                        .id("text larger")
                        .debug_selector(|| "text larger".to_string())
                        .when(text == settings::TEXT_MAX, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.resize(1, cx))),
                )
                .child(
                    small_button("the usual size".to_string(), palette)
                        .id("text usual")
                        .debug_selector(|| "text usual".to_string())
                        .when(text == settings::TEXT, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.usual_size(cx))),
                ),
        );
        let chosen = self.settings.look;
        let look = setting(
            "light or dark",
            format!("{SECONDARY}+shift+l the next of auto, light, dark · auto is the desktop's"),
        )
        .child(div().flex().flex_wrap().items_center().gap_2().children(
            Look::ALL.into_iter().map(|look| {
                small_button(look.name().to_string(), palette)
                    .id(SharedString::from(format!("look {}", look.name())))
                    .debug_selector(|| format!("look {}", look.name()))
                    .when(look == chosen, |el| {
                        el.bg(palette.open)
                            .text_color(palette.text)
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.look(look, cx)))
            }),
        ));
        pane.child(head)
            .child(
                div()
                    .id("settings pane")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(size)
                    .child(look),
            )
            .when_some(self.unkept.clone(), |el, why| {
                el.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(palette.warn)
                        .child(format!("! not kept: {why}")),
                )
            })
            .child(
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("{SECONDARY}+, or esc goes back")),
            )
    }

    fn toggle_decided(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_decided();
            Vec::new()
        });
    }

    /// No yard: why, in words, and that it is tried again.
    fn away(&self, why: &str, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .child(img(self.icon.clone()).size(rems(4.5)))
            .child(
                div()
                    .max_w(rems(32.5))
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
            .child(self.settings_control(format!("settings ({SECONDARY}+,)"), palette, cx))
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
            // As high as the two lines beside it, nearly: 40 pixels at the
            // usual text size, and with the text when that is set.
            .child(
                img(self.icon.clone())
                    .size(rems(2.5))
                    .flex_none()
                    .debug_selector(|| "logo".to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("footplate"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.accent)
                            .truncate()
                            .child(model.me().to_string()),
                    ),
            )
            .child(self.settings_control("settings".to_string(), palette, cx));
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
            // 260 pixels at the usual text size. Its rows are cut to it, so
            // it grows with the text, but never over the conversation: the
            // smallest window still has the larger part for that.
            .w(rems(16.25))
            .max_w(relative(0.45))
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
            .debug_selector(|| format!("decision row {}", row.bead))
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
                    // Resolve the available row width before measuring the
                    // head, so truncate() can draw its ellipsis.
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .debug_selector(|| format!("decision head {}", row.bead))
                            .child(row.head),
                    )
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
                    .debug_selector(|| format!("decision id {}", row.bead))
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
                    // Resolve the available row width before measuring the
                    // head, so truncate() can draw its ellipsis.
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .debug_selector(|| format!("decided head {} {}", row.bead, row.ask))
                            .child(row.head),
                    )
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
                    .debug_selector(|| format!("decided id {} {}", row.bead, row.ask))
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
            .size(rems(ATTACHMENT))
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
                    .top(rems(-0.375))
                    .right(rems(-0.375))
                    .size(rems(1.))
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
            Some(Selected::Decision(bead)) => self
                .talk_decision(model, bead, palette, cx)
                .into_any_element(),
            Some(Selected::Decided(bead, ask)) => self
                .talk_decided(model, bead, *ask, palette, cx)
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
    /// A bead's open asks, each as its card: the question, the asker and
    /// the recommendation, the context, the numbered options, the
    /// recommended one with its why, a line for words, and the keys. One key
    /// expands the card to the bead whole.
    fn talk_decision(
        &self,
        model: &Model,
        bead: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .track_scroll(&self.decision)
            .p_4()
            .flex()
            .flex_col()
            .gap_4();
        let list = if !model.has_bead(bead) {
            list.child(div().text_color(palette.faint).child("Reading the bead."))
        } else {
            list.children(
                detail
                    .asks
                    .iter()
                    .map(|ask| self.decision_card(&detail, ask, palette, cx)),
            )
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
    }

    /// One card of an open bead, in the order the bead asks for: the
    /// question; the asker and the recommendation; the context; the numbered
    /// options; the recommended one with its why; a line for words; the
    /// send, later and decline buttons; the keys. Only the current one takes
    /// the keys, the line and the buttons.
    fn decision_card(
        &self,
        detail: &DecisionDetail,
        ask: &DecisionAsk,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bead_id = detail.bead.clone();
        let ask_seq = ask.ask;
        let current = ask.current;
        let mut card = div().flex().flex_col().gap_2();
        // The question, with the bead small and how long it waits.
        let short = detail
            .bead
            .rsplit('-')
            .next()
            .unwrap_or(detail.bead.as_str());
        card = card.child(
            div()
                .flex()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(ask.question.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(format!("{short} · {}", ask.waits)),
                ),
        );
        // The asker and the recommendation.
        let asker = if ask.recommend_label.is_empty() {
            format!("asked by {} · no recommendation", ask.asker)
        } else {
            format!(
                "asked by {} · recommends: {}",
                ask.asker, ask.recommend_label
            )
        };
        card = card.child(div().text_sm().text_color(palette.faint).child(asker));
        // The context lines.
        for line in &ask.context {
            card = card.child(div().child(line.clone()));
        }
        // The options numbered, each with its consequence, `you decide`
        // last as the yard lists it.
        if ask.options.is_empty() {
            card = card.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("words wanted"),
            );
        } else {
            for option in &ask.options {
                let marker = if option.number == ask.chosen {
                    "●"
                } else {
                    "○"
                };
                let text = format!(
                    "{} {}. {} — {}",
                    marker, option.number, option.label, option.consequence
                );
                let n = option.number;
                let bead_click = bead_id.clone();
                card = card.child(
                    div()
                        .id(SharedString::from(format!("pick {bead_id} {ask_seq} {n}")))
                        .debug_selector(|| format!("pick {bead_id} {ask_seq} {n}"))
                        .cursor_pointer()
                        .rounded_md()
                        .when(n == ask.chosen, |el| el.bg(palette.open))
                        .when(n != ask.chosen, |el| {
                            el.hover(|style| style.bg(palette.hover))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_for(bead_click.clone(), ask_seq, n, cx)
                        }))
                        .child(text),
                );
            }
            // The recommended one with its why.
            if ask.recommend != 0 {
                card = card.child(
                    div()
                        .text_sm()
                        .child(format!("recommended: {}, {}", ask.recommend, ask.why)),
                );
            }
        }
        // Decline is not a pick, and not "you decide".
        card = card.child(
            div()
                .text_sm()
                .text_color(palette.faint)
                .child("Decline is not a pick, and not \u{201c}you decide\u{201d}: the asker must find another way."),
        );
        if current {
            // A line for words.
            let focus = self.line.focus_handle(cx);
            card = card.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.side)
                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                        window.focus(&focus);
                        window.prevent_default();
                    })
                    .child(self.line.clone()),
            );
            // Under it the send button once an option is chosen, with Later
            // and Decline beside it.
            let send = ask
                .options
                .iter()
                .find(|o| o.number == ask.chosen)
                .map(|o| format!("Send: {} · {}", o.number, o.label));
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(send, |el, text| {
                        el.child(send_button(&bead_id, ask_seq, text, palette, cx))
                    })
                    .child(
                        small_button("Later (x)".to_string(), palette)
                            .id(SharedString::from(format!("later {bead_id} {ask_seq}")))
                            .debug_selector(|| format!("later {bead_id} {ask_seq}"))
                            .on_click(cx.listener(|this, _, _, cx| this.later_now(cx))),
                    )
                    .child(
                        small_button("Decline (d)".to_string(), palette)
                            .id(SharedString::from(format!("decline {bead_id} {ask_seq}")))
                            .debug_selector(|| format!("decline {bead_id} {ask_seq}"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.decline_now(window, cx)),
                            ),
                    ),
            );
            // The keys. Enter with nothing chosen and a recommendation picks
            // it, and says so.
            card = card.child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(decision_keys(ask)),
            );
        } else {
            let bead_click = bead_id.clone();
            card = card.child(
                div()
                    .id(SharedString::from(format!("current {bead_id} {ask_seq}")))
                    .text_sm()
                    .text_color(palette.accent)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_ask(bead_click.clone(), ask_seq, cx)
                    }))
                    .child("decide this one"),
            );
        }
        // One key expands the card in place.
        let expand_label = if ask.expanded { "fold" } else { "expand" };
        let bead_click = bead_id.clone();
        card = card.child(
            div()
                .id(SharedString::from(format!("expand {bead_id} {ask_seq}")))
                .text_sm()
                .text_color(palette.faint)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.expand_for(bead_click.clone(), ask_seq, cx)
                }))
                .child(format!("{expand_label} (e)")),
        );
        if ask.expanded {
            card = card.child(self.expanded_body(
                &detail.body,
                &detail.notes,
                &ask.about,
                &detail.doc,
                palette,
            ));
        }
        card
    }

    /// One dealt with: what was decided, in plain words with who and when;
    /// the picked option marked, with its words under it; the same options
    /// to pick again from there; and the same full bead one key away.
    fn talk_decided(
        &self,
        model: &Model,
        bead: &str,
        ask: u64,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(detail) = model.decided_detail(bead, ask, &chrono::Local) else {
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
            .track_scroll(&self.decision)
            .p_4()
            .flex()
            .flex_col()
            .gap_2();
        // The question, and what was decided with who and when.
        body = body
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.question.clone()),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.state_line.clone()),
            );
        if !model.has_bead(bead) {
            body = body.child(div().text_color(palette.faint).child("Reading the bead."));
        } else {
            // The context, the options to pick again, the recommendation.
            for line in &detail.context {
                body = body.child(div().child(line.clone()));
            }
            // An ask with no options shows none: the answer's words are in
            // the state line, and nothing is wanted.
            if !detail.options.is_empty() {
                for option in &detail.options {
                    // The circle marks the pick; a newly chosen option is
                    // the highlighted one, and the button says what it sends.
                    let picked = option.number == detail.picked;
                    let marker = if picked { "●" } else { "○" };
                    let text = format!(
                        "{} {}. {} — {}",
                        marker, option.number, option.label, option.consequence
                    );
                    let n = option.number;
                    body = body.child(
                        div()
                            .id(SharedString::from(format!("pick {bead} {ask} {n}")))
                            .debug_selector(|| format!("pick {bead} {ask} {n}"))
                            .cursor_pointer()
                            .rounded_md()
                            .when(picked, |el| {
                                el.font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.accent)
                            })
                            .when(n == detail.chosen, |el| el.bg(palette.open))
                            .when(n != detail.chosen, |el| {
                                el.hover(|style| style.bg(palette.hover))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.pick(n, cx)))
                            .child(text),
                    );
                    // The words sent with the pick stand under it.
                    if picked && !detail.words.is_empty() {
                        body = body.child(div().child(detail.words.clone()));
                    }
                }
                if detail.recommend != 0 {
                    body = body.child(
                        div()
                            .text_sm()
                            .child(format!("recommended: {}, {}", detail.recommend, detail.why)),
                    );
                }
            }
            // A line for words with the later pick.
            let focus = self.line.focus_handle(cx);
            body = body.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.side)
                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                        window.focus(&focus);
                        window.prevent_default();
                    })
                    .child(self.line.clone()),
            );
            // Under it the button that replaces the pick, once one is
            // chosen.
            if let Some(option) = detail.options.iter().find(|o| o.number == detail.chosen) {
                body = body.child(send_button(
                    bead,
                    ask,
                    format!("Change to: {} · {}", option.number, option.label),
                    palette,
                    cx,
                ));
            }
            body = body.child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(decided_keys(&detail)),
            );
            let expand_label = if detail.expanded { "fold" } else { "expand" };
            let bead_click = bead.to_string();
            body = body.child(
                div()
                    .id(SharedString::from(format!("expand {bead} {ask}")))
                    .text_sm()
                    .text_color(palette.faint)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expand_for(bead_click.clone(), ask, cx)
                    }))
                    .child(format!("{expand_label} (e)")),
            );
            if detail.expanded {
                body = body.child(self.expanded_body(
                    &detail.body,
                    &detail.notes,
                    &detail.about,
                    &detail.doc,
                    palette,
                ));
            }
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

    /// The bead whole, expanded in place: its body, notes and about-beads.
    /// A design bead's document is named by path and branch, not fetched.
    fn expanded_body(
        &self,
        body: &str,
        notes: &[model::CardNote],
        about: &[crate::api::AboutBead],
        doc: &str,
        palette: Palette,
    ) -> impl IntoElement + use<> {
        let mut out = div().flex().flex_col().gap_2();
        if !body.is_empty() {
            out = out
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.faint)
                        .child("body"),
                )
                .child(select::text(
                    select::key(("body", body)),
                    body.to_string(),
                    &self.selection,
                ));
        }
        if !notes.is_empty() {
            out = out.child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.faint)
                    .child("notes"),
            );
            // Who wrote a note stands over it and not before it: a note's
            // first line may be a heading or an item, which begin a line.
            for (ix, note) in notes.iter().enumerate() {
                out = out.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(note.author.clone()),
                        )
                        .child(select::text(
                            select::key(("note", ix, &note.text)),
                            note.text.clone(),
                            &self.selection,
                        )),
                );
            }
        }
        if !about.is_empty() {
            out = out.child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.faint)
                    .child("about"),
            );
            for bead in about {
                let line = if bead.title.is_empty() {
                    format!("{} · {}", bead.id, bead.stage)
                } else {
                    format!("{} · {} · {}", bead.id, bead.title, bead.stage)
                };
                out = out.child(div().child(line));
            }
        }
        if !doc.is_empty() {
            out = out.child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("document: {doc}")),
            );
        }
        out
    }
}

impl Focusable for Footplate {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Footplate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The one place the text size is set: every size that goes with the
        // text is in rems, and the text itself is seven eighths of one.
        window.set_rem_size(self.settings.rem());
        let palette = Palette::now(window, cx);
        // On a card the window holds the keys to pick with, and the line to
        // type words in: with the focus in the line those keys are text.
        let context = match self.model.as_ref().and_then(|m| m.selected().cloned()) {
            // The settings stand where the card or the notices do: the keys
            // that pick and mark read are not for what is not in view.
            _ if self.settings_open => CONTEXT,
            Some(Selected::Conversation(_)) => CONTEXT,
            Some(Selected::Decision(_)) | Some(Selected::Decided(_, _))
                if self.line.focus_handle(cx).is_focused(window) =>
            {
                CONTEXT
            }
            _ => NO_LINE_CONTEXT,
        };
        let root = div()
            .key_context(context)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &Previous, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(|this, _: &Pick1, _, cx| this.pick(1, cx)))
            .on_action(cx.listener(|this, _: &Pick2, _, cx| this.pick(2, cx)))
            .on_action(cx.listener(|this, _: &Pick3, _, cx| this.pick(3, cx)))
            .on_action(cx.listener(|this, _: &Pick4, _, cx| this.pick(4, cx)))
            .on_action(cx.listener(|this, _: &Pick5, _, cx| this.pick(5, cx)))
            .on_action(cx.listener(|this, _: &Pick6, _, cx| this.pick(6, cx)))
            .on_action(cx.listener(|this, _: &Pick7, _, cx| this.pick(7, cx)))
            .on_action(cx.listener(|this, _: &Pick8, _, cx| this.pick(8, cx)))
            .on_action(cx.listener(|this, _: &Pick9, _, cx| this.pick(9, cx)))
            .on_action(cx.listener(|this, _: &ChoiceUp, _, cx| this.choice(-1, cx)))
            .on_action(cx.listener(|this, _: &ChoiceDown, _, cx| this.choice(1, cx)))
            .on_action(cx.listener(|this, _: &Decide, _, cx| this.decide(cx)))
            .on_action(cx.listener(Self::decline))
            .on_action(cx.listener(Self::expand))
            .on_action(cx.listener(Self::focus_words))
            .on_action(cx.listener(Self::leave))
            // Files dropped anywhere on the window are for the draft of the
            // open conversation.
            .on_drop(
                cx.listener(|this, files: &ExternalPaths, _, cx| this.dropped(files.paths(), cx)),
            )
            .size_full()
            .flex()
            .bg(palette.ground)
            .text_color(palette.text)
            .text_size(rems(0.875));
        match (&self.down, &self.model) {
            (Some(_), _) if self.settings_open => root.child(self.talk_settings(palette, cx)),
            (Some(why), _) => root.child(self.away(why, palette, cx)),
            (None, Some(model)) if self.settings_open => root
                .child(self.list(model, palette, cx))
                .child(self.talk_settings(palette, cx)),
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
    use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, WindowAppearance};
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn the_window_icon_is_the_1024px_master_png() {
        let number = |at: usize| u32::from_be_bytes(ICON[at..at + 4].try_into().unwrap());
        assert_eq!(&ICON[12..16], b"IHDR");
        assert_eq!((number(16), number(20)), (1024, 1024));
    }

    /// The words-wanted keys name expand nowhere: the control under them,
    /// "expand (e)" and "fold (e)", is where the card names it once.
    #[test]
    fn the_words_wanted_keys_leave_expand_to_the_control() {
        let ask = DecisionAsk {
            ask: 3,
            question: "Name the release?".to_string(),
            asker: "mayor".to_string(),
            waits: "2h".to_string(),
            recommend_label: String::new(),
            context: Vec::new(),
            options: Vec::new(),
            recommend: 0,
            why: String::new(),
            chosen: 0,
            current: true,
            expanded: false,
            about: Vec::new(),
            state: "open".to_string(),
        };
        assert_eq!(
            decision_keys(&ask),
            "words wanted · i words · enter sends the answer"
        );
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
    fn a_link_inside_a_list_item_still_opens(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let item = format!("- {LINK}");
        mails(&yard, &[&item]);
        let (view, cx) = opened(&yard, &[&item], cx);

        // The item's words are drawn without its marker, set in from the
        // edge of the text.
        let indent = px(16. * select::ITEM_INDENT);
        let at = |cx: &mut VisualTestContext, index: usize, by: f32| {
            let at = place(cx, 12, LINK, index, by);
            gpui::point(at.x + indent, at.y)
        };
        // Beside the link, and where the marker is: nothing opens.
        let beside = at(cx, 1, 1.);
        cx.simulate_click(beside, gpui::Modifiers::default());
        let marker = gpui::point(text_bounds(cx, 12).left() + px(4.), beside.y);
        cx.simulate_click(marker, gpui::Modifiers::default());
        assert_eq!(cx.opened_url(), None);
        let on = at(cx, 10, 1.);
        cx.simulate_click(on, gpui::Modifiers::default());
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.org/a"));
        assert_eq!(selected(&view, cx), None);

        // What is selected of the item is of the text as it was written.
        let (from, to) = (at(cx, 4, 1.), at(cx, 25, -1.));
        drag(cx, from, to);
        assert_eq!(
            selected(&view, cx).as_deref(),
            Some("https://example.org/a")
        );
    }

    #[gpui::test]
    fn a_selection_across_a_code_block_is_copied_as_it_was_written(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let text = "## Run **this**:\n\n```sh\n  ls **/*.rs\n1. kept\n```\n- then `wait`\n- https://example.org/a";
        mails(&yard, &[text]);
        let (view, cx) = opened(&yard, &[text], cx);

        // From the first block to the last, over the heading, the code and
        // the items: none of their marks is drawn, and all are copied.
        let all = text_bounds(cx, 12);
        let (top, bottom) = (
            gpui::point(all.left(), all.top() + px(2.)),
            gpui::point(all.right() - px(2.), all.bottom() - px(2.)),
        );
        drag(cx, top, bottom);
        // The heading's marks stand before its first letter: a selection
        // begins after them, and has every mark after that.
        let copied = &text["## ".len()..];
        assert_eq!(selected(&view, cx).as_deref(), Some(copied));
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(clipboard(cx).as_deref(), Some(copied));
        drag(cx, bottom, top);
        assert_eq!(selected(&view, cx).as_deref(), Some(copied));
    }

    #[gpui::test]
    fn a_line_of_code_wider_than_the_window_is_wrapped(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let short = "```\ncargo test\n```";
        let long = format!("```\n{}\n```", "cargo test --locked ".repeat(60));
        mails(&yard, &[short, &long]);
        let (view, cx) = opened(&yard, &[short, &long], cx);

        // It goes on in the rows under it, where its mark is drawn, and
        // nothing of it is past the edge.
        let (one, wrapped) = (text_bounds(cx, 12), text_bounds(cx, 13));
        assert!(wrapped.size.height > one.size.height * 2., "{wrapped:?}");
        assert_eq!(wrapped.size.width, one.size.width);
        // All of it is still one line when it is copied.
        drag(
            cx,
            gpui::point(wrapped.left(), wrapped.top() + px(2.)),
            gpui::point(wrapped.right() - px(2.), wrapped.bottom() - px(2.)),
        );
        let code = &long["```\n".len()..long.len() - "\n```".len()];
        assert_eq!(selected(&view, cx).as_deref(), Some(code));
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

    /// One bead with three open asks, oldest first: 3 and 4 with a
    /// recommendation, 5 with none, for words alone.
    fn asks_three() -> String {
        json!({"result": {"open": [
            {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
             "ask": 3, "question": "Move them now?", "asker": "yardmaster",
             "since": "2026-10-10T10:00:00Z"},
            {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
             "ask": 4, "question": "Send them to build?", "asker": "yardmaster",
             "since": "2026-10-10T11:00:00Z"},
            {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
             "ask": 5, "question": "Close sketch PR #3?", "asker": "assembly",
             "since": "2026-10-10T11:30:00Z"}
        ], "decided": []}})
        .to_string()
    }

    fn bead_three() -> String {
        let three = json!({
            "ask": 3, "to": "jo", "asker": "yardmaster",
            "since": "2026-10-10T10:00:00Z",
            "question": "Move them now?",
            "context": ["The links file declares the new source.", "The move was tested."],
            "options": [
                {"label": "Move them now", "consequence": "all 64 links point tonight"},
                {"label": "After Friday", "consequence": "keep the old source"},
                {"label": "You decide", "consequence": "leave it to yardmaster"}
            ],
            "recommend": 1, "why": "the test passed",
            "about": [{"id": "yd-train", "title": "Release train", "stage": "new"}],
            "state": "open"
        });
        let four = json!({
            "ask": 4, "to": "jo", "asker": "yardmaster",
            "since": "2026-10-10T11:00:00Z",
            "question": "Send them to build?",
            "context": ["Both beads are shaped."],
            "options": [
                {"label": "Send them", "consequence": "both beads move tonight"},
                {"label": "You decide", "consequence": "leave it to yardmaster"}
            ],
            "recommend": 1, "why": "both are shaped",
            "state": "open"
        });
        let five = json!({
            "ask": 5, "to": "jo", "asker": "assembly",
            "since": "2026-10-10T11:30:00Z",
            "question": "Close sketch PR #3?",
            "context": ["The sketch landed yesterday."],
            "options": [
                {"label": "Close it", "consequence": "PR #3 closes"},
                {"label": "You decide", "consequence": "leave it to assembly"}
            ],
            "state": "open"
        });
        json!({"result": {
            "id": "bead-a", "title": "Move the links",
            "body": "The body of bead-a.",
            "stage": "new", "labels": ["topic:linear-out"],
            "notes": [{"seq": 2, "kind": "note", "author": "yardmaster",
                       "text": "the test passed", "at": "2026-10-10T09:30:00Z"}],
            "asks": [three, four, five]
        }})
        .to_string()
    }

    fn chosen_of(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        bead: &str,
        ask: u64,
    ) -> u64 {
        view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            let now = chrono::Utc::now();
            model
                .decision_detail(bead, &now)
                .unwrap()
                .asks
                .into_iter()
                .find(|a| a.ask == ask)
                .unwrap()
                .chosen
        })
    }

    fn has_bead(view: &Entity<Footplate>, cx: &mut VisualTestContext, bead: &str) -> bool {
        view.read_with(cx, |v, _| v.model.as_ref().unwrap().has_bead(bead))
    }

    fn expanded_of(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        bead: &str,
        ask: u64,
    ) -> bool {
        view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            let now = chrono::Utc::now();
            model
                .decision_detail(bead, &now)
                .unwrap()
                .asks
                .into_iter()
                .find(|a| a.ask == ask)
                .unwrap()
                .expanded
        })
    }

    fn current_ask_of(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> u64 {
        view.read_with(cx, |v, _| {
            v.model.as_ref().unwrap().current_ask().unwrap().1
        })
    }

    #[gpui::test]
    fn the_card_picks_with_words_puts_aside_declines_and_expands(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_three());
        yard.answer("Bead", &bead_three());
        yard.answer("AnswerAsk", "{}");
        yard.answer("DeferAsk", "{}");
        yard.answer("DeclineAsk", "{}");
        yard.answer("AddNote", "{}");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        // The bead opens on its oldest ask, and the bead is read.
        wait(cx, "the decision", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Decision("bead-a".into())))
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));
        assert_eq!(current_ask_of(&view, cx), 3);
        cx.run_until_parked();

        // A number key chooses; Enter sends the pick. The line starts
        // focused, so esc holds the window's keys first.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("2");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 2);
        cx.simulate_keystrokes("enter");
        wait(cx, "the pick", |_| {
            yard.asked("AnswerAsk")
                == [json!({"bead": "bead-a", "ask": 3, "option": 2, "actor": "jo"})]
        });
        // The ask moves to Decided at once in the model; the queue's next
        // read is the yard's stale answer in this test, which brings it
        // back. What matters here is the write.
        cx.run_until_parked();

        // A pick with words on the next ask: choose, type them in, Enter
        // sends the one answer.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 4, cx));
        assert_eq!(current_ask_of(&view, cx), 4);
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("1");
        assert_eq!(chosen_of(&view, cx, "bead-a", 4), 1);
        cx.simulate_keystrokes("i");
        cx.simulate_input("send them tonight");
        assert_eq!(typed(&view, cx), "send them tonight");
        cx.simulate_keystrokes("enter");
        wait(cx, "the pick with words", |_| {
            yard.asked("AnswerAsk").last()
                == Some(&json!({"bead": "bead-a", "ask": 4, "option": 1,
                                "words": "send them tonight", "actor": "jo"}))
        });
        assert_eq!(typed(&view, cx), "");
        cx.run_until_parked();

        // Words with no option chosen and no recommendation go as a plain
        // note; the ask stays open.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 5, cx));
        assert_eq!(current_ask_of(&view, cx), 5);
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("i");
        cx.simulate_input("what about the docs links?");
        cx.simulate_keystrokes("enter");
        wait(cx, "the note", |_| {
            yard.asked("AddNote")
                == [json!({"bead": "bead-a", "text": "what about the docs links?", "actor": "jo"})]
        });
        assert_eq!(current_ask_of(&view, cx), 5);
        assert_eq!(typed(&view, cx), "");

        // `x` puts it aside.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("x");
        wait(cx, "put aside", |_| {
            yard.asked("DeferAsk") == [json!({"bead": "bead-a", "ask": 5, "actor": "jo"})]
        });
        cx.run_until_parked();

        // Decline with words already typed goes at once.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 3, cx));
        cx.simulate_keystrokes("i");
        cx.simulate_input("not mine to take");
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("d");
        wait(cx, "declined", |_| {
            yard.asked("DeclineAsk")
                == [json!({"bead": "bead-a", "ask": 3, "words": "not mine to take", "actor": "jo"})]
        });
        cx.run_until_parked();

        // Decline with an empty line asks for words: the next Enter sends
        // them.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 4, cx));
        cx.simulate_keystrokes("escape");
        assert_eq!(typed(&view, cx), "");
        cx.simulate_keystrokes("d");
        cx.simulate_input("the greeting can wait");
        cx.simulate_keystrokes("enter");
        wait(cx, "declined after asking", |_| {
            yard.asked("DeclineAsk").last()
                == Some(
                    &json!({"bead": "bead-a", "ask": 4, "words": "the greeting can wait", "actor": "jo"}),
                )
        });
        cx.run_until_parked();

        // One key expands the card to the bead whole, and folds it again.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 3, cx));
        cx.simulate_keystrokes("escape");
        assert!(!expanded_of(&view, cx, "bead-a", 3));
        cx.simulate_keystrokes("e");
        assert!(expanded_of(&view, cx, "bead-a", 3));
        // The window draws the expanded card and stays.
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.simulate_keystrokes("e");
        assert!(!expanded_of(&view, cx, "bead-a", 3));

        // The arrows choose around the ends, even while words are typed.
        cx.simulate_keystrokes("i");
        cx.simulate_input("tonight?");
        cx.simulate_keystrokes("down");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 1);
        cx.simulate_keystrokes("up");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 3);
        assert_eq!(typed(&view, cx), "tonight?");
    }

    /// Clicks where `selector` is drawn.
    fn click_on(cx: &mut VisualTestContext, selector: &str) {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let at: &'static str = Box::leak(selector.to_string().into_boxed_str());
        let bounds = cx
            .debug_bounds(at)
            .unwrap_or_else(|| panic!("nothing is drawn for {selector}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    /// Whether `selector` is drawn.
    fn drawn(cx: &mut VisualTestContext, selector: &str) -> bool {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let at: &'static str = Box::leak(selector.to_string().into_boxed_str());
        cx.debug_bounds(at).is_some()
    }

    fn bounds_of(cx: &mut VisualTestContext, selector: &str) -> Bounds<Pixels> {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let at: &'static str = Box::leak(selector.to_string().into_boxed_str());
        cx.debug_bounds(at)
            .unwrap_or_else(|| panic!("nothing is drawn for {selector}"))
    }

    /// Opens one dealt with: "Decided" unfolded, its row selected.
    fn open_decided(view: &Entity<Footplate>, cx: &mut VisualTestContext, bead: &str, ask: u64) {
        view.update(cx, |v, cx| {
            v.change(cx, |m| {
                if !m.decided_expanded() {
                    m.toggle_decided();
                }
                m.select(Selected::Decided(bead.into(), ask))
            })
        });
        cx.run_until_parked();
    }

    fn decided_detail_of(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        bead: &str,
        ask: u64,
    ) -> DecidedDetail {
        view.read_with(cx, |v, _| {
            v.model
                .as_ref()
                .unwrap()
                .decided_detail(bead, ask, &chrono::Utc)
                .unwrap()
        })
    }

    /// Six dealt with on one bead, asked by mayor at 11:07: answered,
    /// answered with words, put aside, declined, left to the asker, and a
    /// words answer. Ask 11's pick is `pick` with `label`.
    fn asks_decided(pick: u64, label: &str) -> String {
        json!({"result": {"open": [], "decided": [
            {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
             "ask": 11, "question": "Switch my inbox to the 'attention' set of events?", "asker": "mayor",
             "since": "2026-10-12T11:07:00Z", "state": "answered", "option": pick, "option_label": label},
            {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
             "ask": 12, "question": "Switch the inbox now or after Friday?", "asker": "mayor",
             "since": "2026-10-12T11:07:00Z", "state": "answered",
             "option": 2, "option_label": "Add only the three", "words": "go slow"},
            {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
             "ask": 13, "question": "Rebuild the index tonight?", "asker": "mayor",
             "since": "2026-10-12T11:07:00Z", "state": "put aside",
             "return_at": "2026-10-13T09:30:00Z"},
            {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
             "ask": 14, "question": "Take the release train?", "asker": "mayor",
             "since": "2026-10-12T11:07:00Z", "state": "declined", "words": "not mine"},
            {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
             "ask": 15, "question": "Name the release?", "asker": "mayor",
             "since": "2026-10-12T11:07:00Z", "state": "answered",
             "option": 4, "option_label": "You decide"},
            {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
             "ask": 16, "question": "Who greets the newcomer?", "asker": "mayor",
             "since": "2026-10-12T11:07:00Z", "state": "answered", "words": "warmly"}
        ]}})
        .to_string()
    }

    fn attention_options() -> serde_json::Value {
        json!([
            {"label": "Switch to attention", "consequence": "I run 'yardr crew set mayor --on attention' now; takes effect at once"},
            {"label": "Add only the three", "consequence": "keep my hand list and append the missing kinds"},
            {"label": "Leave it", "consequence": "no change; doctor keeps flagging it"},
            {"label": "You decide", "consequence": "leave it to mayor"}
        ])
    }

    fn decided_bead_ask(ask: u64, question: &str, state: &str, options: bool) -> serde_json::Value {
        let mut a = json!({
            "ask": ask, "to": "jo", "asker": "mayor",
            "since": "2026-10-12T11:07:00Z",
            "question": question,
            "context": ["My inbox listens to a hand-written list of event kinds."],
            "state": state,
        });
        if options {
            a["options"] = attention_options();
            a["recommend"] = json!(1);
            a["why"] = json!("it follows new kinds without hand edits");
        } else {
            a["options"] = json!([]);
        }
        a
    }

    fn bead_decided() -> String {
        json!({"result": {
            "id": "bead-d", "title": "Switch the inbox",
            "body": "The body of bead-d.",
            "stage": "new", "labels": ["topic:linear-out"],
            "notes": [],
            "asks": [
                decided_bead_ask(11, "Switch my inbox to the 'attention' set of events?", "answered", true),
                decided_bead_ask(12, "Switch the inbox now or after Friday?", "answered", true),
                decided_bead_ask(13, "Rebuild the index tonight?", "put aside", true),
                decided_bead_ask(14, "Take the release train?", "declined", true),
                decided_bead_ask(15, "Name the release?", "answered", true),
                decided_bead_ask(16, "Who greets the newcomer?", "answered", false),
            ]
        }})
        .to_string()
    }

    #[gpui::test]
    fn a_decided_card_shows_what_was_decided_and_a_new_pick_replaces_it(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_decided(1, "Switch to attention"));
        yard.answer("Bead", &bead_decided());
        yard.answer("AnswerAsk", "{}");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        // Nothing open: the yardmaster's conversation is, with the decided
        // folded under the decisions.
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            assert_eq!(model.decided_count(), 6);
            assert!(!model.decided_expanded());
        });

        open_decided(&view, cx, "bead-d", 11);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-d"));

        // Each state in plain words with who and when, and the pick.
        let d = decided_detail_of(&view, cx, "bead-d", 11);
        assert_eq!(d.picked, 1);
        assert_eq!(d.words, "");
        assert_eq!(d.state_line, "you picked 1 · Switch to attention");
        assert_eq!(d.chosen, 0);
        assert_eq!(decided_keys(&d), "1-4 change your pick");

        let d = decided_detail_of(&view, cx, "bead-d", 12);
        assert_eq!(d.picked, 2);
        assert_eq!(d.words, "go slow");
        assert_eq!(d.state_line, "you picked 2 · Add only the three");

        let d = decided_detail_of(&view, cx, "bead-d", 13);
        assert_eq!(d.picked, 0);
        assert_eq!(d.state_line, "put aside, back 10-13 09:30");

        let d = decided_detail_of(&view, cx, "bead-d", 14);
        assert_eq!(d.state_line, "you declined: not mine");

        let d = decided_detail_of(&view, cx, "bead-d", 15);
        assert_eq!(d.picked, 4);
        assert_eq!(d.state_line, "left to mayor");

        // An ask with no options: the answer's words are the state line.
        let d = decided_detail_of(&view, cx, "bead-d", 16);
        assert_eq!(d.picked, 0);
        assert_eq!(d.state_line, "you answered: warmly");
        assert_eq!(decided_keys(&d), "i words · enter sends");

        // The Decided rows keep showing the picks.
        let subs: Vec<String> = view.read_with(cx, |v, _| {
            v.model
                .as_ref()
                .unwrap()
                .decided_rows()
                .into_iter()
                .map(|r| r.sub)
                .collect()
        });
        assert_eq!(
            subs,
            [
                "mayor · Switch to attention",
                "mayor · Add only the three · go slow",
                "mayor · put aside",
                "mayor · declined · not mine",
                "mayor · You decide",
                "mayor · warmly",
            ]
        );

        // The window draws every variant and stays.
        for ask in [11, 12, 13, 14, 15, 16] {
            open_decided(&view, cx, "bead-d", ask);
        }
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();

        // On a decided ask nothing is chosen by default and Enter alone
        // sends nothing: not the recommendation, and no note.
        open_decided(&view, cx, "bead-d", 11);
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("AnswerAsk"), Vec::<serde_json::Value>::new());
        assert_eq!(yard.asked("AddNote"), Vec::<serde_json::Value>::new());
        cx.simulate_keystrokes("i");
        cx.simulate_input("second thoughts");
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("AnswerAsk"), Vec::<serde_json::Value>::new());
        assert_eq!(yard.asked("AddNote"), Vec::<serde_json::Value>::new());
        // Esc leaves the words.
        cx.simulate_keystrokes("escape");
        assert_eq!(typed(&view, cx), "");

        // A number then Enter replaces the pick, as before.
        yard.answer("UserAsks", &asks_decided(2, "Add only the three"));
        cx.simulate_keystrokes("2");
        assert_eq!(decided_detail_of(&view, cx, "bead-d", 11).chosen, 2);
        cx.simulate_keystrokes("enter");
        wait(cx, "the changed pick", |_| {
            yard.asked("AnswerAsk")
                == [json!({"bead": "bead-d", "ask": 11, "option": 2, "actor": "jo"})]
        });
        wait(cx, "the new pick shown", |cx| {
            decided_detail_of(&view, cx, "bead-d", 11).picked == 2
        });
        assert_eq!(
            decided_detail_of(&view, cx, "bead-d", 11).state_line,
            "you picked 2 · Add only the three"
        );
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// A decided head longer than the row ends in an ellipsis before the
    /// bead id, with a gap between them. The ellipsis itself is drawn by
    /// GPUI's truncate(), which the test cannot read back; what it pins
    /// is why the ellipsis appears. The head's column fills the row, so a
    /// short head's column is as wide as a long head's. Sized to its
    /// content instead, the head is measured full-width first, and
    /// truncate() then cuts it hard, with no ellipsis.
    #[gpui::test]
    fn a_long_decided_head_ends_before_the_bead_id_with_a_gap(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_decided(1, "Switch to attention"));
        yard.answer("Bead", &bead_decided());
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the decided", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().is_some_and(|m| m.decided_count() == 6)
            })
        });
        // Ask 11's head is longer than the row; ask 15's is short. Both
        // rows end with the same id.
        open_decided(&view, cx, "bead-d", 11);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-d"));
        let long = bounds_of(cx, "decided head bead-d 11");
        let id = bounds_of(cx, "decided id bead-d 11");
        assert!(
            long.right() < id.left(),
            "the long head touches the id: {long:?} {id:?}"
        );
        let short = bounds_of(cx, "decided head bead-d 15");
        assert_eq!(
            short.size.width, long.size.width,
            "the head's column hugs its content instead of filling the row"
        );
    }

    /// An open decision row has the same width constraint as a decided row:
    /// its head must be measured at the space left before the bead id.
    #[gpui::test]
    fn a_long_decision_head_ends_before_the_bead_id_with_a_gap(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [
                {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
                 "ask": 3, "question": "A deliberately long question that must be cut before the id", "asker": "yardmaster",
                 "since": "2026-10-10T10:00:00Z"},
                {"bead": "bead-b", "bead_title": "Short", "topic": "topic:linear-out",
                 "ask": 4, "question": "Name it?", "asker": "yardmaster",
                 "since": "2026-10-10T11:00:00Z"}
            ], "decided": []}})
            .to_string(),
        );
        yard.answer("Bead", &bead_three());
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the decisions", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().is_some_and(|m| m.decisions_count() == 2)
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));

        let long = bounds_of(cx, "decision head bead-a");
        let id = bounds_of(cx, "decision id bead-a");
        let short = bounds_of(cx, "decision head bead-b");
        assert!(
            long.right() < id.left(),
            "the long head touches the id: {long:?} {id:?}"
        );
        assert_eq!(
            short.size.width, long.size.width,
            "the head's column hugs its content instead of filling the row"
        );
    }

    #[gpui::test]
    fn a_pick_leads_back_to_the_conversation_with_what_was_typed_in_it(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // One bead with one ask: the pick takes its row away.
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [
                {"bead": "bead-a", "bead_title": "Move the links", "topic": "",
                 "ask": 3, "question": "Move them now?", "asker": "yardmaster",
                 "since": "2026-10-10T10:00:00Z"}
            ], "decided": []}})
            .to_string(),
        );
        yard.answer("Bead", &bead_three());
        yard.answer("AnswerAsk", "{}");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        let open = |view: &Entity<Footplate>, cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().and_then(|m| m.selected().cloned())
            })
        };
        wait(cx, "the decision", |cx| {
            open(&view, cx) == Some(Selected::Decision("bead-a".into()))
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));

        // The yardmaster's conversation is not the first row: assembly's is.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Conversation("yardmaster".into()), window, cx)
        });
        mails(
            &yard,
            &["A long enough letter to make the conversation scroll."; 12],
        );
        wait(cx, "the full conversation", |cx| {
            texts(&view, cx).len() == 12
        });
        view.read_with(cx, |v, _| v.conversation.scroll_to_top_of_item(3));
        cx.run_until_parked();
        let place = view.read_with(cx, |v, _| v.conversation.offset());
        assert!(place.y < px(0.));
        cx.simulate_input("half a thought");
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Decision("bead-a".into()), window, cx)
        });
        assert_eq!(typed(&view, cx), "");
        cx.simulate_keystrokes("1");
        cx.simulate_keystrokes("enter");
        wait(cx, "the pick", |_| {
            yard.asked("AnswerAsk")
                == [json!({"bead": "bead-a", "ask": 3, "option": 1, "actor": "jo"})]
        });
        wait(cx, "the conversation again", |cx| {
            open(&view, cx) == Some(Selected::Conversation("yardmaster".into()))
        });
        assert_eq!(typed(&view, cx), "half a thought");
        assert_eq!(view.read_with(cx, |v, _| v.conversation.offset()), place);
    }

    /// One bead with three open asks and one dealt with, for the buttons.
    fn asks_buttons() -> String {
        json!({"result": {
            "open": [
                {"bead": "bead-a", "bead_title": "Sketch", "topic": "",
                 "ask": 3, "question": "Close sketch PR #3?", "asker": "yardmaster",
                 "since": "2026-10-10T10:00:00Z"},
                {"bead": "bead-a", "bead_title": "Sketch", "topic": "",
                 "ask": 4, "question": "Send them to build?", "asker": "yardmaster",
                 "since": "2026-10-10T11:00:00Z"},
                {"bead": "bead-a", "bead_title": "Sketch", "topic": "",
                 "ask": 5, "question": "Name the release?", "asker": "yardmaster",
                 "since": "2026-10-10T11:30:00Z"}
            ],
            "decided": [
                {"bead": "bead-a", "bead_title": "Sketch", "topic": "",
                 "ask": 6, "question": "Merge the docs?", "asker": "yardmaster",
                 "since": "2026-10-10T09:30:00Z", "state": "answered",
                 "option": 1, "option_label": "Close it"}
            ]
        }})
        .to_string()
    }

    fn bead_buttons() -> String {
        let options = json!([
            {"label": "Close it", "consequence": "PR #3 closes"},
            {"label": "Leave it", "consequence": "it stays open"},
            {"label": "You decide", "consequence": "leave it to yardmaster"}
        ]);
        let mut asks = Vec::new();
        for (ask, question) in [
            (3, "Close sketch PR #3?"),
            (4, "Send them to build?"),
            (5, "Name the release?"),
            (6, "Merge the docs?"),
        ] {
            asks.push(json!({
                "ask": ask, "to": "jo", "asker": "yardmaster",
                "since": "2026-10-10T10:00:00Z",
                "question": question,
                "context": ["The sketch landed yesterday."],
                "options": options,
                "recommend": 1, "why": "the sketch landed",
                "state": "open"
            }));
        }
        json!({"result": {
            "id": "bead-a", "title": "Sketch",
            "body": "The body of bead-a.",
            "stage": "new", "labels": [],
            "notes": [],
            "asks": asks
        }})
        .to_string()
    }

    #[gpui::test]
    fn options_are_chosen_with_a_click_and_a_button_sends_what_it_says(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_buttons());
        yard.answer("Bead", &bead_buttons());
        yard.answer("AnswerAsk", "{}");
        yard.answer("DeferAsk", "{}");
        yard.answer("DeclineAsk", "{}");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the decision", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Decision("bead-a".into())))
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));
        assert_eq!(current_ask_of(&view, cx), 3);

        // With nothing chosen there is no send button, even with a
        // recommendation: never one that sends it unasked. Later and
        // Decline stand beside where it will be.
        assert!(!drawn(cx, "send bead-a 3"));
        assert!(drawn(cx, "later bead-a 3"));
        assert!(drawn(cx, "decline bead-a 3"));

        // The keys line stays, shorter.
        let keys = view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            let now = chrono::Utc::now();
            let ask = model
                .decision_detail("bead-a", &now)
                .unwrap()
                .asks
                .into_iter()
                .find(|a| a.ask == 3)
                .unwrap();
            decision_keys(&ask)
        });
        assert_eq!(keys, "enter picks 1 · 1-3 pick · ↑↓ choose · i words");

        // A click on an option's line chooses it, as its number key does.
        click_on(cx, "pick bead-a 3 2");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 2);

        // The button says what it sends; a click sends the pick with the
        // words, exactly as Enter does. The click takes the focus, so the
        // line is focused again for the words.
        assert!(drawn(cx, "send bead-a 3"));
        cx.update(|window, cx| window.focus(&view.read(cx).line.focus_handle(cx)));
        cx.simulate_input("tonight");
        assert_eq!(typed(&view, cx), "tonight");
        click_on(cx, "send bead-a 3");
        wait(cx, "the pick with words", |_| {
            yard.asked("AnswerAsk")
                == [
                    json!({"bead": "bead-a", "ask": 3, "option": 2, "words": "tonight", "actor": "jo"}),
                ]
        });

        // Later is a button too, on the next ask.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 4, cx));
        click_on(cx, "later bead-a 4");
        wait(cx, "put aside", |_| {
            yard.asked("DeferAsk") == [json!({"bead": "bead-a", "ask": 4, "actor": "jo"})]
        });

        // Decline with an empty line asks for words, as the key does.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 5, cx));
        assert_eq!(typed(&view, cx), "");
        click_on(cx, "decline bead-a 5");
        cx.simulate_input("the greeting can wait");
        assert_eq!(typed(&view, cx), "the greeting can wait");
        cx.simulate_keystrokes("enter");
        wait(cx, "declined after asking", |_| {
            yard.asked("DeclineAsk").last()
                == Some(
                    &json!({"bead": "bead-a", "ask": 5, "words": "the greeting can wait", "actor": "jo"}),
                )
        });

        // Decline with words already typed goes at once, once the queue's
        // next read has brought the asks back.
        wait(cx, "the asks back", |cx| {
            view.read_with(cx, |v, _| {
                let model = v.model.as_ref().unwrap();
                let now = chrono::Utc::now();
                model
                    .decision_detail("bead-a", &now)
                    .is_some_and(|d| d.asks.len() == 3)
            })
        });
        cx.run_until_parked();
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 3, cx));
        cx.update(|window, cx| window.focus(&view.read(cx).line.focus_handle(cx)));
        cx.simulate_input("not mine");
        assert_eq!(typed(&view, cx), "not mine");
        click_on(cx, "decline bead-a 3");
        wait(cx, "declined", |_| {
            yard.asked("DeclineAsk").last()
                == Some(&json!({"bead": "bead-a", "ask": 3, "words": "not mine", "actor": "jo"}))
        });

        // On a dealt with the button replaces the pick.
        open_decided(&view, cx, "bead-a", 6);
        assert_eq!(decided_detail_of(&view, cx, "bead-a", 6).picked, 1);
        assert!(!drawn(cx, "send bead-a 6"));
        click_on(cx, "pick bead-a 6 2");
        assert_eq!(decided_detail_of(&view, cx, "bead-a", 6).chosen, 2);
        assert!(drawn(cx, "send bead-a 6"));
        click_on(cx, "send bead-a 6");
        wait(cx, "the changed pick", |_| {
            yard.asked("AnswerAsk").last()
                == Some(&json!({"bead": "bead-a", "ask": 6, "option": 2, "actor": "jo"}))
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// The window on `yard` with its settings kept in `file`, the focus in
    /// the line, once the yardmaster's conversation shows.
    fn opened_kept<'a>(
        yard: &Yard,
        file: &std::path::Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        let (client, file) = (yard.client(), file.to_path_buf());
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(client, "jo".into(), window, cx).kept_in(Some(file), cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        (view, cx)
    }

    fn settings_of(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Settings {
        view.read_with(cx, |v, _| v.settings)
    }

    fn settings_open(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> bool {
        view.read_with(cx, |v, _| v.settings_open)
    }

    /// The colours the window is drawn in, the line and the messages too.
    fn palette(cx: &mut VisualTestContext) -> Palette {
        cx.run_until_parked();
        cx.update(|window, cx| Palette::now(window, cx))
    }

    #[gpui::test]
    fn the_settings_change_the_window_at_once_and_are_kept(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = opened_kept(&yard, &file, cx);
        // No file yet: today's look, and nothing is written for looking.
        let today = Settings {
            text: 14,
            look: Look::Auto,
        };
        assert_eq!(settings_of(&view, cx), today);
        let desktop = cx.update(|window, _| window.appearance());
        assert_eq!(palette(cx), Palette::of(Look::Auto, desktop));
        assert_eq!(cx.update(|window, _| window.rem_size()), px(16.));
        assert!(!file.exists());

        // One key shows the settings where the conversation was. What is
        // typed then is not for the line, which is out of view.
        cx.simulate_input("hal");
        cx.simulate_keystrokes("secondary-,");
        assert!(settings_open(&view, cx));
        assert!(drawn(cx, "text size"));
        assert!(drawn(cx, "look auto"));
        cx.simulate_input("x");
        assert_eq!(typed(&view, cx), "hal");

        // Larger and smaller by a key each: the whole window at once, by
        // its rem, and the file has it.
        cx.simulate_keystrokes("secondary-= secondary-=");
        assert_eq!(settings_of(&view, cx).text, 16);
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, _| window.rem_size()),
            px(16. * 16. / 14.)
        );
        assert_eq!(Settings::read(&file).text, 16);
        cx.simulate_keystrokes("secondary--");
        assert_eq!(settings_of(&view, cx).text, 15);
        cx.simulate_keystrokes("secondary-0");
        assert_eq!(settings_of(&view, cx).text, 14);
        assert_eq!(Settings::read(&file), today);
        // And by the buttons.
        click_on(cx, "text larger");
        click_on(cx, "text larger");
        click_on(cx, "text smaller");
        assert_eq!(settings_of(&view, cx).text, 15);
        click_on(cx, "text usual");
        assert_eq!(settings_of(&view, cx).text, 14);
        // The ends hold.
        for _ in 0..12 {
            cx.simulate_keystrokes("secondary-=");
        }
        assert_eq!(settings_of(&view, cx).text, settings::TEXT_MAX);
        for _ in 0..12 {
            cx.simulate_keystrokes("secondary--");
        }
        assert_eq!(settings_of(&view, cx).text, settings::TEXT_MIN);
        assert_eq!(Settings::read(&file).text, settings::TEXT_MIN);

        // The look: one key steps through them, a button chooses one, and
        // the window is drawn in it at once, whatever the desktop has.
        cx.simulate_keystrokes("secondary-shift-l");
        assert_eq!(settings_of(&view, cx).look, Look::Light);
        assert_eq!(palette(cx), Palette::of(Look::Light, desktop));
        cx.simulate_keystrokes("secondary-shift-l");
        assert_eq!(settings_of(&view, cx).look, Look::Dark);
        assert_eq!(
            palette(cx),
            Palette::of(Look::Dark, WindowAppearance::Light)
        );
        cx.simulate_keystrokes("secondary-shift-l");
        assert_eq!(settings_of(&view, cx).look, Look::Auto);
        click_on(cx, "look light");
        assert_eq!(settings_of(&view, cx).look, Look::Light);
        click_on(cx, "look dark");
        assert_eq!(
            palette(cx),
            Palette::of(Look::Dark, WindowAppearance::Light)
        );
        let set = Settings {
            text: settings::TEXT_MIN,
            look: Look::Dark,
        };
        assert_eq!(Settings::read(&file), set);
        assert_eq!(view.read_with(cx, |v, _| v.unkept.clone()), None);

        // Esc goes back to the conversation, and the line has the keys.
        cx.simulate_keystrokes("escape");
        assert!(!settings_open(&view, cx));
        cx.simulate_input("t");
        assert_eq!(typed(&view, cx), "halt");
        // The control in the head opens them and shuts them; so does the
        // back button, and a row that is opened.
        click_on(cx, "settings");
        assert!(settings_open(&view, cx));
        click_on(cx, "settings");
        assert!(!settings_open(&view, cx));
        click_on(cx, "settings");
        click_on(cx, "settings back");
        assert!(!settings_open(&view, cx));
        cx.simulate_keystrokes("secondary-,");
        cx.simulate_keystrokes("tab");
        assert!(!settings_open(&view, cx));
        // The keys of their own work with the settings shut too.
        cx.simulate_keystrokes("secondary-=");
        assert_eq!(settings_of(&view, cx).text, settings::TEXT_MIN + 1);

        // A restart: the next window is as this one was left.
        let left = settings_of(&view, cx);
        let (again, cx) = opened_kept(&yard, &file, cx);
        assert_eq!(settings_of(&again, cx), left);
        assert_eq!(left.look, Look::Dark);
        assert_eq!(
            palette(cx),
            Palette::of(Look::Dark, WindowAppearance::Light)
        );
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(cx.update(|window, _| window.rem_size()), left.rem());
    }

    /// A file that cannot be written keeps nothing, and says so; the window
    /// has the change all the same.
    #[gpui::test]
    fn a_change_that_cannot_be_kept_is_said_and_still_drawn(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let in_the_way = dir.path().join("footplate");
        std::fs::write(&in_the_way, "a file where the directory would be").unwrap();
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = opened_kept(&yard, &in_the_way.join("settings.json"), cx);
        assert_eq!(settings_of(&view, cx), Settings::default());
        cx.simulate_keystrokes("secondary-=");
        assert_eq!(settings_of(&view, cx).text, 15);
        assert!(view.read_with(cx, |v, _| v.unkept.is_some()));
    }

    /// With the yard away there is no left column: the settings are reached
    /// by the same key, and by a control under the words.
    #[gpui::test]
    fn the_settings_are_reached_while_the_yard_is_away(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yard.sock");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(Client::new(&socket), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "that there is no yard", |cx| {
            view.read_with(cx, |v, _| {
                v.down
                    .as_deref()
                    .is_some_and(|why| why.starts_with("No yard"))
            })
        });
        cx.simulate_keystrokes("secondary-,");
        assert!(settings_open(&view, cx));
        assert!(drawn(cx, "look auto"));
        cx.simulate_keystrokes("secondary-shift-l secondary-=");
        assert_eq!(
            settings_of(&view, cx),
            Settings {
                text: 15,
                look: Look::Light
            }
        );
        cx.simulate_keystrokes("escape");
        assert!(!settings_open(&view, cx));
        click_on(cx, "settings");
        assert!(settings_open(&view, cx));
        click_on(cx, "settings back");
        assert!(!settings_open(&view, cx));
    }

    /// The layout at the smallest and the largest text: the rows of the
    /// left column stand under one another, a long head still ends before
    /// the bead's id (which is why it ends in an ellipsis, footplate-eryf),
    /// and the column, its rows and the logo go with the text.
    #[gpui::test]
    fn the_layout_holds_at_the_smallest_and_the_largest_text(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [
                {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
                 "ask": 3, "question": "A deliberately long question that must be cut before the id", "asker": "yardmaster",
                 "since": "2026-10-10T10:00:00Z"},
                {"bead": "bead-b", "bead_title": "Short", "topic": "topic:linear-out",
                 "ask": 4, "question": "Name it?", "asker": "yardmaster",
                 "since": "2026-10-10T11:00:00Z"}
            ], "decided": []}})
            .to_string(),
        );
        yard.answer("Bead", &bead_three());
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the decisions", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().is_some_and(|m| m.decisions_count() == 2)
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));

        let mut rows = Vec::new();
        for text in [settings::TEXT_MIN, settings::TEXT, settings::TEXT_MAX] {
            view.update(cx, |v, cx| {
                let set = Settings { text, ..v.settings };
                v.set(set, cx)
            });
            let rem = settings_of(&view, cx).rem();
            let first = bounds_of(cx, "decision row bead-a");
            let second = bounds_of(cx, "decision row bead-b");
            let long = bounds_of(cx, "decision head bead-a");
            let short = bounds_of(cx, "decision head bead-b");
            let id = bounds_of(cx, "decision id bead-a");
            assert!(
                first.bottom() <= second.top(),
                "at {text} px a row overlaps the next: {first:?} {second:?}"
            );
            assert!(
                first.top() <= long.top() && long.bottom() <= first.bottom(),
                "at {text} px the head leaves its row: {long:?} {first:?}"
            );
            assert!(
                long.right() < id.left() && id.right() <= first.right(),
                "at {text} px the long head touches the id: {long:?} {id:?}"
            );
            assert_eq!(
                short.size.width, long.size.width,
                "at {text} px the head's column hugs its content"
            );
            // The logo is two and a half rems a side: 40 pixels as a rule.
            let logo = bounds_of(cx, "logo");
            let side = rem * 2.5;
            assert!(
                (logo.size.width - side).abs() < px(1.) && (logo.size.height - side).abs() < px(1.),
                "at {text} px the logo is {logo:?}, not {side:?} a side"
            );
            let control = bounds_of(cx, "settings");
            assert!(
                logo.right() < control.left() && control.right() <= first.right() + rem,
                "at {text} px the head of the column does not hold: {logo:?} {control:?}"
            );
            rows.push((first.size.width, first.size.height, long.size.height));
        }
        // Smaller and larger than the usual in every measure.
        assert!(rows[0].0 < rows[1].0 && rows[1].0 < rows[2].0, "{rows:?}");
        assert!(rows[0].1 < rows[1].1 && rows[1].1 < rows[2].1, "{rows:?}");
        assert!(rows[0].2 < rows[1].2 && rows[1].2 < rows[2].2, "{rows:?}");
        let logo = bounds_of(cx, "logo");
        assert!(logo.size.width > px(40.));
    }
}
