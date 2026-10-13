//! The window: left, whom there is to talk with; right, the open
//! conversation and the line to write in. What it shows is the model's
//! (model.rs); this file draws it and makes the calls the model asks for.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gpui::{
    Action, AnyWindowHandle, App, ClickEvent, ClipboardItem, Context, Div, Entity, ExternalPaths,
    FocusHandle, Focusable, FontWeight, Image, ImageFormat, KeyBinding, MouseButton, ObjectFit,
    Pixels, ScrollHandle, SharedString, Task, Window, actions, div, img, prelude::*, px, relative,
    rems,
};

use crate::api::{self, Address, BlobRef, Client};
use crate::draft::{self, Attachment, Drafts, Found};
use crate::input::{self, Emptied, Line, Pasted, Submitted};
use crate::model::{
    self, AnswerAction, DecidedDetail, DecidedRow, DecisionAsk, DecisionDetail, DecisionRow,
    Effect, Entry, Kind, Model, News, Row, Selected, Target, Top,
};
use crate::notify::{self, Burst, Permission};
use crate::select::{self, Selection};
use crate::session::{self, Update};
use crate::settings::{self, Notify, Settings};
use crate::theme::{Look, Palette};
use crate::yard::{self, Env};

/// The application's icon, from the 1024px master in assets/icon.png.
pub const ICON: &[u8] = include_bytes!("../assets/icon.png");

actions!(
    footplate,
    [
        Next,
        Previous,
        Dismiss,
        DismissAll,
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
        Approve,
        Decline,
        AskAbout,
        Expand,
        FocusWords,
        Leave,
        OpenSettings,
        Larger,
        Smaller,
        UsualSize,
        NextLook,
        SwitchYards,
        Yard1,
        Yard2,
        Yard3,
        Yard4,
        Yard5,
        Yard6,
        Yard7,
        Yard8,
        Yard9,
        JumpNewest,
        JumpOldest,
        PageUp,
        PageDown
    ]
);

const CONTEXT: &str = "Footplate";
/// Where no line is typed in: the yard row, and a decision with the window
/// focused, where a number key picks and `x` puts aside. With a
/// conversation open, or a decision with the line focused to type words in,
/// those keys are the line's text.
const NO_LINE_CONTEXT: &str = "FootplateNoLine";
/// Where the switcher's list is open: the arrows move in it, Enter opens
/// the yard they are on and esc leaves it, and no key is a card's.
const SWITCH_CONTEXT: &str = "FootplateSwitch";

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
/// here; they still work. `foreign` is a bead of another yard: words
/// alone go as no note there, so Enter promises nothing without a pick.
/// `writable` is an asker with a conversation: else no key asks about
/// this.
fn decision_keys(ask: &DecisionAsk, foreign: bool, writable: bool) -> String {
    let mut keys: Vec<String> = Vec::new();
    if ask.options.is_empty() {
        keys.push("words wanted".to_string());
    } else if ask.chosen != 0 {
        keys.push(format!("enter picks {}", ask.chosen));
    } else if ask.recommend != 0 {
        keys.push(format!("enter picks {}", ask.recommend));
    } else if !foreign {
        keys.push("enter sends words as note".to_string());
    }
    if !ask.options.is_empty() {
        keys.push(format!("1-{} pick", ask.options.len()));
        keys.push("↑↓ choose".to_string());
    }
    keys.push("i words".to_string());
    if ask.options.is_empty() {
        keys.push("enter sends the answer".to_string());
    }
    if writable {
        keys.push("a ask about this".to_string());
    }
    keys.join(" · ")
}

/// What the settings say over the list of yards: what a pairing gives that
/// a link between two yards does not.
const PAIRING_GIVES: &str = "A paired yard is called with this device's own token, so your picks and approvals there count as yours, where over a peer link you only see what the link carries and an approval never crosses.";

/// What a secret ask's card says under the yard's line of what is asked
/// for: where the value goes, in the words `yardr chat` has for it.
const SECRET_GOES: &str = "it goes to a file only the yard's server reads; no agent is shown it";

/// The keys of a secret ask's card, with the way for a person without the
/// field: the same answer in a terminal on the yard's machine.
fn secret_keys(bead: &str, ask: u64, writable: bool) -> String {
    let keys = format!(
        "i types it · enter sends it to the yard · esc leaves, and nothing is sent · or in a terminal: yardr bead answer {bead} {ask} --secret"
    );
    if writable {
        format!("{keys} · a ask about this")
    } else {
        keys
    }
}

/// The key that approves a run ask, as its button and its keys name it.
const APPROVE_KEY: &str = if cfg!(target_os = "macos") {
    "\u{2318}\u{21b5}"
} else {
    "ctrl+enter"
};

/// What a run ask's card says over the command, while it can be approved:
/// the words `yardr chat` has for it, led by the yard the command runs in
/// and with the user it runs as, who is the one the approval is called as.
/// With several yards in one window, the sentence over a command is where
/// it must be plain which of them runs it.
fn run_runs(yard: &str, user: &str) -> String {
    format!("in {yard}, when you approve, the yard runs this as {user}, and nothing else:")
}

/// The header of a card: the yard it is of first, then the bead and how
/// long it waits or which ask it is. No card is read without its yard.
fn card_head(yard: &str, bead: &str, rest: &str) -> String {
    format!("{yard} · {bead} · {rest}")
}

/// What the control that approves a run ask reads: the yard its command
/// runs in, and its key.
fn approve_label(yard: &str) -> String {
    format!("Approve in {yard} ({APPROVE_KEY})")
}

/// What it says over the command of one that takes no approval any more.
const RUN_PROPOSED: &str = "the command this ask proposed:";

/// What a run ask's card says in place of the control that approves, where
/// the yard does not have the call or does not serve it to this window.
const APPROVE_ELSEWHERE: &str =
    "this yard takes no approval from footplate. Approve in a terminal on the yard's machine:";

/// The keys of a run ask's card. Later, Decline and expand have their
/// buttons; Enter is named because it does nothing here.
fn run_keys(approves: bool, writable: bool) -> String {
    let mut keys: Vec<String> = Vec::new();
    if approves {
        keys.push(format!("{APPROVE_KEY} approves"));
        keys.push("enter alone approves nothing".to_string());
    }
    if writable {
        keys.push("a ask about this".to_string());
    }
    keys.join(" · ")
}

/// A command as a text that is drawn as one code block: whole, wrapped
/// where the window is narrower and never cut, with the control that copies
/// it exactly. The fence is longer than any run of backticks in the
/// command, so nothing in it ends the block.
fn fenced(command: &str) -> String {
    let (mut longest, mut run) = (0, 0);
    for c in command.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{command}\n{fence}")
}

/// The keys of a dealt with: nothing is chosen by default and Enter alone
/// sends nothing; a number then Enter replaces the pick. One taken back has
/// no keys: nothing is sent from its card.
fn decided_keys(detail: &DecidedDetail) -> String {
    if detail.state == "taken back" {
        return String::new();
    }
    if detail.options.is_empty() {
        return "i words · enter sends".to_string();
    }
    format!("1-{} change your pick", detail.options.len())
}

/// The button that sends a card, exactly as Enter does: `Give NAME` on a
/// secret ask, `Send` where words alone go, `Send: 1 · Move them now` with
/// a pick (`Change to: 2 · …` on a dealt with). Disabled while its field
/// is empty: drawn faint, and no click. A pick button is never disabled:
/// a pick alone sends. The card shows a pick button only once an option is
/// chosen: never a button that sends the recommendation unasked.
fn send_button(
    id: String,
    text: String,
    enabled: bool,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> impl IntoElement {
    let click = enabled.then(|| cx.listener(|this, _, _, cx| this.decide(cx)));
    card_button(id, text, palette, click)
}

/// The button that approves a run ask, exactly as its key does, which it
/// names with the yard that runs the command: `Approve in plantime (⌘↵)`.
/// Disabled while an approval is with the yard.
fn approve_button(
    id: String,
    yard: &str,
    enabled: bool,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> impl IntoElement {
    let click = enabled.then(|| cx.listener(|this, _, _, cx| this.approve(cx)));
    card_button(id, approve_label(yard), palette, click)
}

/// A card's one filled button, with what a click on it does. Without that
/// it is drawn faint, and takes no click.
fn card_button(
    id: String,
    text: String,
    palette: Palette,
    click: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let enabled = click.is_some();
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(|| id.clone())
        .px_3()
        .py_1()
        .rounded_md()
        .when(enabled, |el| {
            el.bg(palette.accent)
                .text_color(palette.on_accent)
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
        })
        .when(!enabled, |el| {
            el.bg(palette.side)
                .text_color(palette.faint)
                .border_1()
                .border_color(palette.line)
        })
        .when_some(click, |el, click| el.on_click(click))
        .child(text)
}

/// What the button on a secret ask's card says: `Give` with the name the
/// field takes now (`Give PUSHOVER_TOKEN`), where the ask names it.
fn give_label(secret: &api::Secret, typed: usize) -> String {
    match secret.env.get(typed) {
        Some(name) => format!("Give {name}"),
        None => "Give".to_string(),
    }
}

/// Whether the words of an open ordinary card want their own `Send` button:
/// they are the answer where there are no options, or a note where there
/// is no pick and none recommended. With a pick the pick's button sends
/// them; with a recommendation and no pick there is no button that would
/// send it unasked; on another yard's bead words alone go as no note.
fn words_send(ask: &DecisionAsk, foreign: bool) -> bool {
    if ask.secret.is_some() || ask.run.is_some() || ask.chosen != 0 {
        return false;
    }
    if ask.options.is_empty() {
        return true;
    }
    ask.recommend == 0 && !foreign
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
/// the line's words, `x` puts aside, `d` declines, `a` asks about this,
/// `e` expands.
pub fn bind_keys(cx: &mut App) {
    for context in [CONTEXT, NO_LINE_CONTEXT] {
        cx.bind_keys([
            KeyBinding::new("tab", Next, Some(context)),
            KeyBinding::new("shift-tab", Previous, Some(context)),
            // The line's own binding is the deeper one: with the focus in
            // the line it copies the line's selection, as before.
            KeyBinding::new("secondary-c", input::Copy, Some(context)),
            // The arrows choose even while words are typed: the line keeps
            // left and right for its cursor, and up and down while its
            // words have a row to go to.
            KeyBinding::new("up", ChoiceUp, Some(context)),
            KeyBinding::new("down", ChoiceDown, Some(context)),
            KeyBinding::new("escape", Leave, Some(context)),
            // The jump control's key works while words are typed: the
            // line has no binding for it.
            KeyBinding::new("secondary-end", JumpNewest, Some(context)),
            // Home and End with the modifier are the oldest loaded and
            // the newest: plain they are the line's caret keys, for the
            // row's ends.
            KeyBinding::new("secondary-home", JumpOldest, Some(context)),
            // A page up or down scrolls what is shown, with the focus
            // anywhere: the line has no binding for these keys.
            KeyBinding::new("pageup", PageUp, Some(context)),
            KeyBinding::new("pagedown", PageDown, Some(context)),
        ]);
    }
    // Where no line is typed in: one key marks one notice read and with
    // shift every one, and on a card the same keys pick, put aside, decline
    // and expand. With the line focused those keys are the line's text.
    cx.bind_keys([
        KeyBinding::new("x", Dismiss, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("shift-x", DismissAll, Some(NO_LINE_CONTEXT)),
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
        // A run ask is approved with a key no other card answers with, and
        // never with Enter alone: the habit of answering picks must not
        // run a command.
        KeyBinding::new("secondary-enter", Approve, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("d", Decline, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("a", AskAbout, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("e", Expand, Some(NO_LINE_CONTEXT)),
        KeyBinding::new("i", FocusWords, Some(NO_LINE_CONTEXT)),
    ]);
    cx.bind_keys([
        KeyBinding::new("up", ChoiceUp, Some(SWITCH_CONTEXT)),
        KeyBinding::new("down", ChoiceDown, Some(SWITCH_CONTEXT)),
        KeyBinding::new("enter", Decide, Some(SWITCH_CONTEXT)),
        KeyBinding::new("escape", Leave, Some(SWITCH_CONTEXT)),
    ]);
    // The switcher and a yard by its place in the list, from anywhere:
    // with the modifier, since a plain digit is a card's pick.
    cx.bind_keys([
        KeyBinding::new("secondary-y", SwitchYards, None),
        KeyBinding::new("secondary-1", Yard1, None),
        KeyBinding::new("secondary-2", Yard2, None),
        KeyBinding::new("secondary-3", Yard3, None),
        KeyBinding::new("secondary-4", Yard4, None),
        KeyBinding::new("secondary-5", Yard5, None),
        KeyBinding::new("secondary-6", Yard6, None),
        KeyBinding::new("secondary-7", Yard7, None),
        KeyBinding::new("secondary-8", Yard8, None),
        KeyBinding::new("secondary-9", Yard9, None),
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

/// A pairing's code as it was typed where the yard's address is known
/// already: what a pairing's QR code holds, `<address>#<code>`, may be
/// given for it, and is its code.
fn code_of(typed: &str) -> String {
    let typed = typed.trim();
    let code = typed.rsplit_once('#').map_or(typed, |(_, code)| code);
    code.trim().to_string()
}

/// A mail of the open conversation to keep in place while older mail is
/// put in above it: its place among the children of what scrolls, and how
/// far its top stood off the top of the view.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    item: usize,
    off: Pixels,
}

/// Where a yard stood when it was left for another: the row that was open
/// and the scroll of what it showed. Of a conversation and the yard row
/// that is how far above its newest the view stood, since a yard opened
/// again has the newest of a conversation first and the older above it;
/// of a card, how far below its top, with the cards that stood expanded:
/// folded again, a card is shorter than where it stood.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    row: Selected,
    off: Pixels,
    expanded: HashSet<(String, u64)>,
    decided_expanded: bool,
}

/// Why a yard that could not be opened is not reached, in a word or two:
/// its address or its token's file is refused, and nothing was asked.
const NOT_OPENED: &str = "cannot be opened";

/// Whether a yard that is away is asked again.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Away {
    /// Every few seconds.
    Trying,
    /// Not until another is chosen: the one that was could not be opened.
    Stopped,
    /// Not until the device is paired again: the yard does not let it in.
    Out,
}

/// What is typed in the settings for a yard: one to add, or a code for one
/// that is listed.
#[derive(Clone, Debug, PartialEq)]
enum Adding {
    /// A yard of this machine: its name and its socket.
    Near,
    /// A yard at an address: its name, its address and a pairing's code.
    Far,
    /// A pairing's code for the yard of this name, to pair its device
    /// again.
    Again(String),
}

/// What a yard has waiting: the switcher shows it for a yard that is not
/// open, as it stands now, since every yard is followed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Counts {
    mail: usize,
    decisions: usize,
    notices: usize,
}

/// What makes a yard entry the same yard while a call is away from the
/// window. Its display name alone may be reused after the entry is removed.
#[derive(Clone, Debug, PartialEq, Eq)]
enum YardIdentity {
    Listed(settings::Yard),
    Environment(String, Option<PathBuf>),
}

impl Counts {
    fn of(model: &Model) -> Counts {
        Counts {
            mail: model.rows().iter().map(|row| row.unread).sum(),
            decisions: model.decisions_count(),
            notices: model.yard_unread(),
        }
    }

    /// In words.
    fn line(&self) -> String {
        let some = |n: usize, one: &str, more: &str| match n {
            0 => None,
            1 => Some(format!("1 {one}")),
            n => Some(format!("{n} {more}")),
        };
        let parts: Vec<String> = [
            some(self.mail, "mail", "mail"),
            some(self.decisions, "decision", "decisions"),
            some(self.notices, "notice", "notices"),
        ]
        .into_iter()
        .flatten()
        .collect();
        match parts.is_empty() {
            true => "nothing waiting".to_string(),
            false => parts.join(", "),
        }
    }
}

/// A yard as the window follows it, and all that is of it and not of the
/// window: the open yard's is in the window's own fields of these names,
/// and that of each other yard of the settings is held here, going on with
/// its yard, until the window shows it.
struct Held {
    /// Which following this is, counted from the window's start: what the
    /// follower's words and the answers to its calls are of, wherever the
    /// yard is by the time they come.
    session: u64,
    /// The yard of the settings it was opened as. One listed otherwise by
    /// now is another yard, and is opened anew.
    entry: Option<settings::Yard>,
    client: Option<Client>,
    following: Option<Task<()>>,
    following_stop: Option<session::FollowControl>,
    /// No row of it is open while it is held ([`Model::leave`]).
    model: Option<Model>,
    down: Option<String>,
    away: Away,
    said: Option<api::Me>,
    approving: Option<(String, u64)>,
    approve_elsewhere: HashSet<(String, u64)>,
    ask_generation: u64,
    returns_read: Option<chrono::DateTime<chrono::Utc>>,
}

impl Drop for Held {
    /// A yard that is held no longer is followed no longer.
    fn drop(&mut self) {
        if let Some(stop) = self.following_stop.take() {
            stop.stop();
        }
    }
}

pub struct Footplate {
    /// The yard's client, or none while the yard that was chosen cannot be
    /// opened: `down` says why.
    client: Option<Client>,
    /// The name the window runs as, for this machine's yard.
    name: String,
    /// What the environment says of the yard, which wins over the settings.
    env: Env,
    /// The yard that is open, by the name the settings list it under, or
    /// by its address where the environment names it: whose the drafts,
    /// the ask words and the counts are.
    yard: String,
    /// Which opening of a yard this is, counted from the window's start.
    /// The answer to a call made in an earlier one is another yard's, or
    /// this one's before it was left, and changes nothing of what is
    /// typed or shown beside the model.
    opening: u64,
    /// Which following the open yard's is, of all the window started
    /// ([`Held::session`]), and how many it started.
    session: u64,
    sessions: u64,
    /// The yard of the settings the open one was opened as, or none of
    /// the one yard the environment names.
    entry: Option<settings::Yard>,
    /// The other yards of the settings, by name: each followed as the
    /// open one is, by a client and a follower of its own, so that what
    /// waits in it is known and told of, and a switch shows it as it
    /// stands. Nothing of one is shown or sent while it is here.
    held: HashMap<String, Held>,
    /// Hands on what the follower of the open yard learns.
    following: Option<Task<()>>,
    /// Stops that follower when the yard is opened anew or is no longer
    /// listed, even while its event stream has no new words to deliver.
    following_stop: Option<session::FollowControl>,
    /// What the yard has, once it was read. It is kept while the yard is
    /// away, for the conversation that was open.
    model: Option<Model>,
    /// Why the yard is not there, while it is not.
    down: Option<String>,
    away: Away,
    /// Of a yard at an address: who it says this device is.
    said: Option<api::Me>,
    line: Entity<Line>,
    conversation: ScrollHandle,
    /// How many mails arrived in the open conversation below its view
    /// while it stood scrolled up: what the jump control carries.
    new_below: usize,
    /// The conversation was told to its newest and the layout has not
    /// put it there yet: the jump control stays out of the frame before
    /// it, whose scroll offset is the old view's.
    bottom_pending: bool,
    /// Older mail was put in above the view: the mail to keep where it
    /// stood, and whether the layout has put it at the top of the view
    /// yet. See [`Footplate::keep_place`].
    anchor: Option<(Anchor, bool)>,
    /// Where the open conversation stood when its older mail was last
    /// asked for: the conversation, its oldest mail and the scroll offset.
    /// Standing there still asks for nothing again, so a page the yard
    /// did not answer is not asked for with every frame.
    older_at: Option<(String, u64, Pixels)>,
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
    /// What is attached and not sent, for each conversation of each yard,
    /// and the text typed in those that are not open; the open one's is in
    /// the line. They speak of the open yard alone.
    drafts: Drafts,
    /// The words typed for each ask of the open yard that is not the open
    /// one, by bead and ask; the open one's are in the line.
    ask_words: HashMap<(String, u64), String>,
    /// The same of the yards that are not open, by yard: they are the
    /// open yard's again when theirs is opened, and no call of another
    /// yard can name them.
    kept_words: HashMap<String, HashMap<(String, u64), String>>,
    /// Why each yard that is not reached is not, in a word or two, by
    /// yard: what its row in the switcher says, of the open one and of
    /// every other. A yard that answers is in it no longer.
    unreached: HashMap<String, &'static str>,
    /// Where each yard stood when it was last left, by yard.
    places: HashMap<String, Place>,
    /// The place the open yard is to be put back to, which it is not
    /// yet: the yard has not answered for what the row shows, or the
    /// layout has not had it. Whether it was laid out as long as it now
    /// is. See [`Footplate::come_back`].
    returning: Option<(Place, bool)>,
    /// What there was to say while a yard was being opened and nothing
    /// showed to say it in: it is said when the yard shows.
    late: Option<String>,
    /// The switcher's list is shown under the head: the row of it the
    /// keys are on.
    switching: Option<usize>,
    /// The ask decline was pressed on with an empty line: the next Enter
    /// sends the typed words as the decline, and esc leaves it.
    declining: Option<(String, u64)>,
    /// The field of a secret ask's card, which shows marks. It is another
    /// than the line: what is typed here is in no draft and with no ask's
    /// words, and it is gone when the card is left.
    secret: Entity<Line>,
    /// The values typed so far for an ask of several names, in the order
    /// of its names: they go together, with the last.
    secret_typed: Vec<String>,
    /// The ask the field and those values are for.
    secret_for: Option<(String, u64)>,
    /// The run ask whose approval is with the yard and not answered yet:
    /// no second one goes after it.
    approving: Option<(String, u64)>,
    /// The run asks this yard took no approval of from this window, having
    /// no such call for it: their cards say where to approve instead.
    approve_elsewhere: HashSet<(String, u64)>,
    /// The attachments that are images, by id, as they are drawn.
    thumbs: HashMap<u64, Arc<Image>>,
    focus: FocusHandle,
    /// The window this is drawn in: a change that comes with no key or
    /// click of its own, as the yard's answer does, moves the focus by it.
    handle: AnyWindowHandle,
    icon: Arc<Image>,
    /// Which read of the queue a put-aside return waits for: an older
    /// timer does nothing when the queue was read since.
    ask_generation: u64,
    /// The moment up to which the queue was read for put-aside returns: a
    /// return at or before it does not read the queue again.
    returns_read: Option<chrono::DateTime<chrono::Utc>>,
    /// The text size and the look, as they are drawn.
    settings: Settings,
    /// The file they are kept in, or none: then they last as long as the
    /// window.
    kept: Option<PathBuf>,
    /// Why the last change was not kept, if it was not.
    unkept: Option<String>,
    /// The settings are shown where the conversation is.
    settings_open: bool,
    /// A yard in the settings, to add or to pair again: its name, its
    /// socket or its address and a pairing's code as they are typed, which
    /// of them show, whether a code is on its way to the yard, and what
    /// came of the last.
    yard_name: Entity<Line>,
    socket: Entity<Line>,
    address: Entity<Line>,
    code: Entity<Line>,
    adding: Option<Adding>,
    pairing: bool,
    yard_said: Option<String>,
    /// The system's notifications, and what it answered when it was asked
    /// whether footplate may notify: it is asked when the settings switch
    /// them on, and at the start when they are on.
    system: Box<dyn notify::System>,
    permission: Option<Permission>,
    /// Hands on the rows of the notifications a person activates, for as
    /// long as it is held here.
    _heeding: Task<()>,
    /// What arrived, in whichever yard, and is told of when the burst it
    /// came in is over.
    burst: Burst,
    /// A settings change invalidates timers for any batch already waiting.
    burst_generation: u64,
    /// Whether the window is the one in front.
    front: bool,
}

impl Footplate {
    /// The window on the yard behind `client`, for the user the yard knows
    /// `name` as. It follows the yard from here on.
    #[cfg(test)]
    pub fn new(client: Client, name: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::bare(Env::default(), name, window, cx);
        this.follow(settings::HERE.to_string(), Ok(client), cx);
        this
    }

    /// The window on the yard the environment names, else the one the
    /// settings kept in `file` had open last: one of this machine, for the
    /// user the yard knows `name` as, or one at an address, for the user
    /// its token is of. A yard that cannot be opened is said why, and the settings
    /// are there to choose again.
    pub fn opened(
        env: Env,
        name: String,
        file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::bare(env, name, window, cx).kept_in(file, cx);
        this.reopen(cx);
        this
    }

    /// The window before it has a yard.
    fn bare(env: Env, name: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let secret = cx.new(Line::masked);
        cx.subscribe(&secret, |this, _, _: &Submitted, cx| this.give(cx))
            .detach();
        // The card's Give and Send buttons are disabled while their field
        // is empty: the window is drawn again with what is typed.
        cx.observe(&line, |_, _, cx| cx.notify()).detach();
        cx.observe(&secret, |_, _, cx| cx.notify()).detach();
        // Enter in a field of a yard adds it, or sends the pairing's code.
        let field = |hint: &'static str, cx: &mut Context<Self>| {
            let field = cx.new(|cx| {
                let mut field = Line::new(cx);
                field.set_placeholder(hint, cx);
                field
            });
            cx.subscribe(&field, |this, _, _: &Submitted, cx| this.submit_yard(cx))
                .detach();
            field
        };
        let yard_name = field("plantime", cx);
        let socket = field("/Users/plantime/.yardr/yard.sock", cx);
        let address = field("https://mac.example.net", cx);
        let code = field("K7QM-2XPD", cx);
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
        // So are the switcher's: another yard is opened from wherever the
        // focus is, and from a yard that is away.
        Self::on_app_action::<SwitchYards>(window, cx, Self::toggle_switcher);
        Self::on_app_action::<Yard1>(window, cx, |this, w, cx| this.switch_nth(0, w, cx));
        Self::on_app_action::<Yard2>(window, cx, |this, w, cx| this.switch_nth(1, w, cx));
        Self::on_app_action::<Yard3>(window, cx, |this, w, cx| this.switch_nth(2, w, cx));
        Self::on_app_action::<Yard4>(window, cx, |this, w, cx| this.switch_nth(3, w, cx));
        Self::on_app_action::<Yard5>(window, cx, |this, w, cx| this.switch_nth(4, w, cx));
        Self::on_app_action::<Yard6>(window, cx, |this, w, cx| this.switch_nth(5, w, cx));
        Self::on_app_action::<Yard7>(window, cx, |this, w, cx| this.switch_nth(6, w, cx));
        Self::on_app_action::<Yard8>(window, cx, |this, w, cx| this.switch_nth(7, w, cx));
        Self::on_app_action::<Yard9>(window, cx, |this, w, cx| this.switch_nth(8, w, cx));
        // What arrives on the open row of a window in front is read there,
        // and the system tells of the rest.
        cx.observe_window_activation(window, |this, window, _| {
            this.front = window.is_window_active()
        })
        .detach();
        let system = notify::system();
        let heeding = Self::heed(system.as_ref(), cx);

        Footplate {
            client: None,
            name,
            env,
            yard: String::new(),
            opening: 0,
            session: 0,
            sessions: 0,
            entry: None,
            held: HashMap::new(),
            following: None,
            following_stop: None,
            model: None,
            down: None,
            away: Away::Stopped,
            said: None,
            line,
            conversation: ScrollHandle::new(),
            new_below: 0,
            bottom_pending: false,
            anchor: None,
            older_at: None,
            back_position: None,
            decision: ScrollHandle::new(),
            selection,
            images: HashMap::new(),
            drafts: Drafts::default(),
            ask_words: HashMap::new(),
            kept_words: HashMap::new(),
            unreached: HashMap::new(),
            places: HashMap::new(),
            returning: None,
            late: None,
            switching: None,
            declining: None,
            secret,
            secret_typed: Vec::new(),
            secret_for: None,
            approving: None,
            approve_elsewhere: HashSet::new(),
            thumbs: HashMap::new(),
            focus: cx.focus_handle(),
            handle: window.window_handle(),
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec())),
            ask_generation: 0,
            returns_read: None,
            settings: Settings::default(),
            kept: None,
            unkept: None,
            settings_open: false,
            yard_name,
            socket,
            address,
            code,
            adding: None,
            pairing: false,
            yard_said: None,
            system,
            permission: None,
            _heeding: heeding,
            burst: Burst::default(),
            burst_generation: 0,
            front: window.is_window_active(),
        }
    }

    /// Brings the window forward on the row of each notification of
    /// `system` a person activates, in the yard it is a row of.
    fn heed(system: &dyn notify::System, cx: &mut Context<Self>) -> Task<()> {
        let activated = system.activated();
        cx.spawn(async move |this, cx| {
            while let Ok((yard, row)) = activated.recv().await {
                if this
                    .update(cx, |this, cx| this.come_forward(yard, row, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    /// The window with `system` for its notifications, in place of this
    /// system's.
    #[cfg(test)]
    fn notified_by(mut self, system: Box<dyn notify::System>, cx: &mut Context<Self>) -> Self {
        self._heeding = Self::heed(system.as_ref(), cx);
        self.system = system;
        self.permission = None;
        if self.settings.notifications.on {
            self.ask_permission(cx);
        }
        self
    }

    /// Asks the system whether footplate may notify, which on macOS asks
    /// the person the first time. What it answers stands in the settings
    /// when it is a refusal.
    fn ask_permission(&mut self, cx: &mut Context<Self>) {
        let answer = self.system.ask();
        cx.spawn(async move |this, cx| {
            if let Ok(permission) = answer.recv().await {
                this.update(cx, |this, cx| {
                    this.permission = Some(permission);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Takes what arrived from the open yard's model, and has the system
    /// tell of what the settings choose and the window does not show.
    fn news(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let news = model.take_news();
        let in_view = self.front && !self.settings_open;
        let yard = self.yard.clone();
        self.tell(&yard, news, in_view, cx);
    }

    /// Has the system tell of `news` that arrived in `yard`, where the
    /// settings choose it and the window does not show it: once the burst
    /// it came in is over, in one notification, which names its yard where
    /// there are several. `in_view` is whether the window is in front on
    /// that yard's rows.
    fn tell(&mut self, yard: &str, news: Vec<News>, in_view: bool, cx: &mut Context<Self>) {
        for news in news {
            let wanted = notify::wanted(&self.settings.notifications, &news, in_view);
            if wanted && self.burst.add(yard, news) {
                let generation = self.burst_generation;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(notify::BURST).await;
                    this.update(cx, |this, _| {
                        let several = this.env.yard.is_none() && this.settings.yards.len() > 1;
                        if this.burst_generation == generation
                            && let Some(telling) = this.burst.take(several)
                        {
                            this.system.show(&telling);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// A notification was activated: the window comes forward on its row,
    /// in the yard it told of, which is opened where another is open. A
    /// row that is gone since leaves open what the yard has open, and a
    /// yard that is no longer listed leaves the window as it is: a row of
    /// that name in the open yard is another, where what is typed would
    /// reach this one.
    fn come_forward(&mut self, yard: String, row: Selected, cx: &mut Context<Self>) {
        cx.activate(true);
        let (this, handle) = (cx.weak_entity(), self.handle);
        // Afterwards: the window is not to be had from here.
        cx.defer(move |cx| {
            // A window that is closed has nothing to bring forward.
            let _ = handle.update(cx, |_, window, cx| {
                window.activate_window();
                this.update(cx, |this, cx| {
                    if this.yard != yard {
                        this.switch_to(&yard, window, cx);
                    }
                    if this.yard == yard {
                        this.select(row, window, cx);
                    }
                })
            });
        });
    }

    /// Follows every yard of the settings, and shows the one they open:
    /// each that is not followed yet is from here on, one that is listed
    /// no longer or otherwise is followed no longer, and the rest go on as
    /// they are. The yard to show is opened anew where it could not be
    /// opened before. With a yard named by the environment there is that
    /// one alone, opened anew.
    fn reopen(&mut self, cx: &mut Context<Self>) {
        self.reopen_anew(None, cx);
    }

    /// As [`Footplate::reopen`], with the yard called `anew` opened anew
    /// whatever is followed of it: its device was paired again, and the
    /// token it was opened with is not the yard's any more.
    fn reopen_anew(&mut self, anew: Option<&str>, cx: &mut Context<Self>) {
        if let Some(named) = self.env.yard.clone() {
            let opening = self.env.open(&self.settings, self.kept.as_deref());
            return self.follow(named, opening, cx);
        }
        let listed = self.settings.yards.clone();
        let show = self.settings.opened().name.clone();
        // Whether what is followed of a yard is followed further: it is
        // listed as it was opened, it is not to be opened anew, and it is
        // not the yard to show without a client to show it by.
        let goes_on = |name: &str, entry: &Option<settings::Yard>, client: &Option<Client>| {
            listed
                .iter()
                .any(|yard| yard.name == name && Some(yard) == entry.as_ref())
                && anew != Some(name)
                && !(name == show && client.is_none())
        };
        self.held
            .retain(|name, held| goes_on(name, &held.entry, &held.client));
        let open_goes_on = goes_on(&self.yard, &self.entry, &self.client);
        for yard in &listed {
            let followed =
                (open_goes_on && yard.name == self.yard) || self.held.contains_key(&yard.name);
            if !followed {
                let chosen = self.settings.clone().opening(&yard.name);
                let opening = self.env.open(&chosen, self.kept.as_deref());
                let held = self.begin(&yard.name, Some(yard.clone()), opening, cx);
                self.held.insert(yard.name.clone(), held);
            }
        }
        if !(open_goes_on && self.yard == show) {
            self.show(&show, open_goes_on, cx);
        }
    }

    /// Follows `yard` behind the client from here on, as the open one, in
    /// place of what was followed under that name. The yard that was open
    /// is held aside, and goes on. Without a client the window says why
    /// there is none, and waits for another yard to be chosen.
    fn follow(&mut self, yard: String, opening: Result<Client, String>, cx: &mut Context<Self>) {
        self.held.remove(&yard);
        let keep = self.yard != yard;
        self.leave_yard(&yard, keep, cx);
        let held = self.begin(&yard, None, opening, cx);
        self.put(held);
    }

    /// Shows the yard called `yard`, which is held, as it stands: on the
    /// row it was left on, with what was typed there. The yard that was
    /// open is held aside in its turn where `keep` says so, and is
    /// followed no longer where not.
    fn show(&mut self, yard: &str, keep: bool, cx: &mut Context<Self>) {
        let Some(held) = self.held.remove(yard) else {
            return;
        };
        self.leave_yard(yard, keep, cx);
        self.put(held);
        self.arrive(cx);
    }

    /// Starts following a yard behind the client, by a follower of its
    /// own, and gives what is held of it from here on: `yard` is its name,
    /// and `entry` the yard of the settings it is. Without a client it
    /// says why there is none, and nothing is asked.
    fn begin(
        &mut self,
        yard: &str,
        entry: Option<settings::Yard>,
        opening: Result<Client, String>,
        cx: &mut Context<Self>,
    ) -> Held {
        self.sessions += 1;
        let session = self.sessions;
        let mut held = Held {
            session,
            entry,
            client: None,
            following: None,
            following_stop: None,
            model: None,
            down: None,
            away: Away::Stopped,
            said: None,
            approving: None,
            approve_elsewhere: HashSet::new(),
            ask_generation: 0,
            returns_read: None,
        };
        let client = match opening {
            Ok(client) => client,
            Err(why) => {
                held.down = Some(why);
                self.unreached.insert(yard.to_string(), NOT_OPENED);
                return held;
            }
        };
        // The yard is followed on a thread of its own, which blocks on the
        // socket; what it learns comes here, as this following's.
        let (tell, told) = async_channel::unbounded::<Update>();
        let stop = session::FollowControl::default();
        held.following_stop = Some(stop.clone());
        let follower = thread::Builder::new().name("yard".into()).spawn({
            let (client, name) = (client.clone(), self.name.clone());
            move || {
                session::follow_with_control(&client, &name, session::RETRY, stop, |update| {
                    tell.send_blocking(update).is_ok()
                })
            }
        });
        held.following = Some(cx.spawn(async move |this, cx| {
            while let Ok(update) = told.recv().await {
                let heard = this.update(cx, |this, cx| this.told_of(session, update, cx));
                if heard.is_err() {
                    break;
                }
            }
        }));
        held.client = Some(client);
        held.away = Away::Trying;
        held.down = Some(match follower {
            Ok(_) => "Looking for the yard.".to_string(),
            Err(err) => format!("footplate could not start following the yard: {err}"),
        });
        held
    }

    /// Leaves the open yard for the one called `to`. Nothing of it is
    /// shown from here on, since a name there is not the same one here;
    /// what was written in it and not sent is kept aside for it, with
    /// where it stood, and what was written for `to` before is its again.
    /// Nothing typed of a secret is kept for either. The yard itself is
    /// held aside, with no row open, and goes on being followed; where
    /// `keep` says not, it is followed no longer.
    fn leave_yard(&mut self, to: &str, keep: bool, cx: &mut Context<Self>) {
        // The line's text is the open row's of the yard that is left: it
        // stays with that, and is in the line again when the row is open.
        if let Some(from) = self.model.as_ref().and_then(|m| m.selected().cloned()) {
            // A yard left before it was put back where it stood has
            // nothing newer to say of that: its place stays the kept
            // one.
            if self.returning.is_none() {
                let place = self.place(from.clone());
                self.places.insert(self.yard.clone(), place);
            }
            self.keep_typed(&from, cx);
        }
        self.returning = None;
        let words = std::mem::take(&mut self.ask_words);
        if !words.is_empty() {
            self.kept_words.insert(self.yard.clone(), words);
        }
        self.ask_words = self.kept_words.remove(to).unwrap_or_default();
        self.drafts.turn_to(to);
        let mut was = Held {
            session: self.session,
            entry: self.entry.take(),
            client: self.client.take(),
            following: self.following.take(),
            following_stop: self.following_stop.take(),
            model: self.model.take(),
            down: self.down.take(),
            away: std::mem::replace(&mut self.away, Away::Stopped),
            said: self.said.take(),
            approving: self.approving.take(),
            approve_elsewhere: std::mem::take(&mut self.approve_elsewhere),
            ask_generation: self.ask_generation,
            returns_read: self.returns_read.take(),
        };
        let from = std::mem::replace(&mut self.yard, to.to_string());
        // The window before it had a yard has none to hold.
        if keep && !from.is_empty() {
            if let Some(model) = was.model.as_mut() {
                model.leave();
            }
            self.held.insert(from, was);
        }
        self.opening += 1;
        self.switching = None;
        self.images.clear();
        self.declining = None;
        self.back_position = None;
        // A paging request and its scroll position belonged to the yard
        // just left; a returning yard may need to ask for the same page.
        self.older_at = None;
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.forget_secret(cx);
        cx.notify();
    }

    /// What was held of a yard is the open yard's from here on.
    fn put(&mut self, mut held: Held) {
        self.session = held.session;
        self.entry = held.entry.take();
        self.client = held.client.take();
        self.following = held.following.take();
        self.following_stop = held.following_stop.take();
        self.model = held.model.take();
        self.down = held.down.take();
        self.away = held.away;
        self.said = held.said.take();
        self.approving = held.approving.take();
        self.approve_elsewhere = std::mem::take(&mut held.approve_elsewhere);
        self.ask_generation = held.ask_generation;
        self.returns_read = held.returns_read.take();
    }

    /// The yard that was held is shown: the model it had all along opens
    /// the row the yard was left on, where that is still there, and the
    /// line has what was typed in it. A yard that has not answered yet has
    /// none, and is shown when it does ([`Footplate::told`]).
    fn arrive(&mut self, cx: &mut Context<Self>) {
        let kept = self.places.get(&self.yard).cloned();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let effects = model.back_to(
            kept.as_ref().map(|place| place.row.clone()),
            kept.as_ref().is_some_and(|place| place.decided_expanded),
        );
        if let Some(place) = &kept {
            model.expand(&place.expanded);
        }
        if let Some(late) = self.late.take() {
            model.notice = late;
        }
        let to = model.selected().cloned();
        self.returning = kept
            .filter(|place| to.as_ref() == Some(&place.row))
            .map(|place| (place, false));
        let typed = self.typed_for(to.as_ref());
        self.line.update(cx, |line, cx| line.set_text(&typed, cx));
        self.new_below = 0;
        self.scroll_to_newest();
        // A card that is to be put back where it stood is not told to its
        // top after that.
        if self.returning.is_none() {
            self.decision.scroll_to_item(0);
        }
        self.shown(cx);
        // A yard that is away is asked for nothing: what the row wants is
        // asked for when the yard is read again.
        if self.down.is_none() {
            self.pictures(cx);
            self.run(effects, cx);
        }
        cx.notify();
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

    /// The window with what `file` keeps of the text size, the look and the
    /// yards, which is where a change of them is kept from here on. The
    /// yard that is open stays open: [`Footplate::opened`] opens the kept
    /// one.
    pub fn kept_in(mut self, file: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        if let Some(file) = &file {
            self.settings = Settings::read(file);
        }
        cx.set_global(self.settings.look);
        self.kept = file;
        if self.settings.notifications.on {
            self.ask_permission(cx);
        }
        self
    }

    /// Which yard the window shows, in words: the whole telling, as the
    /// settings say it. The head's line has its name.
    fn which_yard(&self) -> String {
        let opened = self.settings.opened();
        match (self.env.address(&self.settings), &self.said) {
            (Some(address), Some(me)) => {
                format!(
                    "the yard {} at {address}, as its device {}",
                    me.yard, me.said_by
                )
            }
            (Some(address), None) => format!("the yard at {address}"),
            (None, _) if opened.socket.is_empty() => "this machine's yard".to_string(),
            (None, _) => format!("the yard at the socket {}", opened.socket),
        }
    }

    /// The head's line: who the window is for, where the yard said, the
    /// open yard, and why it is not reached while it is not. It ends in
    /// the mark of the switcher where there are yards to switch between.
    fn head_line(&self, me: Option<&str>) -> String {
        let yard = self.which_yard_short();
        let why = self.unreached.get(&self.yard).copied();
        let mut line = [me, Some(yard.as_str()), why]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        if self.env.yard.is_none() {
            line.push_str(" ▾");
        }
        line
    }

    /// Which yard the head's line names, short: the open one by the name
    /// the switcher lists it under, or the whole telling of the one yard
    /// the environment names. The line has the column's width beside the
    /// logo, and the whole telling stands in the settings.
    fn which_yard_short(&self) -> String {
        match self.env.yard {
            Some(_) => self.which_yard(),
            None => self.yard.clone(),
        }
    }

    pub fn line(&self) -> &Entity<Line> {
        &self.line
    }

    /// What the follower of the following `session` learned: of the open
    /// yard, or of one that is held, which has it without showing it. Its
    /// mail, its asks and its notices are told of as the open yard's are,
    /// and nothing of it is seen. One that is followed no longer says
    /// nothing.
    fn told_of(&mut self, session: u64, update: Update, cx: &mut Context<Self>) {
        if session == self.session {
            return self.told(update, cx);
        }
        let Some((yard, held)) = self.held.iter_mut().find(|(_, h)| h.session == session) else {
            return;
        };
        let yard = yard.clone();
        match update {
            Update::Up(state) => {
                let mut state = *state;
                held.said = state.yard.take();
                let (mut model, effects) = match held.model.as_ref() {
                    Some(before) => before.resumed(state),
                    None => (Model::new_aside(state), Vec::new()),
                };
                let news = model.take_news();
                held.model = Some(model);
                held.down = None;
                self.unreached.remove(&yard);
                self.run_in(session, effects, cx);
                self.schedule_return_in(session, cx);
                // What arrived while the yard was away.
                self.tell(&yard, news, false, cx);
            }
            Update::Event(event) => self.change_in(session, cx, |m| m.event(&event)),
            Update::Down(why, short) => {
                held.down = Some(why);
                held.away = Away::Trying;
                self.unreached.insert(yard, short);
            }
            Update::Out(why) => {
                held.down = Some(why);
                held.away = Away::Out;
                self.unreached.insert(yard, session::PAIR_AGAIN);
            }
        }
        cx.notify();
    }

    /// What the thread that follows the open yard learned.
    fn told(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Up(state) => {
                let mut state = *state;
                self.said = state.yard.take();
                let open = self.model.as_ref().and_then(|m| m.selected().cloned());
                let fresh = self.model.is_none();
                // The yard was opened: on the row it was left on, where
                // that is still there.
                let kept = self.places.get(&self.yard).filter(|_| fresh).cloned();
                let (mut model, effects) = match self.model.as_ref() {
                    Some(before) => before.resumed(state),
                    None => Model::new_returning(
                        state,
                        kept.as_ref().map(|place| place.row.clone()),
                        kept.as_ref().is_some_and(|place| place.decided_expanded),
                    ),
                };
                if fresh {
                    if let Some(place) = &kept {
                        model.expand(&place.expanded);
                    }
                    self.returning = kept
                        .filter(|place| model.selected() == Some(&place.row))
                        .map(|place| (place, false));
                }
                self.model = Some(model);
                self.down = None;
                self.unreached.remove(&self.yard);
                if fresh {
                    // The yard was opened: the line has what was typed in
                    // the row that is open, before the yard was left.
                    if let (Some(model), Some(late)) = (self.model.as_mut(), self.late.take()) {
                        model.notice = late;
                    }
                    let to = self.model.as_ref().and_then(|m| m.selected().cloned());
                    let kept = self.typed_for(to.as_ref());
                    if !kept.is_empty() {
                        self.line.update(cx, |line, cx| {
                            let all = draft::rejoin(kept, line.text().to_string());
                            line.set_text(&all, cx);
                        });
                    }
                }
                self.turned(open, cx);
                self.new_below = 0;
                self.scroll_to_newest();
                // A card that is to be put back where it stood is not
                // told to its top after that.
                if self.returning.is_none() {
                    self.decision.scroll_to_item(0);
                }
                // What the yard did not give while it was away is asked for
                // again.
                self.images
                    .retain(|_, picture| matches!(picture, Picture::Here(_)));
                self.shown(cx);
                self.pictures(cx);
                self.run(effects, cx);
                self.schedule_return(cx);
                // What arrived while the yard was away.
                self.news(cx);
            }
            Update::Event(event) => self.change(cx, |m| m.event(&event)),
            Update::Down(why, short) => {
                self.down = Some(why);
                self.away = Away::Trying;
                self.unreached.insert(self.yard.clone(), short);
            }
            Update::Out(why) => {
                self.down = Some(why);
                self.away = Away::Out;
                self.unreached
                    .insert(self.yard.clone(), session::PAIR_AGAIN);
            }
        }
        cx.notify();
    }

    /// Changes the model of the following `session`, wherever its yard is
    /// by now: the open yard's as [`Footplate::change`] does, and that of
    /// a yard that is held with nothing shown, its calls made by its own
    /// client and what arrived in it told of as its. A yard followed no
    /// longer has none to change.
    fn change_in(
        &mut self,
        session: u64,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Model) -> Vec<Effect>,
    ) {
        if session == self.session {
            return self.change(cx, f);
        }
        let Some((yard, held)) = self.held.iter_mut().find(|(_, h)| h.session == session) else {
            return;
        };
        let Some(model) = held.model.as_mut() else {
            return;
        };
        let effects = f(model);
        let (yard, news) = (yard.clone(), model.take_news());
        self.run_in(session, effects, cx);
        self.tell(&yard, news, false, cx);
        cx.notify();
    }

    /// The model of the following `session`, of the open yard or of one
    /// that is held.
    fn model_in(&mut self, session: u64) -> Option<&mut Model> {
        if session == self.session {
            return self.model.as_mut();
        }
        let held = self.held.values_mut().find(|h| h.session == session)?;
        held.model.as_mut()
    }

    /// Changes the open yard's model and does what the change asks for. A
    /// conversation shows its newest, and the yard its notices; the open
    /// card never moves for mail or notices arriving.
    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Model) -> Vec<Effect>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let before_sel = model.selected().cloned();
        let before_position = matches!(&before_sel, Some(Selected::Conversation(_)))
            .then(|| self.conversation.offset());
        let before_span = model.span();
        let before_notices = model.notices().len() + model.yard_mail().len();
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
            // Another row is open than the one the yard was left on.
            self.returning = None;
            match &after_sel {
                Some(Selected::Decision(_)) | Some(Selected::Decided(_, _)) => {
                    self.decision.scroll_to_item(0);
                }
                _ if returning_position.is_some() => {}
                _ => {
                    self.back_position = None;
                    self.new_below = 0;
                    self.scroll_to_newest();
                }
            }
        } else {
            match &after_sel {
                Some(Selected::Conversation(_)) => {
                    // What is older than the oldest shown before was put in
                    // above it, what is newer than the newest below.
                    let entries = model.entries();
                    let (above, below) = match before_span {
                        Some((oldest, newest)) => (
                            entries.iter().filter(|e| e.seq < oldest).count(),
                            entries.iter().filter(|e| e.seq > newest).count(),
                        ),
                        None => (0, entries.len()),
                    };
                    if above + below > 0 {
                        if self.bottom_pending || self.at_bottom() {
                            self.scroll_to_newest();
                        } else {
                            // New mail below a view that stands scrolled
                            // up: the view does not move by itself, and
                            // the jump control carries how many arrived.
                            // Nor does it move for older mail above it.
                            if above > 0 {
                                self.keep_place(above);
                            }
                            self.new_below += below;
                        }
                    }
                }
                Some(Selected::Yard)
                    if model.notices().len() + model.yard_mail().len() != before_notices =>
                {
                    self.scroll_to_newest();
                }
                _ => {}
            }
        }
        if let Some(position) = returning_position {
            self.conversation.set_offset(position);
        }
        // A decision dealt with by key leaves the focus on the window, and
        // the yard's answer comes with no window to move it by: what is
        // typed next is for the conversation it led back to. Not under the
        // settings, where the window holds the keys.
        if matches!(
            (&before_sel, &after_sel),
            (
                Some(Selected::Decision(_) | Selected::Decided(_, _)),
                Some(Selected::Conversation(_)),
            )
        ) && !self.settings_open
        {
            let (handle, focus) = (self.handle, self.line.focus_handle(cx));
            // Afterwards: the window is not to be had while a key or click
            // that came through it is still on its way.
            cx.defer(move |cx| {
                // A window that is closed has no focus to move.
                let _ = handle.update(cx, |_, window, _| window.focus(&focus));
            });
        }
        self.turned(before_sel, cx);
        self.shown(cx);
        self.pictures(cx);
        self.run(effects, cx);
        self.news(cx);
        cx.notify();
    }

    /// Whether the conversation stands at its newest: its scroll offset
    /// is at the bottom, within a pixel.
    fn at_bottom(&self) -> bool {
        let offset = self.conversation.offset();
        let max = self.conversation.max_offset();
        offset.y + max.height <= px(1.)
    }

    /// Whether the jump control stands over the open conversation: one
    /// is open, the settings are not over it, and its view is above its
    /// newest.
    fn shows_jump(&self) -> bool {
        !self.bottom_pending
            && !self.settings_open
            && self
                .model
                .as_ref()
                .and_then(|m| m.selected_conversation())
                .is_some()
            && !self.at_bottom()
    }

    /// Puts the conversation at its newest on the next frame: the layout
    /// applies it, and the jump control stays out of the frame before
    /// it.
    fn scroll_to_newest(&mut self) {
        self.conversation.scroll_to_bottom();
        self.bottom_pending = true;
        self.anchor = None;
    }

    /// `above` mails were put in above the view of the open conversation:
    /// the mail at the top of the view stays where it stood. The scroll
    /// offset is counted from the top of the conversation, and how high
    /// the new mails are is known only once they are laid out. So the
    /// layout of the next frame puts that mail at the top of the view, and
    /// the frame after it moves the view by what the mail stood off the
    /// top ([`Footplate::reconcile_scroll`]).
    fn keep_place(&mut self, above: usize) {
        let anchor = match self.anchor.take() {
            // More was put in before the last was laid out.
            Some((kept, _)) => Anchor {
                item: kept.item + above,
                off: kept.off,
            },
            None => {
                // The first child is the line above the oldest mail, which
                // stays the first: at the very top the oldest mail is kept.
                let item = self.conversation.logical_scroll_top().0.max(1);
                let off = self
                    .conversation
                    .bounds_for_item(item)
                    .map_or(px(0.), |bounds| {
                        bounds.top() + self.conversation.offset().y
                            - self.conversation.bounds().top()
                    });
                Anchor {
                    item: item + above,
                    off,
                }
            }
        };
        self.conversation.scroll_to_top_of_item(anchor.item);
        self.anchor = Some((anchor, false));
    }

    /// The scroll handle moves outside any model change: the wheel moves
    /// it, and the layout applies a jump to the newest. What the window
    /// keeps of it is brought up to date whenever it draws, before it
    /// decides on the jump control.
    fn reconcile_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.anchor.take() {
            // The layout of the frame before put the kept mail at the top
            // of the view: now it goes back to where it stood.
            Some((anchor, true)) => {
                let mut at = self.conversation.offset();
                at.y = (at.y + anchor.off).min(px(0.));
                self.conversation.set_offset(at);
            }
            Some((anchor, false)) => {
                self.anchor = Some((anchor, true));
                window.request_animation_frame();
                return;
            }
            None => {}
        }
        self.come_back(window);
        if self.bottom_pending {
            self.bottom_pending = false;
            return;
        }
        if self.at_bottom() {
            self.new_below = 0;
        }
        self.reach_top(cx);
    }

    /// Where the open yard stands, on `row`: what is kept of it when
    /// another yard is opened.
    fn place(&self, row: Selected) -> Place {
        let off = match &row {
            // A jump to the newest that the layout has not applied yet
            // is where it stands all the same.
            Selected::Conversation(_) | Selected::Yard if self.bottom_pending => px(0.),
            Selected::Conversation(_) | Selected::Yard => {
                self.conversation.max_offset().height + self.conversation.offset().y
            }
            Selected::Decision(_) | Selected::Decided(_, _) => px(0.) - self.decision.offset().y,
        };
        let model = self.model.as_ref();
        Place {
            row,
            off: off.max(px(0.)),
            expanded: model.map(|m| m.expanded().clone()).unwrap_or_default(),
            decided_expanded: model.is_some_and(Model::decided_expanded),
        }
    }

    /// Puts what is shown where it stood when its yard was left for
    /// another. Not before the yard has answered for what the row shows:
    /// until then the row is shorter than it was, and the layout would
    /// hold the view within that. A conversation's place is counted from
    /// its newest, and how long it is the layout says: so the frame that
    /// first has it whole lays it out, and the one after moves the view.
    fn come_back(&mut self, window: &mut Window) {
        let Some((place, laid)) = self.returning.take() else {
            return;
        };
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let here = match &place.row {
            Selected::Conversation(_) | Selected::Yard => model.answered(),
            Selected::Decision(bead) | Selected::Decided(bead, _) => model.has_bead(bead),
        };
        // Under the settings nothing of the row is laid out.
        if !here || self.settings_open {
            self.returning = Some((place, false));
            return;
        }
        // A jump to the newest asked for since is applied by this frame's
        // layout, over any offset set here: the next one has the place.
        if !laid || self.bottom_pending {
            self.returning = Some((place, true));
            window.request_animation_frame();
            return;
        }
        if matches!(&place.row, Selected::Conversation(_))
            && place.off > self.conversation.max_offset().height
            && model.top() == Top::Older
        {
            // The saved position is beyond the newest page. Reach the
            // oldest loaded mail to fetch older pages, then try the saved
            // distance again after each page is laid out.
            let mut at = self.conversation.offset();
            at.y = px(0.);
            self.conversation.set_offset(at);
            self.returning = Some((place, true));
            return;
        }
        match place.row {
            Selected::Conversation(_) | Selected::Yard => {
                let mut at = self.conversation.offset();
                at.y = (place.off - self.conversation.max_offset().height).min(px(0.));
                self.conversation.set_offset(at);
            }
            // Where the card is shorter now, the layout stops at its end.
            Selected::Decision(_) | Selected::Decided(_, _) => {
                let mut at = self.decision.offset();
                at.y = px(0.) - place.off;
                self.decision.set_offset(at);
            }
        }
    }

    /// Asks for the older mail of the open conversation when its view
    /// stands within half its height of the top, and the yard has older:
    /// before the top is reached, so that reading upwards rarely waits
    /// there.
    fn reach_top(&mut self, cx: &mut Context<Self>) {
        if self.settings_open {
            return;
        }
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let (Some(name), Some((oldest, _)), Top::Older) =
            (model.selected_conversation(), model.span(), model.top())
        else {
            return;
        };
        let at = self.conversation.offset().y;
        if at + self.conversation.bounds().size.height / 2. < px(0.) {
            return;
        }
        let here = (name.to_string(), oldest, at);
        if self.older_at.as_ref() == Some(&here) {
            return;
        }
        self.older_at = Some(here);
        let effects = model.older();
        self.run(effects, cx);
    }

    /// Puts `shown` at its newest at once: the offset is set, not
    /// asked for on the next frame.
    fn to_newest(shown: &ScrollHandle) {
        let max = shown.max_offset();
        let mut at = shown.offset();
        at.y = px(0.) - max.height;
        shown.set_offset(at);
    }

    /// Jumps the open conversation to its newest message: the control
    /// and its key. The offset is set at once, so the control is gone in
    /// the frame that follows, and the line keeps the keys.
    fn jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .model
            .as_ref()
            .and_then(|m| m.selected_conversation())
            .is_none()
        {
            return;
        }
        Self::to_newest(&self.conversation);
        self.new_below = 0;
        // The reader chose a place since the yard was opened.
        self.returning = None;
        window.focus(&self.line.focus_handle(cx));
        cx.notify();
    }

    /// The scroll of what is shown: the open conversation's, the open
    /// card's, or the yard row's, which is the conversation's. None
    /// under the settings, where none of those is shown, and none with
    /// nothing open.
    fn scrolled(&self) -> Option<&ScrollHandle> {
        if self.settings_open {
            return None;
        }
        match self.model.as_ref()?.selected() {
            Some(Selected::Conversation(_) | Selected::Yard) => Some(&self.conversation),
            Some(Selected::Decision(_) | Selected::Decided(_, _)) => Some(&self.decision),
            None => None,
        }
    }

    /// Moves what is shown by a page: down towards the newest, or up
    /// towards the oldest. A page is the height of the view less two
    /// lines, so the eye keeps its place; at an end the view stops.
    fn page(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.scrolled() else {
            return;
        };
        let line = window.line_height();
        let step = (shown.bounds().size.height - line - line).max(px(0.));
        let max = shown.max_offset();
        let mut at = shown.offset();
        if down {
            at.y -= step;
        } else {
            at.y += step;
        }
        at.y = at.y.clamp(px(0.) - max.height, px(0.));
        shown.set_offset(at);
        // The reader chose a place since the yard was opened.
        self.returning = None;
        cx.notify();
    }

    /// Puts what is shown at its oldest: the first loaded mail, the
    /// card's top, the yard row's first notice. The focus stays where
    /// it is: unlike the jump to the newest, this is no return to
    /// writing.
    fn jump_oldest(&mut self, _: &JumpOldest, _: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.scrolled() else {
            return;
        };
        let mut at = shown.offset();
        at.y = px(0.);
        shown.set_offset(at);
        cx.notify();
    }

    /// The jump key: on a conversation the control and its key do the
    /// same thing; on the yard row or a card it puts the view at its
    /// newest too.
    fn jump_newest(&mut self, _: &JumpNewest, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .model
            .as_ref()
            .and_then(|m| m.selected_conversation())
            .is_some()
        {
            self.jump(window, cx);
        } else if let Some(shown) = self.scrolled() {
            Self::to_newest(shown);
            cx.notify();
        }
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
        self.keep_typed(&from, cx);
        let text = self.typed_for(to.as_ref());
        self.line.update(cx, |line, cx| line.set_text(&text, cx));
    }

    /// What is typed in the line stays with `from`, the row it was typed
    /// in: the conversation's draft, or the ask's words.
    fn keep_typed(&mut self, from: &Selected, cx: &mut Context<Self>) {
        let typed = self.line.read(cx).text().to_string();
        match from {
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
    }

    /// What was typed in the row `to`, for the line: the row is open.
    fn typed_for(&mut self, to: Option<&Selected>) -> String {
        match to {
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
        }
    }

    /// The line says whom it writes to, or what words on a card do.
    fn shown(&mut self, cx: &mut Context<Self>) {
        let to = match self.model.as_ref().and_then(|m| m.selected().cloned()) {
            // In the row of a user a name of a linked yard is trusted as,
            // that is the name when it wrote last.
            // Before all of such a row is read, who wrote last is not
            // known, and the line names nobody.
            Some(Selected::Conversation(name)) => {
                match self.model.as_ref().and_then(|m| m.target()) {
                    _ if self.model.as_ref().is_some_and(Model::reading) => {
                        format!("Reading {name}")
                    }
                    Some(Target::Peer { name, peer }) => format!("Write to {name}@{peer}"),
                    _ => format!("Write to {name}"),
                }
            }
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
                } else if self.current_foreign() {
                    // The yard refuses a plain note on another yard's
                    // bead: words go with a pick, and talking to the asker
                    // is "ask about this" below.
                    "Words with the pick".to_string()
                } else {
                    "Words with the pick, or alone as a note".to_string()
                }
            }
            Some(Selected::Decided(_, _)) => "Words with the pick".to_string(),
            _ => String::new(),
        };
        self.line
            .update(cx, |line, cx| line.set_placeholder(&to, cx));
        // What is typed of a secret is for the ask it was typed on, and is
        // kept for no other and for no return to this one.
        let ask = self.open_ask();
        if ask != self.secret_for {
            self.forget_secret(cx);
            self.secret_for = ask.clone();
        }
        // A secret ask's card draws no line for words, nor does the card
        // of an ask taken back. A focus left in
        // the line by the conversation before it would hold the keys where
        // nothing is drawn to take them: the window has them, and `i` or a
        // click types in the field. Never the field at once: what was being
        // typed for a conversation must not go on as a secret.
        // Nor does a run ask's, but for the words of a decline.
        let no_line = ask.as_ref().is_some_and(|open| {
            let (bead, ask) = open;
            let model = self.model.as_ref();
            model.is_some_and(|m| {
                m.secret(bead, *ask).is_some()
                    || m.taken_back(bead, *ask)
                    || (m.run(bead, *ask).is_some() && self.declining.as_ref() != Some(open))
            })
        });
        if no_line && !self.settings_open {
            let (handle, line, focus) =
                (self.handle, self.line.focus_handle(cx), self.focus.clone());
            // Afterwards: the window is not to be had while a key or click
            // that came through it is still on its way.
            cx.defer(move |cx| {
                // A window that is closed has no focus to move.
                let _ = handle.update(cx, |_, window, _| {
                    if line.is_focused(window) {
                        window.focus(&focus);
                    }
                });
            });
        }
        let wanted = self.giving().map(|secret| {
            let at = self.secret_typed.len();
            let name = secret.env.get(at).cloned().unwrap_or_default();
            let of = match secret.env.len() {
                0 | 1 => String::new(),
                n => format!(" ({} of {n})", at + 1),
            };
            format!("{name} for the source {}{of}", secret.source)
        });
        self.secret.update(cx, |field, cx| {
            field.set_placeholder(&wanted.unwrap_or_default(), cx)
        });
    }

    /// What the open card's field takes, when its ask is a secret ask that
    /// can be given.
    fn giving(&self) -> Option<api::Secret> {
        let (bead, ask) = self.open_ask()?;
        self.model.as_ref()?.giving(&bead, ask).cloned()
    }

    /// The command the control on the open card approves, when its ask is
    /// a run ask that can be approved.
    fn approvable(&self) -> Option<api::Run> {
        let (bead, ask) = self.open_ask()?;
        self.model.as_ref()?.approving(&bead, ask).cloned()
    }

    /// Whether the open card is a run ask's, open or dealt with.
    fn run_card(&self) -> bool {
        self.open_ask().is_some_and(|(bead, ask)| {
            let model = self.model.as_ref();
            model.is_some_and(|m| m.run(&bead, ask).is_some())
        })
    }

    /// Nothing typed of a secret stays in the window: not in the field, and
    /// not among the values of an ask of several names.
    fn forget_secret(&mut self, cx: &mut Context<Self>) {
        self.secret_typed.clear();
        self.secret.update(cx, |field, cx| field.set_text("", cx));
    }

    /// Whether the open card's bead is another yard's question.
    fn current_foreign(&self) -> bool {
        self.model
            .as_ref()
            .and_then(|m| m.current_ask())
            .is_some_and(|(bead, _)| model::foreign_bead(&bead))
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

    /// Reads the open yard's queue again when its next put-aside ask comes
    /// back ([`Footplate::schedule_return_in`]).
    fn schedule_return(&mut self, cx: &mut Context<Self>) {
        self.schedule_return_in(self.session, cx);
    }

    /// Reads the queue of the following `session` again when the next
    /// put-aside ask comes back, which arrives with no event: of a yard
    /// that is held as of the open one, since what waits in it is told of
    /// and counted. One timer waits at a time for a yard; a queue read
    /// since replaces it. The queue is read once for a return: a yard that
    /// still lists the ask as put aside after its time is not asked again
    /// for it, and what the stream brings says when it is back.
    fn schedule_return_in(&mut self, session: u64, cx: &mut Context<Self>) {
        let held = self.held.values_mut().find(|h| h.session == session);
        let (model, asked, read) = match held {
            _ if session == self.session => (
                self.model.as_ref(),
                &mut self.ask_generation,
                &mut self.returns_read,
            ),
            Some(held) => (
                held.model.as_ref(),
                &mut held.ask_generation,
                &mut held.returns_read,
            ),
            None => return,
        };
        *asked += 1;
        let generation = *asked;
        let Some(at) = model.and_then(|m| m.next_return(*read)) else {
            return;
        };
        let now = chrono::Utc::now();
        let wait = at.timestamp() - now.timestamp();
        if wait <= 0 {
            // Every return up to now is read by this one read.
            *read = Some(at.max(now));
            self.change_in(session, cx, |_| vec![Effect::Asks]);
            return;
        }
        let wait = std::time::Duration::from_secs(wait as u64);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                let held = this.held.values_mut().find(|h| h.session == session);
                let (asked, read) = match held {
                    _ if session == this.session => (this.ask_generation, &mut this.returns_read),
                    Some(held) => (held.ask_generation, &mut held.returns_read),
                    None => return,
                };
                if asked == generation {
                    *read = Some(at);
                    this.change_in(session, cx, |_| vec![Effect::Asks]);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Makes a call off the window's thread, as the user the window is for,
    /// and hands its answer back to the window. An answer that comes when
    /// another yard was opened since is dropped: it says nothing of what
    /// the window shows now.
    fn ask<R: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&Client, &str) -> R + Send + 'static,
        then: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let opening = self.opening;
        self.ask_through(cx, call, move |this, answer, cx| {
            if this.opening == opening {
                then(this, answer, cx)
            }
        });
    }

    /// As [`Footplate::ask`], with the answer handed back whichever yard is
    /// open by then: for a call that carries what a person wrote, which is
    /// put back where it was written when it did not go, and for one whose
    /// answer changes the yard's model, which is held when another is
    /// shown. The call is the yard's it was made in, since its client is
    /// taken here.
    fn ask_through<R: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&Client, &str) -> R + Send + 'static,
        then: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        self.ask_in(self.session, cx, call, then);
    }

    /// Makes a call of the following `session` off the window's thread, by
    /// that yard's client and as the user the yard has the window for, and
    /// hands its answer back to the window, whichever yard is open by
    /// then. A yard followed no longer is not asked.
    fn ask_in<R: Send + 'static>(
        &self,
        session: u64,
        cx: &mut Context<Self>,
        call: impl FnOnce(&Client, &str) -> R + Send + 'static,
        then: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let (client, model) = match self.held.values().find(|h| h.session == session) {
            _ if session == self.session => (self.client.as_ref(), self.model.as_ref()),
            Some(held) => (held.client.as_ref(), held.model.as_ref()),
            None => return,
        };
        let Some(client) = client.cloned() else {
            return;
        };
        let me = model.map(|m| m.me().to_string()).unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_executor()
                .spawn(async move { call(&client, &me) })
                .await;
            this.update(cx, |this, cx| then(this, answer, cx)).ok();
        })
        .detach();
    }

    /// Says `notice` in the yard of the following `session`: at once in
    /// the open one, and in one that is held when the window shows it.
    fn notice_in(&mut self, session: u64, notice: String, cx: &mut Context<Self>) {
        if session == self.session {
            return self.notice(notice, cx);
        }
        if let Some(model) = self.model_in(session) {
            model.notice = notice;
        }
    }

    fn notice(&mut self, notice: String, cx: &mut Context<Self>) {
        match self.model.as_mut() {
            Some(model) => model.notice = notice,
            // A yard that is being opened has nowhere to say it yet: what
            // did not go in the one before is said when this one shows.
            None if !notice.is_empty() => self.late = Some(notice),
            None => {}
        }
        cx.notify();
    }

    /// Does what a change of the open yard's model asks for.
    fn run(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        self.run_in(self.session, effects, cx);
    }

    /// Does what a change of the model of the following `session` asks
    /// for: each call by that yard's client, and its answer to that yard's
    /// model, wherever the yard is by then.
    fn run_in(&mut self, session: u64, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Read { name, after } => self.ask_in(
                    session,
                    cx,
                    {
                        let name = name.clone();
                        move |client, me| client.conversation(me, &name, after)
                    },
                    move |this, letters, cx| match letters {
                        Ok(letters) => {
                            this.change_in(session, cx, |m| m.conversation(&name, letters))
                        }
                        Err(err) => this.notice_in(
                            session,
                            format!("the conversation with {name} could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Page { name, before } => self.ask_in(
                    session,
                    cx,
                    {
                        let name = name.clone();
                        move |client, me| client.conversation_page(me, &name, model::PAGE, before)
                    },
                    move |this, page, cx| match page {
                        Ok(page) => this.change_in(session, cx, |m| {
                            m.page(&name, before, page.letters, page.more)
                        }),
                        Err(err) => {
                            if let Some(model) = this.model_in(session) {
                                model.page_failed(&name, &err.to_string());
                            }
                            cx.notify();
                        }
                    },
                ),
                Effect::Ack { through } => self.ask_in(
                    session,
                    cx,
                    move |client, me| client.ack_user_inbox(me, through),
                    move |this, acked, cx| {
                        this.change_in(session, cx, |m| {
                            match acked {
                                Ok(()) => m.acked(through),
                                Err(err) => m.ack_failed(&err.to_string()),
                            }
                            Vec::new()
                        })
                    },
                ),
                Effect::Crew => self.ask_in(
                    session,
                    cx,
                    |client, _| Ok::<_, api::Error>((client.head()?, client.crew()?)),
                    move |this, crew, cx| match crew {
                        Ok((head, crew)) => this.change_in(session, cx, |m| m.listed(head, crew)),
                        Err(err) => this.notice_in(
                            session,
                            format!("the crew could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Peers => self.ask_in(
                    session,
                    cx,
                    |client, _| client.peers(),
                    move |this, peers, cx| match peers {
                        Ok(peers) => this.change_in(session, cx, |m| m.linked(&peers)),
                        Err(err) => this.notice_in(
                            session,
                            format!("the linked yards could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Users => self.ask_in(
                    session,
                    cx,
                    |client, _| client.users(),
                    move |this, users, cx| match users {
                        Ok(users) => this.change_in(session, cx, |m| m.users(users)),
                        Err(err) => this.notice_in(
                            session,
                            format!("the users could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Notifications => self.ask_in(
                    session,
                    cx,
                    |client, me| client.user_notifications(me, 0),
                    move |this, notifications, cx| match notifications {
                        Ok(notifications) => this.change_in(session, cx, |m| {
                            m.notifications(notifications.notifications);
                            Vec::new()
                        }),
                        Err(err) => this.notice_in(
                            session,
                            format!("the notices could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::AckNotifications { through } => self.ask_in(
                    session,
                    cx,
                    move |client, me| client.ack_user_notifications(me, through),
                    move |this, acked, cx| {
                        this.change_in(session, cx, |m| {
                            match acked {
                                Ok(()) => m.notifications_acked(through),
                                Err(err) => m.notifications_ack_failed(&err.to_string()),
                            }
                            Vec::new()
                        })
                    },
                ),
                Effect::Asks => self.ask_in(
                    session,
                    cx,
                    |client, me| client.user_asks(me),
                    move |this, asks, cx| match asks {
                        Ok(asks) => {
                            this.change_in(session, cx, |m| m.asks(asks));
                            this.schedule_return_in(session, cx);
                        }
                        Err(err) => this.notice_in(
                            session,
                            format!("the decisions could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Bead { bead } => self.ask_in(
                    session,
                    cx,
                    {
                        let bead = bead.clone();
                        move |client, _| client.bead(&bead)
                    },
                    move |this, answered, cx| match answered {
                        Ok(bead) => this.change_in(session, cx, |m| {
                            m.bead_loaded(bead);
                            Vec::new()
                        }),
                        Err(err) => this.change_in(session, cx, |m| {
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

    fn yard_identity(&self, name: &str) -> Option<YardIdentity> {
        match &self.env.yard {
            Some(address) => Some(YardIdentity::Environment(
                address.clone(),
                self.env.token_file.clone(),
            )),
            None => self.settings.yard(name).cloned().map(YardIdentity::Listed),
        }
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
        // Who wrote last is not known before the conversation is read, and
        // a guess would be the user in this yard: the line is kept.
        if self.model.as_ref().is_some_and(Model::reading) {
            return self.notice(
                format!(
                    "the conversation with {to} is still being read, and with it where your answer goes: enter sends once it shows"
                ),
                cx,
            );
        }
        // The draft is empty while the mail is on its way, so that Enter
        // again does not send it again.
        let images = self.drafts.take(&to);
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.notice(String::new(), cx);
        // Whom it was written to, which in the row of a user of this yard
        // may be a name trusted as them: a refusal names that one.
        let at = match &target {
            Target::Crew(name) => name.clone(),
            Target::Peer { name, peer } => format!("{name}@{peer}"),
        };
        let (yard, opening, session) = (self.yard.clone(), self.opening, self.session);
        let identity = self.yard_identity(&yard);
        self.ask_through(
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
                    // Where another yard was opened since, nothing of
                    // this one's shows them.
                    for (id, blob) in sent {
                        if let Some(image) = this.thumbs.remove(&id)
                            && this.opening == opening
                        {
                            this.images.insert(blob.hash, Picture::Here(image));
                        }
                    }
                    // It went in the yard it was written in, whose model
                    // has it wherever the window is by now.
                    this.change_in(session, cx, |m| m.sent(&to))
                }
                Err(_) if this.yard_identity(&yard) != identity => this.notice(
                    format!(
                        "not sent to {at} in {yard}: that yard entry was removed or changed, so its draft was not restored"
                    ),
                    cx,
                ),
                Err(unsent) => this.unsent(&yard, &to, &at, unsent, cx),
            },
        );
    }

    /// Nothing of a draft is lost to a failed send: what did not go is back
    /// where it was, in the conversation `to` of `yard`, before what was
    /// typed and attached since, the attachment that kept it is marked,
    /// and the window says why `at`, whom it was written to, did not get
    /// it. It is written to nobody else instead, and where another yard is
    /// open by now it is in no line of that one: it waits in its own.
    fn unsent(&mut self, yard: &str, to: &str, at: &str, unsent: Unsent, cx: &mut Context<Self>) {
        let open = self.drafts.turn_to(yard);
        let here = open == yard;
        self.drafts.restore(to, unsent.images);
        if let Some(id) = unsent.at {
            self.drafts.refuse(id, &unsent.why);
        }
        let shown = self.model.as_ref().and_then(|m| m.selected_conversation()) == Some(to);
        if here && shown {
            self.line.update(cx, |line, cx| {
                let back = draft::rejoin(unsent.text, line.text().to_string());
                line.set_text(&back, cx);
            });
        } else {
            self.drafts.unsend(to, unsent.text);
        }
        self.drafts.turn_to(&open);
        let whom = match here {
            true => at.to_string(),
            false => format!("{at} in {yard}"),
        };
        self.notice(format!("not sent to {whom}: {}", unsent.why), cx);
    }

    /// Words for the ask `at` of `yard` that the yard did not take are
    /// back with what was typed since: in the line while that yard is
    /// still the open one, as it was opened then, and else with the ask
    /// they were typed for, in its own yard. `what` did not happen, and
    /// `err` is why.
    fn words_back(
        &mut self,
        (yard, opening): (&str, u64),
        at: (String, u64),
        words: String,
        (what, err): (&str, String),
        cx: &mut Context<Self>,
    ) {
        if opening == self.opening {
            self.line.update(cx, |line, cx| {
                let back = draft::rejoin(words, line.text().to_string());
                line.set_text(&back, cx);
            });
            return self.notice(format!("{what}: {err}"), cx);
        }
        let kept = match yard == self.yard {
            true => &mut self.ask_words,
            false => self.kept_words.entry(yard.to_string()).or_default(),
        };
        let typed = kept.remove(&at).unwrap_or_default();
        kept.insert(at, draft::rejoin(words, typed));
        self.notice(format!("{what} in {yard}: {err}"), cx);
    }

    /// The open card's ask: the current one of a decision, or the dealt
    /// with itself.
    fn card(&self) -> Option<(String, u64)> {
        // The settings stand where the card does: its keys are not for a
        // card that is not in view.
        if self.settings_open {
            return None;
        }
        self.open_ask()
    }

    /// The ask of the card that is open, in view or under the settings.
    fn open_ask(&self) -> Option<(String, u64)> {
        match self.model.as_ref()?.selected()? {
            Selected::Decision(_) => self.model.as_ref()?.current_ask(),
            Selected::Decided(bead, ask) => Some((bead.clone(), *ask)),
            _ => None,
        }
    }

    /// Enter in the field of a secret ask: the value goes to the yard in
    /// its own call and in nothing else. The field is emptied here, before
    /// the yard is asked and whatever it answers, and what the window says
    /// afterwards is the yard's words or footplate's, never the value. An
    /// ask of several names takes them one after the other and sends them
    /// together.
    fn give(&mut self, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        let Some(secret) = self.giving() else {
            return;
        };
        // What is pasted often ends in a line break, which nobody sees
        // among the marks and which the yard refuses at either end.
        let value = self
            .secret
            .update(cx, |field, cx| field.take_text(cx))
            .trim()
            .to_string();
        let Some(name) = secret.env.get(self.secret_typed.len()) else {
            return self.forget_secret(cx);
        };
        if value.is_empty() {
            return self.notice(format!("nothing is typed for {name}"), cx);
        }
        self.secret_typed.push(value);
        self.notice(String::new(), cx);
        if self.secret_typed.len() < secret.env.len() {
            return self.shown(cx);
        }
        let values: Vec<(String, String)> = secret
            .env
            .into_iter()
            .zip(std::mem::take(&mut self.secret_typed))
            .collect();
        self.shown(cx);
        let call_bead = bead.clone();
        // What the yard answers is of the yard it was given in, and is
        // said there, wherever the window is by then.
        let session = self.session;
        self.ask_through(
            cx,
            move |client, me| {
                client
                    .give_secret(&call_bead, ask, &values, me)
                    .map_err(|err| {
                        let why = model::not_given(&err, &call_bead, ask);
                        // The yard repeats no value in a refusal. Should
                        // anything on the way do so, it is not shown.
                        if values.iter().any(|(_, value)| why.contains(value.as_str())) {
                            "not given: the yard refused it".to_string()
                        } else {
                            why
                        }
                    })
            },
            move |this, given, cx| match given {
                Ok(given) => {
                    this.change_in(session, cx, |m| m.gave(&bead, ask, &given.given));
                    this.schedule_return_in(session, cx);
                    this.notice_in(session, model::given_line(&given), cx);
                }
                Err(why) => this.notice_in(session, why, cx),
            },
        );
    }

    /// Approves the open card's run ask: its key and its button. The call
    /// names the ask and nothing of the command, which the yard runs as it
    /// stored it. What the window says afterwards is the yard's: the
    /// command as it ran with what it printed, or the refusal whole. A call
    /// that ended without an answer is never said to be approved or not,
    /// and a yard without the call for this window has its card say where
    /// to approve.
    fn approve(&mut self, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        let at = (bead.clone(), ask);
        if self.approvable().is_none()
            || self.approve_elsewhere.contains(&at)
            || self.approving.is_some()
            || self.client.is_none()
        {
            return;
        }
        self.declining = None;
        self.approving = Some(at.clone());
        self.shown(cx);
        self.notice(String::new(), cx);
        // The approval is the yard's it was pressed in: what came of it is
        // that yard's, and is said there, wherever the window is by then.
        let session = self.session;
        self.ask_through(
            cx,
            move |client, me| client.approve_ask(&bead, ask, me),
            move |this, approved, cx| {
                let held = this.held.values_mut().find(|h| h.session == session);
                let (approving, elsewhere) = match held {
                    _ if session == this.session => {
                        (&mut this.approving, &mut this.approve_elsewhere)
                    }
                    Some(held) => (&mut held.approving, &mut held.approve_elsewhere),
                    None => return,
                };
                *approving = None;
                let (bead, ask) = at.clone();
                match approved {
                    Ok(approved) => {
                        let line = model::approved_line(&approved);
                        this.change_in(session, cx, |m| m.approved(&bead, ask, &approved));
                        this.schedule_return_in(session, cx);
                        this.notice_in(session, line, cx);
                    }
                    Err(err) if model::approves_elsewhere(&err) => {
                        elsewhere.insert(at);
                        this.notice_in(session, String::new(), cx);
                    }
                    Err(err) => this.notice_in(session, model::not_approved(&err), cx),
                }
            },
        );
    }

    /// Enter on a card: the pick with the line's words in the one answer,
    /// or the words alone. Enter with nothing chosen and a recommendation
    /// picks the recommended option.
    fn decide(&mut self, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        // A secret ask has its own field and its own call, and no words.
        if self.giving().is_some() {
            return self.give(cx);
        }
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
        let (yard, opening, session) = (self.yard.clone(), self.opening, self.session);
        let identity = self.yard_identity(&yard);
        self.ask_through(
            cx,
            move |client, me| client.answer_ask(&call_bead, ask, option, &call_words, me),
            move |this, answered, cx| match answered {
                // The yard it was answered in has it, wherever the window
                // is by now.
                Ok(()) => {
                    this.change_in(session, cx, |m| m.picked(&bead, ask, option, &words));
                    this.schedule_return_in(session, cx);
                }
                Err(err) => {
                    let said = ("not answered", err.to_string());
                    if this.yard_identity(&yard) != identity {
                        return this.notice(
                            format!(
                                "not answered in {yard}: that yard entry was removed or changed, so its words were not restored"
                            ),
                            cx,
                        );
                    }
                    this.words_back((&yard, opening), (bead, ask), words, said, cx);
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
        let (yard, opening) = (self.yard.clone(), self.opening);
        let identity = self.yard_identity(&yard);
        self.ask_through(
            cx,
            move |client, me| client.add_note(&call_bead, &call_words, me),
            move |this, noted, cx| match noted {
                Ok(()) => {
                    // The note arrives as a `noted` event, which reads the
                    // queue and the bead again.
                }
                Err(err) => {
                    let said = ("not noted", err.to_string());
                    if this.yard_identity(&yard) != identity {
                        return this.notice(
                            format!(
                                "not noted in {yard}: that yard entry was removed or changed, so its words were not restored"
                            ),
                            cx,
                        );
                    }
                    this.words_back((&yard, opening), (bead, ask), words, said, cx);
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
        let session = self.session;
        self.ask_through(
            cx,
            move |client, me| client.defer_ask(&call_bead, ask, me),
            move |this, deferred, cx| match deferred {
                Ok(()) => {
                    this.change_in(session, cx, |m| m.deferred(&bead, ask));
                    this.schedule_return_in(session, cx);
                }
                Err(err) => this.notice_in(session, format!("not put aside: {err}"), cx),
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
        let (yard, opening, session) = (self.yard.clone(), self.opening, self.session);
        let identity = self.yard_identity(&yard);
        self.ask_through(
            cx,
            move |client, me| client.decline_ask(&call_bead, ask, &call_words, me),
            move |this, declined, cx| match declined {
                Ok(()) => {
                    this.change_in(session, cx, |m| m.declined(&bead, ask, &words));
                    this.schedule_return_in(session, cx);
                }
                Err(err) => {
                    let said = ("not declined", err.to_string());
                    if this.yard_identity(&yard) != identity {
                        return this.notice(
                            format!(
                                "not declined in {yard}: that yard entry was removed or changed, so its words were not restored"
                            ),
                            cx,
                        );
                    }
                    this.words_back((&yard, opening), (bead, ask), words, said, cx);
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

    /// Marks every notice read, on the open yard row: shift-x and the
    /// button in the row's head. Elsewhere it does nothing.
    fn dismiss_all(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| m.dismiss_all());
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
        // In the switcher's list the arrows move among the yards, and stop
        // at its ends.
        if let Some(at) = self.switching {
            let last = self.settings.yards.len().saturating_sub(1);
            self.switching = Some(at.saturating_add_signed(by).min(last));
            return cx.notify();
        }
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
        // A secret ask is declined with no words: the yard takes none on
        // it, since words there could be the value.
        if self.giving().is_some() {
            self.forget_secret(cx);
            return self.send_decline(bead, ask, String::new(), cx);
        }
        let words = self.line.read(cx).text().trim().to_string();
        if !words.is_empty() {
            return self.send_decline(bead, ask, words, cx);
        }
        self.declining = Some((bead, ask));
        self.shown(cx);
        window.focus(&self.line.focus_handle(cx));
        cx.notify();
    }

    /// The `a` key asks about the open card: the conversation with its
    /// asker, with the card's reference in the draft.
    fn ask_about(&mut self, _: &AskAbout, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_about_now(window, cx);
    }

    /// Opens the conversation with the open card's asker, with the card's
    /// reference first in its draft: `re <bead> "<question>"`, then an
    /// empty line with the caret on it. Nothing is sent; what the draft
    /// held is kept after the reference, which is put once. The ask stays
    /// open among the decisions: its row is one step back. An asker with
    /// no conversation says so instead.
    fn ask_about_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            return;
        }
        let open = self.model.as_ref().and_then(|m| {
            let (bead, ask) = m.current_ask()?;
            let now = chrono::Utc::now();
            let card = m
                .decision_detail(&bead, &now)?
                .asks
                .into_iter()
                .find(|a| a.ask == ask)?;
            let why = m.unwritable(&card.asker);
            // One trusted as a user of this yard is written to in that
            // user's row.
            let with = m
                .conversation_with(&card.asker)
                .unwrap_or(&card.asker)
                .to_string();
            Some((bead, with, card.question, why))
        });
        let Some((bead, asker, question, why)) = open else {
            return;
        };
        if let Some(why) = why {
            return self.notice(why, cx);
        }
        let reference = draft::ask_reference(&bead, &question);
        let existing = self.drafts.take_text(&asker);
        let (text, caret) = draft::prepend_reference(&existing, &reference);
        self.drafts.put_text(&asker, text);
        self.select(Selected::Conversation(asker), window, cx);
        self.line.update(cx, |line, cx| line.set_cursor(caret, cx));
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
        let Some((bead, ask)) = self.card() else {
            return;
        };
        // A taken-back ask's card draws no field to type in.
        if self
            .model
            .as_ref()
            .is_some_and(|m| m.taken_back(&bead, ask))
        {
            return;
        }
        // A run ask's card has a line only for the words of a decline.
        if self.run_card() && self.declining.is_none() {
            return;
        }
        let field = match self.giving() {
            Some(_) => &self.secret,
            None => &self.line,
        };
        window.focus(&field.focus_handle(cx));
    }

    /// Esc leaves what it can: a decline asked for words, the words for
    /// the card's keys, the pick, then the words.
    fn leave(&mut self, _: &Leave, window: &mut Window, cx: &mut Context<Self>) {
        if self.switching.is_some() {
            return self.toggle_switcher(window, cx);
        }
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
        // Esc in a secret's field leaves it, and nothing of it stays.
        if self.secret.focus_handle(cx).is_focused(window) {
            self.forget_secret(cx);
            self.shown(cx);
            window.focus(&self.focus.clone());
            return;
        }
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
        // A card under the settings is not in view, and neither is its
        // field: nothing typed of a secret waits there.
        self.forget_secret(cx);
        self.shown(cx);
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
        if settings.notifications != self.settings.notifications {
            self.burst.clear();
            self.burst_generation = self.burst_generation.wrapping_add(1);
        }
        // The line and the messages take their colours from this.
        cx.set_global(settings.look);
        if let Some(file) = &self.kept {
            self.unkept = settings.write(file).err().map(|err| err.to_string());
        }
        let switched_on = settings.notifications.on && !self.settings.notifications.on;
        self.settings = settings;
        if switched_on {
            self.ask_permission(cx);
        }
        cx.notify();
    }

    /// The system tells of what arrives, or no longer.
    fn notify_on(&mut self, on: bool, cx: &mut Context<Self>) {
        self.set(
            Settings {
                notifications: Notify {
                    on,
                    ..self.settings.notifications
                },
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// `kind` is among what the system tells of, or no longer.
    fn notify_of(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let notifications = self.settings.notifications;
        self.set(
            Settings {
                notifications: notifications.with(kind, !notifications.chosen(kind)),
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// The text a step larger or smaller.
    fn resize(&mut self, by: i8, cx: &mut Context<Self>) {
        self.set(self.settings.clone().sized(by), cx);
    }

    fn usual_size(&mut self, cx: &mut Context<Self>) {
        self.set(
            Settings {
                text: settings::TEXT,
                ..self.settings.clone()
            },
            cx,
        );
    }

    fn look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.set(
            Settings {
                look,
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// The switcher's list under the head, or no longer: the yards the
    /// settings list, to open one of. While it shows, the window holds the
    /// keys, on the row of the open yard first. With one yard named by the
    /// environment there is nothing to switch between. A yard that is away
    /// has the list as any other: it is how the person leaves it.
    fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self.switching.take().is_some() {
            let conversation = self.model.as_ref().and_then(|m| m.selected_conversation());
            let focus = match conversation {
                // A yard that is away draws no line to type in.
                Some(_) if !self.settings_open && self.down.is_none() => self.line.focus_handle(cx),
                _ => self.focus.clone(),
            };
            return window.focus(&focus);
        }
        if self.env.yard.is_some() {
            return;
        }
        let yards = &self.settings.yards;
        let open = yards.iter().position(|yard| yard.name == self.yard);
        self.switching = Some(open.unwrap_or(0));
        window.focus(&self.focus.clone());
    }

    /// Opens the yard at place `n` of the settings' list, counted from
    /// nought: the switcher's row, and its key from anywhere.
    fn switch_nth(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.settings.yards.get(n).map(|yard| yard.name.clone()) {
            self.switch_to(&name, window, cx);
        }
    }

    /// Shows the yard called `name` in place of the open one, as it stands:
    /// it was followed all along. It is the one to open at the next start.
    /// The line has the focus, as when the window starts. The yard that is
    /// open already stays as it is, unless it could not be opened: that is
    /// tried again.
    fn switch_to(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.switching = None;
        cx.notify();
        if self.pairing || self.env.yard.is_some() || self.settings.yard(name).is_none() {
            return;
        }
        window.focus(&self.line.focus_handle(cx));
        if name == self.yard && self.client.is_some() {
            return;
        }
        self.set(self.settings.clone().opening(name), cx);
        self.reopen(cx);
    }

    /// A yard is to be added in the settings: its fields are asked for,
    /// empty, with the focus in its name.
    fn add_yard(&mut self, far: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pairing || self.env.yard.is_some() {
            return;
        }
        self.adding = Some(match far {
            true => Adding::Far,
            false => Adding::Near,
        });
        self.yard_said = None;
        for field in [&self.yard_name, &self.socket, &self.address, &self.code] {
            field.update(cx, |line, cx| line.set_text("", cx));
        }
        window.focus(&self.yard_name.focus_handle(cx));
        cx.notify();
    }

    /// The device of the yard called `name` is to be paired again: the
    /// settings, with a code to type for that yard's address. From where a
    /// yard said this device is not let in, it is the open one.
    fn pair_again(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.pairing {
            return;
        }
        if !self.settings_open {
            self.toggle_settings(window, cx);
        }
        self.adding = Some(Adding::Again(name.unwrap_or_else(|| self.yard.clone())));
        self.yard_said = None;
        self.code.update(cx, |line, cx| line.set_text("", cx));
        window.focus(&self.code.focus_handle(cx));
        cx.notify();
    }

    /// Enter in a field of a yard, and the button under them: the yard of
    /// this machine is added, or the code goes to the yard at the address.
    fn submit_yard(&mut self, cx: &mut Context<Self>) {
        match self.adding {
            Some(Adding::Near) => self.add_near(cx),
            Some(Adding::Far) | Some(Adding::Again(_)) => self.pair(cx),
            None => {}
        }
    }

    /// Adds the yard of this machine that is typed, and opens it. What
    /// keeps it out of the list is said, and nothing changed.
    fn add_near(&mut self, cx: &mut Context<Self>) {
        if self.pairing {
            return;
        }
        let name = self.yard_name.read(cx).text().trim().to_string();
        let yard = settings::Yard::near(&name, self.socket.read(cx).text());
        cx.notify();
        match self.settings.clone().with_yard(None, yard, None) {
            Ok(settings) => {
                self.yard_said = None;
                self.shut_adding(cx);
                self.set(settings.opening(&name), cx);
                self.reopen(cx);
            }
            Err(why) => self.yard_said = Some(why),
        }
    }

    /// Sends the code that is typed to the yard it is for, and opens that
    /// yard as the device the code was for: a yard that is added, at the
    /// address that is typed, or one of the list whose device is paired
    /// again. The token goes into that yard's own file and no other's.
    /// What keeps a yard out of the list is said before anything is sent,
    /// and so is a token file that cannot be written: a code is used up by
    /// the yard it is sent to.
    fn pair(&mut self, cx: &mut Context<Self>) {
        if self.pairing {
            return;
        }
        let Some(adding) = self.adding.clone() else {
            return;
        };
        cx.notify();
        let kept = self.kept.clone();
        let (address, code) = (self.address.read(cx).text(), self.code.read(cx).text());
        // The yard's address, the code, the file its token goes in, and
        // the settings once it is paired.
        let asked = match (&self.env.yard, &adding) {
            (_, Adding::Near) => return,
            // The environment's yard is the one there is, and its token's
            // file the environment's.
            (Some(address), _) => self
                .env
                .token_of(&self.settings, "", kept.as_deref())
                .map(|file| (address.clone(), code_of(code), file, self.settings.clone())),
            (None, Adding::Again(name)) => self
                .env
                .token_of(&self.settings, name, kept.as_deref())
                .map(|file| {
                    let listed = self.settings.yard(name);
                    let address = listed.map(|yard| yard.address.clone()).unwrap_or_default();
                    let settings = self.settings.clone().opening(name);
                    (address, code_of(code), file, settings)
                }),
            (None, Adding::Far) => {
                let (address, code) = yard::typed(address, code);
                // Called by its address's host where no name is typed.
                let name = match self.yard_name.read(cx).text().trim() {
                    "" => settings::host_of(&address),
                    name => name.to_string(),
                };
                let token_file = self.settings.token_name(&name, kept.as_deref());
                let yard = settings::Yard::far(&name, &address, &token_file);
                let settings = self.settings.clone();
                settings
                    .with_yard(None, yard, kept.as_deref())
                    .and_then(|settings| {
                        let file = yard::token_path(&token_file, kept.as_deref())?;
                        Ok((address, code, file, settings.opening(&name)))
                    })
            }
        };
        let parsed = asked.and_then(|(text, code, file, settings)| {
            Ok((Address::parse(&text)?, code, file, settings))
        });
        let (address, code, file, settings) = match parsed {
            Ok(asked) => asked,
            Err(why) => {
                self.yard_said = Some(why);
                return;
            }
        };
        if code.is_empty() {
            self.yard_said = Some(
                "A code is wanted. The yard's operator opens a pairing and gives you its code: yardr user device pair <user> <device>"
                    .to_string(),
            );
            return;
        }
        self.pairing = true;
        self.yard_said = None;
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_executor()
                .spawn(async move {
                    // The file is made ready before the code is sent: a
                    // code is used up by the yard it is sent to, and a
                    // token with no file to go in is lost.
                    let place = yard::token_place(&file).map_err(|err| {
                        format!(
                            "The token file {} cannot be written ({err}). The code was not sent, and is still good: name another file for this yard's token.",
                            file.display()
                        )
                    })?;
                    let paired = api::pair(&address, &code).map_err(|err| err.to_string())?;
                    // The token goes from the yard's answer into its file
                    // with the address the code went to, and nowhere else.
                    place.keep(&paired.token, &address).map_err(|err| {
                        format!(
                            "The yard gave this device its token, and {} could not be written ({err}). The token is lost: its operator removes the device (yardr user device rm {} {}) and opens another pairing.",
                            file.display(),
                            paired.user,
                            paired.device
                        )
                    })?;
                    Ok::<_, String>(format!(
                        "Paired: this is the device {} of {} in the yard {}.",
                        paired.device, paired.user, paired.yard
                    ))
                })
                .await;
            this.update(cx, |this, cx| {
                this.pairing = false;
                cx.notify();
                match answer {
                    Ok(said) => {
                        // The code is used up.
                        this.code.update(cx, |line, cx| line.set_text("", cx));
                        this.yard_said = Some(said);
                        this.shut_adding(cx);
                        // Pairing took place in the captured yard, but
                        // ordinary settings (size, look, notifications)
                        // may have changed while its answer was away.
                        let next = match (&this.env.yard, &adding) {
                            (Some(_), _) => Some(this.settings.clone()),
                            (None, Adding::Far) => settings
                                .yard(&settings.opened().name)
                                .cloned()
                                .and_then(|yard| {
                                    let name = yard.name.clone();
                                    this.settings
                                        .clone()
                                        .with_yard(None, yard, this.kept.as_deref())
                                        .ok()
                                        .map(|settings| settings.opening(&name))
                                }),
                            (None, Adding::Again(name)) => this
                                .settings
                                .yard(name)
                                .map(|_| this.settings.clone().opening(name)),
                            (None, Adding::Near) => None,
                        };
                        if let Some(settings) = next {
                            this.set(settings, cx);
                            // The token before it is not the yard's any
                            // more: the yard is opened with the new one.
                            let again = match &adding {
                                Adding::Again(name) => Some(name.as_str()),
                                _ => None,
                            };
                            this.reopen_anew(again, cx);
                        } else {
                            this.yard_said = Some(
                                "The yard paired this device, but its settings entry is no longer there. Add it again or remove the device at the yard: yardr user device rm <user> <device>".to_string(),
                            );
                        }
                    }
                    Err(why) => this.yard_said = Some(why),
                }
            })
            .ok();
        })
        .detach();
    }

    /// The fields of a yard are no longer asked for. The window holds the
    /// keys again: a field that is not drawn would take them nowhere.
    fn shut_adding(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        let (handle, focus) = (self.handle, self.focus.clone());
        // Afterwards: the window is not to be had while a key or click
        // that came through it is still on its way.
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.focus(&focus));
        });
        cx.notify();
    }

    /// Takes the yard called `name` off the list, with what the window
    /// kept for it, and says what became of its token's file and how the
    /// device is removed at the yard: taking it off here does not. Where
    /// it was the open one, the first of the list is shown.
    fn remove_yard(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.pairing || self.env.yard.is_some() {
            return;
        }
        // Who the device is there is what the yard said when it was read.
        let (model, said) = match self.held.get(name) {
            _ if name == self.yard => (self.model.as_ref(), self.said.as_ref()),
            Some(held) => (held.model.as_ref(), held.said.as_ref()),
            None => (None, None),
        };
        let device = model
            .zip(said)
            .map(|(model, said)| (model.me().to_string(), said.said_by.clone()));
        let (settings, left) = self.settings.clone().without(name, self.kept.as_deref());
        let device = device
            .as_ref()
            .map(|(user, device)| (user.as_str(), device.as_str()));
        self.yard_said = Some(left.said(name, device));
        if self.adding == Some(Adding::Again(name.to_string())) {
            self.shut_adding(cx);
        }
        self.set(settings, cx);
        // It is followed no longer, and where it was the open one another
        // is shown.
        self.reopen(cx);
        // The last yard removed leaves this machine's, which may be the
        // one that was to go: then it is still there, with what is its.
        if self.settings.yard(name).is_none() {
            for id in self.drafts.forget(name) {
                self.thumbs.remove(&id);
            }
            self.kept_words.remove(name);
            self.burst.forget(name);
            self.unreached.remove(name);
            self.places.remove(name);
        }
        cx.notify();
    }

    /// The control that opens the settings, and shuts them: at the foot
    /// of the left column, and under the words when the yard is away.
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

    /// The settings: the text size, the look, the yard and what the system
    /// tells of, each with its buttons and its keys, and where they are
    /// kept.
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
                    // Resolve the available head width before measuring the
                    // kept path, so truncate() can draw its ellipsis.
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("settings"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .truncate()
                            .debug_selector(|| "settings kept".to_string())
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
        let chosen = |el: gpui::Stateful<Div>, is: bool| {
            el.when(is, |el| {
                el.bg(palette.open)
                    .text_color(palette.text)
                    .font_weight(FontWeight::SEMIBOLD)
            })
        };
        let yard = self.talk_yards(setting, palette, cx);
        // What the system tells of: nothing, or the kinds that are chosen,
        // and what the system said if it does not let footplate notify.
        let notify = self.settings.notifications;
        let mut notifications = setting(
            "notifications",
            "the system tells of what arrives while the window is behind another or shows another row"
                .to_string(),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .children([("off", false), ("on", true)].map(|(name, on)| {
                    chosen(
                        small_button(name.to_string(), palette)
                            .id(SharedString::from(format!("notify {name}"))),
                        on == notify.on,
                    )
                    .debug_selector(move || format!("notify {name}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.notify_on(on, cx)))
                })),
        );
        if notify.on {
            notifications = notifications.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(palette.faint).child("of"))
                    .children(
                        [
                            ("mail", Kind::Mail),
                            ("decisions", Kind::Decision),
                            ("the yard's notices", Kind::Notice),
                        ]
                        .map(|(name, kind)| {
                            chosen(
                                small_button(name.to_string(), palette)
                                    .id(SharedString::from(format!("notify {name}"))),
                                notify.chosen(kind),
                            )
                            .debug_selector(move || format!("notify {name}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.notify_of(kind, cx)))
                        }),
                    ),
            );
            if let Some(Permission::Refused(why)) = &self.permission {
                notifications = notifications.child(
                    div()
                        .debug_selector(|| "notify said".to_string())
                        .text_sm()
                        .text_color(palette.warn)
                        .child(why.clone()),
                );
            }
        }
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
                    .child(look)
                    .child(yard)
                    .child(notifications),
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

    /// What the switcher and the settings say of a yard of the list after
    /// its name and its kind: that it is not reached, with the short why,
    /// that it is the open one, or what waits in it now. Every yard is
    /// followed, so one that is not reached is asked again by itself, and
    /// says so until it answers.
    fn yard_state(&self, yard: &settings::Yard) -> String {
        let model = self.held.get(&yard.name).and_then(|h| h.model.as_ref());
        match (self.unreached.get(&yard.name), model) {
            (Some(why), _) => format!("not reached: {why}"),
            _ if yard.name == self.yard => "open".to_string(),
            (None, Some(model)) => Counts::of(model).line(),
            (None, None) => "looking for the yard".to_string(),
        }
    }

    /// The yards in the settings: what a pairing is for, every yard of the
    /// list with its kind, its reach and its state and with the buttons
    /// that open it, pair its device again and remove it, and the fields
    /// of one that is added. With a yard named by the environment there is
    /// that one alone, to pair again.
    fn talk_yards(
        &self,
        setting: impl Fn(&'static str, String) -> Div,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let typed = |name: &'static str, line: &Entity<Line>, cx: &mut Context<Self>| {
            let focus = line.focus_handle(cx);
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(rems(4.)).flex_none().child(name))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(rems(28.))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(palette.line)
                        .bg(palette.side)
                        .debug_selector(move || format!("yard {name}"))
                        .on_mouse_down(MouseButton::Left, move |_, window, _| {
                            window.focus(&focus);
                            window.prevent_default();
                        })
                        .child(line.clone()),
                )
        };
        let faint = |text: String| div().text_sm().text_color(palette.faint).child(text);
        let mut yard = setting(
            "yards",
            format!(
                "the window shows {} · {SECONDARY}+y lists them · {SECONDARY}+1 to 9 opens one",
                self.which_yard()
            ),
        )
        .child(
            div()
                .debug_selector(|| "pairing gives".to_string())
                .text_sm()
                .child(PAIRING_GIVES),
        );
        match &self.env.yard {
            Some(address) => {
                yard = yard
                    .child(faint(format!(
                        "FOOTPLATE_YARD is set, and this window is of the one yard it names: {address}. The yards of the settings are for a start without it."
                    )))
                    .child(
                        div().flex().child(
                            small_button("pair again".to_string(), palette)
                                .id("yard again")
                                .debug_selector(|| "yard again".to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pair_again(None, window, cx)
                                })),
                        ),
                    );
            }
            None => {
                for listed in &self.settings.yards {
                    let name = listed.name.clone();
                    let reach = match (listed.is_far(), listed.socket.is_empty()) {
                        (true, _) => listed.address.clone(),
                        (false, true) => "by this run's socket".to_string(),
                        (false, false) => listed.socket.clone(),
                    };
                    let open = name == self.yard;
                    let button = |what: &str, text: &str| {
                        let at = format!("yard {what} {name}");
                        small_button(text.to_string(), palette)
                            .id(SharedString::from(at.clone()))
                            .debug_selector(move || at.clone())
                            .flex_none()
                    };
                    let mut row = div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .debug_selector({
                            let name = name.clone();
                            move || format!("yard row {name}")
                        })
                        .child(
                            div()
                                .when(open, |el| el.font_weight(FontWeight::SEMIBOLD))
                                .child(format!(
                                    "{name} · {} · {}",
                                    listed.kind(),
                                    self.yard_state(listed)
                                )),
                        )
                        .child(faint(reach));
                    if !open {
                        row = row.child(button("open", "open").on_click(cx.listener({
                            let name = name.clone();
                            move |this, _, window, cx| this.switch_to(&name, window, cx)
                        })));
                    }
                    if listed.is_far() {
                        row = row.child(button("again", "pair again").on_click(cx.listener({
                            let name = name.clone();
                            move |this, _, window, cx| {
                                this.pair_again(Some(name.clone()), window, cx)
                            }
                        })));
                    }
                    row = row.child(button("remove", "remove").on_click(cx.listener({
                        let name = name.clone();
                        move |this, _, _, cx| this.remove_yard(&name, cx)
                    })));
                    yard = yard.child(row);
                }
                yard = yard.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            small_button("add a yard of this machine".to_string(), palette)
                                .id("add near")
                                .debug_selector(|| "add near".to_string())
                                .when(self.adding == Some(Adding::Near), |el| el.bg(palette.open))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_yard(false, window, cx)
                                })),
                        )
                        .child(
                            small_button("add a yard at an address".to_string(), palette)
                                .id("add far")
                                .debug_selector(|| "add far".to_string())
                                .when(self.adding == Some(Adding::Far), |el| el.bg(palette.open))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_yard(true, window, cx)
                                })),
                        ),
                );
            }
        }
        let submit = |text: &str, cx: &mut Context<Self>| {
            div().flex().child(
                small_button(text.to_string(), palette)
                    .id("yard pair")
                    .debug_selector(|| "yard pair".to_string())
                    .when(self.pairing, |el| el.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| this.submit_yard(cx))),
            )
        };
        let pair = match self.pairing {
            true => "asking the yard…",
            false => "pair (enter)",
        };
        const OPERATOR: &str = "The yard's operator opens a pairing for this device and gives you the address and the code: yardr user device pair <user> <device>. A code is for one yard, and is used up there.";
        match &self.adding {
            None => {}
            Some(Adding::Near) => {
                yard = yard
                    .child(typed("name", &self.yard_name, cx))
                    .child(typed("socket", &self.socket, cx))
                    .child(submit("add (enter)", cx))
                    .child(faint(
                        "A yard of this machine is reached by its socket, as whoever runs footplate: no token and no pairing. With no socket named it is this machine's own yard; a path is another home's, <its home>/.yardr/yard.sock."
                            .to_string(),
                    ));
            }
            Some(Adding::Far) => {
                yard = yard
                    .child(typed("name", &self.yard_name, cx))
                    .child(typed("address", &self.address, cx))
                    .child(typed("code", &self.code, cx))
                    .child(submit(pair, cx))
                    .child(faint(format!(
                        "{OPERATOR} With no name it is called by its address's host. The device's token is kept in a file of this yard's own beside the settings, for you alone, and never with the settings."
                    )));
            }
            Some(Adding::Again(name)) => {
                let (whose, address) = match &self.env.yard {
                    Some(address) => ("the yard".to_string(), address.clone()),
                    None => {
                        let listed = self.settings.yard(name);
                        let address = listed.map(|yard| yard.address.clone());
                        (name.clone(), address.unwrap_or_default())
                    }
                };
                let token = match self
                    .env
                    .token_of(&self.settings, name, self.kept.as_deref())
                {
                    Ok(file) => format!(
                        "The device's token is kept in {}, for you alone, and never with the settings.",
                        file.display()
                    ),
                    Err(why) => why,
                };
                yard = yard
                    .child(
                        div()
                            .debug_selector(|| "yard again for".to_string())
                            .child(format!("a code for {whose}, at {address}")),
                    )
                    .child(typed("code", &self.code, cx))
                    .child(submit(pair, cx))
                    .child(faint(format!("{OPERATOR} {token}")));
            }
        }
        if let Some(said) = &self.yard_said {
            yard = yard.child(
                div()
                    .debug_selector(|| "yard said".to_string())
                    .child(said.clone()),
            );
        }
        yard
    }

    /// The switcher's list, under the head of the left column: a row for
    /// each yard of the settings, with its kind and its state, which of
    /// one that is not open is what waits in it now, the open one marked. A click opens a yard, as Enter does on the row the
    /// arrows are on.
    fn switcher(&self, at: usize, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .debug_selector(|| "switcher".to_string())
            .flex()
            .flex_col()
            .flex_none()
            .py_1()
            .border_b_1()
            .border_color(palette.line);
        for (place, yard) in self.settings.yards.iter().enumerate() {
            let name = yard.name.clone();
            let open = name == self.yard;
            let mark = if open { "▸" } else { " " };
            list = list.child(
                div()
                    .id(SharedString::from(format!("switch {name}")))
                    .debug_selector({
                        let name = name.clone();
                        move || format!("switch {name}")
                    })
                    .mx_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .when(place == at, |el| el.bg(palette.open))
                    // A yard that is not reached has nothing to open
                    // it for, and is drawn so.
                    .when(self.unreached.contains_key(&yard.name), |el| {
                        el.text_color(palette.faint)
                    })
                    .hover(|style| style.bg(palette.hover))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.switch_to(&name, window, cx)),
                    )
                    .child(format!(
                        "{mark} {} · {} · {}",
                        yard.name,
                        yard.kind(),
                        self.yard_state(yard)
                    )),
            );
        }
        list.child(
            div()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(palette.faint)
                .child(format!(
                    "↑↓ enter opens · esc leaves · {SECONDARY}+1 to 9 from anywhere"
                )),
        )
    }

    fn toggle_decided(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_decided();
            Vec::new()
        });
    }

    /// No yard: which, and why in a word or two, then why in words, where
    /// it was looked for, and whether it is tried again. A yard that no
    /// longer lets this device in is not: the device is paired again from
    /// here. It stands beside the head of the left column, which opens
    /// another yard; the one yard the environment names has no other, and
    /// its settings are reached from under the words.
    fn away(&self, why: &str, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let place = self.client.as_ref().map(Client::place);
        let alone = self.env.yard.is_some();
        let short = self.unreached.get(&self.yard).copied();
        let again = match self.away {
            Away::Trying => format!(
                "footplate tries again every {} seconds.",
                session::RETRY.as_secs()
            ),
            Away::Out => {
                "footplate does not ask again until this device has a token again.".to_string()
            }
            Away::Stopped if alone => {
                "footplate does not try again: the settings open a yard, and so does the next start."
                    .to_string()
            }
            Away::Stopped => {
                "footplate does not try again: the head opens another yard, and so do the settings."
                    .to_string()
            }
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
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
                    .children(short.map(|short| {
                        div()
                            .debug_selector(|| "away why".to_string())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} · {short}", self.which_yard_short()))
                    }))
                    .child(div().child(why.to_string()))
                    .children(place.map(|place| div().text_color(palette.faint).child(place)))
                    .child(div().text_color(palette.faint).child(again)),
            )
            // A yard at an address that is away for whatever reason: the
            // device can be paired again, which is what helps when the yard
            // no longer knows it.
            .when(self.client.as_ref().is_some_and(Client::is_remote), |el| {
                el.child(
                    small_button("pair again".to_string(), palette)
                        .id("pair again")
                        .debug_selector(|| "pair again".to_string())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.pair_again(None, window, cx)),
                        ),
                )
            })
            // The column's foot has the control where there is a column.
            .when(alone, |el| {
                el.child(self.settings_control(format!("settings ({SECONDARY}+,)"), palette, cx))
            })
    }

    /// The left column: the head, which names the open yard and is the
    /// switcher, the switcher's list while it is open, `body`, and the
    /// column's own controls at its foot. It stands the same beside a yard
    /// that is away, with nothing of that yard in `body`: the person goes
    /// to another from where they always do.
    fn column(
        &self,
        me: Option<&str>,
        body: impl IntoElement,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
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
                    // Resolve the available row width before measuring the
                    // name, so truncate() can draw its ellipsis.
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("footplate"))
                    .child(
                        div()
                            .id("which yard")
                            // Leave enough room for the operator and the
                            // local yard name at the narrowest window.
                            .text_size(rems(0.8))
                            .text_color(palette.accent)
                            .truncate()
                            .debug_selector(|| "which yard".to_string())
                            // The line is the switcher: it names the open
                            // yard and opens the list of them. The one
                            // yard the environment names has none.
                            .when(self.env.yard.is_none(), |el| {
                                el.cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_switcher(window, cx)
                                    }))
                            })
                            .child(self.head_line(me)),
                    ),
            );
        // A quiet row for the column's own controls, under the rows and
        // outside what scrolls, so it stays in place.
        let foot = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(palette.line)
            .child(self.settings_control("settings".to_string(), palette, cx));
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
            .children(self.switching.map(|at| self.switcher(at, palette, cx)))
            .child(body)
            .child(foot)
    }

    fn list(&self, model: &Model, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = model.rows();
        let now = chrono::Utc::now();
        let mut body = div()
            .id("rows")
            .debug_selector(|| "rows".to_string())
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
        self.column(Some(model.me()), body, palette, cx)
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
        let name = row.name.clone();
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
                    // Resolve the available row width before measuring the
                    // name, so truncate() can draw its ellipsis.
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .debug_selector(|| format!("row name {}", name))
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
                        .debug_selector(|| format!("row unread {}", name))
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
            Some(Selected::Yard) => self.talk_yard(model, palette, cx).into_any_element(),
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

    /// The control that jumps to the newest message: an arrow over the
    /// mail, with how many mails arrived below while the view stood
    /// scrolled up, in the unread count's colours when there are any.
    fn jump_button(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let marked = self.new_below > 0;
        let label = match marked {
            true => format!("↓ {}", self.new_below),
            false => "↓".to_string(),
        };
        div()
            .id("jump newest")
            .debug_selector(|| "jump newest".to_string())
            .absolute()
            .bottom(rems(0.75))
            .right(rems(0.75))
            .px_3()
            .py_1()
            .rounded_full()
            .border_1()
            .border_color(palette.line)
            .bg(palette.ground)
            .when(marked, |el| {
                el.bg(palette.accent)
                    .text_color(palette.on_accent)
                    .border_color(palette.accent)
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!marked, |el| el.hover(|style| style.bg(palette.hover)))
            .cursor_pointer()
            .child(
                div()
                    .when(marked, |el| {
                        el.debug_selector(|| "jump newest count".to_string())
                    })
                    .child(label),
            )
            .on_click(cx.listener(|this, _, window, cx| this.jump(window, cx)))
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
            mail.child(
                div()
                    .text_color(palette.faint)
                    .child(if with == model.me() {
                        "No notes to yourself yet. What you write here is mail to yourself."
                            .to_string()
                    } else {
                        format!("No mail between you and {with} yet. What you write here is mail.")
                    }),
            )
        } else {
            // The line above the oldest mail is always the first child:
            // a mail keeps its place among the children but for the mails
            // put in above it.
            let top = match model.top() {
                Top::Older => "Loading older mail…",
                Top::Start => "The conversation starts here.",
                Top::Unread => "",
            };
            mail.child(
                div()
                    .debug_selector(|| "conversation top".to_string())
                    .flex()
                    .justify_center()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(top),
            )
            .children(entries.into_iter().map(|entry| self.entry(entry, palette)))
        };
        // The jump control stands over the mail, at its bottom right,
        // while the view is above its newest.
        let mail = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(mail)
            .when(self.shows_jump(), |el| {
                el.child(self.jump_button(palette, cx))
            });

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
    /// in. One key marks one read, and with shift every one, as the button
    /// in the head does while a notice is unread.
    fn talk_yard(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let unread = model.notices().iter().filter(|n| !n.read).count();
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("yard"))
                    .when(unread > 0, |el| {
                        el.child(
                            small_button(format!("mark {unread} read (X)"), palette)
                                .id("mark all read")
                                .debug_selector(|| "mark all read".to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.dismiss_all(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("What the yard reports, apart from every conversation, and mail from anyone who has no row."),
            );
        // Its notices and the mail it sent itself, in the order they came.
        enum Said {
            Notice(model::Notice),
            Mail(Entry),
        }
        let mut said: Vec<(u64, Said)> = model
            .notices()
            .into_iter()
            .map(|n| (n.seq, Said::Notice(n)))
            .chain(
                model
                    .yard_mail()
                    .into_iter()
                    .map(|e| (e.seq, Said::Mail(e))),
            )
            .collect();
        said.sort_by_key(|(seq, _)| *seq);
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
        let list = if said.is_empty() {
            list.child(
                div()
                    .text_color(palette.faint)
                    .child("No notices. The yard reports here what you subscribe to."),
            )
        } else {
            list.children(said.into_iter().map(|(_, said)| {
                let n = match said {
                    Said::Notice(n) => n,
                    Said::Mail(entry) => return self.entry(entry, palette).into_any_element(),
                };
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
                    .into_any_element()
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
                    .child("x marks one notice read, X all of them; mail is read when shown here"),
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
        // The question, with the yard, the bead small and how long it
        // waits.
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
                        .debug_selector(|| format!("in {} {bead_id} {ask_seq}", self.yard))
                        .flex_none()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(card_head(&self.yard, short, &ask.waits)),
                ),
        );
        // The asker and the recommendation.
        let asker = if ask.secret.is_some() || ask.run.is_some() {
            format!("asked by {}", ask.asker)
        } else if ask.recommend_label.is_empty() {
            format!("asked by {} · no recommendation", ask.asker)
        } else {
            format!(
                "asked by {} · recommends: {}",
                ask.asker, ask.recommend_label
            )
        };
        card = card.child(div().text_sm().text_color(palette.faint).child(asker));
        // The context lines: text as it was written, and no block in it. A
        // run ask's each begin with who says them.
        for line in &ask.context {
            card = card.child(div().child(line.clone()));
        }
        // The options numbered, each with its consequence, `you decide`
        // last as the yard lists it. A secret ask has none and wants no
        // words: what it asks for is said in the yard's line, as it is.
        // Nor has a run ask: the yard's block is the last of its card, with
        // the control that approves directly under it.
        let elsewhere = self.approve_elsewhere.contains(&(bead_id.clone(), ask_seq));
        if let Some(secret) = &ask.secret {
            card = card.child(self.secret_lines(secret, palette));
        } else if let Some(run) = &ask.run {
            card = card.child(self.run_block(&bead_id, ask_seq, run, true, palette));
        } else if ask.options.is_empty() {
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
        // Decline is not a pick, and not "you decide". A run ask has no
        // pick to take it for.
        card = card.when(ask.run.is_none(), |card| {
            card.child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("Decline is not a pick, and not \u{201c}you decide\u{201d}: the asker must find another way."),
            )
        });
        if current {
            // A line for words, or the field of a secret in its place.
            let field = match ask.secret {
                Some(_) => self.secret.clone(),
                None => self.line.clone(),
            };
            let focus = field.focus_handle(cx);
            let kind = if ask.secret.is_some() {
                "secret"
            } else {
                "words"
            };
            // Where this yard takes no approval from this window, the card
            // says where it does, in place of the control.
            if ask.run.is_some() && elsewhere {
                card = card.child(self.approve_elsewhere_lines(&bead_id, ask_seq, palette));
            }
            // A run ask takes words only with a decline: its card has the
            // line while one is asked for, and at no other time.
            let declining = self.declining.as_ref() == Some(&(bead_id.clone(), ask_seq));
            card = card.when(ask.run.is_none() || declining, |card| {
                card.child(
                    div()
                        .debug_selector(|| format!("{kind} {bead_id} {ask_seq}"))
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
                        .child(field),
                )
            });
            // Under it the button that sends the field, exactly as Enter
            // does, with Later, Decline and ask-about-this beside it. An
            // asker with no conversation has the reason in place of that
            // button.
            let foreign = model::foreign_bead(&detail.bead);
            let send = match (&ask.secret, self.giving()) {
                (Some(_), Some(secret)) => Some((
                    format!("give {bead_id} {ask_seq}"),
                    give_label(&secret, self.secret_typed.len()),
                    !self.secret.read(cx).text().is_empty(),
                )),
                (Some(_), None) => None,
                // A run ask's one button approves, and sends no words.
                (None, _) if ask.run.is_some() => None,
                (None, _) => ask
                    .options
                    .iter()
                    .find(|o| o.number == ask.chosen)
                    .map(|o| {
                        (
                            format!("send {bead_id} {ask_seq}"),
                            format!("Send: {} · {}", o.number, o.label),
                            true,
                        )
                    })
                    .or_else(|| {
                        words_send(ask, foreign).then(|| {
                            (
                                format!("send {bead_id} {ask_seq}"),
                                "Send".to_string(),
                                !self.line.read(cx).text().is_empty(),
                            )
                        })
                    }),
            };
            let unwritable = self.model.as_ref().and_then(|m| m.unwritable(&ask.asker));
            let ask_about = match &unwritable {
                None => small_button("Ask about this (a)".to_string(), palette)
                    .id(SharedString::from(format!("ask {bead_id} {ask_seq}")))
                    .debug_selector(|| format!("ask {bead_id} {ask_seq}"))
                    .on_click(cx.listener(|this, _, window, cx| this.ask_about_now(window, cx))),
                Some(why) => div()
                    .id(SharedString::from(format!(
                        "ask unreachable {bead_id} {ask_seq}"
                    )))
                    .debug_selector(|| format!("ask unreachable {bead_id} {ask_seq}"))
                    .text_sm()
                    .text_color(palette.faint)
                    .child(why.clone()),
            };
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(send, |el, (id, text, enabled)| {
                        el.child(send_button(id, text, enabled, palette, cx))
                    })
                    .when(ask.run.is_some() && !elsewhere, |el| {
                        el.child(approve_button(
                            format!("approve {bead_id} {ask_seq}"),
                            &self.yard,
                            self.approving.is_none(),
                            palette,
                            cx,
                        ))
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
                    )
                    .child(ask_about),
            );
            // The keys. Enter with nothing chosen and a recommendation picks
            // it, and says so.
            card = card.child(div().text_sm().text_color(palette.faint).child(
                match (&ask.secret, &ask.run) {
                    (Some(_), _) => secret_keys(&bead_id, ask_seq, unwritable.is_none()),
                    (None, Some(_)) => run_keys(!elsewhere, unwritable.is_none()),
                    (None, None) => decision_keys(ask, foreign, unwritable.is_none()),
                },
            ));
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
                select::key((bead_id, ask_seq)),
                &detail.body,
                &detail.notes,
                &ask.about,
                &detail.doc,
                palette,
            ));
        }
        card
    }

    /// What a secret ask asks for: the yard's line, which is made from the
    /// source's declaration and never from the asker's words, and where
    /// the value goes.
    fn secret_lines(&self, secret: &api::Secret, palette: Palette) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| secret.line.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(secret.line.clone()),
            )
            .child(div().text_sm().text_color(palette.faint).child(SECRET_GOES))
    }

    /// What a run ask proposes, in the one block of its card, which is all
    /// the yard's and none of it the asker's: the command whole, in a code
    /// block that wraps where the window is narrower and is never cut; the
    /// yard's sentences for what it changes; and the command that takes it
    /// back. `open` is an ask that can still be approved.
    fn run_block(
        &self,
        bead: &str,
        ask: u64,
        run: &api::Run,
        open: bool,
        palette: Palette,
    ) -> impl IntoElement + use<> {
        let card_key = select::key((bead, ask));
        let command = model::plain(&run.line());
        // The user the approval is called as, and so the one it runs as.
        let user = self.model.as_ref().map(|m| m.me()).unwrap_or_default();
        let over = match open {
            true => run_runs(&self.yard, user),
            false => RUN_PROPOSED.to_string(),
        };
        let mut block = div()
            .debug_selector(|| format!("run {bead} {ask}"))
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(palette.line)
            .bg(palette.side)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .debug_selector(|| over.clone())
                    .text_sm()
                    .text_color(palette.faint)
                    .child(over.clone()),
            )
            .child(
                div()
                    .debug_selector(|| format!("run command {bead} {ask}"))
                    .child(select::text(
                        expanded_text_key(card_key, "run", 0, &command),
                        fenced(&command),
                        &self.selection,
                    )),
            );
        for does in &run.does {
            block = block.child(div().child(format!("the yard says: {}", model::plain(does))));
        }
        // Who may take it back is not said: the yard says it of some
        // commands and not of others, and sends no word of it here.
        let back = model::plain(&run.back);
        if !back.is_empty() {
            block = block
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.faint)
                        .child("the command that takes it back:"),
                )
                .child(
                    div()
                        .debug_selector(|| format!("run back {bead} {ask}"))
                        .child(select::text(
                            expanded_text_key(card_key, "back", 0, &back),
                            fenced(&back),
                            &self.selection,
                        )),
                );
        }
        block
    }

    /// Where a run ask is approved when this yard takes no approval from
    /// this window: a terminal on the yard's machine, with the command in a
    /// block that copies it.
    fn approve_elsewhere_lines(
        &self,
        bead: &str,
        ask: u64,
        palette: Palette,
    ) -> impl IntoElement + use<> {
        let command = model::approve_command(bead, ask);
        div()
            .debug_selector(|| format!("approve elsewhere {bead} {ask}"))
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_color(palette.warn).child(APPROVE_ELSEWHERE))
            .child(select::text(
                expanded_text_key(select::key((bead, ask)), "approve", 0, &command),
                fenced(&command),
                &self.selection,
            ))
    }

    /// One dealt with: what was decided, in plain words with who and when;
    /// the picked option marked, with its words under it; the same options
    /// to pick again from there; and the same full bead one key away. One
    /// its asker took back reads only: the yard refuses an answer to it, so
    /// its card has no options to pick and no field.
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
                    .debug_selector(|| format!("in {} {bead} {ask}", self.yard))
                    .text_sm()
                    .text_color(palette.faint)
                    .child(card_head(&self.yard, &detail.bead, &format!("ask {ask}"))),
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
        // Under an approved run ask's command, what it printed.
        let printed = detail.ran.as_ref().map(|ran| model::plain(&ran.result));
        if let Some(printed) = printed.filter(|printed| !printed.is_empty()) {
            body = body.child(
                div()
                    .debug_selector(|| format!("ran {bead} {ask}"))
                    .child(printed),
            );
        }
        if !model.has_bead(bead) {
            body = body.child(div().text_color(palette.faint).child("Reading the bead."));
        } else {
            // The context, the options to pick again, the recommendation.
            for line in &detail.context {
                body = body.child(div().child(line.clone()));
            }
            // An ask with no options shows none: the answer's words are in
            // the state line, and nothing is wanted. Nor does one taken
            // back: there is nothing to pick.
            let taken_back = detail.state == "taken back";
            if !detail.options.is_empty() && !taken_back {
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
            // A secret ask says what it asks for in the yard's line.
            if let Some(secret) = &detail.secret {
                body = body.child(self.secret_lines(secret, palette));
            }
            // A run ask has the yard's block, and under it the control
            // that approves while it is only put aside: one approved,
            // declined or taken back takes no approval.
            let approving = model.approving(bead, ask).is_some();
            let elsewhere = self.approve_elsewhere.contains(&(bead.to_string(), ask));
            if let Some(run) = &detail.run {
                body = body.child(self.run_block(bead, ask, run, approving, palette));
                if approving && elsewhere {
                    body = body.child(self.approve_elsewhere_lines(bead, ask, palette));
                } else if approving {
                    body = body.child(div().flex().child(approve_button(
                        format!("approve {bead} {ask}"),
                        &self.yard,
                        self.approving.is_none(),
                        palette,
                        cx,
                    )));
                }
            }
            // A line for words with the later pick, or the field of a
            // secret to give it again: a declined one takes none, one taken
            // back has neither, and no
            // line for words stands where a value could be typed in sight.
            let giving = model.giving(bead, ask);
            let field = match detail.secret {
                Some(_) => self.secret.clone(),
                None => self.line.clone(),
            };
            let focus = field.focus_handle(cx);
            let kind = if detail.secret.is_some() {
                "secret"
            } else {
                "words"
            };
            let words = detail.secret.is_none() && detail.run.is_none();
            let fielded = !taken_back && (words || giving.is_some());
            body = body.when(fielded, |body| {
                body.child(
                    div()
                        .debug_selector(|| format!("{kind} {bead} {ask}"))
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
                        .child(field),
                )
            });
            // Under it the button that sends the field, exactly as Enter
            // does: the value again on a secret, the words where they are
            // the answer, the new pick where one is chosen.
            let send = match (&detail.secret, giving) {
                _ if taken_back => None,
                (Some(_), Some(secret)) => Some((
                    format!("give {bead} {ask}"),
                    give_label(secret, self.secret_typed.len()),
                    !self.secret.read(cx).text().is_empty(),
                )),
                (Some(_), None) => None,
                (None, _) if detail.run.is_some() => None,
                (None, _) => detail
                    .options
                    .iter()
                    .find(|o| o.number == detail.chosen)
                    .map(|o| {
                        (
                            format!("send {bead} {ask}"),
                            format!("Change to: {} · {}", o.number, o.label),
                            true,
                        )
                    })
                    .or_else(|| {
                        detail.options.is_empty().then(|| {
                            (
                                format!("send {bead} {ask}"),
                                "Send".to_string(),
                                !self.line.read(cx).text().is_empty(),
                            )
                        })
                    }),
            };
            body = body.when_some(send, |body, (id, text, enabled)| {
                body.child(send_button(id, text, enabled, palette, cx))
            });
            body = body.child(div().text_sm().text_color(palette.faint).child(
                match detail.secret {
                    Some(_) if giving.is_some() => secret_keys(bead, ask, false),
                    Some(_) => String::new(),
                    None if detail.run.is_some() => run_keys(approving && !elsewhere, false),
                    None => decided_keys(&detail),
                },
            ));
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
                    select::key((bead, ask)),
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
        card_key: u64,
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
                    expanded_text_key(card_key, "body", 0, body),
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
                            expanded_text_key(card_key, "note", ix, &note.text),
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

impl Drop for Footplate {
    fn drop(&mut self) {
        if let Some(stop) = self.following_stop.take() {
            stop.stop();
        }
    }
}

impl Focusable for Footplate {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Footplate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reconcile_scroll(window, cx);
        // The one place the text size is set: every size that goes with the
        // text is in rems, and the text itself is seven eighths of one.
        window.set_rem_size(self.settings.rem());
        let palette = Palette::now(window, cx);
        // On a card the window holds the keys to pick with, and the line to
        // type words in: with the focus in the line those keys are text.
        let context = match self.model.as_ref().and_then(|m| m.selected().cloned()) {
            // The switcher's list has the keys while it is open.
            _ if self.switching.is_some() => SWITCH_CONTEXT,
            // The settings stand where the card or the notices do: the keys
            // that pick and mark read are not for what is not in view.
            _ if self.settings_open => CONTEXT,
            Some(Selected::Conversation(_)) => CONTEXT,
            Some(Selected::Decision(_)) | Some(Selected::Decided(_, _))
                if self.line.focus_handle(cx).is_focused(window)
                    || self.secret.focus_handle(cx).is_focused(window) =>
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
            .on_action(cx.listener(|this, _: &DismissAll, _, cx| this.dismiss_all(cx)))
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
            .on_action(
                cx.listener(|this, _: &Decide, window, cx| match this.switching {
                    Some(at) => this.switch_nth(at, window, cx),
                    None => this.decide(cx),
                }),
            )
            .on_action(cx.listener(|this, _: &Approve, _, cx| this.approve(cx)))
            .on_action(cx.listener(Self::decline))
            .on_action(cx.listener(Self::ask_about))
            .on_action(cx.listener(Self::expand))
            .on_action(cx.listener(Self::focus_words))
            .on_action(cx.listener(Self::leave))
            .on_action(cx.listener(Self::jump_newest))
            .on_action(cx.listener(Self::jump_oldest))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| this.page(false, window, cx)))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| this.page(true, window, cx)))
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
        // A yard that is away keeps the column with its head, and none of
        // its rows: what they open is not there. The one yard the
        // environment names has no other to go to, and no column.
        let beside = match (&self.down, &self.model) {
            (Some(_), model) if self.env.yard.is_none() => {
                let me = model.as_ref().map(Model::me);
                Some(self.column(me, div().flex_1(), palette, cx))
            }
            _ => None,
        };
        let root = root.children(beside);
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

fn expanded_text_key(card_key: u64, section: &str, index: usize, text: &str) -> u64 {
    select::key((card_key, section, index, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Yard;
    use crate::input;
    use gpui::{Bounds, TestAppContext, VisualTestContext, WindowAppearance};
    use serde_json::{Value, json};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn expanded_text_keys_include_the_card_identity() {
        assert_ne!(
            expanded_text_key(1, "body", 0, "same body"),
            expanded_text_key(2, "body", 0, "same body")
        );
        assert_ne!(
            expanded_text_key(1, "note", 0, "same note"),
            expanded_text_key(2, "note", 0, "same note")
        );
    }

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
            secret: None,
            run: None,
        };
        assert_eq!(
            decision_keys(&ask, false, true),
            "words wanted · i words · enter sends the answer · a ask about this"
        );
    }

    /// The card's keys name ask-about-this with the others; a foreign one
    /// promises nothing of Enter without a pick, and an asker with no
    /// conversation names no key for it.
    #[test]
    fn the_card_s_keys_name_ask_about_this_and_nothing_it_cannot_do() {
        let ask = DecisionAsk {
            ask: 3,
            question: "Move them now?".to_string(),
            asker: "yardmaster".to_string(),
            waits: "2h".to_string(),
            recommend_label: "Move them now".to_string(),
            context: Vec::new(),
            options: vec![
                model::CardOption {
                    number: 1,
                    label: "Move them now".to_string(),
                    consequence: "all 64 links point tonight".to_string(),
                },
                model::CardOption {
                    number: 2,
                    label: "You decide".to_string(),
                    consequence: "leave it to yardmaster".to_string(),
                },
            ],
            recommend: 1,
            why: "the test passed".to_string(),
            chosen: 0,
            current: true,
            expanded: false,
            about: Vec::new(),
            state: "open".to_string(),
            secret: None,
            run: None,
        };
        assert_eq!(
            decision_keys(&ask, false, true),
            "enter picks 1 · 1-2 pick · ↑↓ choose · i words · a ask about this"
        );
        let mut none = ask.clone();
        none.recommend = 0;
        none.recommend_label.clear();
        assert_eq!(
            decision_keys(&none, false, true),
            "enter sends words as note · 1-2 pick · ↑↓ choose · i words · a ask about this"
        );
        // A foreign card: words alone go as no note, so Enter promises
        // nothing; asking about this is how to talk instead.
        assert_eq!(
            decision_keys(&none, true, true),
            "1-2 pick · ↑↓ choose · i words · a ask about this"
        );
        // An asker with no conversation: the control says so, and no key
        // reaches for it.
        assert_eq!(
            decision_keys(&none, false, false),
            "enter sends words as note · 1-2 pick · ↑↓ choose · i words"
        );
        let mut words = none.clone();
        words.options.clear();
        assert_eq!(
            decision_keys(&words, true, false),
            "words wanted · i words · enter sends the answer"
        );
    }

    #[test]
    fn a_writable_secret_card_lists_ask_about_this_in_its_footer() {
        let ordinary = "i types it · enter sends it to the yard · esc leaves, and nothing is sent · or in a terminal: yardr bead answer bead-s 7 --secret";
        assert_eq!(
            secret_keys("bead-s", 7, true),
            format!("{ordinary} · a ask about this")
        );
        assert_eq!(secret_keys("bead-s", 7, false), ordinary);
    }

    /// The secret's button names the value the field takes now.
    #[test]
    fn the_give_button_names_the_value_the_field_takes_now() {
        let secret = api::Secret {
            source: "books".to_string(),
            env: vec!["CRM_TOKEN".to_string(), "CRM_UPDATE_TOKEN".to_string()],
            line: "a secret".to_string(),
        };
        assert_eq!(give_label(&secret, 0), "Give CRM_TOKEN");
        assert_eq!(give_label(&secret, 1), "Give CRM_UPDATE_TOKEN");
        assert_eq!(give_label(&secret, 2), "Give");
        let one = api::Secret {
            env: vec!["PUSHOVER_TOKEN".to_string()],
            ..Default::default()
        };
        assert_eq!(give_label(&one, 0), "Give PUSHOVER_TOKEN");
    }

    /// An open ordinary ask with `options` options, `chosen` chosen and
    /// `recommend` recommended.
    fn ordinary(options: usize, chosen: u64, recommend: u64) -> DecisionAsk {
        DecisionAsk {
            ask: 3,
            question: "Move them now?".to_string(),
            asker: "yardmaster".to_string(),
            waits: "2h".to_string(),
            recommend_label: if recommend == 0 {
                String::new()
            } else {
                "Move them now".to_string()
            },
            context: Vec::new(),
            options: (1..=options as u64)
                .map(|n| model::CardOption {
                    number: n,
                    label: format!("option {n}"),
                    consequence: "it happens".to_string(),
                })
                .collect(),
            recommend,
            why: if recommend == 0 {
                String::new()
            } else {
                "the test passed".to_string()
            },
            chosen,
            current: true,
            expanded: false,
            about: Vec::new(),
            state: "open".to_string(),
            secret: None,
            run: None,
        }
    }

    /// Words alone want their own Send button where Enter sends them: as
    /// the answer where there are no options, as a note where there is no
    /// pick and none recommended.
    #[test]
    fn words_alone_want_a_send_button_where_enter_sends_them() {
        // No options: the words are the answer, here and on another yard's
        // bead.
        assert!(words_send(&ordinary(0, 0, 0), false));
        assert!(words_send(&ordinary(0, 0, 0), true));
        // No pick and none recommended: the words go as a note, but not on
        // another yard's bead.
        assert!(words_send(&ordinary(2, 0, 0), false));
        assert!(!words_send(&ordinary(2, 0, 0), true));
        // With a pick the pick's button sends the words with it.
        assert!(!words_send(&ordinary(2, 2, 0), false));
        assert!(!words_send(&ordinary(2, 2, 1), false));
        // With a recommendation and no pick no button sends it unasked.
        assert!(!words_send(&ordinary(2, 0, 1), false));
        // A secret ask has its Give button, never Send.
        let mut secret = ordinary(0, 0, 0);
        secret.secret = Some(api::Secret::default());
        assert!(!words_send(&secret, false));
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
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
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

    fn placeholder(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, cx| v.line.read(cx).placeholder().to_string())
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

        // Text on the clipboard is pasted into the line, as before, and
        // with its line break.
        cx.simulate_keystrokes("shift-tab");
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(" Two\nlines.".into()));
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(typed(&view, cx), "And this. Two\nlines.");
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
            assert_eq!(rows, ["assembly", "yardmaster", "clerk@north", "jo"]);
        });
        wait(cx, "the mail acknowledged", |_| {
            yard.asked("AckUserInbox") == [json!({"name": "jo", "through": 12})]
        });
        assert_eq!(
            yard.asked("Conversation")[0],
            json!({"a": "jo", "b": "yardmaster", "limit": 50})
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

    #[gpui::test]
    fn mail_from_the_yard_is_read_on_the_yard_row_and_leaves_no_badge_behind(
        cx: &mut TestAppContext,
    ) {
        // footplate-knh4: two refusals of a linked yard came back as mail
        // from the yard, between the mail of one of that yard.
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [
                {"seq": 12, "from": "clerk@north", "to": "jo", "text": "One.", "read": false},
                {"seq": 13, "from": "yard", "to": "jo", "text": "not delivered: rate_limited", "read": false},
                {"seq": 14, "from": "yard", "to": "jo", "text": "not delivered: rate_limited", "read": false},
                {"seq": 15, "from": "clerk@north", "to": "jo", "text": "Two.", "read": false}],
                "unread": 4, "last": 15, "from": ["clerk@north", "yard"]}}"#,
        );
        yard.answer("Conversation", r#"{"result": []}"#);
        let (view, cx) = opened(&yard, &[], cx);
        let counts = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| {
                let m = v.model.as_ref().unwrap();
                (m.unread("clerk@north"), m.yard_row().unread)
            })
        };
        wait(cx, "the unread mail counted", |cx| {
            view.read_with(cx, |v, _| v.model.is_some()) && counts(cx) == (2, 2)
        });

        // The conversation is opened: all its mail is shown, and it has no
        // badge, though the yard's mail keeps 15 from being acknowledged.
        yard.answer(
            "Conversation",
            r#"{"result": [
                {"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "clerk@north", "to": "jo", "text": "One.", "read": false},
                {"seq": 15, "at": "2026-10-08T09:33:00Z", "from": "clerk@north", "to": "jo", "text": "Two.", "read": false}]}"#,
        );
        cx.simulate_keystrokes("tab");
        wait(cx, "the conversation with the linked yard", |cx| {
            texts(&view, cx) == ["One.", "Two."]
        });
        wait(cx, "its first mail acknowledged", |_| {
            yard.asked("AckUserInbox") == [json!({"name": "jo", "through": 12})]
        });
        assert_eq!(counts(cx), (0, 2));

        // The yard row shows the yard's mail, which is read with that.
        yard.answer(
            "Conversation",
            r#"{"result": [
                {"seq": 13, "at": "2026-10-08T09:31:00Z", "from": "yard", "to": "jo", "text": "not delivered: rate_limited", "read": false},
                {"seq": 14, "at": "2026-10-08T09:32:00Z", "from": "yard", "to": "jo", "text": "not delivered: rate_limited", "read": false}]}"#,
        );
        cx.simulate_keystrokes("tab tab");
        wait(cx, "the yard's mail on the yard row", |cx| {
            view.read_with(cx, |v, _| {
                let m = v.model.as_ref().unwrap();
                m.selected() == Some(&Selected::Yard)
                    && m.yard_mail().iter().map(|e| e.seq).eq([13, 14])
            })
        });
        assert_eq!(
            yard.asked("Conversation").last(),
            Some(&json!({"a": "jo", "b": "yard", "after": 0}))
        );
        wait(cx, "all of it acknowledged", |_| {
            yard.asked("AckUserInbox").last() == Some(&json!({"name": "jo", "through": 15}))
        });
        assert_eq!(counts(cx), (0, 0));
        // Its text is drawn there.
        text_bounds(cx, 13);
    }

    #[gpui::test]
    fn mail_of_the_user_to_themselves_and_of_another_user_is_read_and_written_in_their_rows(
        cx: &mut TestAppContext,
    ) {
        // footplate-evk8: two mails of the user to themselves were the
        // oldest unread and had no row, so the twenty of the yardmaster
        // after them stayed unread in the yard though all were shown.
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "Users",
            r#"{"result": [{"name": "ada", "cursor": 0, "notification_cursor": 0},
                           {"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
        );
        let own = json!([
            {"seq": 6, "at": "2026-10-08T09:29:00Z", "from": "jo", "to": "jo", "text": "testing messaging myself", "read": false},
            {"seq": 7, "at": "2026-10-08T09:29:30Z", "from": "jo", "to": "jo", "text": "works", "read": false}]);
        let theirs: Vec<serde_json::Value> = (8..28)
            .map(|seq| json!({"seq": seq, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": format!("Mail {seq}."), "read": false}))
            .collect();
        let mut unread = own.as_array().unwrap().clone();
        unread.extend(theirs.iter().cloned());
        yard.answer(
            "UserInbox",
            &json!({"result": {"mail": unread, "unread": 22, "last": 27, "from": ["jo", "yardmaster"]}})
                .to_string(),
        );
        yard.answer("Conversation", &json!({"result": theirs}).to_string());
        let shown: Vec<String> = (8..28).map(|seq| format!("Mail {seq}.")).collect();
        let shown: Vec<&str> = shown.iter().map(String::as_str).collect();
        let (view, cx) = opened(&yard, &shown, cx);
        let counts = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| {
                let m = v.model.as_ref().unwrap();
                let rows: Vec<(String, usize)> = m
                    .rows()
                    .into_iter()
                    .filter(|r| r.unread > 0)
                    .map(|r| (r.name, r.unread))
                    .collect();
                (rows, m.yard_row().unread)
            })
        };
        // The yardmaster's twenty are shown and count nowhere. The two
        // that are not have a row that counts them: the user's own. The
        // yard is told nothing yet, since they are older.
        cx.run_until_parked();
        assert_eq!(counts(cx), (vec![("jo".to_string(), 2)], 0));
        assert_eq!(yard.asked("AckUserInbox"), Vec::<serde_json::Value>::new());

        // Past the linked yard's row and ada's to the user's own: it shows
        // both, and all the mail there is is acknowledged.
        let read_with = |with: &str| {
            yard.asked("Conversation").last() == Some(&json!({"a": "jo", "b": with, "limit": 50}))
        };
        yard.answer("Conversation", r#"{"result": []}"#);
        cx.simulate_keystrokes("tab");
        wait(cx, "the linked yard's conversation read", |_| {
            read_with("clerk@north")
        });
        cx.simulate_keystrokes("tab");
        wait(cx, "ada's conversation read", |_| read_with("ada"));
        yard.answer("Conversation", &json!({"result": own}).to_string());
        cx.simulate_keystrokes("tab");
        wait(cx, "the user's own row", |cx| {
            texts(&view, cx) == ["testing messaging myself", "works"]
        });
        assert!(read_with("jo"));
        wait(cx, "all of it acknowledged", |_| {
            yard.asked("AckUserInbox") == [json!({"name": "jo", "through": 27})]
        });
        assert_eq!(counts(cx), (vec![], 0));
        assert_eq!(placeholder(&view, cx), "Write to jo");

        // What is typed there is mail to oneself, shown and read as it
        // comes from the stream.
        cx.simulate_input("A note.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the note sent", |_| {
            yard.asked("Mail") == [json!({"to": "jo", "text": "A note.", "actor": "jo"})]
        });
        yard.emit(r#"{"seq": 28, "kind": "mail", "actor": "jo", "data": {"to": "jo", "to_user": true, "text": "A note."}, "at": "2026-10-08T09:40:00Z"}"#);
        wait(cx, "the note shown", |cx| {
            texts(&view, cx) == ["testing messaging myself", "works", "A note."]
        });
        wait(cx, "the note acknowledged", |_| {
            yard.asked("AckUserInbox").last() == Some(&json!({"name": "jo", "through": 28}))
        });

        // Another user of the yard writes: her row counts it, shows it and
        // takes the answer.
        yard.emit(r#"{"seq": 29, "kind": "mail", "actor": "ada", "data": {"to": "jo", "to_user": true, "text": "Hello jo."}, "at": "2026-10-08T09:41:00Z"}"#);
        wait(cx, "ada's mail counted", |cx| {
            counts(cx) == (vec![("ada".to_string(), 1)], 0)
        });
        cx.simulate_keystrokes("shift-tab");
        wait(cx, "ada's mail shown", |cx| {
            texts(&view, cx) == ["Hello jo."]
        });
        wait(cx, "ada's mail acknowledged", |_| {
            yard.asked("AckUserInbox").last() == Some(&json!({"name": "jo", "through": 29}))
        });
        assert_eq!(counts(cx), (vec![], 0));
        cx.simulate_input("Hello ada.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the answer sent", |_| {
            yard.asked("Mail").last()
                == Some(&json!({"to": "ada", "text": "Hello ada.", "actor": "jo"}))
        });
    }

    /// What the yard says of mail over a link that carries none out.
    const NO_MAIL: &str = "peer north: the link does not carry mail out: the operator allows it with yardr peer allow north --send mail";

    #[gpui::test]
    fn an_answer_to_a_trusted_name_goes_back_to_it_once_and_a_refusal_writes_nothing_instead(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // north's mckean is trusted as the user mckean of this yard.
        yard.answer(
            "Peers",
            r#"{"result": [{"name": "north", "send": ["mail"], "receive": ["mail"], "trust": {"mckean": "mckean"},
                "said": {"roster": [{"name": "mckean", "kind": "user", "mail": true}], "at": "2026-10-08T09:30:00Z"}}]}"#,
        );
        yard.answer(
            "Users",
            r#"{"result": [{"name": "mckean", "cursor": 0, "notification_cursor": 0},
                           {"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
        );
        yard.answer_for(
            "Conversation",
            "b",
            "mckean",
            r#"{"result": [{"seq": 14, "at": "2026-10-08T09:28:00Z", "from": "mckean", "to": "jo", "text": "I am here until two.", "read": true}]}"#,
        );
        yard.answer_for(
            "Conversation",
            "b",
            "mckean@north",
            r#"{"result": [{"seq": 15, "at": "2026-10-08T09:29:00Z", "from": "mckean@north", "to": "jo", "text": "I am at the other machine now.", "read": true}]}"#,
        );
        let (view, cx) = opened(&yard, &["Shall I land it?"], cx);
        // The name trusted as the user has no row of its own.
        view.read_with(cx, |v, _| {
            let rows: Vec<String> = v
                .model
                .as_ref()
                .unwrap()
                .rows()
                .into_iter()
                .map(|r| r.name)
                .collect();
            assert_eq!(rows, ["assembly", "yardmaster", "mckean", "jo"]);
        });

        // The user's row has the mail of both, in the order written, and
        // says where the next answer goes before anything is typed.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Conversation("mckean".into()), window, cx)
        });
        wait(cx, "the user's conversation", |cx| {
            texts(&view, cx) == ["I am here until two.", "I am at the other machine now."]
        });
        assert_eq!(placeholder(&view, cx), "Write to mckean@north");
        view.read_with(cx, |v, _| {
            assert_eq!(
                v.model.as_ref().unwrap().about(),
                [
                    "a person of this yard, also trusted as mckean@north: mail from that name is in this conversation, and what you write goes to mckean@north, who wrote last"
                ]
            )
        });

        // The answer is one mail, to the name at its yard, and none to the
        // user in this yard.
        cx.simulate_input("Fine, the review can wait.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the answer queued for the peer", |_| {
            yard.asked("MailPeer")
                == [
                    json!({"to": "mckean", "peer": "north", "text": "Fine, the review can wait.", "actor": "jo"}),
                ]
        });
        wait(cx, "the line empty", |cx| typed(&view, cx).is_empty());
        cx.run_until_parked();
        assert_eq!(yard.asked("Mail"), Vec::<serde_json::Value>::new());

        // The user writes in this yard: the next answer goes to them here.
        yard.emit(r#"{"seq": 21, "kind": "mail", "actor": "mckean", "data": {"to": "jo", "to_user": true, "text": "Back at this one."}, "at": "2026-10-08T09:31:00Z"}"#);
        wait(cx, "where the next answer goes", |cx| {
            placeholder(&view, cx) == "Write to mckean"
        });
        cx.simulate_input("Good.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the answer to the user here", |_| {
            yard.asked("Mail") == [json!({"to": "mckean", "text": "Good.", "actor": "jo"})]
        });
        assert_eq!(yard.asked("MailPeer").len(), 1);

        // The name writes again, and the link carries no mail out by now:
        // the yard's refusal is shown, the line kept, and nothing written
        // to the user in this yard instead.
        yard.emit(r#"{"seq": 22, "kind": "mail", "actor": "mckean@north", "data": {"to": "jo", "to_user": true, "text": "Away again."}, "at": "2026-10-08T09:32:00Z"}"#);
        wait(cx, "where the next answer goes", |cx| {
            placeholder(&view, cx) == "Write to mckean@north"
        });
        yard.answer(
            "MailPeer",
            &json!({"error": {"code": "error", "message": NO_MAIL}}).to_string(),
        );
        let before = texts(&view, cx);
        cx.simulate_input("Again.");
        cx.simulate_keystrokes("enter");
        wait(cx, "why it was not sent", |cx| {
            notice(&view, cx) == format!("not sent to mckean@north: {NO_MAIL}")
        });
        cx.run_until_parked();
        assert_eq!(typed(&view, cx), "Again.");
        assert_eq!(yard.asked("MailPeer").len(), 2);
        assert_eq!(yard.asked("Mail").len(), 1);
        assert_eq!(texts(&view, cx), before);
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
    fn a_code_block_s_copy_control_copies_it_exactly_and_changes_no_selection(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let short = "```\ncargo test\n```";
        let code = format!(
            "  deploy --all\n\techo done\n\n{}",
            "echo a very long line that wraps in the window ".repeat(20)
        );
        let long = format!("```sh\n{code}\n```");
        let two = "First:\n```\none\n```\nSecond:\n```\ntwo\n```";
        mails(&yard, &[short, &long, two]);
        let (view, cx) = opened(&yard, &[short, &long, two], cx);
        cx.write_to_clipboard(ClipboardItem::new_string("before".into()));

        // The long line wraps: one block, much higher than the short one's.
        let (one, wrapped) = (text_bounds(cx, 12), text_bounds(cx, 13));
        assert!(wrapped.size.height > one.size.height * 2., "{wrapped:?}");

        // Something selected in another message: a click on a copy control
        // changes no selection, anywhere.
        drag(
            cx,
            gpui::point(one.left(), one.top() + px(2.)),
            gpui::point(one.right() - px(2.), one.bottom() - px(2.)),
        );
        assert_eq!(selected(&view, cx).as_deref(), Some("cargo test"));
        assert_eq!(clipboard(cx).as_deref(), Some("before"));

        // Each block's control copies its own block byte for byte: the
        // tab, indentation, empty line and long wrapped line, without the
        // fences or the language word.
        let chip = |cx: &mut VisualTestContext, seq: u64, ix: usize| {
            view.read_with(cx, |v, cx| v.selection.read(cx).chips_in(seq))[ix]
                .1
                .center()
        };
        let at = chip(cx, 13, 0);
        cx.simulate_click(at, gpui::Modifiers::default());
        assert_eq!(clipboard(cx).as_deref(), Some(code.as_str()));
        assert_eq!(selected(&view, cx).as_deref(), Some("cargo test"));
        let at = chip(cx, 14, 1);
        cx.simulate_click(at, gpui::Modifiers::default());
        assert_eq!(clipboard(cx).as_deref(), Some("two"));
        assert_eq!(selected(&view, cx).as_deref(), Some("cargo test"));
        let at = chip(cx, 14, 0);
        cx.simulate_click(at, gpui::Modifiers::default());
        assert_eq!(clipboard(cx).as_deref(), Some("one"));

        // The control that was clicked reads "copied", then "copy" again:
        // only the last one does.
        let copied = |cx: &mut VisualTestContext, seq: u64| {
            view.read_with(cx, |v, cx| v.selection.read(cx).copied_in(seq))
        };
        assert_eq!(copied(cx, 14), Some(1));
        assert_eq!(copied(cx, 13), None);
        cx.executor().advance_clock(Duration::from_secs(3));
        cx.run_until_parked();
        assert_eq!(copied(cx, 14), None);
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

    /// What is typed with line breaks is one mail, and the yard gets them:
    /// in the text of the call, and kept while another conversation is
    /// open.
    #[gpui::test]
    fn a_mail_with_line_breaks_arrives_with_them(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        mails(&yard, &[LINK]);
        let (view, cx) = opened(&yard, &[LINK], cx);

        cx.simulate_input("Land it today:");
        cx.simulate_keystrokes("shift-enter");
        cx.simulate_input("- the links");
        cx.simulate_keystrokes("shift-enter shift-enter");
        cx.simulate_input("Then tell me.");
        let text = "Land it today:\n- the links\n\nThen tell me.";
        assert_eq!(typed(&view, cx), text);
        assert_eq!(yard.asked("Mail"), Vec::<serde_json::Value>::new());
        // Four rows, and the box as high.
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, cx| v.line.read(cx).rows()), (4, 4));

        // Another conversation and back: the draft has its line breaks.
        cx.simulate_keystrokes("tab");
        assert_eq!(typed(&view, cx), "");
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(typed(&view, cx), text);

        cx.simulate_keystrokes("enter");
        wait(cx, "the mail of four lines", |_| {
            yard.asked("Mail") == [json!({"to": "yardmaster", "text": text, "actor": "jo"})]
        });
        assert_eq!(typed(&view, cx), "");
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

    /// The words of a card are typed in the same line as a mail: shift
    /// and Enter break the line, the words go with their line break, and
    /// the arrows are the words' while they have a row to go to.
    #[gpui::test]
    fn the_words_of_a_card_take_line_breaks_and_the_arrows_choose_at_their_ends(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_three());
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

        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("2");
        cx.simulate_keystrokes("i");
        cx.simulate_input("after Friday,");
        cx.simulate_keystrokes("shift-enter");
        cx.simulate_input("the freeze is over then");
        assert_eq!(typed(&view, cx), "after Friday,\nthe freeze is over then");
        // The words are two rows, and the field as high.
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, cx| v.line.read(cx).rows()), (2, 2));

        // Up is the words' from the second row: the option stays. From the
        // first row it chooses, and down from the last.
        cx.simulate_keystrokes("up");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 2);
        cx.simulate_keystrokes("up");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 1);
        cx.simulate_keystrokes("down");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 1);
        cx.simulate_keystrokes("down");
        assert_eq!(chosen_of(&view, cx, "bead-a", 3), 2);

        // Enter sends the pick with the words as typed.
        cx.simulate_keystrokes("enter");
        wait(cx, "the pick with words of two lines", |_| {
            yard.asked("AnswerAsk")
                == [json!({"bead": "bead-a", "ask": 3, "option": 2,
                           "words": "after Friday,\nthe freeze is over then", "actor": "jo"})]
        });
        assert_eq!(typed(&view, cx), "");
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
    /// words answer. Ask 11's pick is `pick` with `label`. The one put aside
    /// returns in a year no run of the tests reaches: the window compares
    /// that time with the clock.
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
             "return_at": "2099-10-13T09:30:00Z"},
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

    /// A put-aside ask whose return is past while the yard still lists it as
    /// put aside: the queue is read once more for it and no more. Read again
    /// for every answer that still lists it, the window asked without end.
    #[gpui::test]
    fn a_return_that_is_past_reads_the_queue_once(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [
                {"bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
                 "ask": 13, "question": "Rebuild the index tonight?", "asker": "mayor",
                 "since": "2000-01-01T11:07:00Z", "state": "put aside",
                 "return_at": "2000-01-02T09:30:00Z"}
            ]}})
            .to_string(),
        );
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        // Once as the yard is opened, and once for the return that is past.
        wait(cx, "the queue read for the return", |_| {
            yard.asked("UserAsks").len() == 2
        });
        for _ in 0..20 {
            cx.run_until_parked();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(yard.asked("UserAsks").len(), 2);
        view.read_with(cx, |v, _| {
            assert_eq!(v.model.as_ref().unwrap().decided_count(), 1);
        });
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

    /// A conversation name longer than the row ends in an ellipsis before
    /// the unread badge, with a gap between them. The ellipsis itself is
    /// drawn by GPUI's truncate(), which the test cannot read back; what
    /// it pins is why the ellipsis appears. The name's column fills the
    /// row, so a short name's column is as wide as a long name's. Sized
    /// to its content instead, the name is measured full-width first, and
    /// truncate() then cuts it hard, with no ellipsis.
    #[gpui::test]
    fn a_long_conversation_name_ends_before_the_unread_badge_with_a_gap(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        // A long name and a short one, neither open, each with one unread
        // mail; the yardmaster's conversation is open, as usual. Both
        // badges read 1, so both rows leave the name the same width.
        let long_name = "a-crew-member-with-a-deliberately-long-name";
        yard.answer(
            "Crew",
            &json!({"result": [
                {"name": "yardmaster", "enabled": true, "state": "running", "status": "idle"},
                {"name": long_name, "enabled": true, "state": "running", "status": "idle"},
                {"name": "al", "enabled": true, "state": "stopped"}
            ]})
            .to_string(),
        );
        yard.answer(
            "UserInbox",
            &json!({"result": {"mail": [
                {"seq": 12, "from": long_name, "to": "jo", "text": "The first of many words.", "read": false},
                {"seq": 13, "from": "al", "to": "jo", "text": "Short.", "read": false}
            ], "unread": 2, "last": 13, "from": [long_name, "al"]}})
            .to_string(),
        );
        yard.answer("Conversation", r#"{"result": []}"#);
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the unread rows", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.unread(long_name) == 1 && m.unread("al") == 1)
            })
        });
        let long = bounds_of(cx, &format!("row name {long_name}"));
        let badge = bounds_of(cx, &format!("row unread {long_name}"));
        assert!(
            long.right() < badge.left(),
            "the long name touches the badge: {long:?} {badge:?}"
        );
        let short = bounds_of(cx, "row name al");
        assert_eq!(
            short.size.width, long.size.width,
            "the name's column hugs its content instead of filling the row"
        );
    }

    /// A kept path longer than the settings pane ends in an ellipsis
    /// before the back button, with a gap between them, at the narrowest
    /// window and at a wide one. The ellipsis itself is drawn by GPUI's
    /// truncate(), which the test cannot read back; what it pins is why
    /// the ellipsis appears. The kept path's column fills the head, so a
    /// short path's column is as wide as a long path's. Sized to its
    /// content instead, the path is measured full-width first, and
    /// truncate() then cuts it hard, with no ellipsis.
    #[gpui::test]
    fn a_long_kept_path_ends_before_the_back_button_with_a_gap(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let long = dir
            .path()
            .join(format!("{}/settings.json", "long-".repeat(30)));
        // Short enough to fit the head at the narrowest window, so its
        // column hugs it there unless the column fills the head.
        let short = std::path::PathBuf::from("s.json");
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let (view, cx) = opened_kept(&yard, &long, cx);
        view.update_in(cx, |v, window, cx| v.toggle_settings(window, cx));
        assert!(settings_open(&view, cx));
        // The narrowest window the application opens (main.rs), and a wide
        // one: the path overflows the pane at both.
        for (wide, high) in [(560., 360.), (1200., 800.)] {
            cx.simulate_resize(gpui::size(px(wide), px(high)));
            view.update(cx, |v, cx| {
                v.kept = Some(long.clone());
                cx.notify();
            });
            let kept = bounds_of(cx, "settings kept");
            let back = bounds_of(cx, "settings back");
            assert!(
                kept.right() < back.left(),
                "at {wide} px the long path touches the back button: {kept:?} {back:?}"
            );
            view.update(cx, |v, cx| {
                v.kept = Some(short.clone());
                cx.notify();
            });
            let brief = bounds_of(cx, "settings kept");
            assert_eq!(
                brief.size.width, kept.size.width,
                "at {wide} px the kept path's column hugs its content instead of filling the head"
            );
        }
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

    /// A pick made with the window's keys leaves the focus on the window;
    /// the conversation it leads back to has it in the line again.
    #[gpui::test]
    fn a_pick_by_key_leads_back_to_the_conversation_with_the_focus_in_its_line(
        cx: &mut TestAppContext,
    ) {
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
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Conversation("yardmaster".into()), window, cx)
        });
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Decision("bead-a".into()), window, cx)
        });
        cx.simulate_keystrokes("1");
        cx.simulate_keystrokes("enter");
        wait(cx, "the conversation again", |cx| {
            open(&view, cx) == Some(Selected::Conversation("yardmaster".into()))
        });
        cx.run_until_parked();
        cx.simulate_input("x");
        assert_eq!(typed(&view, cx), "x");
    }

    /// One bead with one open ask, for asking about this.
    fn asks_about(bead: &str, asker: &str, question: &str) -> String {
        json!({"result": {"open": [
            {"bead": bead, "bead_title": "The title", "topic": "",
             "ask": 3, "question": question, "asker": asker,
             "since": "2026-10-10T10:00:00Z"}
        ], "decided": []}})
        .to_string()
    }

    /// That bead whole, with two options and the recommendation given.
    fn bead_about(bead: &str, asker: &str, question: &str, recommend: u64) -> String {
        json!({"result": {
            "id": bead, "title": "The title",
            "body": "The body of the bead.",
            "stage": "new", "labels": [],
            "notes": [],
            "asks": [{
                "ask": 3, "to": "jo", "asker": asker,
                "since": "2026-10-10T10:00:00Z",
                "question": question,
                "context": ["The books close at six."],
                "options": [
                    {"label": "Close them", "consequence": "the books close"},
                    {"label": "You decide", "consequence": format!("leave it to {asker}")}
                ],
                "recommend": recommend, "why": "six is six",
                "state": "open"
            }]
        }})
        .to_string()
    }

    fn open_of(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Option<Selected> {
        view.read_with(cx, |v, _| {
            v.model.as_ref().and_then(|m| m.selected().cloned())
        })
    }

    fn card_keys_of(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        bead: &str,
        ask: u64,
    ) -> String {
        view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            let now = chrono::Utc::now();
            let card = model
                .decision_detail(bead, &now)
                .unwrap()
                .asks
                .into_iter()
                .find(|a| a.ask == ask)
                .unwrap();
            let writable = model.can_write(&card.asker);
            decision_keys(&card, model::foreign_bead(bead), writable)
        })
    }

    #[gpui::test]
    fn ask_about_this_opens_the_asker_with_the_card_named_and_the_ask_stays_open(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &asks_about("bead-a", "yardmaster", "Move them now?"),
        );
        yard.answer(
            "Bead",
            &bead_about("bead-a", "yardmaster", "Move them now?", 1),
        );
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
        wait(cx, "the decision", |cx| {
            open_of(&view, cx) == Some(Selected::Decision("bead-a".into()))
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));

        // The control stands beside Later and Decline, and the words field
        // still takes a note alone.
        assert!(drawn(cx, "ask bead-a 3"));
        assert_eq!(
            placeholder(&view, cx),
            "Words with the pick, or alone as a note"
        );

        // The `a` key opens the yardmaster with the card named first in
        // the draft, the caret on the empty line after it. Nothing is
        // sent.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("a");
        wait(cx, "the yardmaster", |cx| {
            open_of(&view, cx) == Some(Selected::Conversation("yardmaster".into()))
        });
        assert_eq!(typed(&view, cx), "re bead-a \"Move them now?\"\n\n");
        assert!(yard.asked("Mail").is_empty());
        assert!(yard.asked("MailPeer").is_empty());
        cx.simulate_input("a question");
        assert_eq!(
            typed(&view, cx),
            "re bead-a \"Move them now?\"\na question\n"
        );

        // The ask stays open: its row is one step back, and nothing was
        // answered, deferred or declined.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Decision("bead-a".into()), window, cx)
        });
        assert_eq!(current_ask_of(&view, cx), 3);
        assert!(yard.asked("AnswerAsk").is_empty());
        assert!(yard.asked("DeferAsk").is_empty());
        assert!(yard.asked("DeclineAsk").is_empty());

        // And a pick from there leads back to the conversation, with the
        // draft as it was left: the two do not fight.
        cx.simulate_keystrokes("1");
        cx.simulate_keystrokes("enter");
        wait(cx, "the pick", |_| {
            yard.asked("AnswerAsk")
                == [json!({"bead": "bead-a", "ask": 3, "option": 1, "actor": "jo"})]
        });
        wait(cx, "the yardmaster again", |cx| {
            open_of(&view, cx) == Some(Selected::Conversation("yardmaster".into()))
        });
        assert_eq!(
            typed(&view, cx),
            "re bead-a \"Move them now?\"\na question\n"
        );
    }

    #[gpui::test]
    fn a_foreign_card_asks_its_asker_and_words_alone_are_the_yard_s_refusal(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &asks_about("bead-f@north", "clerk@north", "Close the books tonight?"),
        );
        yard.answer(
            "Bead",
            &bead_about("bead-f@north", "clerk@north", "Close the books tonight?", 0),
        );
        // The yard refuses a plain note on another yard's bead, in its
        // words, and names whom to mail instead.
        yard.answer(
            "AddNote",
            r#"{"error": {"code": "error", "message": "bead bead-f@north is a question north asked: words for the asker go as mail, yardr mail clerk@north \"...\""}}"#,
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
        wait(cx, "the decision", |cx| {
            open_of(&view, cx) == Some(Selected::Decision("bead-f@north".into()))
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-f@north"));

        // The words field says what holds on a foreign card, and the keys
        // promise nothing of Enter without a pick: asking about this is
        // how to talk instead.
        assert_eq!(placeholder(&view, cx), "Words with the pick");
        assert_eq!(
            card_keys_of(&view, cx, "bead-f@north", 3),
            "1-2 pick · ↑↓ choose · i words · a ask about this"
        );

        // With words already in the clerk's draft, the button opens them
        // with the card named before what was there.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Conversation("clerk@north".into()), window, cx)
        });
        cx.simulate_input("half a thought");
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Decision("bead-f@north".into()), window, cx)
        });
        click_on(cx, "ask bead-f@north 3");
        wait(cx, "the clerk", |cx| {
            open_of(&view, cx) == Some(Selected::Conversation("clerk@north".into()))
        });
        assert_eq!(
            typed(&view, cx),
            "re bead-f@north \"Close the books tonight?\"\n\nhalf a thought"
        );

        // A second click names the card once: the draft is as it was.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Decision("bead-f@north".into()), window, cx)
        });
        click_on(cx, "ask bead-f@north 3");
        wait(cx, "the clerk again", |cx| {
            open_of(&view, cx) == Some(Selected::Conversation("clerk@north".into()))
        });
        assert_eq!(
            typed(&view, cx),
            "re bead-f@north \"Close the books tonight?\"\n\nhalf a thought"
        );

        // Words alone on the foreign card go as a plain note, which the
        // yard refuses: they are back in the line, with the yard's words,
        // and the ask stays open.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Decision("bead-f@north".into()), window, cx)
        });
        cx.simulate_keystrokes("i");
        cx.simulate_input("what about June?");
        cx.simulate_keystrokes("enter");
        wait(cx, "the note refused", |_| {
            yard.asked("AddNote")
                == [json!({"bead": "bead-f@north", "text": "what about June?", "actor": "jo"})]
        });
        wait(cx, "the refusal", |cx| {
            notice(&view, cx).starts_with("not noted: bead bead-f@north")
        });
        assert_eq!(
            notice(&view, cx),
            "not noted: bead bead-f@north is a question north asked: words for the asker go as mail, yardr mail clerk@north \"...\""
        );
        assert_eq!(typed(&view, cx), "what about June?");
        assert_eq!(current_ask_of(&view, cx), 3);
    }

    #[gpui::test]
    fn an_asker_with_no_conversation_says_so_instead_of_opening_one(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &asks_about("bead-u", "sage@east", "Name the release?"),
        );
        yard.answer(
            "Bead",
            &bead_about("bead-u", "sage@east", "Name the release?", 1),
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
        wait(cx, "the decision", |cx| {
            open_of(&view, cx) == Some(Selected::Decision("bead-u".into()))
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-u"));

        // No button: the control says the asker cannot be written to, and
        // the keys name no key for it.
        assert!(!drawn(cx, "ask bead-u 3"));
        assert!(drawn(cx, "ask unreachable bead-u 3"));
        assert_eq!(
            card_keys_of(&view, cx, "bead-u", 3),
            "enter picks 1 · 1-2 pick · ↑↓ choose · i words"
        );

        // The key says so too, and opens no conversation: the card stays
        // open.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("a");
        cx.run_until_parked();
        assert_eq!(
            notice(&view, cx),
            "sage@east cannot be written to from here: the link does not expose them"
        );
        assert_eq!(
            open_of(&view, cx),
            Some(Selected::Decision("bead-u".into()))
        );
        assert_eq!(current_ask_of(&view, cx), 3);
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
            decision_keys(&ask, false, true)
        });
        assert_eq!(
            keys,
            "enter picks 1 · 1-3 pick · ↑↓ choose · i words · a ask about this"
        );

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

    /// Words alone on an ordinary card have a Send button where Enter sends
    /// them as a note: no pick and none recommended. It is disabled while
    /// the line is empty; with a recommendation and no pick there is still
    /// no button that would send it unasked. The look is untried.
    #[gpui::test]
    fn words_alone_have_a_send_button_where_enter_sends_a_note(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_three());
        yard.answer("Bead", &bead_three());
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
        wait(cx, "the decision", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Decision("bead-a".into())))
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));

        // With a recommendation and no pick there is no send button, even
        // with words typed: never one that sends it unasked.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 3, cx));
        cx.update(|window, cx| window.focus(&view.read(cx).line.focus_handle(cx)));
        cx.simulate_input("some words");
        assert!(!drawn(cx, "send bead-a 3"));

        // With no pick and none recommended the words go as a note, from
        // the button as from Enter.
        view.update(cx, |v, cx| v.select_ask("bead-a".into(), 5, cx));
        assert!(drawn(cx, "send bead-a 5"));
        assert_eq!(typed(&view, cx), "");
        click_on(cx, "send bead-a 5");
        cx.run_until_parked();
        assert_eq!(yard.asked("AddNote"), Vec::<Value>::new());
        cx.update(|window, cx| window.focus(&view.read(cx).line.focus_handle(cx)));
        cx.simulate_input("what about the docs links?");
        click_on(cx, "send bead-a 5");
        wait(cx, "the note", |_| {
            yard.asked("AddNote")
                == [json!({"bead": "bead-a", "text": "what about the docs links?", "actor": "jo"})]
        });
        assert_eq!(typed(&view, cx), "");
    }

    /// One open ask that wants words, on its own bead.
    fn asks_words_only() -> String {
        json!({"result": {"open": [
            {"bead": "bead-w", "bead_title": "Greet", "topic": "",
             "ask": 9, "question": "Who greets the newcomer?", "asker": "yardmaster",
             "since": "2026-10-12T11:07:00Z"}
        ], "decided": []}})
        .to_string()
    }

    fn bead_words_only() -> String {
        json!({"result": {
            "id": "bead-w", "title": "Greet",
            "body": "The body of bead-w.",
            "stage": "new", "labels": [],
            "notes": [],
            "asks": [{
                "ask": 9, "to": "jo", "asker": "yardmaster",
                "since": "2026-10-12T11:07:00Z",
                "question": "Who greets the newcomer?",
                "context": ["Somebody should."],
                "options": [],
                "state": "open"
            }]
        }})
        .to_string()
    }

    /// A words-wanted card sends the answer from its Send button, open and
    /// dealt with, as Enter does. The look is untried.
    #[gpui::test]
    fn a_words_wanted_card_sends_the_answer_from_its_send_button(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_words_only());
        yard.answer("Bead", &bead_words_only());
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
        wait(cx, "the decision", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Decision("bead-w".into())))
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-w"));
        assert!(drawn(cx, "send bead-w 9"));
        assert_eq!(typed(&view, cx), "");
        click_on(cx, "send bead-w 9");
        cx.run_until_parked();
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
        cx.update(|window, cx| window.focus(&view.read(cx).line.focus_handle(cx)));
        cx.simulate_input("warmly");
        click_on(cx, "send bead-w 9");
        wait(cx, "the answer", |_| {
            yard.asked("AnswerAsk")
                == [
                    json!({"bead": "bead-w", "ask": 9, "option": 0, "words": "warmly", "actor": "jo"}),
                ]
        });
        assert_eq!(typed(&view, cx), "");
    }

    /// A dealt-with words answer is sent again from its Send button, as
    /// Enter sends it. The look is untried.
    #[gpui::test]
    fn a_decided_words_answer_is_sent_again_from_its_send_button(cx: &mut TestAppContext) {
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
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        open_decided(&view, cx, "bead-d", 16);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-d"));
        assert!(drawn(cx, "send bead-d 16"));
        assert_eq!(typed(&view, cx), "");
        click_on(cx, "send bead-d 16");
        cx.run_until_parked();
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
        cx.simulate_keystrokes("i");
        cx.simulate_input("kindly");
        click_on(cx, "send bead-d 16");
        wait(cx, "the answer again", |_| {
            yard.asked("AnswerAsk")
                == [
                    json!({"bead": "bead-d", "ask": 16, "option": 0, "words": "kindly", "actor": "jo"}),
                ]
        });
        assert_eq!(typed(&view, cx), "");
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
        view.read_with(cx, |v, _| v.settings.clone())
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
            ..Settings::default()
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
        // The settings have no text of their own to type: shift and Enter
        // break no line there, and Enter sends nothing.
        cx.simulate_keystrokes("shift-enter enter");
        assert_eq!(typed(&view, cx), "hal");
        assert_eq!(yard.asked("Mail"), Vec::<serde_json::Value>::new());

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
            ..Settings::default()
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

    /// With the yard away the left column has its head and its foot: the
    /// settings are reached by the same key, and by the control there.
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
                look: Look::Light,
                ..Settings::default()
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
                let set = Settings {
                    text,
                    ..v.settings.clone()
                };
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
            let yard = bounds_of(cx, "which yard");
            assert!(
                logo.right() < yard.left() && yard.right() <= first.right() + rem,
                "at {text} px the head of the column does not hold: {logo:?} {yard:?}"
            );
            let control = bounds_of(cx, "settings");
            assert!(
                yard.bottom() <= control.top(),
                "at {text} px the button shares the head's row: {yard:?} {control:?}"
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

    /// The head keeps its line and the button has the foot of the list: at
    /// the narrowest window the name and this machine's are whole, the line
    /// ends at the column's edge, and the button is under the rows and opens
    /// the settings by a click and by its key.
    #[gpui::test]
    fn the_button_has_the_foot_and_the_head_its_line(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the yard", |cx| {
            view.read_with(cx, |v, _| v.model.is_some())
        });
        // The narrowest window the application opens (main.rs).
        cx.simulate_resize(gpui::size(px(560.), px(360.)));
        let head = bounds_of(cx, "which yard");
        let rows = bounds_of(cx, "rows");
        let control = bounds_of(cx, "settings");
        assert!(
            head.bottom() <= control.top(),
            "the head's line shares a row with the button: {head:?} {control:?}"
        );
        assert!(
            rows.bottom() <= control.top()
                && rows.left() <= control.left()
                && control.right() <= rows.right(),
            "the button is not at the foot of the column: {rows:?} {control:?}"
        );
        // The line has the column's width: it ends at the column's edge,
        // short of the head's own padding only.
        let rem = settings_of(&view, cx).rem();
        assert!(
            (head.right() - (rows.right() - rem * 0.75)).abs() < px(1.),
            "the head's line ends before the column's edge: {head:?} {rows:?}"
        );
        // Whole: the line as shaped fits where it is drawn. Shaped as the
        // window shapes it (place()).
        // The fixture's short login would not catch the real operator name
        // that exposed this line's last-glyph truncation.
        let line = view.read_with(cx, |v, _| format!("mckean · {}", v.which_yard_short()));
        let width = cx.update(|window, _| {
            let run = gpui::TextRun {
                len: line.len(),
                font: window.text_style().font(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(line.clone().into(), rem * 0.8, &[run], None)
                .width
        });
        assert!(
            width <= head.size.width,
            "at the narrowest window the head's line is cut: {line:?} is {width:?} in {head:?}"
        );
        // The button opens the settings, and its key shuts them again.
        click_on(cx, "settings");
        assert!(settings_open(&view, cx));
        cx.simulate_keystrokes("secondary-,");
        assert!(!settings_open(&view, cx));
    }

    /// A yard at its listener, told from `a_yard_of_jo` by its mail.
    /// A system for a test: what it was asked and shown is read here, and
    /// what it answers and what a person activates is said here.
    #[derive(Clone)]
    struct Told {
        answer: Rc<RefCell<Permission>>,
        asked: Rc<Cell<usize>>,
        shown: Rc<RefCell<Vec<notify::Telling>>>,
        activated: (
            async_channel::Sender<notify::Activated>,
            async_channel::Receiver<notify::Activated>,
        ),
    }

    impl Told {
        fn new() -> Told {
            Told {
                answer: Rc::new(RefCell::new(Permission::Allowed)),
                asked: Rc::new(Cell::new(0)),
                shown: Rc::default(),
                activated: async_channel::unbounded(),
            }
        }

        /// What was shown since the last look, once a burst's time is over.
        fn shown(&self, cx: &mut VisualTestContext) -> Vec<(String, String, Selected)> {
            self.tellings(cx)
                .into_iter()
                .map(|telling| (telling.title, telling.body, telling.target))
                .collect()
        }

        /// The same, each with the yard it tells of.
        fn tellings(&self, cx: &mut VisualTestContext) -> Vec<notify::Telling> {
            cx.run_until_parked();
            cx.executor().advance_clock(notify::BURST);
            cx.run_until_parked();
            std::mem::take(&mut *self.shown.borrow_mut())
        }
    }

    impl notify::System for Told {
        fn ask(&self) -> async_channel::Receiver<Permission> {
            self.asked.set(self.asked.get() + 1);
            let (tell, told) = async_channel::bounded(1);
            tell.try_send(self.answer.borrow().clone()).unwrap();
            told
        }

        fn show(&self, telling: &notify::Telling) {
            self.shown.borrow_mut().push(telling.clone());
        }

        fn activated(&self) -> async_channel::Receiver<notify::Activated> {
            self.activated.1.clone()
        }
    }

    fn mail_event(seq: u64, from: &str, text: &str) -> String {
        json!({"seq": seq, "kind": "mail", "actor": from, "at": "2026-10-08T09:31:00Z",
            "data": {"to": "jo", "to_user": true, "text": text}})
        .to_string()
    }

    fn unread_of(view: &Entity<Footplate>, cx: &mut VisualTestContext, from: &str) -> usize {
        view.read_with(cx, |v, _| v.model.as_ref().unwrap().unread(from))
    }

    #[gpui::test]
    fn the_system_tells_of_what_arrives_where_the_window_does_not_show_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let told = Told::new();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::new(yard.client(), "jo".into(), window, cx)
                .kept_in(Some(file.clone()), cx)
                .notified_by(Box::new(told.clone()), cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "the yardmaster's conversation", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        // The window is behind another: everything that arrives is news.
        cx.deactivate_window();
        assert!(!view.read_with(cx, |v, _| v.front));

        // Nobody chose: the system is asked nothing and shows nothing,
        // not for what was unread at the start and not for what arrives.
        yard.emit(&mail_event(21, "assembly", "Built."));
        wait(cx, "the unread mail", |cx| {
            unread_of(&view, cx, "assembly") == 1
        });
        assert_eq!(told.shown(cx), []);
        assert_eq!(told.asked.get(), 0);

        // Switched on in the settings: the system is asked, once, and what
        // it refuses is said there in its words.
        *told.answer.borrow_mut() = Permission::Refused("The system says no.".into());
        cx.simulate_keystrokes("cmd-,");
        assert!(!drawn(cx, "notify mail"));
        click_on(cx, "notify on");
        assert!(settings_of(&view, cx).notifications.on);
        assert!(Settings::read(&file).notifications.on);
        assert_eq!(told.asked.get(), 1);
        assert!(drawn(cx, "notify said"));
        assert_eq!(
            view.read_with(cx, |v, _| v.permission.clone()),
            Some(Permission::Refused("The system says no.".into()))
        );
        // Off and on again, it is asked again, and what it says now
        // stands for what it said before.
        click_on(cx, "notify off");
        assert!(!Settings::read(&file).notifications.on);
        *told.answer.borrow_mut() = Permission::Allowed;
        click_on(cx, "notify on");
        assert_eq!(told.asked.get(), 2);
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |v, _| v.permission.clone()),
            Some(Permission::Allowed)
        );
        // What tells is chosen kind by kind, and kept.
        click_on(cx, "notify the yard's notices");
        click_on(cx, "notify decisions");
        let chosen = Notify {
            on: true,
            mail: true,
            decisions: false,
            notices: true,
        };
        assert_eq!(settings_of(&view, cx).notifications, chosen);
        assert_eq!(Settings::read(&file).notifications, chosen);
        click_on(cx, "notify the yard's notices");
        click_on(cx, "notify decisions");
        assert_eq!(
            Settings::read(&file).notifications,
            Notify {
                on: true,
                ..Notify::default()
            }
        );
        // Choosing told of nothing that was there before.
        click_on(cx, "settings back");
        assert_eq!(told.shown(cx), []);

        // Behind another window, mail in the open conversation is told of:
        // who it is from, and its first line.
        yard.emit(&mail_event(22, "yardmaster", "Landed.\nThe tests held."));
        wait(cx, "the mail", |cx| texts(&view, cx).len() == 2);
        assert_eq!(
            told.shown(cx),
            [(
                "yardmaster".to_string(),
                "Landed.".to_string(),
                Selected::Conversation("yardmaster".into())
            )]
        );

        // Turning notifications off during a pending burst cancels it.
        yard.emit(&mail_event(23, "assembly", "This should not arrive."));
        wait(cx, "the pending mail", |cx| {
            unread_of(&view, cx, "assembly") == 2
        });
        cx.simulate_keystrokes("cmd-,");
        click_on(cx, "notify off");
        assert_eq!(told.shown(cx), []);
        click_on(cx, "notify on");
        click_on(cx, "settings back");

        // In front on that conversation it is read where it stands.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.front));
        yard.emit(&mail_event(24, "yardmaster", "And pushed."));
        wait(cx, "the mail", |cx| texts(&view, cx).len() == 3);
        assert_eq!(told.shown(cx), []);
        // With the settings where the conversation would be, it is not.
        cx.simulate_keystrokes("cmd-,");
        yard.emit(&mail_event(25, "yardmaster", "Shall I go on?"));
        wait(cx, "the mail", |cx| texts(&view, cx).len() == 4);
        assert_eq!(told.shown(cx).len(), 1);
        cx.simulate_keystrokes("cmd-,");

        // In front on another conversation: a burst is one notification.
        for (seq, text) in [(26, "One."), (27, "Two."), (28, "Three.")] {
            yard.emit(&mail_event(seq, "assembly", text));
        }
        wait(cx, "the burst", |cx| unread_of(&view, cx, "assembly") == 5);
        assert_eq!(
            told.shown(cx),
            [(
                "3 from assembly".to_string(),
                "Three.".to_string(),
                Selected::Conversation("assembly".into())
            )]
        );
        // And the next after it is one of its own.
        yard.emit(&mail_event(29, "assembly", "Four."));
        wait(cx, "the mail", |cx| unread_of(&view, cx, "assembly") == 6);
        assert_eq!(told.shown(cx).len(), 1);

        // What the person wrote, here or elsewhere, tells of nothing.
        cx.deactivate_window();
        yard.emit(r#"{"seq": 30, "kind": "mail", "actor": "jo", "data": {"to": "yardmaster", "text": "Go on."}, "at": "2026-10-08T09:31:00Z"}"#);
        wait(cx, "the user's own mail", |cx| texts(&view, cx).len() == 5);
        assert_eq!(told.shown(cx), []);

        // A new decision is told of, by who asks and the question.
        yard.answer(
            "UserAsks",
            r#"{"result": {"open": [{"bead": "bead-a", "bead_title": "Move the links", "topic": "",
                "ask": 3, "question": "Move them now?", "asker": "yardmaster",
                "since": "2026-10-10T10:00:00Z"}], "decided": []}}"#,
        );
        yard.emit(r#"{"seq": 31, "kind": "noted", "actor": "yardmaster", "bead": "bead-a", "data": {"note": 3}, "at": "2026-10-08T09:31:00Z"}"#);
        wait(cx, "the decision", |cx| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().decisions_count() == 1)
        });
        assert_eq!(
            told.shown(cx),
            [(
                "yardmaster asks".to_string(),
                "Move them now?".to_string(),
                Selected::Decision("bead-a".into())
            )]
        );

        // Activating a notification brings the window forward on its row,
        // out of the settings too.
        cx.simulate_keystrokes("cmd-,");
        assert!(settings_open(&view, cx));
        let open = view.read_with(cx, |v, _| v.yard.clone());
        told.activated
            .0
            .try_send((open.clone(), Selected::Conversation("assembly".into())))
            .unwrap();
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.front));
        assert!(!settings_open(&view, cx));
        assert_eq!(
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().selected().cloned()),
            Some(Selected::Conversation("assembly".into()))
        );
        // A row that is gone since leaves open what was.
        told.activated
            .0
            .try_send((open, Selected::Decision("bead-gone".into())))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().selected().cloned()),
            Some(Selected::Conversation("assembly".into()))
        );

        // Kept on, the next start asks the system at once.
        let (again, cx) = {
            let (client, file, told) = (yard.client(), file.clone(), told.clone());
            let asked = told.asked.get();
            let (again, cx) = cx.add_window_view(|window, cx| {
                Footplate::new(client, "jo".into(), window, cx)
                    .notified_by(Box::new(told.clone()), cx)
                    .kept_in(Some(file), cx)
            });
            assert_eq!(told.asked.get(), asked + 1);
            (again, cx)
        };
        cx.run_until_parked();
        assert_eq!(
            again.read_with(cx, |v, _| v.permission.clone()),
            Some(Permission::Allowed)
        );
    }

    /// A window on this machine's yard and a second home's, both followed,
    /// with `told` for its notifications, which are on: the mail and the
    /// decisions tell, and the notices where `notices` says so.
    fn opened_on_two_yards<'a>(
        dir: &std::path::Path,
        (near, second): (&Yard, &Yard),
        told: &Told,
        notices: bool,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        let file = dir.join("footplate/settings.json");
        let mut set = Settings::default()
            .with_yard(
                None,
                settings::Yard::near("second home", second.socket().to_str().unwrap()),
                None,
            )
            .unwrap();
        set.notifications.on = true;
        set.notifications.notices = notices;
        set.write(&file).unwrap();
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let told = told.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = Footplate::opened(env, "jo".into(), Some(file.clone()), window, cx)
                .notified_by(Box::new(told), cx);
            window.focus(&view.line.focus_handle(cx));
            view
        });
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        wait(cx, "what waits in the second home's yard", |cx| {
            state_of(&view, cx, "second home") == "2 mail"
        });
        (view, cx)
    }

    /// Every yard of the settings is followed at once. What arrives in one
    /// that is not open is counted in the switcher as it arrives, with no
    /// switch, and nothing of it is marked read. The system tells of it by
    /// the settings' kinds, in a notification that names its yard, and a
    /// burst of two yards says each yard's part. Activated, a notification
    /// opens its yard on its row, as the yard stands: it is not read from
    /// nothing.
    #[gpui::test]
    fn a_yard_that_is_not_open_counts_what_arrives_and_its_notification_opens_it(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let told = Told::new();
        let (view, cx) = opened_on_two_yards(dir.path(), (&near, &second), &told, true, cx);
        let open_row = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().selected().cloned())
        };
        let tellings = |cx: &mut VisualTestContext| -> Vec<(String, String, String, Selected)> {
            told.tellings(cx)
                .into_iter()
                .map(|t| (t.title, t.body, t.yard, t.target))
                .collect()
        };
        let telling = |title: &str, body: &str, yard: &str, target: Selected| {
            (
                title.to_string(),
                body.to_string(),
                yard.to_string(),
                target,
            )
        };
        // What was unread when the window started tells of nothing.
        assert_eq!(tellings(cx), []);
        let (loads, streams) = (second.asked("Head").len(), second.streams_asked().len());
        assert_eq!(streams, 1);

        // Mail arrives in the yard that is not open, with the window in
        // front on the other: it is counted where the switcher says what
        // waits there, and told of as that yard's. Nothing of it is shown,
        // so the yard is not told it was read.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.front));
        second.emit(&mail_event(21, "assembly", "Built."));
        wait(cx, "the mail in the second home's yard", |cx| {
            state_of(&view, cx, "second home") == "3 mail"
        });
        assert_eq!(open_yard(&view, cx), "this machine");
        assert_eq!(texts(&view, cx), ["Shall I land it?"]);
        assert_eq!(
            tellings(cx),
            [telling(
                "second home · assembly",
                "Built.",
                "second home",
                Selected::Conversation("assembly".into())
            )]
        );
        assert_eq!(second.asked("AckUserInbox").len(), 0);

        // A card: one notification, naming the yard it waits in.
        second.answer("UserAsks", &asks_one());
        second.emit(r#"{"seq": 31, "kind": "noted", "actor": "yardmaster", "bead": "bead-a", "data": {"note": 3}, "at": "2026-10-08T09:31:00Z"}"#);
        wait(cx, "the card in the second home's yard", |cx| {
            state_of(&view, cx, "second home") == "3 mail, 1 decision"
        });
        assert_eq!(
            tellings(cx),
            [telling(
                "second home · yardmaster asks",
                "Move them now?",
                "second home",
                Selected::Decision("bead-a".into())
            )]
        );

        // A notice of the yard, which the settings here have tell too.
        second.answer("UserNotifications", &notices_many(1));
        second.emit(r#"{"seq": 32, "kind": "created", "actor": "yard", "bead": "yd-shed", "data": {"notified": {"jo": "all"}}, "at": "2026-10-08T09:32:00Z"}"#);
        wait(cx, "the notice in the second home's yard", |cx| {
            state_of(&view, cx, "second home") == "3 mail, 1 decision, 1 notice"
        });
        assert_eq!(
            tellings(cx),
            [telling(
                "second home · yard",
                "notice 0 of the evening's work",
                "second home",
                Selected::Yard
            )]
        );

        // A burst of both yards is one notification, each yard's part by
        // itself: a sender of one name in both is not one sender. It leads
        // to the first, in the yard that arrived in.
        near.emit(&mail_event(21, "assembly", "Built here."));
        wait(cx, "the mail in this machine's yard", |cx| {
            unread_of(&view, cx, "assembly") == 1
        });
        second.emit(&mail_event(33, "assembly", "Tested."));
        second.emit(&mail_event(34, "yardmaster", "Landed."));
        wait(cx, "more mail in the second home's yard", |cx| {
            state_of(&view, cx, "second home") == "5 mail, 1 decision, 1 notice"
        });
        assert_eq!(
            tellings(cx),
            [telling(
                "3 new in 2 yards",
                "this machine: 1 from assembly; second home: 1 from assembly, 1 from yardmaster",
                "this machine",
                Selected::Conversation("assembly".into())
            )]
        );

        // The card's notification is activated: the window shows the yard
        // it waits in, on the card's row. The yard is shown as it was
        // held: it is not read again, and its stream is the one it had.
        told.activated
            .0
            .try_send(("second home".into(), Selected::Decision("bead-a".into())))
            .unwrap();
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.front));
        assert_eq!(open_yard(&view, cx), "second home");
        assert_eq!(open_row(cx), Some(Selected::Decision("bead-a".into())));
        assert_eq!(state_of(&view, cx, "second home"), "open");
        assert_eq!(second.asked("Head").len(), loads);
        assert_eq!(second.streams_asked().len(), streams);
        // The yard that was left is followed still, and says what waits.
        assert_eq!(state_of(&view, cx, "this machine"), "1 mail");
        near.emit(&mail_event(22, "assembly", "And pushed."));
        wait(cx, "mail in the yard that was left", |cx| {
            state_of(&view, cx, "this machine") == "2 mail"
        });
        assert_eq!(
            tellings(cx),
            [telling(
                "this machine · assembly",
                "And pushed.",
                "this machine",
                Selected::Conversation("assembly".into())
            )]
        );

        // A notification of a yard that is listed no longer opens nothing:
        // a row of its name in the open yard is another conversation, and
        // what is typed in it would be sent to the wrong yard.
        let before = open_row(cx);
        told.activated
            .0
            .try_send(("gone".into(), Selected::Conversation("assembly".into())))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(open_yard(&view, cx), "second home");
        assert_eq!(open_row(cx), before);
        // One of the open yard leads to its row, as before.
        told.activated
            .0
            .try_send((
                "second home".into(),
                Selected::Conversation("assembly".into()),
            ))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(open_yard(&view, cx), "second home");
        assert_eq!(
            open_row(cx),
            Some(Selected::Conversation("assembly".into()))
        );
    }

    /// One yard that goes away changes nothing of the others: the stream,
    /// the counts and the sends of the open yard go on while one that is
    /// not open is stopped, and that one is asked again by itself until it
    /// answers. The same the other way: a yard that is not open goes on
    /// counting while the open one is away.
    #[gpui::test]
    fn a_yard_that_goes_away_leaves_the_others_as_they_are_and_is_asked_again(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let told = Told::new();
        let (view, cx) = opened_on_two_yards(dir.path(), (&near, &second), &told, false, cx);

        // The yard that is not open stops.
        let socket = second.socket().to_path_buf();
        second.stop();
        wait(cx, "that the second home's yard is not reached", |cx| {
            state_of(&view, cx, "second home") == "not reached: nothing answers"
        });
        // The open yard shows as it did, on the stream it had: what
        // arrives there arrives, and what is written there is sent.
        assert_eq!(open_yard(&view, cx), "this machine");
        assert_eq!(view.read_with(cx, |v, _| v.down.clone()), None);
        near.emit(&mail_event(21, "yardmaster", "Landed."));
        wait(cx, "the mail in the open yard", |cx| {
            texts(&view, cx) == ["Shall I land it?", "Landed."]
        });
        near.emit(&mail_event(22, "assembly", "Built."));
        wait(cx, "the unread mail in the open yard", |cx| {
            unread_of(&view, cx, "assembly") == 1
        });
        cx.simulate_input("Go on.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the mail to be sent", |_| near.asked("Mail").len() == 1);
        assert_eq!(notice(&view, cx), "");
        assert_eq!(near.streams_asked().len(), 1);
        assert_eq!(
            state_of(&view, cx, "second home"),
            "not reached: nothing answers"
        );

        // It is started again: asked again by itself, it says what waits
        // in it, with no switch to it.
        let second = Yard::start_at(&socket);
        a_second_yard_of_jo(&second);
        wait(cx, "the second home's yard to answer by itself", |cx| {
            state_of(&view, cx, "second home") == "2 mail"
        });
        assert_eq!(open_yard(&view, cx), "this machine");
        assert_eq!(near.streams_asked().len(), 1);

        // The open yard stops: the one that is not open goes on, counts
        // what arrives, and has the system tell of it.
        told.tellings(cx);
        near.stop();
        wait(cx, "that the open yard is away", |cx| {
            view.read_with(cx, |v, _| v.down.is_some())
        });
        second.emit(&mail_event(21, "assembly", "Built."));
        wait(cx, "the mail in the yard that is not open", |cx| {
            state_of(&view, cx, "second home") == "3 mail"
        });
        let tellings = told.tellings(cx);
        let [telling] = tellings.as_slice() else {
            panic!("{tellings:?}");
        };
        assert_eq!(telling.title, "second home · assembly");
        assert_eq!(telling.yard, "second home");
        // And it is shown as it stands, from beside the yard that is away.
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        assert_eq!(
            state_of(&view, cx, "this machine"),
            "not reached: nothing answers"
        );
    }

    fn a_far_yard_of_jo(yard: &Yard) {
        yard.answer("Head", r#"{"result": 40}"#);
        yard.answer(
            "Crew",
            r#"{"result": [{"name": "yardmaster", "enabled": true, "state": "running", "status": "idle"}]}"#,
        );
        yard.answer("Peers", r#"{"result": []}"#);
        yard.answer(
            "Users",
            r#"{"result": [{"name": "jo", "cursor": 5, "notification_cursor": 0}]}"#,
        );
        yard.answer(
            "GetUser",
            r#"{"result": {"name": "jo", "cursor": 30, "notification_cursor": 0}}"#,
        );
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [{"seq": 32, "from": "yardmaster", "to": "jo", "text": "From afar.", "read": false}], "unread": 1, "last": 32, "from": ["yardmaster"]}}"#,
        );
        yard.answer(
            "Conversation",
            r#"{"result": [{"seq": 32, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "From afar.", "read": false}]}"#,
        );
        yard.answer("AckUserInbox", "{}");
        yard.answer(
            "UserNotifications",
            r#"{"result": {"notifications": [], "unread": 0, "last": 0}}"#,
        );
        yard.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
    }

    /// The window as footplate opens it: on the yard `env` names, else the
    /// one the settings in `file` choose.
    fn opened_at<'a>(
        env: Env,
        file: &std::path::Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        let file = file.to_path_buf();
        cx.update(|cx| {
            input::bind_keys(cx);
            bind_keys(cx);
        });
        cx.add_window_view(|window, cx| {
            let view = Footplate::opened(env, "jo".into(), Some(file), window, cx);
            window.focus(&view.line.focus_handle(cx));
            view
        })
    }

    /// Puts a yard's name, its address and a code where they are typed.
    fn type_in(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        name: &str,
        address: &str,
        code: &str,
    ) {
        view.update(cx, |v, cx| {
            v.yard_name.update(cx, |line, cx| line.set_text(name, cx));
            v.address.update(cx, |line, cx| line.set_text(address, cx));
            v.code.update(cx, |line, cx| line.set_text(code, cx));
        });
    }

    /// What came of the last change to the yards.
    fn yard_said(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, _| v.yard_said.clone().unwrap_or_default())
    }

    fn which_yard(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, _| v.which_yard())
    }

    /// The name of the yard that is open, as the head has it.
    fn open_yard(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, _| v.which_yard_short())
    }

    /// What the switcher says of the yard called `name` after its kind.
    fn state_of(view: &Entity<Footplate>, cx: &mut VisualTestContext, name: &str) -> String {
        view.read_with(cx, |v, _| v.yard_state(v.settings.yard(name).unwrap()))
    }

    fn switching(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Option<usize> {
        view.read_with(cx, |v, _| v.switching)
    }

    fn mode_of(file: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(file).unwrap().permissions().mode() & 0o777
    }

    /// A second home's yard on this machine: jo's too, with other mail, one
    /// of it in a conversation that is not open.
    fn a_second_yard_of_jo(yard: &Yard) {
        a_yard_of_jo(yard);
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [{"seq": 12, "from": "yardmaster", "to": "jo", "text": "From the second home.", "read": false},
                {"seq": 13, "from": "assembly", "to": "jo", "text": "Assembled.", "read": false}], "unread": 2, "last": 13, "from": ["yardmaster", "assembly"]}}"#,
        );
        yard.answer(
            "Conversation",
            r#"{"result": [{"seq": 12, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "From the second home.", "read": false}]}"#,
        );
    }

    #[test]
    fn what_a_yard_has_waiting_is_said_in_words() {
        let counts = |mail, decisions, notices| Counts {
            mail,
            decisions,
            notices,
        };
        assert_eq!(counts(0, 0, 0).line(), "nothing waiting");
        assert_eq!(counts(2, 1, 0).line(), "2 mail, 1 decision");
        assert_eq!(counts(1, 0, 1).line(), "1 mail, 1 notice");
        assert_eq!(counts(0, 3, 12).line(), "3 decisions, 12 notices");
    }

    /// The yards in the settings: this machine's, as before, and one at an
    /// address that is added by a pairing's code. A wrong code and a
    /// refused address are said and change nothing; a removed device is
    /// said, not asked after, and paired again into the same file.
    #[gpui::test]
    fn the_settings_list_the_yards_and_a_code_pairs_a_new_one(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("footplate/yard-far.token");
        let near = Yard::start();
        a_yard_of_jo(&near);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        // Nothing chosen: this machine's yard, by its socket.
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(which_yard(&view, cx), "this machine's yard");
        assert_eq!(open_yard(&view, cx), "this machine");
        assert!(drawn(cx, "which yard"));
        assert_eq!(far.requests(), 0);

        // The settings list it, say what a pairing gives over a peer link,
        // and ask for nothing until a yard is added.
        cx.simulate_keystrokes("secondary-,");
        assert!(drawn(cx, "yard row this machine"));
        assert!(drawn(cx, "pairing gives"));
        assert_eq!(
            PAIRING_GIVES,
            "A paired yard is called with this device's own token, so your picks and approvals there count as yours, where over a peer link you only see what the link carries and an approval never crosses."
        );
        assert!(drawn(cx, "add near") && drawn(cx, "add far"));
        assert!(!drawn(cx, "yard pair"));
        click_on(cx, "add far");
        assert!(drawn(cx, "yard name") && drawn(cx, "yard address") && drawn(cx, "yard code"));
        assert!(drawn(cx, "yard pair"));

        // The name is typed first. Plain http off this machine is refused
        // with the reason, and nothing is sent anywhere.
        cx.simulate_input("far");
        assert_eq!(
            view.read_with(cx, |v, cx| v.yard_name.read(cx).text().to_string()),
            "far"
        );
        type_in(&view, cx, "far", "http://mac.example.net", "K7QM-2XPD");
        cx.simulate_keystrokes("enter");
        let said = yard_said(&view, cx);
        assert!(
            said.starts_with("http://mac.example.net is refused: plain http"),
            "{said}"
        );
        // No code: one is wanted.
        type_in(&view, cx, "far", far.address(), "");
        click_on(cx, "yard pair");
        assert!(yard_said(&view, cx).starts_with("A code is wanted."));
        // No pairing is open at the yard.
        type_in(&view, cx, "far", far.address(), "K7QM-2XPD");
        click_on(cx, "yard pair");
        wait(cx, "that no pairing is open", |cx| {
            yard_said(&view, cx).starts_with("No pairing is open at this yard")
        });

        // The operator opens one, and the code is typed wrong: the yard
        // says so, and nothing changed.
        far.pair("K7QM-2XPD");
        type_in(&view, cx, "far", far.address(), "WXYZ-2345");
        click_on(cx, "yard pair");
        wait(cx, "that it is not the code", |cx| {
            yard_said(&view, cx).starts_with("That is not the code.")
        });
        assert!(!token.exists());
        assert!(!file.exists());
        assert_eq!(which_yard(&view, cx), "this machine's yard");
        assert_eq!(far.devices(), [crate::fake::TOKEN]);
        assert!(drawn(cx, "yard said"));

        // The right code, here as the pairing's QR code holds it: the
        // window shows that yard, as the user and the device it says.
        type_in(
            &view,
            cx,
            "far",
            &format!("{}#k7qm-2xpd", far.address()),
            "",
        );
        click_on(cx, "yard pair");
        wait(cx, "the yard at the address", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        let here = format!("the yard south at {}, as its device laptop", far.address());
        assert_eq!(which_yard(&view, cx), here);
        assert_eq!(open_yard(&view, cx), "far");
        assert!(yard_said(&view, cx).starts_with("Paired: this is the device laptop of jo"));
        // Its fields are asked for no longer. (By the window's own word:
        // gpui's test keeps the bounds of what an earlier frame drew.)
        assert_eq!(view.read_with(cx, |v, _| v.adding.clone()), None);
        // The yard is kept with the settings after this machine's, as the
        // open one, its token in a file of its own that only its user
        // reads, and nowhere else.
        let kept = Settings::read(&file);
        assert_eq!(
            kept.yards,
            [
                settings::Yard::near(settings::HERE, ""),
                settings::Yard::far("far", far.address(), "yard-far.token")
            ]
        );
        assert_eq!(kept.open, "far");
        let first = far.devices()[1].clone();
        assert_eq!(yard::read_token(&token).unwrap(), first);
        assert_eq!(mode_of(&token), 0o600);
        assert!(
            !std::fs::read_to_string(&file)
                .unwrap()
                .contains("yardr-dev")
        );
        assert_eq!(
            view.read_with(cx, |v, cx| v.code.read(cx).text().to_string()),
            ""
        );
        // Every call there is the device's.
        assert!(
            far.last_request()
                .contains(&format!("Authorization: Bearer {first}\r\n"))
        );

        // The operator removes the device. The window says so, and does
        // not ask again; it offers to pair again.
        far.remove_device(&first);
        wait(cx, "that the device is not let in", |cx| {
            view.read_with(cx, |v, _| v.away == Away::Out)
        });
        let why = view.read_with(cx, |v, _| v.down.clone().unwrap());
        assert!(why.starts_with("This device is no longer let in"), "{why}");
        cx.simulate_keystrokes("escape");
        assert!(!settings_open(&view, cx));
        assert!(drawn(cx, "pair again"));
        click_on(cx, "pair again");
        assert!(settings_open(&view, cx));
        assert!(drawn(cx, "yard pair"));
        assert_eq!(
            view.read_with(cx, |v, _| v.adding.clone()),
            Some(Adding::Again("far".into()))
        );
        // The yard's address is known, so the code is what is typed, and
        // Enter sends it: the token is the same yard's, in the same file.
        far.pair("ABCD-EFGH");
        cx.simulate_input("abcd efgh");
        cx.simulate_keystrokes("enter");
        wait(cx, "the yard again", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        let second = far.devices()[1].clone();
        assert_ne!(second, first);
        assert_eq!(yard::read_token(&token).unwrap(), second);
        assert_eq!(mode_of(&token), 0o600);
        assert_eq!(Settings::read(&file), kept);

        // This machine's yard again: the socket, as before. The far yard
        // and its token stay, to go back to.
        click_on(cx, "yard open this machine");
        wait(cx, "this machine's yard again", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(which_yard(&view, cx), "this machine's yard");
        let again = Settings::read(&file);
        assert_eq!(
            (again.open.as_str(), &again.yards),
            (settings::HERE, &kept.yards)
        );
        // And back with no code: the device is paired already.
        click_on(cx, "yard open far");
        wait(cx, "the yard at the address once more", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        assert_eq!(far.devices()[1], second);
        assert_eq!(Settings::read(&file), kept);

        // The next start opens the yard that was open.
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (again, cx) = opened_at(env, &file, cx);
        wait(cx, "the open yard at the next start", |cx| {
            texts(&again, cx) == ["From afar."]
        });
        assert_eq!(which_yard(&again, cx), here);
    }

    /// The settings file of a window paired before there were several
    /// yards, with `yard.token` beside it, opens paired as it was, as one
    /// yard called by its address's host; the next change keeps it so.
    #[gpui::test]
    fn a_settings_file_from_before_opens_paired_as_it_was(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("footplate/yard.token");
        let near = Yard::start();
        a_yard_of_jo(&near);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        std::fs::write(
            &file,
            format!(
                "{{\n  \"text\": 14,\n  \"look\": \"auto\",\n  \"remote\": true,\n  \"address\": \"{}\"\n}}\n",
                far.address()
            ),
        )
        .unwrap();
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "the yard it was paired with", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        assert_eq!(
            which_yard(&view, cx),
            format!("the yard south at {}, as its device laptop", far.address())
        );
        assert_eq!(open_yard(&view, cx), "127.0.0.1");
        assert_eq!(near.requests(), 0);
        assert!(
            far.last_request()
                .contains(&format!("Authorization: Bearer {}\r\n", crate::fake::TOKEN))
        );
        // Reading it changed nothing; the next change keeps the list.
        assert!(std::fs::read_to_string(&file).unwrap().contains("remote"));
        cx.simulate_keystrokes("secondary-=");
        let kept = Settings::read(&file);
        assert_eq!(
            kept.yards,
            [settings::Yard::far(
                "127.0.0.1",
                far.address(),
                "yard.token"
            )]
        );
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("remote") && text.contains("\"yards\""),
            "{text}"
        );
        assert_eq!(yard::read_token(&token).unwrap(), crate::fake::TOKEN);
    }

    /// Three yards kept with the settings: this machine's, a second
    /// home's by its socket, and a paired one. Their settings file, and the
    /// paired one's token beside it.
    fn three_yards(dir: &std::path::Path, second: &Yard, far: &Yard) -> PathBuf {
        let file = dir.join("footplate/settings.json");
        let set = Settings::default()
            .with_yard(
                None,
                settings::Yard::near("second home", second.socket().to_str().unwrap()),
                None,
            )
            .unwrap()
            .with_yard(
                None,
                settings::Yard::far("far", far.address(), "yard-far.token"),
                Some(&file),
            )
            .unwrap();
        set.write(&file).unwrap();
        yard::keep_token(&dir.join("footplate/yard-far.token"), crate::fake::TOKEN).unwrap();
        file
    }

    /// The head of the left column names the open yard and switches: a
    /// click or its key opens the list of the yards, each with its kind
    /// and what waits in it now; a click on a row, Enter on the row the
    /// arrows are on, or a yard's own key from anywhere opens it, as it
    /// stands: every yard is followed from the start.
    #[gpui::test]
    fn the_head_switches_between_the_yards_by_click_and_by_key(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        // The first of the list, where none was open before. The others
        // are read too, and say what waits in them.
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(open_yard(&view, cx), "this machine");
        wait(cx, "what waits in the other yards", |cx| {
            state_of(&view, cx, "second home") == "2 mail" && state_of(&view, cx, "far") == "1 mail"
        });
        assert!(!drawn(cx, "switcher"));

        // A click on the head opens the list: each yard, its kind, and
        // its state.
        click_on(cx, "which yard");
        assert_eq!(switching(&view, cx), Some(0));
        assert!(drawn(cx, "switcher"));
        assert!(drawn(cx, "switch this machine") && drawn(cx, "switch far"));
        assert_eq!(state_of(&view, cx, "this machine"), "open");
        let kinds: Vec<&str> = view.read_with(cx, |v, _| {
            v.settings.yards.iter().map(|y| y.kind()).collect()
        });
        assert_eq!(kinds, ["on this machine", "on this machine", "paired"]);
        // A click on a row opens that yard, and it is the one to open at
        // the next start.
        click_on(cx, "switch second home");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        assert_eq!(open_yard(&view, cx), "second home");
        assert_eq!(switching(&view, cx), None);
        assert_eq!(Settings::read(&file).open, "second home");
        // What the one that was left has: its mail was read in the open
        // conversation.
        assert_eq!(state_of(&view, cx, "this machine"), "nothing waiting");
        let had = view.read_with(cx, |v, _| Counts::of(v.model.as_ref().unwrap()));
        assert_eq!((had.mail, had.decisions), (1, 0), "{had:?}");

        // The switcher's key opens the list on the open yard; the arrows
        // move and stop at its ends, esc leaves it, Enter opens.
        cx.simulate_keystrokes("secondary-y");
        assert_eq!(switching(&view, cx), Some(1));
        cx.simulate_keystrokes("up up");
        assert_eq!(switching(&view, cx), Some(0));
        cx.simulate_keystrokes("escape");
        assert_eq!(switching(&view, cx), None);
        assert_eq!(texts(&view, cx), ["From the second home."]);
        cx.simulate_keystrokes("secondary-y down down down");
        assert_eq!(switching(&view, cx), Some(2));
        // No key is a card's or the line's while the list has them.
        cx.simulate_keystrokes("x 1");
        assert_eq!(typed(&view, cx), "");
        cx.simulate_keystrokes("enter");
        wait(cx, "the paired yard", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        assert_eq!(open_yard(&view, cx), "far");
        assert_eq!(switching(&view, cx), None);
        assert_eq!(state_of(&view, cx, "second home"), "1 mail");
        // Every call there is the device's, with the token of that yard.
        assert!(
            far.last_request()
                .contains(&format!("Authorization: Bearer {}\r\n", crate::fake::TOKEN))
        );

        // A yard's place in the list is its key, from anywhere: the line
        // has the focus here. A place with no yard opens nothing.
        cx.simulate_input("typed");
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's yard by its key", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(state_of(&view, cx, "far"), "nothing waiting");
        cx.simulate_keystrokes("secondary-9");
        cx.run_until_parked();
        assert_eq!(open_yard(&view, cx), "this machine");
        cx.simulate_keystrokes("secondary-3");
        wait(cx, "the paired yard by its key", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        // What was typed there is there again.
        assert_eq!(typed(&view, cx), "typed");
        assert_eq!(Settings::read(&file).open, "far");
    }

    /// What is written for one yard stays with it across a switch and is
    /// sent only there: the line's text, the attachments and the words of
    /// an ask. A mail that is on its way when another yard is opened goes
    /// to the yard it was written in, and what of it does not go is back
    /// in that yard's draft, never in the open one's. Nothing typed of a
    /// secret outlasts a switch.
    #[gpui::test]
    fn what_is_written_for_one_yard_is_untouched_by_a_switch_and_sent_only_there(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        near.answer(
            "PutBlob",
            &json!({"result": {"hash": HASH, "type": "image/png", "bytes": 70}}).to_string(),
        );
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        let mails = |yard: &Yard| -> Vec<String> {
            let asked = yard.asked("Mail");
            asked.iter().map(|args| args.to_string()).collect()
        };

        // A draft for this machine's yardmaster: text, an image, and the
        // words of an ask; and a value half typed in a secret's field.
        cx.simulate_input("For the mac.");
        let image = Image::from_bytes(ImageFormat::Png, png());
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        cx.simulate_keystrokes("cmd-v");
        wait(cx, "the image in the draft", |cx| {
            attached(&view, cx) == [(1, PASTED.to_string(), None)]
        });
        let words = (("bead-a".to_string(), 3), "Not yet.".to_string());
        view.update(cx, |v, cx| {
            v.ask_words.insert(words.0.clone(), words.1.clone());
            v.secret
                .update(cx, |field, cx| field.set_text("hunter2", cx));
            v.secret_typed.push("first of two".into());
        });

        // The second home's yard: its line is empty, nothing is attached,
        // no ask has words, and nothing of a secret is anywhere.
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        assert_eq!(typed(&view, cx), "");
        assert_eq!(attached(&view, cx), []);
        view.read_with(cx, |v, cx| {
            assert!(v.ask_words.is_empty());
            assert_eq!(v.secret.read(cx).text(), "");
            assert!(v.secret_typed.is_empty());
        });
        // What is written here is sent here, and nothing of the other.
        cx.simulate_input("For the second.");
        cx.simulate_keystrokes("enter");
        wait(cx, "the mail in the second home's yard", |cx| {
            let _ = cx;
            second.asked("Mail").len() == 1
        });
        assert!(
            mails(&second)[0].contains("For the second."),
            "{:?}",
            mails(&second)
        );
        assert!(!mails(&second)[0].contains("For the mac."));
        assert_eq!(second.asked("PutBlob").len(), 0);
        assert_eq!(near.asked("Mail").len(), 0);
        cx.simulate_input("Half a thought.");

        // Back: all of the first draft is as it was left.
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's yard again", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(typed(&view, cx), "For the mac.");
        assert_eq!(attached(&view, cx), [(1, PASTED.to_string(), None)]);
        view.read_with(cx, |v, _| {
            assert_eq!(v.ask_words.get(&words.0), Some(&words.1));
            assert!(v.thumbs.contains_key(&1));
        });

        // It is sent, and with the mail on its way the other yard is
        // opened, with text in the line that was typed since: the keys
        // come one after the other before the call's answer is heard. The
        // yard it was written in refuses it.
        near.answer(
            "Mail",
            r#"{"error": {"code": "error", "message": "mail to yardmaster: the yard is full"}}"#,
        );
        cx.simulate_keystrokes("enter s i n c e secondary-2");
        wait(cx, "the second home's yard with a mail on its way", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        wait(cx, "that it did not go", |cx| {
            notice(&view, cx).starts_with("not sent to yardmaster in this machine: ")
        });
        // It went to the yard it was written in and to no other, and what
        // did not go is in no line and no draft of the open yard.
        assert_eq!(near.asked("Mail").len(), 1);
        assert!(
            mails(&near)[0].contains("For the mac."),
            "{:?}",
            mails(&near)
        );
        assert!(!mails(&near)[0].contains("Half a thought."));
        assert_eq!(typed(&view, cx), "Half a thought.");
        assert_eq!(attached(&view, cx), []);
        assert_eq!(second.asked("Mail").len(), 1);
        assert_eq!(second.asked("PutBlob").len(), 0);

        // Back in its own yard it is in the line again, before what was
        // typed since, with its image.
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's yard once more", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(typed(&view, cx), "For the mac. since");
        assert_eq!(attached(&view, cx), [(1, PASTED.to_string(), None)]);

        // Sent again, and the other yard opened over it: it goes where it
        // was written, and the open yard's line is its own.
        near.answer("Mail", "{}");
        cx.simulate_keystrokes("enter secondary-2");
        wait(cx, "the second home's yard over it", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        assert_eq!(near.asked("Mail").len(), 2);
        assert!(
            mails(&near)[1].contains("For the mac. since"),
            "{:?}",
            mails(&near)
        );
        assert_eq!(typed(&view, cx), "Half a thought.");
        assert_eq!(notice(&view, cx), "");
        assert_eq!(second.asked("Mail").len(), 1);
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's yard at last", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(typed(&view, cx), "");
        assert_eq!(attached(&view, cx), []);
    }

    #[gpui::test]
    fn a_failed_send_is_not_restored_to_a_new_yard_that_reuses_its_name(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let old = Yard::start_tcp();
        a_far_yard_of_jo(&old);
        old.answer(
            "Mail",
            r#"{"error": {"code": "error", "message": "mail refused"}}"#,
        );
        let replacement = Yard::start_tcp();
        a_far_yard_of_jo(&replacement);
        replacement.answer(
            "UserInbox",
            r#"{"result": {"mail": [{"seq": 32, "from": "yardmaster", "to": "jo", "text": "From the replacement.", "read": false}], "unread": 1, "last": 32, "from": ["yardmaster"]}}"#,
        );
        replacement.answer(
            "Conversation",
            r#"{"result": [{"seq": 32, "at": "2026-10-08T09:30:00Z", "from": "yardmaster", "to": "jo", "text": "From the replacement.", "read": false}]}"#,
        );
        let token = dir.path().join("footplate/yard-far.token");
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        let settings = Settings::default()
            .with_yard(
                None,
                settings::Yard::far("far", old.address(), "yard-far.token"),
                Some(&file),
            )
            .unwrap()
            .opening("far");
        settings.write(&file).unwrap();
        let env = Env::default();
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "the original yard", |cx| {
            texts(&view, cx) == ["From afar."]
        });

        // Queue a failed send through the old client, then replace that
        // settings entry under the same display name before its answer is
        // handled by the window.
        view.update(cx, |v, cx| {
            assert!(v.open().is_some());
            v.line
                .update(cx, |line, cx| line.set_text("private to old far", cx));
            v.send(cx);
            let (without, _) = v.settings.clone().without("far", Some(&file));
            yard::keep_token(&token, crate::fake::TOKEN).unwrap();
            let replacement = without
                .with_yard(
                    None,
                    settings::Yard::far("far", replacement.address(), "yard-far.token"),
                    Some(&file),
                )
                .unwrap()
                .opening("far");
            v.set(replacement, cx);
            v.reopen(cx);
        });

        wait(cx, "the replacement yard", |cx| {
            texts(&view, cx) == ["From the replacement."]
        });
        wait(cx, "the old send refusal", |cx| {
            notice(&view, cx).starts_with(
                "not sent to yardmaster in far: that yard entry was removed or changed",
            )
        });
        assert_eq!(typed(&view, cx), "");
        assert_eq!(old.asked("Mail").len(), 1);
        assert!(replacement.asked("Mail").is_empty());
    }

    /// The words of an answer that is on its way when another yard is
    /// opened, and that its yard refuses, are back with the ask they were
    /// typed for, in its own yard: never in the line of the open one.
    #[gpui::test]
    fn words_a_yard_refused_are_back_with_their_ask_and_not_in_another_yard_s_line(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        near.answer("UserAsks", &asks_three());
        near.answer("Bead", &bead_three());
        near.answer(
            "AnswerAsk",
            r#"{"error": {"code": "error", "message": "bead-a: the ask is closed"}}"#,
        );
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        let on_the_card = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Decision("bead-a".into())))
            })
        };
        wait(cx, "the decision", on_the_card);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));
        cx.run_until_parked();
        // A pick with words, and the other yard opened before the yard's
        // answer to it is heard.
        cx.simulate_keystrokes("escape 2 i");
        cx.simulate_input("because");
        cx.simulate_keystrokes("enter secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        wait(cx, "that it was not answered", |cx| {
            notice(&view, cx) == "not answered in this machine: bead-a: the ask is closed"
        });
        assert_eq!(near.asked("AnswerAsk").len(), 1);
        assert_eq!(second.asked("AnswerAsk").len(), 0);
        assert_eq!(typed(&view, cx), "");
        view.read_with(cx, |v, _| {
            assert!(v.ask_words.is_empty());
            let kept = &v.kept_words["this machine"];
            assert_eq!(
                kept.get(&("bead-a".to_string(), 3)).map(String::as_str),
                Some("because")
            );
        });
        // Its own yard has them in the line again, on its card.
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "the decision again", on_the_card);
        assert_eq!(typed(&view, cx), "because");
    }

    /// Yards are added and removed in the settings. A name that is taken
    /// is refused before a code is sent anywhere; a new yard's token file
    /// is one no other has; removing a yard deletes its token's file where
    /// it is footplate's alone, and says how the device is removed at the
    /// yard.
    #[gpui::test]
    fn yards_are_added_and_removed_in_the_settings(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        // This machine's and a paired one, to begin with.
        let file = three_yards(dir.path(), &second, &far);
        let (set, _) = Settings::read(&file).without("second home", Some(&file));
        set.write(&file).unwrap();
        let beside = |name: &str| dir.path().join("footplate").join(name);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        cx.simulate_keystrokes("secondary-,");

        // A second home's yard, by its socket: no code, and it is opened.
        click_on(cx, "add near");
        assert!(drawn(cx, "yard name") && drawn(cx, "yard socket"));
        view.update(cx, |v, cx| {
            v.yard_name.update(cx, |line, cx| line.set_text("far", cx));
            let socket = second.socket().to_str().unwrap().to_string();
            v.socket.update(cx, |line, cx| line.set_text(&socket, cx));
        });
        // The name is another yard's: said, and nothing changed.
        click_on(cx, "yard pair");
        assert!(
            yard_said(&view, cx).starts_with("There is a yard called far already"),
            "{}",
            yard_said(&view, cx)
        );
        assert_eq!(Settings::read(&file), set);
        view.update(cx, |v, cx| {
            v.yard_name
                .update(cx, |line, cx| line.set_text("second home", cx));
        });
        click_on(cx, "yard pair");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        let kept = Settings::read(&file);
        assert_eq!(
            kept.yards[2],
            settings::Yard::near("second home", second.socket().to_str().unwrap())
        );
        assert_eq!(kept.open, "second home");

        // A yard at an address under a name that is taken: refused before
        // the code is sent, which would use it up.
        far.pair("K7QM-2XPD");
        click_on(cx, "add far");
        type_in(&view, cx, "second home", far.address(), "K7QM-2XPD");
        let before = far.requests();
        click_on(cx, "yard pair");
        assert!(yard_said(&view, cx).starts_with("There is a yard called second home already"));
        cx.run_until_parked();
        assert_eq!(far.requests(), before);
        assert!(far.pairing_open());
        // Under its own name it is paired, into a file no other yard has:
        // the name's own is the yard far's.
        type_in(&view, cx, "FAR", far.address(), "K7QM-2XPD");
        click_on(cx, "yard pair");
        wait(cx, "the pairing", |cx| {
            yard_said(&view, cx).starts_with("Paired")
        });
        wait(cx, "the yard that was paired", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        let kept = Settings::read(&file);
        assert_eq!(
            kept.yards[3],
            settings::Yard::far("FAR", far.address(), "yard-far-2.token")
        );
        assert_eq!(
            yard::read_token(&beside("yard-far-2.token")).unwrap(),
            far.devices()[1]
        );
        assert_eq!(
            yard::read_token(&beside("yard-far.token")).unwrap(),
            crate::fake::TOKEN
        );

        // Removing the open yard: its token's file goes with it, the
        // other's stays, the settings say the command that removes the
        // device at the yard, and the first of the list is opened.
        click_on(cx, "yard remove FAR");
        let said = yard_said(&view, cx);
        assert!(
            said.starts_with(&format!(
                "FAR is removed, and its token's file {} with it.",
                beside("yard-far-2.token").display()
            )),
            "{said}"
        );
        assert!(said.ends_with("yardr user device rm jo laptop"), "{said}");
        assert!(!beside("yard-far-2.token").exists());
        assert!(beside("yard-far.token").exists());
        wait(cx, "the first yard of the list", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        let kept = Settings::read(&file);
        assert_eq!(kept.yards.len(), 3);
        assert_eq!(kept.opened().name, settings::HERE);
        // A yard that is not open is removed too, and one of this machine
        // has no token to say anything of.
        click_on(cx, "yard remove second home");
        assert_eq!(
            yard_said(&view, cx),
            "second home is removed from the list. Its yard is as it was."
        );
        // Who the device is at a yard that is not open is known too: it
        // is followed, and said so when it was read.
        wait(cx, "the paired yard to be read", |cx| {
            state_of(&view, cx, "far") == "1 mail"
        });
        click_on(cx, "yard remove far");
        let said = yard_said(&view, cx);
        assert!(said.ends_with("yardr user device rm jo laptop"), "{said}");
        assert!(!beside("yard-far.token").exists());
        assert_eq!(
            Settings::read(&file).yards,
            [settings::Yard::near(settings::HERE, "")]
        );
        assert_eq!(texts(&view, cx), ["Shall I land it?"]);
    }

    /// A pairing writes the address the code went to under the token: the
    /// file is of that yard from the moment it is there.
    #[gpui::test]
    fn a_pairing_writes_the_yard_s_address_with_the_token(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("footplate/yard-far.token");
        let near = Yard::start();
        a_yard_of_jo(&near);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        far.pair("K7QM-2XPD");
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        cx.simulate_keystrokes("secondary-,");
        click_on(cx, "add far");
        type_in(&view, cx, "far", far.address(), "K7QM-2XPD");
        click_on(cx, "yard pair");
        wait(cx, "the pairing", |cx| {
            yard_said(&view, cx).starts_with("Paired")
        });
        assert_eq!(
            std::fs::read_to_string(&token).unwrap(),
            format!(
                "{}\n!footplate-token-v1\n{}\n",
                far.devices()[1],
                Address::parse(far.address()).unwrap()
            )
        );
        assert_eq!(mode_of(&token), 0o600);
        // Nothing else is left beside it.
        assert_eq!(
            std::fs::read_dir(dir.path().join("footplate"))
                .unwrap()
                .count(),
            2
        );
    }

    /// A token file typed by hand that cannot be written is found before
    /// the code is sent: the yard is asked nothing, adds no device, and
    /// its pairing stays open for the code.
    #[gpui::test]
    fn a_token_file_that_cannot_be_written_keeps_the_code_from_the_yard(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let tokens = dir.path().join("tokens");
        let token = tokens.join("yard.token");
        let near = Yard::start();
        a_yard_of_jo(&near);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        far.pair("K7QM-2XPD");
        Settings::default()
            .with_yard(
                None,
                settings::Yard::far("far", far.address(), token.to_str().unwrap()),
                Some(&file),
            )
            .unwrap()
            .write(&file)
            .unwrap();
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        let pair = |cx: &mut VisualTestContext| {
            view.update_in(cx, |v, window, cx| {
                v.pair_again(Some("far".into()), window, cx);
                v.code.update(cx, |line, cx| line.set_text("K7QM-2XPD", cx));
                v.pair(cx);
            });
        };
        let unasked = |cx: &mut VisualTestContext| {
            assert_eq!(far.requests(), 0);
            assert_eq!(far.devices(), [crate::fake::TOKEN]);
            assert!(far.pairing_open());
            assert_eq!(which_yard(&view, cx), "this machine's yard");
        };

        // A directory where the token's file would be.
        std::fs::create_dir_all(&token).unwrap();
        pair(cx);
        wait(cx, "that a directory is no token file", |cx| {
            yard_said(&view, cx).contains("is not a regular file")
        });
        unasked(cx);
        assert!(token.is_dir());

        // A file where its directory would be.
        std::fs::remove_dir_all(&tokens).unwrap();
        std::fs::write(&tokens, "").unwrap();
        pair(cx);
        wait(cx, "that the file cannot be checked", |cx| {
            yard_said(&view, cx).contains("cannot be checked")
        });
        unasked(cx);

        // A directory that cannot be written to, and nothing is left in it.
        std::fs::remove_file(&tokens).unwrap();
        std::fs::create_dir(&tokens).unwrap();
        let mode = |mode| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tokens, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        mode(0o500);
        pair(cx);
        wait(cx, "that the directory cannot be written to", |cx| {
            !view.read_with(cx, |v, _| v.pairing) && !yard_said(&view, cx).is_empty()
        });
        let said = yard_said(&view, cx);
        assert!(
            said.starts_with(&format!(
                "The token file {} cannot be written (",
                token.display()
            )) && said.contains("The code was not sent"),
            "{said}"
        );
        unasked(cx);
        mode(0o700);
        assert_eq!(std::fs::read_dir(&tokens).unwrap().count(), 0);

        // The same code pairs once the file can be written.
        pair(cx);
        wait(cx, "the yard that was paired", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        assert_eq!(yard::read_token(&token).unwrap(), far.devices()[1]);
        assert_eq!(std::fs::read_dir(&tokens).unwrap().count(), 1);
    }

    #[gpui::test]
    fn pairing_keeps_settings_changed_while_the_code_is_away(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let near = Yard::start();
        a_yard_of_jo(&near);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        far.pair("K7QM-2XPD");
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        cx.simulate_keystrokes("secondary-,");
        click_on(cx, "add far");
        type_in(&view, cx, "far", far.address(), "K7QM-2XPD");

        // Pairing runs away from the window. A settings change made before
        // its answer must be kept with the new yard.
        view.update(cx, |v, cx| {
            v.pair(cx);
            v.set(
                Settings {
                    text: 16,
                    ..v.settings.clone()
                },
                cx,
            );
        });
        wait(cx, "the paired yard", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        assert_eq!(Settings::read(&file).text, 16);
        assert_eq!(Settings::read(&file).opened().name, "far");
        assert_eq!(far.devices().len(), 2);
    }

    /// `FOOTPLATE_YARD` and `FOOTPLATE_TOKEN_FILE` name the one yard of a
    /// run: the window has no switcher and the settings no list, and a
    /// pairing there writes the environment's file.
    #[gpui::test]
    fn the_environment_s_yard_is_the_one_yard_of_the_window(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("mac.token");
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        let file = three_yards(dir.path(), &second, &far);
        let kept = std::fs::read_to_string(&file).unwrap();
        let env = Env {
            yard: Some(far.address().to_string()),
            token_file: Some(token.clone()),
            socket: near.socket().to_path_buf(),
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "the environment's yard", |cx| {
            texts(&view, cx) == ["From afar."]
        });
        assert_eq!(near.requests(), 0);
        assert!(which_yard(&view, cx).starts_with("the yard south at http://127.0.0.1"));
        // No switcher: neither the head nor a key opens another yard.
        click_on(cx, "which yard");
        assert_eq!(switching(&view, cx), None);
        cx.simulate_keystrokes("secondary-y secondary-1 secondary-2");
        cx.run_until_parked();
        assert_eq!(switching(&view, cx), None);
        assert_eq!(texts(&view, cx), ["From afar."]);
        assert_eq!((near.requests(), second.requests()), (0, 0));
        // Nor do the settings list the yards they keep.
        cx.simulate_keystrokes("secondary-,");
        assert!(drawn(cx, "yard again"));
        view.update_in(cx, |v, window, cx| {
            v.add_yard(true, window, cx);
            v.remove_yard("far", cx);
        });
        assert_eq!(view.read_with(cx, |v, _| v.adding.clone()), None);
        // A pairing there goes to the environment's address, and its token
        // into the environment's file: no yard's of the settings.
        click_on(cx, "yard again");
        far.pair("K7QM-2XPD");
        view.update(cx, |v, cx| {
            v.code.update(cx, |line, cx| line.set_text("K7QM-2XPD", cx));
        });
        click_on(cx, "yard pair");
        wait(cx, "the pairing", |cx| {
            yard_said(&view, cx).starts_with("Paired")
        });
        assert_eq!(yard::read_token(&token).unwrap(), far.devices()[1]);
        assert_eq!(
            yard::read_token(&dir.path().join("footplate/yard-far.token")).unwrap(),
            crate::fake::TOKEN
        );
        assert!(!dir.path().join("footplate/yard.token").exists());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), kept);
    }

    /// A yard that cannot be opened is said why, and not tried: plain http
    /// off this machine, and a token file open to others. The settings
    /// are there to open another.
    #[gpui::test]
    fn a_yard_that_is_refused_is_said_why_and_not_tried(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("mac.token");
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        let env = |yard: &str| Env {
            yard: Some(yard.to_string()),
            token_file: Some(token.clone()),
            socket: near.socket().to_path_buf(),
        };

        let (view, cx) = opened_at(env("http://mac.example.net:8790"), &file, cx);
        cx.run_until_parked();
        let (why, away) = view.read_with(cx, |v, _| (v.down.clone().unwrap(), v.away));
        assert!(
            why.starts_with("http://mac.example.net:8790 is refused: plain http"),
            "{why}"
        );
        assert_eq!(away, Away::Stopped);
        assert!(drawn(cx, "settings"));
        assert!(!drawn(cx, "pair again"));
        cx.simulate_keystrokes("secondary-,");
        assert!(drawn(cx, "yard again"));

        // The token's file is readable by others: refused before anything
        // is sent with it.
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (view, cx) = opened_at(env(far.address()), &file, cx);
        cx.run_until_parked();
        let (why, away) = view.read_with(cx, |v, _| (v.down.clone().unwrap(), v.away));
        assert!(why.contains("is open to others (mode 0644)"), "{why}");
        assert_eq!(away, Away::Stopped);
        assert_eq!(far.requests(), 0);
        assert_eq!(near.requests(), 0);
    }

    /// A yard of the settings that cannot be opened leaves the others to
    /// open: its key and the settings' row do, from where it says why.
    #[gpui::test]
    fn another_yard_is_opened_from_one_that_cannot_be(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        let file = three_yards(dir.path(), &second, &far);
        // The paired yard was open last, and its token is gone.
        Settings::read(&file).opening("far").write(&file).unwrap();
        std::fs::remove_file(dir.path().join("footplate/yard-far.token")).unwrap();
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        cx.run_until_parked();
        let (why, away) = view.read_with(cx, |v, _| (v.down.clone().unwrap(), v.away));
        assert!(why.starts_with("This device has no token"), "{why}");
        assert_eq!(away, Away::Stopped);
        assert_eq!(far.requests(), 0);
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        // And back to it, as the one that still cannot be opened.
        cx.simulate_keystrokes("secondary-,");
        click_on(cx, "yard open far");
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.down.is_some()));
        assert_eq!(open_yard(&view, cx), "far");
    }

    /// The head's line, as it is drawn.
    fn head_line(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, _| v.head_line(v.model.as_ref().map(Model::me)))
    }

    /// A yard that is not reached says so with its short why, in the
    /// switcher's row and where it is open: one that is stopped and one
    /// away from the network, "nothing answers"; one that no longer lets
    /// the device in, "pair again". The head stays with the switcher, so
    /// another yard is opened from there and never through the settings,
    /// and the yard that answers has all that was its.
    #[gpui::test]
    fn a_yard_that_is_not_reached_says_why_and_the_switcher_leaves_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        let shop = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        Settings::read(&file)
            .with_yard(
                None,
                settings::Yard::far("shop", shop.address(), "yard-shop.token"),
                Some(&file),
            )
            .unwrap()
            .write(&file)
            .unwrap();
        yard::keep_token(
            &dir.path().join("footplate/yard-shop.token"),
            crate::fake::TOKEN,
        )
        .unwrap();
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(head_line(&view, cx), "jo · this machine ▾");
        cx.simulate_input("kept");
        // The others are followed too, and the two that answer were read.
        wait(cx, "what waits in the yards that answer", |cx| {
            state_of(&view, cx, "second home") == "2 mail" && state_of(&view, cx, "far") == "1 mail"
        });

        // The second home's yard is stopped, the shop is away from the
        // network, and the far yard's operator removed this device, one of
        // two.
        let socket = second.socket().to_path_buf();
        second.stop();
        shop.stop();
        far.add_device("yardr-dev-phone");
        far.remove_device(crate::fake::TOKEN);
        let down = |view: &Entity<Footplate>, cx: &mut VisualTestContext, start: &str| {
            view.read_with(cx, |v, _| {
                v.down.as_deref().is_some_and(|why| why.starts_with(start))
            })
        };

        // The stopped yard: its why in a word or two, in the head, in the
        // away view and in its row, with the telling under it. The column
        // has its head and none of a yard's rows.
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "that the second home's yard is stopped", |cx| {
            down(&view, cx, "No yard: nothing answers at the yard's socket")
        });
        // Who the window is for there is known of a yard that was read.
        assert_eq!(head_line(&view, cx), "jo · second home · nothing answers ▾");
        assert_eq!(
            state_of(&view, cx, "second home"),
            "not reached: nothing answers"
        );
        assert!(drawn(cx, "which yard") && drawn(cx, "away why"));
        assert_eq!(view.read_with(cx, |v, _| v.away), Away::Trying);
        // What the yard that answers has is as it was left.
        assert_eq!(state_of(&view, cx, "this machine"), "nothing waiting");

        // The head is the switcher there too: a click lists the yards, the
        // arrows move, and Enter opens the one away from the network.
        click_on(cx, "which yard");
        assert_eq!(switching(&view, cx), Some(1));
        assert!(drawn(cx, "switcher") && drawn(cx, "switch this machine"));
        cx.simulate_keystrokes("down down enter");
        wait(cx, "that nothing answers at the shop", |cx| {
            down(&view, cx, "No yard: nothing answers at the yard's address")
        });
        assert_eq!(head_line(&view, cx), "shop · nothing answers ▾");
        assert_eq!(state_of(&view, cx, "shop"), "not reached: nothing answers");
        // The one that was left is said as it was left.
        assert_eq!(
            state_of(&view, cx, "second home"),
            "not reached: nothing answers"
        );

        // The switcher's key from a yard that is away, to the one that no
        // longer lets the device in: that is not asked again.
        cx.simulate_keystrokes("secondary-y");
        assert_eq!(switching(&view, cx), Some(3));
        cx.simulate_keystrokes("up enter");
        wait(cx, "that the device is not let in", |cx| {
            view.read_with(cx, |v, _| v.away == Away::Out)
        });
        assert_eq!(head_line(&view, cx), "jo · far · pair again ▾");
        assert_eq!(state_of(&view, cx, "far"), "not reached: pair again");
        assert!(drawn(cx, "away why") && drawn(cx, "pair again"));
        // Esc leaves the list, and the yard stays as it is.
        cx.simulate_keystrokes("secondary-y");
        assert!(drawn(cx, "switcher"));
        cx.simulate_keystrokes("escape");
        assert_eq!(switching(&view, cx), None);
        assert_eq!(open_yard(&view, cx), "far");

        // Back to the yard that answers by a click on its row, with no
        // settings: what was typed there is in its line, and the three
        // that were not reached are said so, each with its own why.
        click_on(cx, "which yard");
        click_on(cx, "switch this machine");
        wait(cx, "this machine's yard again", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert!(!settings_open(&view, cx));
        assert_eq!(typed(&view, cx), "kept");
        assert_eq!(head_line(&view, cx), "jo · this machine ▾");
        assert_eq!(state_of(&view, cx, "this machine"), "open");
        assert_eq!(
            state_of(&view, cx, "second home"),
            "not reached: nothing answers"
        );
        assert_eq!(state_of(&view, cx, "shop"), "not reached: nothing answers");
        assert_eq!(state_of(&view, cx, "far"), "not reached: pair again");
        // (By the window's own word: gpui's test keeps the bounds of what
        // an earlier frame drew.)
        assert_eq!(view.read_with(cx, |v, _| v.down.clone()), None);

        // The stopped yard is started: it is asked again by itself, with
        // no switch to it, and is said not reached no longer, there or
        // when it is opened and left.
        let second = Yard::start_at(&socket);
        a_second_yard_of_jo(&second);
        wait(cx, "the second home's yard to answer by itself", |cx| {
            state_of(&view, cx, "second home") == "2 mail"
        });
        assert_eq!(open_yard(&view, cx), "this machine");
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        assert_eq!(head_line(&view, cx), "jo · second home ▾");
        assert_eq!(state_of(&view, cx, "second home"), "open");
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's yard once more", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        assert_eq!(state_of(&view, cx, "second home"), "1 mail");
    }

    /// The one yard the environment names has no other to go to: away, it
    /// has no column, and its settings are reached from under the words.
    #[gpui::test]
    fn the_environment_s_yard_away_has_no_switcher(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("mac.token");
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        let far = Yard::start_tcp();
        far.stop();
        let env = Env {
            yard: Some(far.address().to_string()),
            token_file: Some(token),
            socket: dir.path().join("yard.sock"),
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "that nothing answers", |cx| {
            view.read_with(cx, |v, _| {
                v.down
                    .as_deref()
                    .is_some_and(|why| why.starts_with("No yard: nothing answers"))
            })
        });
        assert!(drawn(cx, "away why") && drawn(cx, "settings"));
        assert!(!drawn(cx, "which yard"));
        cx.simulate_keystrokes("secondary-y");
        assert_eq!(switching(&view, cx), None);
    }

    /// What the yard says a secret ask asks for.
    const BOOKS_LINE: &str =
        "a secret: BOOKS_TOKEN, for the source books (https://books.example.com)";

    /// A value as a person has it on the clipboard.
    const VALUE: &str = "bk-live-3f9a-7Qx";

    fn secret_of(env: &[&str]) -> Value {
        json!({"source": "books", "env": env, "line": BOOKS_LINE})
    }

    /// The secret ask 7 on bead-s as the queue lists it, open or in `state`.
    fn queued_secret(env: &[&str], ask: u64, state: &str, given: &[&str]) -> Value {
        let mut queued = json!({
            "bead": "bead-s", "bead_title": "The books", "topic": "", "ask": ask,
            "question": "The token for the books", "asker": "yardmaster",
            "since": "2026-10-10T10:00:00Z", "secret": secret_of(env)
        });
        if !state.is_empty() {
            queued["state"] = json!(state);
            queued["given"] = json!(given);
        }
        queued
    }

    fn bead_secret(env: &[&str], asks: &[u64]) -> String {
        let asks: Vec<Value> = asks
            .iter()
            .map(|ask| {
                json!({"ask": ask, "to": "jo", "asker": "yardmaster",
                       "since": "2026-10-10T10:00:00Z",
                       "question": "The token for the books",
                       "context": ["Create one in the books' settings."],
                       "options": [], "state": "open", "secret": secret_of(env)})
            })
            .collect();
        json!({"result": {"id": "bead-s", "title": "The books", "body": "The body.",
                          "stage": "new", "labels": [], "notes": [], "asks": asks}})
        .to_string()
    }

    /// The window on a yard whose one open ask is the secret ask 7 on
    /// bead-s for `env`, its card open and read, the settings kept in
    /// `file`.
    fn opened_on_a_secret_ask<'a>(
        yard: &Yard,
        env: &[&str],
        file: &std::path::Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        a_yard_of_jo(yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [queued_secret(env, 7, "", &[])], "decided": []}})
                .to_string(),
        );
        yard.answer("Bead", &bead_secret(env, &[7]));
        let at = Env {
            socket: yard.socket().to_path_buf(),
            ..Env::default()
        };
        opened_on_the_ask(at, file, cx)
    }

    /// The window at `at`, a yard whose one open ask is on bead-s, with
    /// that ask's card open and read.
    fn opened_on_the_ask<'a>(
        at: Env,
        file: &std::path::Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        let (view, cx) = opened_at(at, file, cx);
        wait(cx, "the decision", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Decision("bead-s".into())))
            })
        });
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-s"));
        cx.run_until_parked();
        (view, cx)
    }

    /// What is typed in the field of a secret, what the field draws of it,
    /// and what it says while empty.
    fn secret_field(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
    ) -> (String, String, String) {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        view.read_with(cx, |v, cx| {
            let field = v.secret.read(cx);
            let (drawn, says) = field.drawn();
            (field.text().to_string(), drawn, says)
        })
    }

    /// Everything the window keeps or says that is not the field of a
    /// secret: the line, the drafts, the words of the asks, the notice,
    /// the settings and every file beside them.
    fn kept_outside_the_field(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        dir: &std::path::Path,
    ) -> String {
        fn files(dir: &std::path::Path, all: &mut String) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files(&path, all);
                } else {
                    all.push_str(&String::from_utf8_lossy(&std::fs::read(&path).unwrap()));
                }
            }
        }
        let mut kept = view.read_with(cx, |v, cx| {
            format!(
                "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
                v.line.read(cx).text(),
                v.line.read(cx).drawn(),
                v.drafts,
                v.ask_words,
                v.model.as_ref().map(|m| m.notice.clone()),
                v.settings,
                v.yard_said,
                v.down,
                v.secret.read(cx).drawn(),
            )
        });
        files(dir, &mut kept);
        kept
    }

    /// A secret ask's card: the yard's line and a field that shows marks
    /// in place of the words line. Paste works and nothing copies out;
    /// Enter calls `GiveSecret` and the field is emptied whatever is
    /// answered; a call that ends without an answer says the value may be
    /// written; a yard without the call sends the person to a terminal.
    /// The value is in no draft, no setting and no notice at any point.
    #[gpui::test]
    fn a_secret_is_typed_as_marks_given_in_its_call_and_kept_nowhere(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let (view, cx) = opened_on_a_secret_ask(&yard, &["BOOKS_TOKEN"], &file, cx);
        let nowhere = |view: &Entity<Footplate>, cx: &mut VisualTestContext, when: &str| {
            let kept = kept_outside_the_field(view, cx, dir.path());
            assert!(!kept.contains(VALUE), "{when}: {kept}");
        };
        let marks = |n: usize| "\u{2022}".repeat(n);

        // The window opened with the focus in the line, which this card
        // does not draw: the window's keys have it, not a line out of
        // sight.
        let in_line = view.update_in(cx, |v, window, cx| {
            v.line.focus_handle(cx).is_focused(window)
        });
        assert!(!in_line);

        // The yard's line as it is, and the field where another card has
        // its line for words.
        assert!(drawn(cx, BOOKS_LINE));
        assert!(drawn(cx, "secret bead-s 7"));
        assert!(!drawn(cx, "words bead-s 7"));
        assert_eq!(
            secret_field(&view, cx),
            (
                String::new(),
                String::new(),
                "BOOKS_TOKEN for the source books".to_string()
            )
        );

        // Typed, it is marks, and the line for words has none of it.
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        let (typed_in, shown, _) = secret_field(&view, cx);
        assert_eq!((typed_in.as_str(), shown), (VALUE, marks(VALUE.len())));
        assert_eq!(typed(&view, cx), "");
        nowhere(&view, cx, "typed");
        // Nothing copies out, by copy or by cut.
        cx.write_to_clipboard(ClipboardItem::new_string("before".to_string()));
        cx.simulate_keystrokes("cmd-a cmd-c cmd-x");
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("before".to_string())
        );
        assert_eq!(secret_field(&view, cx).0, VALUE);
        // A setting kept while it is typed keeps none of it.
        cx.simulate_keystrokes("cmd-=");
        cx.run_until_parked();
        assert!(file.exists());
        nowhere(&view, cx, "a setting kept");
        // Leaving the card forgets it: it is in no draft of the
        // conversation that opens, and is not back with the card.
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        assert_eq!(secret_field(&view, cx).0, "");
        assert_eq!(typed(&view, cx), "");
        nowhere(&view, cx, "the card left");
        cx.simulate_keystrokes("shift-tab");
        wait(cx, "the card again", |cx| drawn(cx, "secret bead-s 7"));
        assert_eq!(secret_field(&view, cx).0, "");
        nowhere(&view, cx, "the card again");

        // Pasted, with the line break a copy often ends in. This yard does
        // not have the call: the field is empty at once, the value went in
        // the call without the break, and the card says where to answer.
        cx.write_to_clipboard(ClipboardItem::new_string(format!("{VALUE}\n")));
        cx.simulate_keystrokes("i");
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(secret_field(&view, cx).1, marks(VALUE.len() + 1));
        cx.simulate_keystrokes("enter");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "the yard without the call", |cx| {
            notice(&view, cx)
                == "this yard takes no secret from footplate. Answer in a terminal on the yard's machine: yardr bead answer bead-s 7 --secret"
        });
        assert_eq!(
            yard.asked("GiveSecret"),
            [json!({"bead": "bead-s", "ask": 7, "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo"})]
        );
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
        nowhere(&view, cx, "no such call");

        // The yard goes away over the call: never "sent", and not "not
        // sent" either.
        yard.answer_nothing("GiveSecret");
        cx.simulate_input(VALUE);
        cx.simulate_keystrokes("enter");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "no answer", |cx| {
            notice(&view, cx) == "the yard did not answer. The value may be written. Give it again."
        });
        assert_eq!(yard.asked("GiveSecret").len(), 2);
        nowhere(&view, cx, "no answer");

        // The yard says no, in its words.
        yard.answer(
            "GiveSecret",
            r#"{"error": {"code": "error", "message": "GiveSecret is the operator's to give"}}"#,
        );
        cx.simulate_input(VALUE);
        cx.simulate_keystrokes("enter");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "refused", |cx| {
            notice(&view, cx) == "not given: GiveSecret is the operator's to give"
        });
        nowhere(&view, cx, "refused");

        // A refusal that repeated the value, which the yard's never does,
        // is not shown.
        yard.answer(
            "GiveSecret",
            &json!({"error": {"code": "error", "message": format!("cannot take {VALUE}")}})
                .to_string(),
        );
        cx.simulate_input(VALUE);
        cx.simulate_keystrokes("enter");
        wait(cx, "a refusal with the value", |cx| {
            notice(&view, cx) == "not given: the yard refused it"
        });
        nowhere(&view, cx, "a refusal with the value");

        // Given: the yard's words for where it is set, and the ask among
        // the decided as one that gave the name.
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [
                queued_secret(&["BOOKS_TOKEN"], 7, "answered", &["BOOKS_TOKEN"])
            ]}})
            .to_string(),
        );
        yard.answer(
            "GiveSecret",
            r#"{"result": {"given": ["BOOKS_TOKEN"], "file": "/home/jo/.yardr/sources/books.env",
                "credential": "BOOKS_TOKEN is set in /home/jo/.yardr/sources/books.env"}}"#,
        );
        cx.simulate_input(VALUE);
        cx.simulate_keystrokes("enter");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "given", |cx| {
            notice(&view, cx)
                == "gave BOOKS_TOKEN · BOOKS_TOKEN is set in /home/jo/.yardr/sources/books.env"
        });
        assert_eq!(yard.asked("GiveSecret").len(), 5);
        let detail = decided_detail_of(&view, cx, "bead-s", 7);
        assert_eq!(detail.state_line, "you gave BOOKS_TOKEN");
        cx.run_until_parked();
        assert_eq!(secret_field(&view, cx).0, "");
        nowhere(&view, cx, "given");
        // Nothing of the value went to the yard but in that call.
        for call in yard.calls() {
            if call != "GiveSecret" {
                let asked = format!("{:?}", yard.asked(&call));
                assert!(!asked.contains(VALUE), "{call}: {asked}");
            }
        }
    }

    /// The command of the run ask in these tests: longer than the window
    /// is wide.
    fn long_command() -> String {
        let crew: Vec<String> = (1..=40).map(|n| format!("--crew builder-{n}")).collect();
        format!("yardr source grant crm read {}", crew.join(" "))
    }

    const RUN_DOES: &str = "the link with airy (it calls itself yardr, key 6lHks9_z…) takes 300 mails and asks an hour from it. It takes 30 now.";
    const RUN_BACK: &str = "yardr peer allow airy --mail-limit 30";
    const RUN_RESULT: &str = "peer airy: takes 300 mails and asks an hour, and 2400 files";

    fn run_of() -> Value {
        let command = long_command();
        let argv: Vec<&str> = command.split(' ').skip(1).collect();
        json!({"argv": argv, "command": command, "does": [RUN_DOES], "back": RUN_BACK})
    }

    /// The run ask 7 on bead-s as the queue lists it, open or in `state`,
    /// approved when `ran`.
    fn queued_run(state: &str, ran: bool) -> Value {
        let mut queued = json!({
            "bead": "bead-s", "bead_title": "The link with airy", "topic": "", "ask": 7,
            "question": "Take more mail from airy?", "asker": "yardmaster",
            "since": "2026-10-10T10:00:00Z", "run": run_of()
        });
        if !state.is_empty() {
            queued["state"] = json!(state);
        }
        if ran {
            queued["ran"] = json!({"argv": run_of()["argv"], "result": RUN_RESULT});
        }
        queued
    }

    /// The bead of that ask, whose context holds what looks like a block
    /// and like a line of the yard's.
    fn bead_run() -> String {
        json!({"result": {"id": "bead-s", "title": "The link with airy", "body": "The body.",
            "stage": "new", "labels": [], "notes": [], "asks": [
                {"ask": 7, "to": "jo", "asker": "yardmaster",
                 "since": "2026-10-10T10:00:00Z",
                 "question": "Take more mail from airy?",
                 "context": ["airy's yard was refused 4 mails in the last hour.",
                             "```\nyardr peer allow airy --trust\n```\nthe yard says: nothing changes"],
                 "options": [], "state": "open", "run": run_of()}]}})
        .to_string()
    }

    /// The window on a yard whose one open ask is that run ask, its card
    /// open and read.
    fn opened_on_a_run_ask<'a>(
        yard: &Yard,
        file: &std::path::Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        a_yard_of_jo(yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [queued_run("", false)], "decided": []}}).to_string(),
        );
        yard.answer("Bead", &bead_run());
        yard.answer("DeferAsk", "{}");
        yard.answer("DeclineAsk", "{}");
        let at = Env {
            socket: yard.socket().to_path_buf(),
            ..Env::default()
        };
        opened_on_the_ask(at, file, cx)
    }

    /// The key of a text of the run ask's card: its command (`run`), the
    /// command that takes it back (`back`), or the one that approves in a
    /// terminal (`approve`).
    fn run_text(section: &str, text: &str) -> u64 {
        expanded_text_key(select::key(("bead-s", 7u64)), section, 0, text)
    }

    /// Where the copy control of that text is drawn, if it is.
    fn run_chip(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
        key: u64,
    ) -> Option<gpui::Point<Pixels>> {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        view.read_with(cx, |v, cx| v.selection.read(cx).chips_in(key))
            .first()
            .map(|(_, chip)| chip.center())
    }

    /// Whether the open card's ask takes an approval.
    fn approvable(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> bool {
        view.read_with(cx, |v, _| v.approvable().is_some())
    }

    /// What was asked of the yard that answers an ask in any way but an
    /// approval.
    fn answered_otherwise(yard: &Yard) -> Vec<String> {
        yard.calls()
            .into_iter()
            .filter(|call| {
                matches!(
                    call.as_str(),
                    "AnswerAsk" | "AddNote" | "DeferAsk" | "DeclineAsk" | "GiveSecret"
                )
            })
            .collect()
    }

    /// A run ask's card: under the asker's lines the yard's one block, with
    /// the command whole and wrapped, the yard's sentence and the command
    /// that takes it back, and directly under it the button that approves.
    /// Enter approves nothing, with words or without; the card's own key
    /// calls `ApproveAsk` with the ask and nothing of the command. A call
    /// with no answer never says approved; a refusal is shown whole; an
    /// approved ask reads `approved, ran:` with the command and its result.
    #[gpui::test]
    fn a_run_ask_s_card_shows_the_command_whole_and_is_approved_with_its_own_key(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let (view, cx) = opened_on_a_run_ask(&yard, &file, cx);
        let command = long_command();

        // The card draws no line for words, and the window has the keys.
        let in_line = view.update_in(cx, |v, window, cx| {
            v.line.focus_handle(cx).is_focused(window)
        });
        assert!(!in_line);
        assert!(drawn(cx, "run bead-s 7"));
        assert!(!drawn(cx, "words bead-s 7"));
        assert!(!drawn(cx, "send bead-s 7"));

        // The asker's lines are marked as the asker's, each of them, and
        // none is a block: the card's blocks are the yard's two commands.
        let context = view.read_with(cx, |v, _| {
            let model = v.model.as_ref().unwrap();
            let detail = model.decision_detail("bead-s", &chrono::Utc::now());
            detail.unwrap().asks[0].context.clone()
        });
        assert_eq!(context.len(), 5);
        assert!(
            context
                .iter()
                .all(|line| line.starts_with("yardmaster says: "))
        );
        assert_eq!(
            view.read_with(cx, |v, cx| v.selection.read(cx).chip_count()),
            2
        );

        // The command is whole and wraps: its block stands many lines
        // high, where the one-line command under it stands one, and both
        // are inside the yard's block. Its control copies it to the letter.
        let block = bounds_of(cx, "run bead-s 7");
        let (whole, back) = (
            bounds_of(cx, "run command bead-s 7"),
            bounds_of(cx, "run back bead-s 7"),
        );
        assert!(whole.size.height > back.size.height * 2., "{whole:?}");
        assert!(whole.right() <= block.right(), "{whole:?} in {block:?}");
        assert!(block.top() <= whole.top() && back.bottom() <= block.bottom());
        assert!(whole.bottom() <= back.top());
        let chip = run_chip(&view, cx, run_text("run", &command)).unwrap();
        cx.simulate_click(chip, gpui::Modifiers::default());
        assert_eq!(clipboard(cx).as_deref(), Some(command.as_str()));

        // The control that approves is directly under the yard's block.
        let approve = bounds_of(cx, "approve bead-s 7");
        assert!(
            approve.top() >= block.bottom(),
            "{approve:?} under {block:?}"
        );
        assert!(approve.top() - block.bottom() < px(24.), "{approve:?}");

        // Enter alone approves nothing, and nothing typed goes with it:
        // the card has no line to type in, and the picking keys no option.
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("i");
        cx.simulate_input("yes");
        cx.simulate_keystrokes("enter 1 enter up enter");
        cx.run_until_parked();
        assert_eq!(typed(&view, cx), "");
        assert_eq!(yard.asked("ApproveAsk"), Vec::<Value>::new());
        assert_eq!(answered_otherwise(&yard), Vec::<String>::new());

        // Its own key. The yard goes away over the call: never "approved".
        yard.answer_nothing("ApproveAsk");
        cx.simulate_keystrokes("secondary-enter");
        wait(cx, "no answer", |cx| {
            notice(&view, cx) == "the yard did not answer; read the card again"
        });
        assert_eq!(yard.asked("ApproveAsk"), [json!(["bead-s", 7, "jo"])]);
        assert!(drawn(cx, "approve bead-s 7"));

        // The yard refuses, since it changed after the ask was written:
        // its words, whole.
        let stale = "ask 7 says the link with airy takes 30 mails an hour now, and it takes 100: the card is from before that changed. Nothing ran. Decline it; it is asked again if it is still wanted.";
        yard.answer(
            "ApproveAsk",
            &json!({"error": {"code": "error", "message": stale}}).to_string(),
        );
        click_on(cx, "approve bead-s 7");
        wait(cx, "refused", |cx| {
            notice(&view, cx) == format!("not approved: {stale}")
        });
        assert!(drawn(cx, "approve bead-s 7"));

        // Approved: the command as it ran and what it printed, and the
        // ask among the decided, which reads the same.
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [queued_run("answered", true)]}}).to_string(),
        );
        yard.answer(
            "ApproveAsk",
            &json!({"result": {"bead": "bead-s", "ask": 7, "asker": "yardmaster",
                "argv": run_of()["argv"], "does": [RUN_DOES], "result": RUN_RESULT}})
            .to_string(),
        );
        cx.simulate_keystrokes("secondary-enter");
        wait(cx, "approved", |cx| {
            notice(&view, cx) == format!("approved, ran: {command} · {RUN_RESULT}")
        });
        assert_eq!(yard.asked("ApproveAsk").len(), 3);
        assert_eq!(answered_otherwise(&yard), Vec::<String>::new());
        let detail = decided_detail_of(&view, cx, "bead-s", 7);
        assert_eq!(detail.state_line, format!("approved, ran: {command}"));
        open_decided(&view, cx, "bead-s", 7);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-s"));
        assert!(drawn(cx, "ran bead-s 7"));
        assert!(drawn(cx, "run bead-s 7"));
        // A command runs once for one ask: no control, and no key.
        assert!(!approvable(&view, cx));
        assert!(!drawn(cx, "words bead-s 7"));
        cx.simulate_keystrokes("secondary-enter enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("ApproveAsk").len(), 3);
        assert_eq!(answered_otherwise(&yard), Vec::<String>::new());
    }

    /// Every card names the yard it is of, and a run ask's block and its
    /// control name it again; with one yard in the settings as with
    /// several. The approval goes to the yard the card names and to no
    /// other, after a switch too: each yard here holds the same ask on the
    /// same bead, so only the yard tells them apart.
    #[gpui::test]
    fn a_run_ask_is_approved_in_the_yard_its_card_names_after_a_switch_too(
        cx: &mut TestAppContext,
    ) {
        assert_eq!(card_head("plantime", "k3vd", "2h"), "plantime · k3vd · 2h");
        assert_eq!(
            run_runs("plantime", "mckean"),
            "in plantime, when you approve, the yard runs this as mckean, and nothing else:"
        );
        assert_eq!(
            approve_label("plantime"),
            format!("Approve in plantime ({APPROVE_KEY})")
        );

        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        let second = Yard::start();
        let far = Yard::start_tcp();
        a_yard_of_jo(&near);
        a_yard_of_jo(&second);
        a_far_yard_of_jo(&far);
        let yards = [
            ("this machine", &near),
            ("second home", &second),
            ("far", &far),
        ];
        for (name, yard) in yards {
            yard.answer(
                "UserAsks",
                &json!({"result": {"open": [queued_run("", false)], "decided": []}}).to_string(),
            );
            yard.answer("Bead", &bead_run());
            // Each refuses in words of its own: what the window says then
            // is the word of the yard that was asked.
            yard.answer(
                "ApproveAsk",
                &json!({"error": {"code": "error", "message": format!("{name} refuses")}})
                    .to_string(),
            );
        }
        let approvals = |cx: &mut VisualTestContext| {
            cx.run_until_parked();
            yards.map(|(_, yard)| yard.asked("ApproveAsk").len())
        };
        // On the card: the header and the sentence over the command. That
        // it is the open yard's is asked of the window, since what was
        // drawn once is found by its selector in every frame after.
        fn names(view: &Entity<Footplate>, cx: &mut VisualTestContext, name: &str) -> bool {
            let read = view.read_with(cx, |v, _| {
                v.yard == name && v.model.as_ref().is_some_and(|m| m.has_bead("bead-s"))
            });
            cx.update(|window, _| window.refresh());
            read && drawn(cx, &format!("in {name} bead-s 7")) && drawn(cx, &run_runs(name, "jo"))
        }

        // One yard in the settings: its name is on the card all the same.
        let alone = dir.path().join("alone/settings.json");
        let at = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_on_the_ask(at.clone(), &alone, cx);
        assert_eq!(settings_of(&view, cx).yards.len(), 1);
        assert!(names(&view, cx, "this machine"));
        assert!(drawn(cx, "approve bead-s 7"));
        cx.update(|window, _| window.remove_window());

        // Several: the approval goes to the yard on the card.
        let file = three_yards(dir.path(), &second, &far);
        let (view, cx) = opened_on_the_ask(at, &file, cx);
        assert!(names(&view, cx, "this machine"));
        cx.simulate_keystrokes("secondary-enter");
        wait(cx, "this machine's refusal", |cx| {
            notice(&view, cx) == "not approved: this machine refuses"
        });
        assert_eq!(approvals(cx), [1, 0, 0]);

        // After a switch the card names the other yard, and its key and
        // its button approve there.
        for (key, name, asked) in [
            ("secondary-2", "second home", [1, 1, 0]),
            ("secondary-3", "far", [1, 1, 1]),
        ] {
            cx.simulate_keystrokes(key);
            wait(cx, "the other yard's card", |cx| names(&view, cx, name));
            cx.simulate_keystrokes("secondary-enter");
            wait(cx, "that yard's refusal", |cx| {
                notice(&view, cx) == format!("not approved: {name} refuses")
            });
            assert_eq!(approvals(cx), asked);
        }
        click_on(cx, "approve bead-s 7");
        wait(cx, "a second call in the far yard", |cx| {
            let _ = cx;
            far.asked("ApproveAsk").len() == 2
        });
        assert_eq!(approvals(cx), [1, 1, 2]);

        // And back: the first yard's card, and its yard alone is asked.
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's card again", |cx| {
            names(&view, cx, "this machine")
        });
        cx.simulate_keystrokes("secondary-enter");
        wait(cx, "this machine's refusal again", |cx| {
            notice(&view, cx) == "not approved: this machine refuses"
        });
        assert_eq!(approvals(cx), [2, 1, 2]);
    }

    /// Against a yard that does not have the call, by its socket, or does
    /// not serve it to this window, at its listener, the card says to
    /// approve at the yard's terminal, with that command in a block that
    /// copies it, and has no control that approves. Later works as on any
    /// card.
    #[gpui::test]
    fn a_yard_that_takes_no_approval_from_the_window_sends_it_to_a_terminal(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("mac.token");
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        let near = Yard::start();
        let far = Yard::start_tcp();
        a_yard_of_jo(&near);
        a_far_yard_of_jo(&far);
        for yard in [&near, &far] {
            yard.answer(
                "UserAsks",
                &json!({"result": {"open": [queued_run("", false)], "decided": []}}).to_string(),
            );
            yard.answer("Bead", &bead_run());
            yard.answer("DeferAsk", "{}");
        }
        // The window is opened on the far yard: the near one is asked by
        // its client, as the window asks.
        let err = near.client().approve_ask("bead-s", 7, "jo").unwrap_err();
        assert!(model::approves_elsewhere(&err), "{err}");

        let at = Env {
            yard: Some(far.address().to_string()),
            token_file: Some(token),
            socket: dir.path().join("no.sock"),
        };
        let (view, cx) = opened_on_the_ask(at, &file, cx);
        assert!(drawn(cx, "approve bead-s 7"));
        assert!(!drawn(cx, "approve elsewhere bead-s 7"));

        cx.simulate_keystrokes("secondary-enter");
        wait(cx, "where to approve", |cx| {
            drawn(cx, "approve elsewhere bead-s 7")
        });
        assert_eq!(
            far.asked("ApproveAsk"),
            [json!({"bead": "bead-s", "ask": 7, "actor": "jo"})]
        );
        assert!(far.last_request().contains("/user/call/ApproveAsk"));
        // In place of the control, and not beside it.
        let elsewhere = view.read_with(cx, |v, _| {
            v.approve_elsewhere.contains(&("bead-s".to_string(), 7))
        });
        assert!(elsewhere && approvable(&view, cx));
        assert!(drawn(cx, "run bead-s 7"));
        assert_eq!(notice(&view, cx), "");
        // The command that approves there, copied to the letter.
        let terminal = "yardr bead answer bead-s 7 --approve";
        let chip = run_chip(&view, cx, run_text("approve", terminal)).unwrap();
        cx.simulate_click(chip, gpui::Modifiers::default());
        assert_eq!(clipboard(cx).as_deref(), Some(terminal));
        // The key asks the yard no second time.
        cx.simulate_keystrokes("secondary-enter");
        cx.run_until_parked();
        assert_eq!(far.asked("ApproveAsk").len(), 1);

        // Later, as on any card.
        cx.simulate_keystrokes("x");
        wait(cx, "put aside", |_| {
            far.asked("DeferAsk") == [json!({"bead": "bead-s", "ask": 7, "actor": "jo"})]
        });
    }

    /// A run ask is declined with words as any ask is: `d` asks for them
    /// in a line the card has for nothing else, and Enter there sends the
    /// decline and approves nothing. One put aside is approved from its
    /// card among the decided.
    #[gpui::test]
    fn a_run_ask_is_declined_with_words_and_approved_after_it_was_put_aside(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let (view, cx) = opened_on_a_run_ask(&yard, &file, cx);
        yard.answer(
            "ApproveAsk",
            &json!({"result": {"argv": run_of()["argv"], "result": RUN_RESULT}}).to_string(),
        );

        cx.simulate_keystrokes("d");
        wait(cx, "the line of a decline", |cx| {
            drawn(cx, "words bead-s 7")
        });
        // With the line focused the card's key is the line's: nothing is
        // approved while a decline is typed.
        cx.simulate_input("not this week");
        cx.simulate_keystrokes("secondary-enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("ApproveAsk"), Vec::<Value>::new());
        // Esc leaves the decline, and the line with it.
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        let (declining, in_line) = view.update_in(cx, |v, window, cx| {
            (
                v.declining.is_some(),
                v.line.focus_handle(cx).is_focused(window),
            )
        });
        assert!(!declining && !in_line);
        assert!(yard.asked("DeclineAsk").is_empty());
        view.update(cx, |v, cx| {
            v.line.update(cx, |line, cx| line.set_text("", cx))
        });

        // Put aside, it is among the decided with its block and the
        // control, and is approved from there.
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [queued_run("put aside", false)]}})
                .to_string(),
        );
        cx.simulate_keystrokes("x");
        wait(cx, "put aside", |_| yard.asked("DeferAsk").len() == 1);
        open_decided(&view, cx, "bead-s", 7);
        wait(cx, "the card among the decided", |cx| {
            drawn(cx, "approve bead-s 7")
        });
        assert!(drawn(cx, "run bead-s 7"));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("ApproveAsk"), Vec::<Value>::new());
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [queued_run("answered", true)]}}).to_string(),
        );
        click_on(cx, "approve bead-s 7");
        wait(cx, "approved", |cx| {
            notice(&view, cx).starts_with("approved, ran: yardr source grant crm read")
        });
        assert_eq!(yard.asked("ApproveAsk"), [json!(["bead-s", 7, "jo"])]);
    }

    /// A taken-back run ask keeps the proposal for the record but has no
    /// approval control or answer field, including on its card among the
    /// decided asks.
    #[gpui::test]
    fn a_taken_back_run_ask_shows_its_proposal_without_an_approval(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [queued_run("taken back", false)]}})
                .to_string(),
        );
        let mut bead: Value = serde_json::from_str(&bead_run()).unwrap();
        bead["result"]["asks"][0]["state"] = json!("taken back");
        yard.answer("Bead", &bead.to_string());
        let at = Env {
            socket: yard.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(at, &file, cx);
        wait(cx, "the taken-back ask", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|model| model.decided_count() == 1)
            })
        });
        open_decided(&view, cx, "bead-s", 7);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-s"));
        wait(cx, "the taken-back card", |cx| {
            drawn(cx, "run bead-s 7") && !approvable(&view, cx)
        });

        assert_eq!(
            decided_detail_of(&view, cx, "bead-s", 7).state_line,
            "taken back"
        );
        assert!(!drawn(cx, "approve bead-s 7"));
        assert!(!drawn(cx, "words bead-s 7"));
        cx.simulate_keystrokes("secondary-enter enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("ApproveAsk"), Vec::<Value>::new());
        assert_eq!(answered_otherwise(&yard), Vec::<String>::new());
    }

    /// `d` on a run ask's card, then words and Enter: `DeclineAsk` with
    /// the words, and no approval.
    #[gpui::test]
    fn a_run_ask_s_decline_sends_its_words(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let (_view, cx) = opened_on_a_run_ask(&yard, &file, cx);
        cx.simulate_keystrokes("d");
        wait(cx, "the line of a decline", |cx| {
            drawn(cx, "words bead-s 7")
        });
        cx.simulate_input("not this week");
        cx.simulate_keystrokes("enter");
        wait(cx, "declined", |_| {
            yard.asked("DeclineAsk")
                == [json!({"bead": "bead-s", "ask": 7, "words": "not this week", "actor": "jo"})]
        });
        assert_eq!(yard.asked("ApproveAsk"), Vec::<Value>::new());
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
    }

    #[test]
    fn a_command_is_fenced_past_any_backticks_in_it() {
        assert_eq!(
            fenced("yardr peer show airy"),
            "```\nyardr peer show airy\n```"
        );
        assert_eq!(fenced("a ```` b ` c"), "`````\na ```` b ` c\n`````");
        let blocks = crate::markdown::blocks(&fenced("a\n```\nb"));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "a\n```\nb");
    }

    /// What the yard says an ask for a person's own address in a notify
    /// source asks for.
    const ADDRESS_LINE: &str = "a secret: PUSHOVER_USER, ada's own address in the source pushover (https://api.pushover.net); ada is told through pushover once it is given and yardr user set ada notify=pushover is run";

    /// The ask 7 on bead-s for ada's own address in the source pushover,
    /// as the queue lists it and as the bead has it.
    fn an_address_ask(state: &str, given: &[&str]) -> (Value, String) {
        let secret = json!({"source": "pushover", "env": ["PUSHOVER_USER"],
                            "user": "ada", "line": ADDRESS_LINE});
        let mut queued = queued_secret(&["PUSHOVER_USER"], 7, state, given);
        queued["secret"] = secret.clone();
        let mut bead: Value = serde_json::from_str(&bead_secret(&["PUSHOVER_USER"], &[7])).unwrap();
        bead["result"]["asks"][0]["secret"] = secret;
        (queued, bead.to_string())
    }

    /// An ask for a person's own address in a notify source is a secret
    /// ask as any other: the yard's line whole, which names the person and
    /// the command that tells them, and the field that shows marks. The
    /// card leaves with the giving, so the notice ends with what the yard
    /// says is left to do.
    #[gpui::test]
    fn an_address_given_ends_with_what_the_yard_says_is_next(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let (queued, bead) = an_address_ask("", &[]);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [queued], "decided": []}}).to_string(),
        );
        yard.answer("Bead", &bead);
        let at = Env {
            socket: yard.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_on_the_ask(at, &file, cx);

        assert!(drawn(cx, ADDRESS_LINE));
        assert!(drawn(cx, "secret bead-s 7"));
        assert!(!drawn(cx, "words bead-s 7"));
        assert_eq!(
            secret_field(&view, cx).2,
            "PUSHOVER_USER for the source pushover"
        );
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        let (typed_in, shown, _) = secret_field(&view, cx);
        assert_eq!(
            (typed_in.as_str(), shown),
            (VALUE, "\u{2022}".repeat(VALUE.len()))
        );

        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [
                an_address_ask("answered", &["PUSHOVER_USER"]).0
            ]}})
            .to_string(),
        );
        yard.answer(
            "GiveSecret",
            r#"{"result": {"given": ["PUSHOVER_USER"], "file": "/home/jo/.yardr/sources/pushover.env",
                "user": "ada",
                "credential": "PUSHOVER_USER is set in /home/jo/.yardr/sources/pushover.env",
                "next": "yardr user set ada notify=pushover tells ada through it"}}"#,
        );
        cx.simulate_keystrokes("enter");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "given, and what is next", |cx| {
            notice(&view, cx)
                == "gave PUSHOVER_USER · PUSHOVER_USER is set in /home/jo/.yardr/sources/pushover.env · yardr user set ada notify=pushover tells ada through it"
        });
        assert_eq!(
            yard.asked("GiveSecret"),
            [
                json!({"bead": "bead-s", "ask": 7, "values": {"PUSHOVER_USER": VALUE}, "actor": "jo"})
            ]
        );
        let kept = kept_outside_the_field(&view, cx, dir.path());
        assert!(!kept.contains(VALUE), "{kept}");
    }

    /// At its listener the yard takes a secret from the device of a user
    /// who is not the operator only for that user's own address. For a
    /// source's ask it answers such a device as a yard without the call
    /// does, and the card sends its person to a terminal.
    #[gpui::test]
    fn a_device_the_yard_takes_no_secret_from_is_sent_to_a_terminal(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let token = dir.path().join("mac.token");
        yard::keep_token(&token, crate::fake::TOKEN).unwrap();
        let far = Yard::start_tcp();
        a_far_yard_of_jo(&far);
        far.answer(
            "UserAsks",
            &json!({"result": {"open": [queued_secret(&["BOOKS_TOKEN"], 7, "", &[])], "decided": []}})
                .to_string(),
        );
        far.answer("Bead", &bead_secret(&["BOOKS_TOKEN"], &[7]));
        let at = Env {
            yard: Some(far.address().to_string()),
            token_file: Some(token),
            socket: dir.path().join("no.sock"),
        };
        let (view, cx) = opened_on_the_ask(at, &file, cx);
        assert!(drawn(cx, BOOKS_LINE));
        assert!(drawn(cx, "secret bead-s 7"));

        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        cx.simulate_keystrokes("enter");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "the yard that takes none from this device", |cx| {
            notice(&view, cx)
                == "this yard takes no secret from footplate. Answer in a terminal on the yard's machine: yardr bead answer bead-s 7 --secret"
        });
        assert_eq!(
            far.asked("GiveSecret"),
            [json!({"bead": "bead-s", "ask": 7, "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo"})]
        );
        assert!(far.last_request().contains("/user/call/"));
        let kept = kept_outside_the_field(&view, cx, dir.path());
        assert!(!kept.contains(VALUE), "{kept}");
    }

    /// An ask of two names takes one after the other and sends them
    /// together; esc leaves and nothing is sent.
    #[gpui::test]
    fn a_secret_of_two_names_goes_together_and_esc_forgets(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let names = ["CRM_TOKEN", "CRM_UPDATE_TOKEN"];
        let (view, cx) = opened_on_a_secret_ask(&yard, &names, &file, cx);
        yard.answer(
            "GiveSecret",
            r#"{"result": {"given": ["CRM_TOKEN", "CRM_UPDATE_TOKEN"], "file": "/home/jo/.yardr/sources/books.env"}}"#,
        );
        let says = |view: &Entity<Footplate>, cx: &mut VisualTestContext| secret_field(view, cx).2;
        assert_eq!(says(&view, cx), "CRM_TOKEN for the source books (1 of 2)");

        // The first: nothing goes yet, and the field asks for the second.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("i");
        cx.simulate_input("first-value");
        cx.simulate_keystrokes("enter");
        assert_eq!(
            secret_field(&view, cx),
            (
                String::new(),
                String::new(),
                "CRM_UPDATE_TOKEN for the source books (2 of 2)".to_string()
            )
        );
        // Enter on nothing sends nothing, and says what is wanted.
        cx.simulate_keystrokes("enter");
        assert_eq!(notice(&view, cx), "nothing is typed for CRM_UPDATE_TOKEN");
        // Esc leaves: the first is forgotten with what is typed.
        cx.simulate_input("half");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(says(&view, cx), "CRM_TOKEN for the source books (1 of 2)");
        assert_eq!(secret_field(&view, cx).0, "");
        assert_eq!(yard.asked("GiveSecret"), Vec::<Value>::new());
        let kept = kept_outside_the_field(&view, cx, dir.path());
        assert!(
            !kept.contains("first-value") && !kept.contains("half"),
            "{kept}"
        );

        // Both, one after the other, in the one call.
        cx.simulate_keystrokes("i");
        cx.simulate_input("first-value");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("second-value");
        cx.simulate_keystrokes("enter");
        wait(cx, "both given", |_| {
            yard.asked("GiveSecret")
                == [json!({"bead": "bead-s", "ask": 7, "actor": "jo",
                           "values": {"CRM_TOKEN": "first-value", "CRM_UPDATE_TOKEN": "second-value"}})]
        });
    }

    /// Decline on a secret ask goes at once and with no words, whatever is
    /// typed in the field, which is forgotten.
    #[gpui::test]
    fn a_secret_ask_is_declined_at_once_with_no_words(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let (view, cx) = opened_on_a_secret_ask(&yard, &["BOOKS_TOKEN"], &file, cx);
        yard.answer("DeclineAsk", "{}");
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        assert_eq!(secret_field(&view, cx).0, VALUE);
        cx.simulate_keystrokes("escape");
        assert_eq!(secret_field(&view, cx).0, "");
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        click_on(cx, "decline bead-s 7");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "declined", |_| {
            yard.asked("DeclineAsk")
                == [json!({"bead": "bead-s", "ask": 7, "words": "", "actor": "jo"})]
        });
        cx.run_until_parked();
        assert_eq!(typed(&view, cx), "");
        assert_eq!(yard.asked("GiveSecret"), Vec::<Value>::new());
        let kept = kept_outside_the_field(&view, cx, dir.path());
        assert!(!kept.contains(VALUE), "{kept}");
    }

    /// One that was given has the field to give it again, and no line for
    /// words; one that was declined has neither.
    #[gpui::test]
    fn a_secret_given_is_given_again_from_its_card_and_a_declined_one_takes_none(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [
                queued_secret(&["BOOKS_TOKEN"], 7, "answered", &["BOOKS_TOKEN"]),
                queued_secret(&["BOOKS_TOKEN"], 8, "declined", &[])
            ]}})
            .to_string(),
        );
        yard.answer("Bead", &bead_secret(&["BOOKS_TOKEN"], &[7, 8]));
        yard.answer(
            "GiveSecret",
            r#"{"result": {"given": ["BOOKS_TOKEN"], "file": "/home/jo/.yardr/sources/books.env"}}"#,
        );
        let (view, cx) = opened(&yard, &["Shall I land it?"], cx);
        wait(cx, "the decided", |cx| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().decided_count() == 2)
        });

        open_decided(&view, cx, "bead-s", 7);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-s"));
        assert!(drawn(cx, BOOKS_LINE));
        assert!(drawn(cx, "secret bead-s 7"));
        assert!(!drawn(cx, "words bead-s 7"));
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        assert_eq!(typed(&view, cx), "");
        cx.simulate_keystrokes("enter");
        wait(cx, "given again", |_| {
            yard.asked("GiveSecret")
                == [
                    json!({"bead": "bead-s", "ask": 7, "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo"}),
                ]
        });
        cx.run_until_parked();

        open_decided(&view, cx, "bead-s", 8);
        assert!(drawn(cx, BOOKS_LINE));
        assert!(!drawn(cx, "secret bead-s 8"));
        assert!(!drawn(cx, "words bead-s 8"));
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(yard.asked("GiveSecret").len(), 1);
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
    }

    /// A secret ask its asker took back says who took it back and why, as
    /// the yard serves it, and its card has no field: the yard would refuse
    /// the value. One another yard took back says where it ended.
    #[gpui::test]
    fn a_taken_back_secret_ask_says_who_and_why_and_offers_no_field(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let mut taken = queued_secret(&["BOOKS_TOKEN"], 7, "taken back", &[]);
        taken["words"] = json!("the books moved");
        taken["by"] = json!("yardmaster");
        let mut foreign = queued_secret(&["BOOKS_TOKEN"], 8, "taken back", &[]);
        foreign["words"] = json!("it was taken back in north's yard");
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [taken, foreign]}}).to_string(),
        );
        yard.answer("Bead", &bead_secret(&["BOOKS_TOKEN"], &[7, 8]));
        let (view, cx) = opened(&yard, &["Shall I land it?"], cx);
        wait(cx, "the decided", |cx| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().decided_count() == 2)
        });
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
                "yardmaster · taken back by yardmaster: the books moved",
                "yardmaster · it was taken back in north's yard",
            ]
        );

        open_decided(&view, cx, "bead-s", 7);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-s"));
        assert_eq!(
            decided_detail_of(&view, cx, "bead-s", 7).state_line,
            "taken back by yardmaster: the books moved"
        );
        assert!(drawn(cx, BOOKS_LINE));
        assert!(!drawn(cx, "secret bead-s 7"));
        assert!(!drawn(cx, "words bead-s 7"));
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        open_decided(&view, cx, "bead-s", 8);
        assert_eq!(
            decided_detail_of(&view, cx, "bead-s", 8).state_line,
            "it was taken back in north's yard"
        );
        assert!(!drawn(cx, "secret bead-s 8"));
        cx.run_until_parked();
        assert_eq!(yard.asked("GiveSecret"), Vec::<Value>::new());
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
    }

    /// An ask its asker took back reads only: its card draws no option to
    /// pick and no line for words, with options or without, and no key
    /// sends anything from it, since the yard refuses an answer to an ask
    /// that is asked no more. An answered or a declined one keeps its
    /// options and its line.
    #[gpui::test]
    fn a_taken_back_ask_reads_only_and_an_answered_or_declined_one_is_picked_again(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let mut asks: Value =
            serde_json::from_str(&asks_decided(1, "Switch to attention")).unwrap();
        let decided = asks["result"]["decided"].as_array_mut().unwrap();
        for (ask, question) in [(17, "Move the depot?"), (18, "Who drives it?")] {
            decided.push(json!({
                "bead": "bead-d", "bead_title": "Switch the inbox", "topic": "topic:linear-out",
                "ask": ask, "question": question, "asker": "mayor",
                "since": "2026-10-12T11:07:00Z", "state": "taken back",
                "by": "mayor", "words": "the train left"
            }));
        }
        let mut bead: Value = serde_json::from_str(&bead_decided()).unwrap();
        let on_bead = bead["result"]["asks"].as_array_mut().unwrap();
        on_bead.push(decided_bead_ask(17, "Move the depot?", "taken back", true));
        on_bead.push(decided_bead_ask(18, "Who drives it?", "taken back", false));
        yard.answer("UserAsks", &asks.to_string());
        yard.answer("Bead", &bead.to_string());
        yard.answer("AnswerAsk", "{}");
        let (view, cx) = opened(&yard, &["Shall I land it?"], cx);
        wait(cx, "the decided", |cx| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().decided_count() == 8)
        });

        // With options: none is drawn to click, and no line. The focus the
        // conversation left in the line is the window's, and neither a
        // number, an arrow nor `i` and Enter chooses or sends anything.
        open_decided(&view, cx, "bead-d", 17);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-d"));
        let d = decided_detail_of(&view, cx, "bead-d", 17);
        assert_eq!(d.state_line, "taken back by mayor: the train left");
        assert_eq!(d.options.len(), 4);
        assert_eq!(decided_keys(&d), "");
        for n in 1..=4 {
            assert!(!drawn(cx, &format!("pick bead-d 17 {n}")));
        }
        assert!(!drawn(cx, "words bead-d 17"));
        assert!(!drawn(cx, "send bead-d 17"));
        cx.simulate_keystrokes("2");
        cx.simulate_keystrokes("down");
        assert_eq!(decided_detail_of(&view, cx, "bead-d", 17).chosen, 0);
        cx.simulate_keystrokes("i");
        cx.simulate_input("anyway");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(typed(&view, cx), "");
        assert!(!drawn(cx, "send bead-d 17"));

        // Without options: no line for the words that would be the answer.
        open_decided(&view, cx, "bead-d", 18);
        let d = decided_detail_of(&view, cx, "bead-d", 18);
        assert_eq!(d.state_line, "taken back by mayor: the train left");
        assert_eq!(decided_keys(&d), "");
        assert!(!drawn(cx, "words bead-d 18"));
        assert!(!drawn(cx, "send bead-d 18"));
        cx.simulate_keystrokes("i");
        cx.simulate_input("me");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(typed(&view, cx), "");
        assert_eq!(yard.asked("AnswerAsk"), Vec::<Value>::new());
        assert_eq!(yard.asked("AddNote"), Vec::<Value>::new());

        // A declined one and an answered one are drawn as before: every
        // option to click, and the line for words.
        for ask in [14, 11] {
            open_decided(&view, cx, "bead-d", ask);
            for n in 1..=4 {
                assert!(drawn(cx, &format!("pick bead-d {ask} {n}")));
            }
            assert!(drawn(cx, &format!("words bead-d {ask}")));
        }
        click_on(cx, "pick bead-d 11 2");
        assert_eq!(decided_detail_of(&view, cx, "bead-d", 11).chosen, 2);
        click_on(cx, "send bead-d 11");
        wait(cx, "the changed pick", |_| {
            yard.asked("AnswerAsk")
                == [json!({"bead": "bead-d", "ask": 11, "option": 2, "actor": "jo"})]
        });
    }

    /// The Give button beside a secret's field gives the typed value by the
    /// same path as Enter: `GiveSecret`, the field empty whatever is
    /// answered, the yard's line shown. It is disabled while the field is
    /// empty: a click then sends nothing. The value is in no label, no
    /// notice and nothing kept; the footer still names Enter. The look is
    /// untried.
    #[gpui::test]
    fn a_secret_card_s_give_button_gives_what_enter_gives(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("footplate/settings.json");
        let yard = Yard::start();
        let (view, cx) = opened_on_a_secret_ask(&yard, &["BOOKS_TOKEN"], &file, cx);
        yard.answer(
            "GiveSecret",
            r#"{"result": {"given": ["BOOKS_TOKEN"], "file": "/home/jo/.yardr/sources/books.env",
                "credential": "BOOKS_TOKEN is set in /home/jo/.yardr/sources/books.env"}}"#,
        );
        yard.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [
                queued_secret(&["BOOKS_TOKEN"], 7, "answered", &["BOOKS_TOKEN"])
            ]}})
            .to_string(),
        );
        let nowhere = |view: &Entity<Footplate>, cx: &mut VisualTestContext, when: &str| {
            let kept = kept_outside_the_field(view, cx, dir.path());
            assert!(!kept.contains(VALUE), "{when}: {kept}");
        };

        // The button stands by the field, and the footer still names Enter.
        assert!(drawn(cx, "give bead-s 7"));
        assert!(drawn(cx, "secret bead-s 7"));
        assert!(!drawn(cx, "send bead-s 7"));
        assert!(secret_keys("bead-s", 7, true).contains("enter sends it to the yard"));

        // Disabled on an empty field: a click sends nothing and says
        // nothing.
        assert_eq!(secret_field(&view, cx).0, "");
        click_on(cx, "give bead-s 7");
        cx.run_until_parked();
        assert_eq!(yard.asked("GiveSecret"), Vec::<Value>::new());
        assert_eq!(notice(&view, cx), "");

        // Typed, a click gives the value as Enter does: the call, the field
        // empty at once, the yard's line shown.
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        assert_eq!(secret_field(&view, cx).0, VALUE);
        click_on(cx, "give bead-s 7");
        assert_eq!(secret_field(&view, cx).0, "");
        wait(cx, "given by the button", |_| {
            yard.asked("GiveSecret")
                == [
                    json!({"bead": "bead-s", "ask": 7, "values": {"BOOKS_TOKEN": VALUE}, "actor": "jo"}),
                ]
        });
        wait(cx, "the yard's line", |cx| {
            notice(&view, cx)
                == "gave BOOKS_TOKEN · BOOKS_TOKEN is set in /home/jo/.yardr/sources/books.env"
        });
        nowhere(&view, cx, "given by the button");

        // The label names the variable, never the value: what the button
        // draws, from the ask as the window has it.
        let label = view.read_with(cx, |v, _| {
            let secret = v
                .model
                .as_ref()
                .unwrap()
                .secret("bead-s", 7)
                .unwrap()
                .clone();
            give_label(&secret, 0)
        });
        assert_eq!(label, "Give BOOKS_TOKEN");
        assert!(!label.contains(VALUE));

        // Given, the decided card gives again from the same button.
        open_decided(&view, cx, "bead-s", 7);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-s"));
        assert!(drawn(cx, "give bead-s 7"));
        assert!(drawn(cx, "secret bead-s 7"));
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("i");
        cx.simulate_input(VALUE);
        click_on(cx, "give bead-s 7");
        wait(cx, "given again by the button", |_| {
            yard.asked("GiveSecret").len() == 2
        });
        assert_eq!(secret_field(&view, cx).0, "");
        nowhere(&view, cx, "given again");
    }

    /// Whether the jump control stands over the open conversation.
    fn shows_jump(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> bool {
        view.read_with(cx, |v, _| v.shows_jump())
    }

    fn at_bottom(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> bool {
        view.read_with(cx, |v, _| v.at_bottom())
    }

    fn new_below(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> usize {
        view.read_with(cx, |v, _| v.new_below)
    }

    fn offset_of(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
    ) -> gpui::Point<gpui::Pixels> {
        view.read_with(cx, |v, _| v.conversation.offset())
    }

    /// Stands the open conversation forty pixels above its newest, as a
    /// reader that scrolled up: the layout keeps the offset, and the next
    /// frame draws the conversation there.
    fn scroll_up(view: &Entity<Footplate>, cx: &mut VisualTestContext) {
        view.read_with(cx, |v, _| {
            let mut at = v.conversation.offset();
            at.y += px(40.);
            v.conversation.set_offset(at);
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// Stands the open conversation at its newest by hand, as a reader
    /// that scrolled down to it.
    fn scroll_down(view: &Entity<Footplate>, cx: &mut VisualTestContext) {
        view.read_with(cx, |v, _| {
            let max = v.conversation.max_offset();
            let mut at = v.conversation.offset();
            at.y = px(0.) - max.height;
            v.conversation.set_offset(at);
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// Scrolled up, a control over the conversation jumps to its newest
    /// message on a click or its key; at the newest it is gone; new mail
    /// while scrolled up does not move the view and marks the control.
    /// That the control is gone is the window's own word: gpui's test
    /// keeps the bounds of what an earlier frame drew.
    #[gpui::test]
    fn scrolled_up_a_control_jumps_to_the_newest_and_marks_new_mail_below(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let full = ["A long enough letter to make the conversation scroll."; 30];
        mails(&yard, &full);
        let (view, cx) = opened(&yard, &full, cx);
        // Thirty mails overflow the pane: there is further up to stand.
        let over = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        assert!(over > px(40.), "the conversation does not scroll: {over:?}");

        // At the newest the control is nowhere.
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));
        assert!(!drawn(cx, "jump newest"));

        // Forty pixels above it the control stands over the text,
        // unmarked: no mail arrived below.
        scroll_up(&view, cx);
        assert!(!at_bottom(&view, cx));
        assert!(shows_jump(&view, cx));
        assert!(drawn(cx, "jump newest"));
        assert!(!drawn(cx, "jump newest count"));

        // One click puts the newest message in view, and the control is
        // gone.
        click_on(cx, "jump newest");
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));
        assert_eq!(new_below(&view, cx), 0);

        // Its key does the same from above again.
        scroll_up(&view, cx);
        assert!(shows_jump(&view, cx));
        cx.simulate_keystrokes("secondary-end");
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));

        // New mail while scrolled up does not move the view, and marks
        // the control with how many arrived below.
        scroll_up(&view, cx);
        let place = offset_of(&view, cx);
        for seq in [42, 43] {
            yard.emit(
                &json!({"seq": seq, "kind": "mail", "actor": "yardmaster",
                    "at": "2026-10-08T09:31:00Z",
                    "data": {"to": "jo", "to_user": true, "text": "Landed."}})
                .to_string(),
            );
        }
        wait(cx, "the mail below", |cx| texts(&view, cx).len() == 32);
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx), place);
        assert!(!at_bottom(&view, cx));
        assert_eq!(new_below(&view, cx), 2);
        assert!(shows_jump(&view, cx));
        assert!(drawn(cx, "jump newest count"));

        // Reaching the newest by hand clears the mark too: what arrived
        // below was seen.
        scroll_down(&view, cx);
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));
        assert_eq!(new_below(&view, cx), 0);

        // Marked again, the control's click takes the view to the newest
        // of them.
        scroll_up(&view, cx);
        yard.emit(
            &json!({"seq": 44, "kind": "mail", "actor": "yardmaster",
                "at": "2026-10-08T09:32:00Z",
                "data": {"to": "jo", "to_user": true, "text": "And moored."}})
            .to_string(),
        );
        wait(cx, "the mail below", |cx| texts(&view, cx).len() == 33);
        assert_eq!(new_below(&view, cx), 1);
        click_on(cx, "jump newest");
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
        assert_eq!(texts(&view, cx).len(), 33);
        assert_eq!(new_below(&view, cx), 0);
        assert!(!shows_jump(&view, cx));

        // At the newest new mail moves the view with it, as before.
        yard.emit(
            &json!({"seq": 45, "kind": "mail", "actor": "yardmaster",
                "at": "2026-10-08T09:33:00Z",
                "data": {"to": "jo", "to_user": true, "text": "All fast."}})
            .to_string(),
        );
        wait(cx, "the mail at the newest", |cx| {
            texts(&view, cx).len() == 34
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));
    }

    /// The yardmaster's conversation is the mails 1 to `n`, each of three
    /// lines so that a page of them is several views high, and none of
    /// them unread. The yard holds it, and answers it in pages.
    fn long_conversation(yard: &Yard, n: u64) {
        yard.answer(
            "UserInbox",
            r#"{"result": {"mail": [], "unread": 0, "last": 0, "from": []}}"#,
        );
        yard.converse(
            "yardmaster",
            (1..=n)
                .map(|seq| {
                    json!({"seq": seq, "at": "2026-10-08T09:30:00Z", "from": "yardmaster",
                        "to": "jo", "read": true, "text": format!("Mail {seq}\nof the shed\nand the fence.")})
                })
                .collect(),
        );
    }

    /// The window on `yard`, once the yardmaster's conversation shows `n`
    /// mails and is drawn at its newest.
    fn opened_on<'a>(
        yard: &Yard,
        n: usize,
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
            texts(&view, cx).len() == n
        });
        frames(cx);
        (view, cx)
    }

    /// Draws until what the layout of one frame leaves to the next is done.
    fn frames(cx: &mut VisualTestContext) {
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
        }
    }

    /// Stands the open conversation `down` pixels below its top, as a
    /// reader that scrolled up there.
    fn scroll_to(view: &Entity<Footplate>, cx: &mut VisualTestContext, down: f32) {
        view.read_with(cx, |v, _| {
            let mut at = v.conversation.offset();
            at.y = px(0.) - px(down);
            v.conversation.set_offset(at);
        });
        frames(cx);
    }

    fn top_of(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Top {
        view.read_with(cx, |v, _| v.model.as_ref().unwrap().top())
    }

    /// A long conversation opens on its newest page alone. Reaching its
    /// top asks for the page before, and the mail that stood in view
    /// stands where it stood once the older mails are in above it; new
    /// mail still arrives below.
    #[gpui::test]
    fn a_long_conversation_loads_its_newest_page_and_older_ones_at_its_top(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        long_conversation(&yard, 500);
        let (view, cx) = opened_on(&yard, 50, cx);
        let first = json!({"a": "jo", "b": "yardmaster", "limit": 50});
        assert_eq!(yard.asked("Conversation"), std::slice::from_ref(&first));
        assert!(at_bottom(&view, cx));
        assert_eq!(top_of(&view, cx), Top::Older);
        let entries = view.read_with(cx, |v, _| v.model.as_ref().unwrap().entries());
        assert_eq!((entries[0].seq, entries[49].seq), (451, 500));
        // A page is several views high: at its newest nothing is asked.
        let view_height = view.read_with(cx, |v, _| v.conversation.bounds().size.height);
        let over = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        assert!(over > view_height, "a page is no view high: {over:?}");

        // Above the top half of a view nothing is asked either.
        let half: f32 = (view_height / 2.).into();
        scroll_to(&view, cx, half + 40.);
        assert_eq!(yard.asked("Conversation").len(), 1);
        let far = [451, 452, 453].map(|seq| text_bounds(cx, seq));

        // Near the top, with a mail cut by the top of the view, the page
        // before is asked for, and that mail and those below it stand
        // where the view put them: as far below where they stood as the
        // view went up, with the older mails above them or without.
        scroll_to(&view, cx, 100.);
        let stood = far.map(|mut bounds| {
            bounds.origin.y += px(half + 40. - 100.);
            bounds
        });
        wait(cx, "the page before", |cx| texts(&view, cx).len() == 100);
        frames(cx);
        assert_eq!(
            yard.asked("Conversation"),
            [
                first.clone(),
                json!({"a": "jo", "b": "yardmaster", "limit": 50, "before": 451})
            ]
        );
        assert_eq!([451, 452, 453].map(|seq| text_bounds(cx, seq)), stood);
        assert!(!at_bottom(&view, cx));
        // The older mails are no mail that arrived below.
        assert_eq!(new_below(&view, cx), 0);
        assert_eq!(yard.asked("Conversation").len(), 2);

        // At the very top the oldest mail is the one that stays.
        scroll_to(&view, cx, 0.);
        wait(cx, "the third page", |cx| texts(&view, cx).len() == 150);
        frames(cx);
        // 401 was the oldest, at the top of the view where 451 would
        // have stood there: it stands there still, under the mails before.
        let mut at_top = far[0];
        at_top.origin.y += px(half + 40.);
        assert_eq!(text_bounds(cx, 401), at_top);
        assert_eq!(
            yard.asked("Conversation").last(),
            Some(&json!({"a": "jo", "b": "yardmaster", "limit": 50, "before": 401}))
        );

        // New mail still arrives below, and does not move the view.
        let place = offset_of(&view, cx);
        yard.emit(
            &json!({"seq": 501, "kind": "mail", "actor": "yardmaster",
                "at": "2026-10-08T09:31:00Z",
                "data": {"to": "jo", "to_user": true, "text": "Landed."}})
            .to_string(),
        );
        wait(cx, "the mail below", |cx| texts(&view, cx).len() == 151);
        frames(cx);
        assert_eq!(texts(&view, cx).last().map(String::as_str), Some("Landed."));
        assert_eq!(offset_of(&view, cx), place);
        assert_eq!(new_below(&view, cx), 1);

        assert_eq!(top_of(&view, cx), Top::Older);
        assert_eq!(yard.asked("Conversation").len(), 3);
    }

    /// At the oldest mail of a conversation its top says that it starts
    /// there, and standing there asks for nothing more.
    #[gpui::test]
    fn the_top_of_a_conversation_says_where_it_starts_and_asks_no_more(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        long_conversation(&yard, 120);
        let (view, cx) = opened_on(&yard, 50, cx);
        assert_eq!(top_of(&view, cx), Top::Older);
        for shown in [100, 120] {
            scroll_to(&view, cx, 0.);
            wait(cx, "a page before", |cx| texts(&view, cx).len() == shown);
            frames(cx);
        }
        // The third page was the twenty that were left, and the last.
        assert_eq!(
            yard.asked("Conversation"),
            [
                json!({"a": "jo", "b": "yardmaster", "limit": 50}),
                json!({"a": "jo", "b": "yardmaster", "limit": 50, "before": 71}),
                json!({"a": "jo", "b": "yardmaster", "limit": 50, "before": 21}),
            ]
        );
        assert_eq!(top_of(&view, cx), Top::Start);
        scroll_to(&view, cx, 0.);
        scroll_to(&view, cx, 10.);
        scroll_to(&view, cx, 0.);
        assert_eq!(yard.asked("Conversation").len(), 3);
        assert_eq!(texts(&view, cx).len(), 120);
        assert!(drawn(cx, "conversation top"));
    }

    /// A yard built before the conversation had pages still shows all of
    /// it: one that refuses the page is asked for the conversation whole,
    /// one that answers everything to it is taken at that, and neither is
    /// asked again at the top.
    #[gpui::test]
    fn a_yard_without_pages_shows_the_whole_conversation_and_is_not_asked_again(
        cx: &mut TestAppContext,
    ) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        long_conversation(&yard, 120);
        yard.old();
        let (view, cx) = opened_on(&yard, 120, cx);
        let asked = [
            json!({"a": "jo", "b": "yardmaster", "limit": 50}),
            json!({"a": "jo", "b": "yardmaster", "after": 0}),
        ];
        assert_eq!(yard.asked("Conversation"), asked);
        assert_eq!(top_of(&view, cx), Top::Start);
        scroll_to(&view, cx, 0.);
        scroll_to(&view, cx, 10.);
        assert_eq!(yard.asked("Conversation"), asked);
        assert_eq!(texts(&view, cx).len(), 120);
    }

    #[gpui::test]
    fn a_yard_that_answers_all_to_a_page_is_asked_once(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let full = ["A long enough letter to make the conversation scroll."; 120];
        mails(&yard, &full);
        let (view, cx) = opened(&yard, &full, cx);
        frames(cx);
        scroll_to(&view, cx, 0.);
        scroll_to(&view, cx, 10.);
        assert_eq!(
            yard.asked("Conversation"),
            [json!({"a": "jo", "b": "yardmaster", "limit": 50})]
        );
        assert_eq!(top_of(&view, cx), Top::Start);
    }

    fn card_offset_of(
        view: &Entity<Footplate>,
        cx: &mut VisualTestContext,
    ) -> gpui::Point<gpui::Pixels> {
        view.read_with(cx, |v, _| v.decision.offset())
    }

    /// Page Up and Page Down move the open conversation by a page — the
    /// height of the view less two lines — and stop at its ends, with
    /// the focus in the line; with the modifier, Home and End go to the
    /// oldest loaded and the newest message. Typing is unchanged, and
    /// plain Home and End are still the caret's.
    #[gpui::test]
    fn page_keys_move_the_conversation_by_a_page_and_stop_at_its_ends(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        let full = ["A long enough letter to make the conversation scroll."; 60];
        mails(&yard, &full);
        let (view, cx) = opened(&yard, &full, cx);
        // Sixty mails overflow the pane twice over: there is more than
        // a page above to stand.
        let over = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        assert!(over > px(40.), "the conversation does not scroll: {over:?}");
        assert!(at_bottom(&view, cx));
        let in_line = view.update_in(cx, |v, window, cx| {
            v.line.focus_handle(cx).is_focused(window)
        });
        assert!(in_line);

        // A page is the view less two lines: smaller than what stands
        // above, so one key moves by exactly it.
        let viewport = view.read_with(cx, |v, _| v.conversation.bounds().size.height);
        let line = cx.update(|window, _| window.line_height());
        let page = viewport - line - line;
        assert!(
            page > px(0.) && page < over,
            "no page to move by: {page:?} of {over:?}"
        );

        // What is typed stays through the paging, and typing goes on.
        cx.simulate_input("hello");
        assert_eq!(typed(&view, cx), "hello");
        cx.simulate_keystrokes("pageup");
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx).y, px(0.) - over + page);
        assert!(shows_jump(&view, cx));
        assert_eq!(typed(&view, cx), "hello");
        cx.simulate_input(" world");
        assert_eq!(typed(&view, cx), "hello world");

        // Up to the oldest loaded, where further keys stop; down to the
        // newest, where they stop too.
        for _ in 0..20 {
            cx.simulate_keystrokes("pageup");
        }
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx).y, px(0.));
        for _ in 0..20 {
            cx.simulate_keystrokes("pagedown");
        }
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));

        // With the modifier, Home is the oldest loaded and End the
        // newest, as the jump control's key.
        cx.simulate_keystrokes("secondary-home");
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx).y, px(0.));
        assert!(shows_jump(&view, cx));
        cx.simulate_keystrokes("secondary-end");
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
        assert!(!shows_jump(&view, cx));
        assert_eq!(new_below(&view, cx), 0);

        // Plain Home and End are still the caret's: the view stands, and
        // what is typed stays.
        cx.simulate_keystrokes("secondary-home");
        cx.run_until_parked();
        let place = offset_of(&view, cx);
        cx.simulate_keystrokes("home");
        cx.simulate_keystrokes("end");
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx), place);
        assert_eq!(typed(&view, cx), "hello world");
    }

    /// One open ask, for a card that stands alone.
    fn asks_one() -> String {
        json!({"result": {"open": [
            {"bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
             "ask": 3, "question": "Move them now?", "asker": "yardmaster",
             "since": "2026-10-10T10:00:00Z"},
        ], "decided": []}})
        .to_string()
    }

    /// The bead of that ask, with `n` notes: expanded, the card scrolls.
    fn bead_of_notes(n: usize) -> String {
        let notes: Vec<serde_json::Value> = (0..n)
            .map(|i| {
                json!({"seq": i as u64 + 2, "kind": "note", "author": "yardmaster",
                       "text": format!("note {i} of the evening's work"),
                       "at": "2026-10-10T09:30:00Z"})
            })
            .collect();
        json!({"result": {
            "id": "bead-a", "title": "Move the links",
            "body": "The body of bead-a.",
            "stage": "new", "labels": ["topic:linear-out"],
            "notes": notes,
            "asks": [{
                "ask": 3, "to": "jo", "asker": "yardmaster",
                "since": "2026-10-10T10:00:00Z",
                "question": "Move them now?",
                "context": ["The links file declares the new source."],
                "options": [
                    {"label": "Move them now", "consequence": "all 64 links point tonight"},
                    {"label": "You decide", "consequence": "leave it to yardmaster"}
                ],
                "recommend": 1, "why": "the test passed",
                "state": "open"
            }]
        }})
        .to_string()
    }

    /// On a card the page keys move the card by a page and stop at its
    /// ends, with the focus in the line or in the window's keys; with
    /// the modifier, Home and End go to its top and its end.
    #[gpui::test]
    fn page_keys_move_the_open_card_by_a_page(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserAsks", &asks_one());
        yard.answer("Bead", &bead_of_notes(60));
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
        cx.run_until_parked();

        // Expanded over sixty notes the card scrolls.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("e");
        cx.run_until_parked();
        assert!(expanded_of(&view, cx, "bead-a", 3));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let over = view.read_with(cx, |v, _| v.decision.max_offset().height);
        assert!(
            over > px(40.),
            "the expanded card does not scroll: {over:?}"
        );

        // A page is the view less two lines, and two of them stand above:
        // one key after another moves by exactly it.
        let viewport = view.read_with(cx, |v, _| v.decision.bounds().size.height);
        let line = cx.update(|window, _| window.line_height());
        let page = viewport - line - line;
        assert!(
            page > px(0.) && page + page < over,
            "no two pages to move by: {page:?} of {over:?}"
        );

        // The card starts at its top; one page down moves by a page,
        // with the window's keys held.
        assert_eq!(card_offset_of(&view, cx).y, px(0.));
        cx.simulate_keystrokes("pagedown");
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(0.) - page);

        // And with the focus in the line for words.
        cx.simulate_keystrokes("i");
        cx.run_until_parked();
        let in_line = view.update_in(cx, |v, window, cx| {
            v.line.focus_handle(cx).is_focused(window)
        });
        assert!(in_line);
        cx.simulate_keystrokes("pagedown");
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(0.) - page - page);

        // Down to its end, where further keys stop; with the modifier,
        // Home and End go to its top and its end.
        for _ in 0..20 {
            cx.simulate_keystrokes("pagedown");
        }
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(0.) - over);
        cx.simulate_keystrokes("secondary-home");
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(0.));
        cx.simulate_keystrokes("secondary-end");
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(0.) - over);
    }

    /// Sixty notices, for a yard row that scrolls.
    fn notices_many(n: usize) -> String {
        let notices: Vec<serde_json::Value> = (0..n)
            .map(|i| {
                json!({"seq": i as u64 + 10, "kind": "created", "bead": "yd-shed",
                       "text": format!("notice {i} of the evening's work"), "read": false})
            })
            .collect();
        json!({"result": {"notifications": notices, "unread": n, "last": n as u64 + 10}})
            .to_string()
    }

    /// On the yard row the page keys move the notices by a page and
    /// stop at their ends; with the modifier, Home and End go to the
    /// first notice and the newest.
    #[gpui::test]
    fn page_keys_move_the_yard_row_by_a_page(cx: &mut TestAppContext) {
        let yard = Yard::start();
        a_yard_of_jo(&yard);
        yard.answer("UserNotifications", &notices_many(60));
        mails(&yard, &["Hello."]);
        let (view, cx) = opened(&yard, &["Hello."], cx);
        view.update_in(cx, |v, window, cx| v.select(Selected::Yard, window, cx));
        wait(cx, "the yard row", |cx| {
            view.read_with(cx, |v, _| {
                v.model
                    .as_ref()
                    .is_some_and(|m| m.selected() == Some(&Selected::Yard))
            })
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        // Sixty notices overflow the pane; the row opens at its newest.
        let over = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        assert!(over > px(40.), "the yard row does not scroll: {over:?}");
        assert!(at_bottom(&view, cx));

        // A page is the view less two lines: smaller than what stands
        // above, so one key moves by exactly it.
        let viewport = view.read_with(cx, |v, _| v.conversation.bounds().size.height);
        let line = cx.update(|window, _| window.line_height());
        let page = viewport - line - line;
        assert!(
            page > px(0.) && page < over,
            "no page to move by: {page:?} of {over:?}"
        );

        // The window holds the keys on the yard row; one page up moves
        // by a page, and paging stops at both ends.
        let in_line = view.update_in(cx, |v, window, cx| {
            v.line.focus_handle(cx).is_focused(window)
        });
        assert!(!in_line);
        cx.simulate_keystrokes("pageup");
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx).y, px(0.) - over + page);
        for _ in 0..20 {
            cx.simulate_keystrokes("pageup");
        }
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx).y, px(0.));
        for _ in 0..20 {
            cx.simulate_keystrokes("pagedown");
        }
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));

        // With the modifier, Home is the first notice and End the newest.
        cx.simulate_keystrokes("secondary-home");
        cx.run_until_parked();
        assert_eq!(offset_of(&view, cx).y, px(0.));
        cx.simulate_keystrokes("secondary-end");
        cx.run_until_parked();
        assert!(at_bottom(&view, cx));
    }

    /// The window on the yard row of a yard with `n` unread notices.
    fn opened_on_the_yard_row<'a>(
        yard: &Yard,
        n: usize,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Footplate>, &'a mut VisualTestContext) {
        a_yard_of_jo(yard);
        yard.answer("UserNotifications", &notices_many(n));
        mails(yard, &["Hello."]);
        let (view, cx) = opened(yard, &["Hello."], cx);
        view.update_in(cx, |v, window, cx| v.select(Selected::Yard, window, cx));
        wait(cx, "the yard row", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().is_some_and(|m| {
                    m.selected() == Some(&Selected::Yard) && m.notices().len() == n
                })
            })
        });
        (view, cx)
    }

    /// The yard row's head has a button that marks every notice read, only
    /// while one is unread: a click asks the yard once, through the newest.
    #[gpui::test]
    fn the_yard_row_has_a_button_that_marks_every_notice_read(cx: &mut TestAppContext) {
        let yard = Yard::start();
        let (view, cx) = opened_on_the_yard_row(&yard, 3, cx);
        assert!(drawn(cx, "mark all read"));

        click_on(cx, "mark all read");
        wait(cx, "every notice read", |_| {
            yard.asked("AckUserNotifications") == [json!({"name": "jo", "through": 12})]
        });
        // Whether it is still drawn the test cannot ask: what was drawn
        // once keeps its place in the test window's record. The head draws
        // it by this count.
        wait(cx, "none unread", |cx| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().yard_unread() == 0)
        });
    }

    /// Shift-x marks every notice read, as the button does.
    #[gpui::test]
    fn shift_x_marks_every_notice_read(cx: &mut TestAppContext) {
        let yard = Yard::start();
        let (view, cx) = opened_on_the_yard_row(&yard, 3, cx);
        cx.simulate_keystrokes("shift-x");
        wait(cx, "every notice read", |_| {
            yard.asked("AckUserNotifications") == [json!({"name": "jo", "through": 12})]
        });
        wait(cx, "none unread", |cx| {
            view.read_with(cx, |v, _| v.model.as_ref().unwrap().yard_unread() == 0)
        });
    }

    /// With no notice unread the head has no button.
    #[gpui::test]
    fn the_yard_row_has_no_button_with_no_notice_unread(cx: &mut TestAppContext) {
        let yard = Yard::start();
        let (_view, cx) = opened_on_the_yard_row(&yard, 0, cx);
        assert!(!drawn(cx, "mark all read"));
    }

    /// How far above its newest the open conversation stands.
    fn above_newest(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> gpui::Pixels {
        view.read_with(cx, |v, _| {
            v.conversation.max_offset().height + v.conversation.offset().y
        })
    }

    fn open_row(view: &Entity<Footplate>, cx: &mut VisualTestContext) -> Option<Selected> {
        view.read_with(cx, |v, _| {
            v.model.as_ref().and_then(|m| m.selected().cloned())
        })
    }

    /// A yard opened again is on the row it was left on, and that row's
    /// conversation as far above its newest as it stood: not on the row a
    /// yard opens on, at its newest. Each yard has its own.
    #[gpui::test]
    fn a_yard_opened_again_is_on_its_row_where_its_conversation_stood(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        let full = ["A long enough letter to make the conversation scroll."; 30];
        mails(&near, &full);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "this machine's yard", |cx| texts(&view, cx) == full);
        let yardmaster = Some(Selected::Conversation("yardmaster".into()));
        let assembly = Some(Selected::Conversation("assembly".into()));
        assert_eq!(open_row(&view, cx), yardmaster);

        // Another row than the one the yard opens on, forty pixels above
        // its newest.
        view.update_in(cx, |v, window, cx| {
            v.select(Selected::Conversation("assembly".into()), window, cx)
        });
        wait(cx, "the other conversation", |cx| {
            cx.update(|window, _| window.refresh());
            texts(&view, cx) == full && above_newest(&view, cx) == px(0.)
        });
        let over = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        assert!(over > px(40.), "the conversation does not scroll: {over:?}");
        scroll_up(&view, cx);
        assert_eq!(above_newest(&view, cx), px(40.));

        // The second home's yard opens on its own row, at its newest.
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        assert_eq!(open_row(&view, cx), yardmaster);

        // This machine's again: the row it was left on, where it stood.
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "the place it was left at", |cx| {
            cx.update(|window, _| window.refresh());
            texts(&view, cx) == full && above_newest(&view, cx) == px(40.)
        });
        assert_eq!(open_row(&view, cx), assembly);
        assert!(shows_jump(&view, cx));
        view.read_with(cx, |v, _| assert!(v.returning.is_none()));
        // And it stays there over the frames that follow.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(above_newest(&view, cx), px(40.));

        // The second home's again: its row, at its newest as it was left.
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard again", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(open_row(&view, cx), yardmaster);
        assert!(at_bottom(&view, cx));

        // A yard left before it was put back keeps the place it had.
        cx.simulate_keystrokes("secondary-1");
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard once more", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "the place it was left at, still", |cx| {
            cx.update(|window, _| window.refresh());
            texts(&view, cx) == full && above_newest(&view, cx) == px(40.)
        });
        assert_eq!(open_row(&view, cx), assembly);
    }

    /// A yard left on a card is on that card again, expanded as it was
    /// and as far down it as it stood. A row that is gone when the yard is opened again is not
    /// opened: the yard opens as one opened for the first time.
    #[gpui::test]
    fn a_yard_opened_again_is_on_its_card_where_it_stood_or_as_new_when_the_row_is_gone(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        near.answer("UserAsks", &asks_one());
        near.answer("Bead", &bead_of_notes(60));
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        let card = Some(Selected::Decision("bead-a".into()));
        wait(cx, "the decision", |cx| open_row(&view, cx) == card);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));
        cx.run_until_parked();
        // Expanded over sixty notes the card scrolls, further than it is
        // long when folded.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("e");
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let over = view.read_with(cx, |v, _| v.decision.max_offset().height);
        assert!(over > px(400.), "the card does not scroll: {over:?}");
        view.read_with(cx, |v, _| {
            v.decision.set_offset(gpui::point(px(0.), px(-400.)))
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(-400.));

        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();

        cx.simulate_keystrokes("secondary-1");
        wait(cx, "the card where it stood", |cx| {
            cx.update(|window, _| window.refresh());
            open_row(&view, cx) == card && card_offset_of(&view, cx).y == px(-400.)
        });
        assert!(expanded_of(&view, cx, "bead-a", 3));
        view.read_with(cx, |v, _| assert!(v.returning.is_none()));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(-400.));

        // Its asks are dealt with elsewhere while the other yard is open:
        // the card is gone, and the yard opens on the yardmaster's
        // conversation at its newest, with nothing to be put back.
        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard again", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        near.answer("UserAsks", r#"{"result": {"open": [], "decided": []}}"#);
        near.emit(r#"{"seq": 31, "kind": "noted", "actor": "yardmaster", "bead": "bead-a", "data": {"note": 4}, "at": "2026-10-10T10:31:00Z"}"#);
        assert_eq!(state_of(&view, cx, "this machine"), "1 mail, 1 decision");
        wait(cx, "the yard that is not open to lose its card", |cx| {
            state_of(&view, cx, "this machine") == "1 mail"
        });
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "this machine's yard without the card", |cx| {
            texts(&view, cx) == ["Shall I land it?"]
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(
            open_row(&view, cx),
            Some(Selected::Conversation("yardmaster".into()))
        );
        assert!(at_bottom(&view, cx));
        view.read_with(cx, |v, _| assert!(v.returning.is_none()));
    }

    /// A dealt-with decision is a row too: its containing list must be
    /// open before the saved selection can be restored.
    #[gpui::test]
    fn a_yard_opened_again_restores_a_scrolled_decided_card(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        mails(&near, &["Shall I land it?"]);
        near.answer(
            "UserAsks",
            &json!({"result": {"open": [], "decided": [{
                "bead": "bead-a", "bead_title": "Move the links", "topic": "topic:linear-out",
                "ask": 3, "question": "Move them now?", "asker": "yardmaster",
                "since": "2026-10-10T10:00:00Z", "state": "answered",
                "option": 1, "option_label": "Move them now"
            }]}})
            .to_string(),
        );
        near.answer("Bead", &bead_of_notes(60));
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "the dealt-with ask", |cx| {
            view.read_with(cx, |v, _| {
                v.model.as_ref().is_some_and(|m| m.decided_count() == 1)
            })
        });
        let card = Some(Selected::Decided("bead-a".into(), 3));
        open_decided(&view, cx, "bead-a", 3);
        wait(cx, "the dealt-with card", |cx| open_row(&view, cx) == card);
        wait(cx, "the bead", |cx| has_bead(&view, cx, "bead-a"));
        view.update_in(cx, |v, _, cx| v.expand_for("bead-a".into(), 3, cx));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let over = view.read_with(cx, |v, _| v.decision.max_offset().height);
        assert!(over > px(400.), "the card does not scroll: {over:?}");
        view.read_with(cx, |v, _| {
            v.decision.set_offset(gpui::point(px(0.), px(-400.)))
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(card_offset_of(&view, cx).y, px(-400.));
        assert!(decided_detail_of(&view, cx, "bead-a", 3).expanded);

        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "the dealt-with card where it stood", |cx| {
            cx.update(|window, _| window.refresh());
            open_row(&view, cx) == card && card_offset_of(&view, cx).y == px(-400.)
        });
        assert!(decided_detail_of(&view, cx, "bead-a", 3).expanded);
        view.read_with(cx, |v, _| assert!(v.returning.is_none()));
    }

    /// A saved conversation position can be older than the newest page
    /// loaded when the yard is reopened. Older pages are fetched until the
    /// same distance from the newest can be restored.
    #[gpui::test]
    fn a_yard_opened_again_loads_older_pages_to_restore_its_conversation_scroll(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let near = Yard::start();
        a_yard_of_jo(&near);
        long_conversation(&near, 120);
        let second = Yard::start();
        a_second_yard_of_jo(&second);
        let far = Yard::start_tcp();
        let file = three_yards(dir.path(), &second, &far);
        let env = Env {
            socket: near.socket().to_path_buf(),
            ..Env::default()
        };
        let (view, cx) = opened_at(env, &file, cx);
        wait(cx, "the newest page", |cx| texts(&view, cx).len() == 50);
        let newest_page = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        scroll_to(&view, cx, 0.);
        wait(cx, "the older page", |cx| texts(&view, cx).len() == 100);
        frames(cx);

        let target = newest_page + px(200.);
        let loaded = view.read_with(cx, |v, _| v.conversation.max_offset().height);
        assert!(
            loaded > target,
            "the saved position is beyond the loaded mail"
        );
        view.read_with(cx, |v, _| {
            let mut at = v.conversation.offset();
            at.y = target - v.conversation.max_offset().height;
            v.conversation.set_offset(at);
        });
        frames(cx);
        assert_eq!(above_newest(&view, cx), target);

        cx.simulate_keystrokes("secondary-2");
        wait(cx, "the second home's yard", |cx| {
            texts(&view, cx) == ["From the second home."]
        });
        cx.simulate_keystrokes("secondary-1");
        wait(cx, "the older page", |cx| {
            cx.update(|window, _| window.refresh());
            texts(&view, cx).len() == 100
        });
        frames(cx);
        assert_eq!(above_newest(&view, cx), target);
        assert!(view.read_with(cx, |v, _| v.returning.is_none()));
    }
}
