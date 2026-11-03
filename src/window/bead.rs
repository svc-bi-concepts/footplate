//! One bead's views: the bead whole, the conversation thread with
//! its mail, and the pictures and attachments of both.

use std::any::Any;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Context, Div, Focusable, FontWeight, Image, ImageFormat, MouseButton,
    ObjectFit, Pixels, ScrollHandle, SharedString, WeakEntity, Window, div, img, prelude::*, px,
    rems, rgba, white,
};

use super::Expanded;
use super::Footplate;
use super::ask::ask_preview_head;
use super::expanded_text_key;
use super::firstrun::Clock;
use super::metrics::{
    ATTACHMENT, IMAGE_HEIGHT, IMAGE_MARGIN, LETTER_GUTTER, READING_WIDTH, drawer_fits,
};
use super::panel::{day_break, reading_column, reading_row};
use super::rail::{ImageOpen, OpenTip, Picture, RailTip, Unsent, post};
use super::screen::{screen_control, screen_icon};
use super::state::{Piece, Talk};
use super::ui::{keys_hint, send_held, small_button};
use super::words::{ask_words, bead_back_name, bead_sub, remark_color, worker_words};
use crate::api::{self, Address, BlobRef, Event, Reaction};
use crate::draft::{self, Attachment, Found};
use crate::model::{
    self, ArrivalDetail, AskPreview, Entry, Kind, Model, News, Selected, Target, Top,
};
use crate::select;
use crate::theme::Palette;
use crate::voice::{self, Ask, Caption, Speaking, Start, State, Update};

/// Whether the letter `seq` shows its time in the gutter: it reads
/// under another's head, and the pointer stands on it. A letter with
/// a head of its own already says its time there, so hovering it
/// shows nothing more. It wants the pointer alone, not the bar's yard
/// and overlays: the time is the window's own to say.
pub(crate) fn hover_clock(head: bool, hover: Option<(u64, Piece)>, seq: u64) -> bool {
    !head && hover.is_some_and(|(at, _)| at == seq)
}

/// The rows of the conversation's list that stand before its mail: the
/// line above the oldest mail, which `talk_conversation` puts first.
pub(crate) const ROWS_BEFORE_MAIL: usize = 1;

/// How often a voice session that stands looks at itself: the state words it
/// draws from the speaker's queue, the minutes it counts, and whether it has
/// stood idle too long. The update loop's own timer.
const VOICE_TICK: Duration = Duration::from_millis(200);

/// What the voice session's loop is told: something the session said, or
/// that a moment passed with nothing said.
pub(crate) enum Heard {
    Said(Update),
    Tick,
}

/// What the voice is given to speak of a request that went nowhere because
/// the yard was away, or went: Enter holds its mail in the same case, and a
/// session queues nothing, since a mail sent as the connection broke cannot
/// be told apart from one that never arrived.
const YARD_AWAY: &str = "the yard does not answer";

/// Where the bar of reactions stands over the row of the list that draws
/// it, as the `top` it takes from that row's top edge: its own 36 pixels
/// up, but no higher than the list's top (mckean's pick, ask 13938). `below`
/// is how far under the list's top the row's top edge stands, negative
/// where the scroll has carried it above, and `height` the row's. A row
/// that is wholly above the list has no bar: the bar would stay at the
/// list's top for a letter that is gone, since a wheel moves no pointer
/// and the letter stays the pointer's until the next move.
pub(crate) fn bar_top(below: Pixels, height: Pixels) -> Option<Pixels> {
    (below + height > px(0.)).then(|| px(-36.).max(-below))
}

/// What the composer's one button is, at its bottom right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Button {
    /// Starts a voice session, where the box holds nothing to send.
    Voice,
    /// Sends the draft, exactly as Enter does.
    Send,
    /// Send with nothing to send and no voice to offer in its place, or with
    /// the yard away: drawn faint, and it takes no tap.
    SendDisabled,
    /// Ends the session that stands, or is starting, and sends nothing.
    Stop,
}

impl Button {
    /// The button by whether a voice session `session` stands or is starting,
    /// whether voice is `offered` in this conversation, and whether the
    /// draft is `empty` ([`draft::empty`]). A session comes first, whatever
    /// the box holds, so that a tap can never send when the person meant to
    /// end; Enter sends what is typed all the same.
    pub(crate) fn of(session: bool, offered: bool, empty: bool) -> Button {
        match (session, empty, offered) {
            (true, _, _) => Button::Stop,
            (false, false, _) => Button::Send,
            (false, true, true) => Button::Voice,
            (false, true, false) => Button::SendDisabled,
        }
    }
}

impl Footplate {
    /// Asks the yard for the images shown it was not asked for yet: the
    /// open conversation's and the open row's expanded cards' notes'.
    pub(crate) fn pictures(&mut self, cx: &mut Context<Self>) {
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
        self.view_probe(cx);
        self.run_probe(cx);
    }

    /// Sends the draft of the open conversation as one mail: what is typed,
    /// and what is attached. A draft with a marked attachment is not sent.
    /// On a card Enter sends the pick with the line's words, or the words
    /// alone.
    pub(crate) fn send(&mut self, cx: &mut Context<Self>) {
        // While the yard is reconnecting Send is held: the draft stays
        // as typed, with the reason beside the line, and goes on Enter
        // once the yard answers. Nothing is queued: a mail sent as the
        // connection broke cannot be told apart from one that never
        // arrived, so a queue could send twice.
        if self.reconnecting() {
            return;
        }
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
        let files = self.drafts.take(&to);
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
        // The voice session standing in this conversation, if one does: a
        // typed mail sent while it stands awaits its answer there. Another
        // conversation or yard has none standing in this one, and a session
        // that ends before the mail goes registers nothing.
        let voice_id = self
            .voice
            .as_ref()
            .filter(|talk| talk.with == to)
            .map(|talk| talk.id);
        self.ask_through(
            cx,
            move |client, me| match post(client, me, &target, &text, &files) {
                Ok(blobs) => Ok(files.iter().map(|a| a.id).zip(blobs).collect::<Vec<_>>()),
                Err((at, why)) => Err(Unsent {
                    text,
                    files,
                    at,
                    why,
                }),
            },
            move |this, sent, cx| match sent {
                Ok(sent) => {
                    // It went while the session that was standing when it
                    // was written still stands in this conversation: the
                    // mail awaits its answer in the voice.
                    if let Some(talk) = this
                        .voice
                        .as_mut()
                        .filter(|talk| Some(talk.id) == voice_id && talk.with == to)
                    {
                        talk.voice.typed_sent();
                        talk.unanswered = true;
                        talk.quiet_since = (this.means.now)();
                    }
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

    /// The composer's Voice and Stop: starts a voice session in the open
    /// conversation, or ends the one that stands, as a second tap does. A
    /// session that cannot be made says why in the head, in the words of
    /// whoever refused it, and is made again only by another tap. While voice
    /// is off nothing is started and no key is read.
    pub(crate) fn toggle_voice(&mut self, cx: &mut Context<Self>) {
        if self.voice.is_some() {
            return self.end_voice(cx);
        }
        let Some((with, _)) = self.open() else {
            return;
        };
        self.voice_refusal = None;
        let (tell, told) = mpsc::channel();
        // The voice chosen in the settings speaks, and is the provider's own
        // where none was: a change since the last session reaches this one,
        // and never one that stands.
        let voice = match (self.means.voice)(
            self.settings.voice,
            self.kept.as_deref(),
            &with,
            &self.settings.voice_name,
            tell,
        ) {
            Start::On(voice) => voice,
            Start::Off => return cx.notify(),
            Start::Refused(words) => {
                self.voice_refusal = Some(words);
                return cx.notify();
            }
        };
        // What the session tells comes to the window as the yard's follower's
        // words do: from a thread of its own, through a channel, to one loop
        // that applies each on the window. The same thread is the loop's timer.
        let (feed, fed) = async_channel::unbounded::<Heard>();
        let forwarding = thread::Builder::new()
            .name("voice updates".into())
            .spawn(move || {
                loop {
                    let heard = match told.recv_timeout(VOICE_TICK) {
                        Ok(update) => Heard::Said(update),
                        Err(RecvTimeoutError::Timeout) => Heard::Tick,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    if feed.send_blocking(heard).is_err() {
                        break;
                    }
                }
            });
        if let Err(err) = forwarding {
            self.voice_refusal = Some(format!(
                "footplate could not start the voice session: {err}"
            ));
            return cx.notify();
        }
        self.voices += 1;
        let id = self.voices;
        let updates = cx.spawn(async move |this, cx| {
            while let Ok(heard) = fed.recv().await {
                if this
                    .update(cx, |this, cx| this.voice_heard(id, heard, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        self.voice = Some(Talk {
            id,
            with,
            devices: None,
            speaker: voice.speaker(),
            voice,
            _updates: updates,
            stood: None,
            caption: Caption::default(),
            words: None,
            unanswered: false,
            quiet_since: (self.means.now)(),
            speaking: Speaking::default(),
            answered_through: 0,
            state: State::Listening,
            meter: String::new(),
        });
        // A session starts on a window that shows: it starts on a frame of
        // its own. A stamp as old as the window's last frame would end it at
        // its first tick under a desktop that is read by them.
        self.means.frame.set((self.means.now)());
        self.watch_frames(id, cx);
        cx.notify();
    }

    /// Stamps `means.frame` at every frame the window has, for as long as
    /// session `id` stands: a Wayland compositor tells a client nothing of a
    /// window out of view, but sends it no frames, and `shown` reads that
    /// from how long ago the stamp is. Registered anew at each frame, as
    /// gpui runs each registration once, and by the session's id, so that a
    /// session that came after another is not stamped twice.
    fn watch_frames(&self, id: u64, cx: &mut Context<Self>) {
        fn stamp(
            window: &Window,
            id: u64,
            this: WeakEntity<Footplate>,
            frame: Rc<Cell<Instant>>,
            now: Clock,
        ) {
            window.on_next_frame(move |window, cx| {
                frame.set(now());
                let standing = this
                    .read_with(cx, |this, _| {
                        this.voice.as_ref().is_some_and(|talk| talk.id == id)
                    })
                    .unwrap_or(false);
                if standing {
                    stamp(window, id, this, frame, now);
                }
            });
        }
        let (this, handle) = (cx.weak_entity(), self.handle);
        let (frame, now) = (self.means.frame.clone(), self.means.now.clone());
        // Afterwards: the window is not to be had while a key or click that
        // came through it is still on its way, and the session a yard's answer
        // starts (the voice shot's) comes with none.
        cx.defer(move |cx| {
            // A window that is closed has no frames to stamp.
            let _ = handle.update(cx, |_, window, _| stamp(window, id, this, frame, now));
        });
    }

    /// What voice session `id` said, or that a moment passed. A session that
    /// ended says nothing to the one that came after it.
    fn voice_heard(&mut self, id: u64, heard: Heard, cx: &mut Context<Self>) {
        if self.voice.as_ref().map(|talk| talk.id) != Some(id) {
            return;
        }
        let now = (self.means.now)();
        match heard {
            Heard::Tick => self.voice_tick(now, cx),
            Heard::Said(update) => self.voice_told(update, now, cx),
        }
    }

    /// The session said `update`, in plain terms; the window does what it
    /// asks for. A session that could not be made, or lost, says why in the
    /// head; one that ended otherwise (the person's hang-up, the provider's
    /// close, its moment) leaves the composer's button saying Voice again,
    /// or Send where the box holds something.
    fn voice_told(&mut self, update: Update, now: Instant, cx: &mut Context<Self>) {
        let Some(talk) = self.voice.as_mut() else {
            return;
        };
        match update {
            // The session stands: its minutes count from here, and the
            // devices open, off the window's thread, since a Mac waits for
            // the person's answer to the microphone's consent.
            Update::Started => {
                talk.stood = Some(now);
                talk.quiet_since = now;
                talk.meter = voice::meter(Duration::ZERO);
                let (id, mic, speaker) = (talk.id, talk.voice.microphone(), talk.speaker.clone());
                let plug = self.means.devices.clone();
                cx.spawn(async move |this, cx| {
                    let opened = cx
                        .background_executor()
                        .spawn(async move { plug(mic, speaker) })
                        .await;
                    this.update(cx, |this, cx| this.voice_plugged(id, opened, cx))
                        .ok();
                })
                .detach();
            }
            // The session dropped what the voice had queued as the person
            // began: what the state words say of the voice speaking goes
            // with it.
            Update::SpeechStarted => talk.speaking.stop(),
            Update::Words(fragment) => {
                talk.caption.heard(fragment);
                talk.words = None;
                talk.quiet_since = now;
            }
            // What the caption held has gone to the yard, or was no request
            // for it: the line starts again with the person's next words.
            Update::Taken => talk.caption.taken(),
            Update::Request(text) => return self.voice_request(text, now, cx),
            Update::Ended(_) => return self.end_voice(cx),
            Update::Refused(words) => {
                self.voice_refusal = Some(words);
                return self.end_voice(cx);
            }
        }
        cx.notify();
    }

    /// The devices of voice session `id` are open, or why they are not.
    fn voice_plugged(
        &mut self,
        id: u64,
        opened: Result<Box<dyn Any + Send>, String>,
        cx: &mut Context<Self>,
    ) {
        // A session that ended meanwhile has no devices to hold: they close
        // as `opened` is dropped.
        let Some(talk) = self.voice.as_mut().filter(|talk| talk.id == id) else {
            return;
        };
        match opened {
            Ok(devices) => talk.devices = Some(devices),
            Err(why) => {
                self.voice_refusal = Some(why);
                self.end_voice(cx);
            }
        }
    }

    /// A request: what the person said, to go to the open conversation as
    /// their own mail, as Enter sends what they typed, with no file and in
    /// the yard it was spoken in. Under the same refusal: while the yard is
    /// reconnecting nothing is sent, and nothing is queued, since a mail
    /// sent as the connection broke cannot be told apart from one that never
    /// arrived. Where it is not sent the voice is told once, and the caption
    /// says so.
    fn voice_request(&mut self, text: String, now: Instant, cx: &mut Context<Self>) {
        let Some(talk) = self.voice.as_mut() else {
            return;
        };
        let (id, with) = (talk.id, talk.with.clone());
        talk.quiet_since = now;
        talk.unanswered = true;
        talk.words = None;
        // The conversation spoken in is the open one, or the mail goes to
        // nobody.
        let Some((to, target)) = self.open().filter(|(to, _)| *to == with) else {
            return self.end_voice(cx);
        };
        if self.reconnecting() {
            let shown = "the yard is away, so nothing was sent".to_string();
            return self.voice_refused(id, YARD_AWAY, shown, cx);
        }
        let session = self.session;
        self.ask_through(
            cx,
            move |client, me| {
                post(client, me, &target, &text, &[])
                    .map(|_| ())
                    .map_err(|(_, why)| why)
            },
            move |this, sent, cx| match sent {
                // It went in the yard it was spoken in, whose model has it
                // wherever the window is by now.
                Ok(()) => this.change_in(session, cx, |m| m.sent(&to)),
                Err(why) => {
                    let shown = format!("not sent to {to}: {why}");
                    this.voice_refused(id, &why, shown, cx)
                }
            },
        );
        cx.notify();
    }

    /// The request of voice session `id` did not go, or the yard went away
    /// after it: the voice is given `words` to speak, once, and the caption
    /// says `shown`. The mail is not sent again.
    fn voice_refused(&mut self, id: u64, words: &str, shown: String, cx: &mut Context<Self>) {
        let Some(talk) = self.voice.as_mut().filter(|talk| talk.id == id) else {
            return;
        };
        talk.voice.refused(words);
        talk.unanswered = false;
        talk.quiet_since = (self.means.now)();
        talk.words = Some(shown);
        cx.notify();
    }

    /// The yard went away. A request that waits for its answer will not get
    /// one spoken: the answer, if it comes, comes with the state the yard
    /// is read in when it is back. The voice says the yard does not answer,
    /// as it does of a mail the yard refused.
    pub(crate) fn voice_yard_away(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .voice
            .as_ref()
            .filter(|talk| talk.unanswered)
            .map(|talk| talk.id)
        else {
            return;
        };
        let shown = "the yard went away, so its answer will not be spoken".to_string();
        self.voice_refused(id, YARD_AWAY, shown, cx);
    }

    /// The stream told of `event`. Where it is the other side's mail in the
    /// voiced conversation, the voice is given its text, and its text only:
    /// the session knows whether an answer is waited for, and speaks it only
    /// then. The person's own mail never goes.
    pub(crate) fn voice_hears(&mut self, event: &Event) {
        let (Some(talk), Some(model)) = (self.voice.as_mut(), self.model.as_ref()) else {
            return;
        };
        let Some(text) = model.answer_in(event, &talk.with) else {
            return;
        };
        if event.seq <= talk.answered_through {
            return;
        }
        talk.answered_through = event.seq;
        talk.unanswered = false;
        talk.quiet_since = (self.means.now)();
        talk.voice.answered(text);
    }

    /// Asks that arrived from the other side of the voiced conversation: the
    /// voice is directed to the card in the window, never given anything to
    /// answer it with, and of a secret ask it is told nothing at all.
    pub(crate) fn voice_asked(&self, news: &[News]) {
        let (Some(talk), Some(model)) = (self.voice.as_ref(), self.model.as_ref()) else {
            return;
        };
        for news in news
            .iter()
            .filter(|news| news.kind == Kind::Decision && news.from == talk.with)
        {
            talk.voice.asked(match model.secret_ask(news) {
                true => Ask::Secret,
                false => Ask::Plain,
            });
        }
    }

    /// Whether the voiced conversation is the open row of the open yard.
    /// A session belongs to it, and ends when another is open.
    pub(crate) fn voice_is_here(&self) -> bool {
        self.voice.as_ref().is_none_or(|talk| {
            self.model.as_ref().and_then(|m| m.selected_conversation()) == Some(talk.with.as_str())
        })
    }

    /// A moment of a standing session, at `now`: it ends where the window no
    /// longer shows its conversation, or has stood idle for the settings'
    /// minutes (Live bills by the second); else the state words and the
    /// meter are drawn again if what they say changed.
    pub(crate) fn voice_tick(&mut self, now: Instant, cx: &mut Context<Self>) {
        let here = self.voice_is_here();
        let hidden = (self.means.hidden)();
        let limit = self.settings.voice_idle_limit();
        let Some(talk) = self.voice.as_mut() else {
            return;
        };
        let idle =
            talk.stood.is_some() && voice::idle(talk.quiet_since, now, limit, talk.unanswered);
        if !here || hidden || idle {
            return self.end_voice(cx);
        }
        let queued = talk.speaker.lock().map_or(0, |queue| queue.queued_ms());
        let state = State::of(talk.speaking.seen(queued, now), talk.unanswered);
        let meter = talk
            .stood
            .map(|at| voice::meter(now.saturating_duration_since(at)))
            .unwrap_or_default();
        if (state, &meter) != (talk.state, &talk.meter) {
            talk.state = state;
            talk.meter = meter;
            cx.notify();
        }
    }

    /// Ends the voice session, if one stands: the devices first, so that
    /// nothing more of the room goes to the session, then the hang-up, which
    /// waits for the provider's close and so is no call for the window's
    /// thread. Every leaving of the conversation comes here.
    pub(crate) fn end_voice(&mut self, cx: &mut Context<Self>) {
        let Some(Talk { devices, voice, .. }) = self.voice.take() else {
            return;
        };
        drop(devices);
        // A thread that cannot be started leaves the hang-up to the
        // handle's drop, which stops the session as the control does.
        let _ = thread::Builder::new()
            .name("voice hang-up".into())
            .spawn(move || voice.stop());
        cx.notify();
    }

    /// The session's state in quiet words, as the row names the counterpart,
    /// beside the composer's button: starting… until the session stands, then
    /// what is going on. None where no session stands.
    pub(crate) fn voice_label(&self) -> Option<String> {
        self.voice.as_ref().map(|talk| match talk.stood {
            None => "starting…".to_string(),
            Some(_) => talk.state.words(&talk.with),
        })
    }

    /// Whether voice is offered in the conversation with `with`: it is on,
    /// the conversation is with someone other than the person themselves, and
    /// what they say has somewhere to go.
    fn voice_offered(&self, model: &Model, with: &str) -> bool {
        self.settings.voice && with != model.me() && model.target().is_some()
    }

    /// The composer's button now: the rule's, for the session, the
    /// conversation and the draft as they stand, and Send stands disabled
    /// while the yard is reconnecting, as the card's buttons do. Enter is
    /// held then too; the reason stands under the line.
    pub(crate) fn composer_button(&self, model: &Model, with: &str, cx: &App) -> Button {
        let empty = draft::empty(self.line.read(cx).text(), self.drafts.attachments(with));
        match Button::of(self.voice.is_some(), self.voice_offered(model, with), empty) {
            Button::Send if self.reconnecting() => Button::SendDisabled,
            button => button,
        }
    }

    /// The composer's one button, at its bottom right, in the shape of
    /// `button`: Send in the accent, as the card's send button is; Voice and
    /// Stop bordered; a disabled Send faint, with no tap. The words are the
    /// design system's plainest, as it has no mark for voice or send. A tap
    /// sends as Enter does, or starts or ends the session. A press here
    /// claims no focus of its own: the composer's frame has it for the line,
    /// so what is typed stays there.
    fn composer_control(
        &self,
        button: Button,
        with: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (id, words, tip) = match button {
            Button::Voice => (
                "voice button",
                "Voice",
                Some(format!("voice: talk with {with} by voice")),
            ),
            Button::Send | Button::SendDisabled => ("send button", "Send", None),
            Button::Stop => (
                "stop button",
                "Stop",
                Some("voice: tap to end the session".to_string()),
            ),
        };
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .flex_none()
            .min_w(rems(4.))
            .min_h(px(32.))
            .px_3()
            .py_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .text_sm()
            .map(|el| match button {
                Button::Send => el
                    .bg(palette.accent)
                    .border_color(palette.accent)
                    .text_color(palette.on_accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer(),
                Button::SendDisabled => el.border_color(palette.line).text_color(palette.faint),
                Button::Voice | Button::Stop => el
                    .border_color(palette.line)
                    .text_color(palette.text)
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.hover)),
            })
            .when_some(tip, |el, tip| {
                el.tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
            })
            .when(button != Button::SendDisabled, |el| {
                el.on_click(cx.listener(move |this, _, _, cx| match button {
                    Button::Send => this.send(cx),
                    _ => this.toggle_voice(cx),
                }))
            })
            .child(words)
    }

    /// The session's state beside the composer's button: quiet words, while
    /// a session stands.
    fn voice_words(&self, palette: Palette) -> Option<impl IntoElement> {
        let words = self.voice_label()?;
        Some(
            div()
                .debug_selector(|| "voice words".to_string())
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(palette.faint)
                .child(words),
        )
    }

    /// The session's minutes and what they have cost, quiet at the head's
    /// right: counts are words, at the right.
    fn voice_meter(&self, palette: Palette) -> Option<impl IntoElement> {
        let talk = self.voice.as_ref().filter(|talk| !talk.meter.is_empty())?;
        Some(
            div()
                .debug_selector(|| "voice meter".to_string())
                .flex_none()
                .text_sm()
                .text_color(palette.faint)
                .child(talk.meter.clone()),
        )
    }

    /// Why the last session could not be made, or ended, in the words of
    /// whoever refused it: a line of the head, in the colour of an error,
    /// and of the Voice section, which says it too (`name` is its debug
    /// selector, one for each place).
    pub(super) fn voice_refusal_line(
        &self,
        name: &'static str,
        palette: Palette,
    ) -> Option<impl IntoElement> {
        self.voice_refusal.as_ref().map(|words| {
            div()
                .debug_selector(|| name.to_string())
                .text_sm()
                .text_color(palette.warn)
                .child(words.clone())
        })
    }

    /// What the caption's line says while a session stands: the window's own
    /// words where what was said did not go, else what the person has said
    /// that no request has taken yet, its last words where it is long (the
    /// caption's own rule). None where no session stands.
    pub(crate) fn voice_caption_words(&self) -> Option<String> {
        let talk = self.voice.as_ref()?;
        Some(match &talk.words {
            Some(words) => words.clone(),
            None => talk.caption.line(),
        })
    }

    /// The caption: [`Footplate::voice_caption_words`], one line under the
    /// head and over the mail while a session stands. The line keeps its
    /// height when empty, so that the mail does not move as the person speaks.
    fn voice_caption(&self, palette: Palette) -> Option<impl IntoElement> {
        let said = self.voice_caption_words()?;
        let own = self.voice.as_ref().is_some_and(|talk| talk.words.is_some());
        let color = if own { palette.warn } else { palette.text };
        Some(
            div()
                .debug_selector(|| "voice caption".to_string())
                .flex_none()
                .px_3()
                .py_1()
                .min_h(rems(1.75))
                .text_sm()
                .text_color(color)
                .child(said),
        )
    }

    /// Puts `emoji` on the mail `seq` as the user: a tap on the user's
    /// own current reaction takes it away instead, and a tap on another
    /// replaces it. Once the yard took it the conversation is read again;
    /// what the yard refused is said, and a yard with no such call hides
    /// the row.
    pub(crate) fn react(&mut self, seq: u64, emoji: &str, cx: &mut Context<Self>) {
        let mine = self.model.as_ref().and_then(|m| m.own_reaction(seq));
        let emoji = match mine.as_deref() == Some(emoji) {
            true => String::new(),
            false => emoji.to_string(),
        };
        let session = self.session;
        self.ask_through(
            cx,
            move |client, me| client.react(seq, &emoji, me),
            move |this, reacted, cx| match reacted {
                Ok(()) => this.change_in(session, cx, |m| m.reacted(seq)),
                Err(err) => this.change_in(session, cx, |m| {
                    m.react_failed(&err);
                    Vec::new()
                }),
            },
        );
    }

    /// Attaches what was pasted or dropped to the draft for `with`. An
    /// image among it is drawn from the bytes that came.
    pub(crate) fn attach(&mut self, with: &str, found: Found, cx: &mut Context<Self>) -> u64 {
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

    /// Attaches a file that was pasted to the draft of the open
    /// conversation: Enter sends it. One over the yard's cap or past its
    /// count is marked as soon as the yard said them.
    pub(crate) fn paste(&mut self, image: &Image, cx: &mut Context<Self>) {
        let Some((with, _)) = self.open() else {
            return;
        };
        let found = draft::pasted(image.bytes().to_vec());
        let bytes = found.bytes.clone();
        let name = found.name.clone();
        let id = self.attach(&with, found, cx);
        self.ask(
            cx,
            |client, _| client.settings().ok(),
            move |this, settings, cx| {
                // A yard that says nothing now is asked at Enter.
                let Some(settings) = settings else {
                    return;
                };
                if let Some(why) = model::over_cap_for(&bytes, bytes.len(), settings.blob_cap()) {
                    this.drafts.refuse(id, &why);
                    cx.notify();
                    return;
                }
                let attached = this.drafts.attachments(&with).len();
                if attached as u64 > settings.files_cap() {
                    let why = draft::one_too_many(
                        &name,
                        attached.saturating_sub(1),
                        settings.files_cap(),
                    );
                    this.drafts.refuse(id, &why);
                    cx.notify();
                }
            },
        );
    }

    /// Attaches the files that were dropped on the window: to the note
    /// of the open card, where one is, else to the draft of the open
    /// conversation. A zip or a folder drops to its files, each in the
    /// order it came.
    pub(crate) fn dropped(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.card().is_some() {
            return self.dropped_on_card(paths, cx);
        }
        let Some((with, _)) = self.open() else {
            return;
        };
        let paths = paths.to_vec();
        let existing = self.drafts.attachments(&with).len();
        self.ask(
            cx,
            move |client, _| {
                let settings = client.settings().ok();
                let cap = settings.as_ref().map(|settings| settings.blob_cap());
                let max = settings
                    .map(|settings| settings.files_cap())
                    .unwrap_or(api::FILES_MAX);
                let mut at = existing;
                let mut found = Vec::new();
                for path in &paths {
                    let unpacked = draft::dropped(path, cap, max, at);
                    at += unpacked.len();
                    found.extend(unpacked);
                }
                found
            },
            move |this, found, cx| {
                for found in found {
                    this.attach(&with, found, cx);
                }
            },
        );
    }

    /// Attaches the files that were dropped on the open card to its note,
    /// in the order they came: a note takes files as a conversation does.
    /// A secret, a run, or a taken-back ask takes no note, so a drop on
    /// its card attaches nothing, and says so. They are read off the
    /// window's thread, and one that cannot go only as far as it takes to
    /// say so.
    pub(crate) fn dropped_on_card(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        let model = self.model.as_ref();
        if model.is_some_and(|m| m.secret(&bead, ask).is_some()) {
            return self.notice(
                "a secret ask takes no note: the file was not attached".into(),
                cx,
            );
        }
        if model.is_some_and(|m| m.run(&bead, ask).is_some()) {
            return self.notice(
                "a run ask takes no note: the file was not attached".into(),
                cx,
            );
        }
        if model.is_some_and(|m| m.taken_back(&bead, ask)) {
            return self.notice(
                "an ask taken back takes no note: the file was not attached".into(),
                cx,
            );
        }
        let paths = paths.to_vec();
        let existing = self.card_files(&bead, ask).len();
        self.ask(
            cx,
            move |client, _| {
                let settings = client.settings().ok();
                let cap = settings.as_ref().map(|settings| settings.blob_cap());
                let max = settings
                    .map(|settings| settings.files_cap())
                    .unwrap_or(api::FILES_MAX);
                let mut at = existing;
                let mut found = Vec::new();
                for path in &paths {
                    let unpacked = draft::dropped_note(path, cap, max, at);
                    at += unpacked.len();
                    found.extend(unpacked);
                }
                found
            },
            move |this, found, cx| {
                for found in found {
                    this.attach_file(&bead, ask, found, cx);
                }
            },
        );
    }

    /// Attaches what was dropped on the open card to its note, after what
    /// is there. A file is drawn by its name: footplate shows no preview
    /// of one.
    pub(crate) fn attach_file(
        &mut self,
        bead: &str,
        ask: u64,
        found: Found,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.drafts.next_id();
        self.note_files
            .entry((bead.to_string(), ask))
            .or_default()
            .push(Attachment {
                id,
                name: found.name,
                bytes: found.bytes,
                size: found.size,
                refused: found.refused,
            });
        cx.notify();
        id
    }

    /// Takes an attachment off its draft or its card's note.
    pub(crate) fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.drafts.remove(id);
        for files in self.note_files.values_mut() {
            files.retain(|a| a.id != id);
        }
        self.thumbs.remove(&id);
        cx.notify();
    }

    /// Backspace in the empty line takes the last attachment off the
    /// draft, or off the open card's note.
    pub(crate) fn remove_last(&mut self, cx: &mut Context<Self>) {
        if let Some((bead, ask)) = self.card()
            && let Some(files) = self.note_files.get_mut(&(bead, ask))
            && let Some(file) = files.pop()
        {
            self.thumbs.remove(&file.id);
            cx.notify();
            return;
        }
        let Some((with, _)) = self.open() else {
            return;
        };
        if let Some(id) = self.drafts.remove_last(&with) {
            self.thumbs.remove(&id);
            cx.notify();
        }
    }

    /// Opens the bead a mail names: in the drawer beside the reading
    /// pane, which stays as it was, or full width where the window holds
    /// no drawer beside it.
    pub(crate) fn open_bead(&mut self, bead: String, window: &mut Window, cx: &mut Context<Self>) {
        if !drawer_fits(window.viewport_size().width, f32::from(self.settings.rem())) {
            return self.select(Selected::Bead(bead), window, cx);
        }
        // Another bead than the one the drawer shows starts at its top;
        // the same one stays where it stood.
        if self.drawer.as_ref() != Some(&bead) {
            self.drawer_scroll.scroll_to_item(0);
        }
        self.drawer = Some(bead.clone());
        self.change(cx, |m| m.read_bead(&bead));
    }

    /// Shows the run ask's carried texts behind the yard's sentences, or
    /// hides them again: the tap of its card.
    pub(crate) fn texts_for(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_texts(&bead, ask);
            Vec::new()
        });
    }

    /// Clicks a thumbnail: the image shows full size over the window,
    /// from the bytes the window holds, or the line while they are not
    /// there yet. Over one already shown the second shows instead, and
    /// the focus still goes back where the first found it. While one
    /// shows, the window holds the keys: nothing typed goes to a line
    /// that is not in view.
    pub(crate) fn open_image(
        &mut self,
        hash: String,
        bytes: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let back = self
            .image_open
            .take()
            .and_then(|open| open.back)
            .or_else(|| window.focused(cx));
        self.image_open = Some(ImageOpen { hash, bytes, back });
        window.focus(&self.focus);
        cx.notify();
    }

    /// Shuts the image shown over the window, and the focus goes back
    /// where it was before it opened. Shutting none shuts nothing: the
    /// × stands on the backdrop, and its click shuts through both.
    pub(crate) fn shut_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.image_open.take() {
            match open.back {
                Some(back) => window.focus(&back),
                None => window.focus(&self.focus.clone()),
            }
        }
        cx.notify();
    }

    /// The image open over the window: the dark backdrop, the image
    /// scaled down to fit it with a margin, and the × in a corner. A
    /// click outside the image or on the × shuts it, as Esc does; a
    /// click on the image shuts nothing. Drawn from the bytes the
    /// window holds, with no second fetch; while they are not there
    /// yet the line, as the thumbnail shows it.
    pub(crate) fn image_layer(
        &self,
        open: &ImageOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hash = open.hash.clone();
        let shown = match self.images.get(&open.hash) {
            Some(Picture::Here(image)) => {
                // The box the image draws in: its own size, scaled down
                // to fit the window minus the margin, the shape kept.
                // The element's own sizing caps each side apart, and
                // where one side binds the box would outgrow the image:
                // a click there must shut. While the bytes are not
                // drawn yet the caps stand in.
                let fit = image
                    .clone()
                    .use_render_image(window, cx)
                    .filter(|rendered| rendered.frame_count() > 0)
                    .map(|rendered| rendered.size(0))
                    .map(|size| {
                        let scale = window.scale_factor();
                        (
                            i32::from(size.width) as f32 / scale,
                            i32::from(size.height) as f32 / scale,
                        )
                    })
                    .filter(|(w, h)| *w > 0. && *h > 0.)
                    .map(|(w, h)| {
                        let margin = f32::from(window.rem_size()) * IMAGE_MARGIN;
                        let avail = window.viewport_size();
                        let scale = ((f32::from(avail.width) - 2. * margin) / w)
                            .min((f32::from(avail.height) - 2. * margin) / h)
                            .clamp(0., 1.);
                        (w * scale, h * scale)
                    });
                let mut shown = img(image.clone()).object_fit(ObjectFit::ScaleDown);
                match fit {
                    Some((w, h)) => shown = shown.w(px(w)).h(px(h)),
                    None => shown = shown.max_w_full().max_h_full(),
                }
                shown
                    .debug_selector({
                        let hash = hash.clone();
                        move || format!("image shown {hash}")
                    })
                    .with_fallback(move || {
                        div()
                            .text_sm()
                            .text_color(white())
                            .child("an image footplate cannot draw".to_string())
                            .into_any_element()
                    })
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .into_any_element()
            }
            Some(Picture::Not(why)) => {
                let why = why.clone();
                div()
                    .id(SharedString::from(format!("image line {hash}")))
                    .text_sm()
                    .text_color(white())
                    .debug_selector(move || format!("image line {hash}"))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(why)
                    .into_any_element()
            }
            Some(Picture::Asked) | None => div()
                .id(SharedString::from(format!("image line {hash}")))
                .text_sm()
                .text_color(white())
                .debug_selector(move || format!("image line {hash}"))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(format!("image, {}…", model::size(open.bytes)))
                .into_any_element(),
        };
        div()
            .id("image overlay")
            .debug_selector(|| "image overlay".to_string())
            // A click outside the image shuts it and is no click on what
            // stands under it.
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
            .on_click(cx.listener(|this, _, window, cx| this.shut_image(window, cx)))
            .child(shown)
            .child(
                div()
                    .id("image close")
                    .debug_selector(|| "image close".to_string())
                    .absolute()
                    .top(rems(0.75))
                    .right(rems(1.))
                    .text_xl()
                    .text_color(white())
                    .cursor_pointer()
                    .child("×")
                    .on_click(cx.listener(|this, _, window, cx| this.shut_image(window, cx))),
            )
    }

    /// A file of a mail or of a note: an image, a page's row, a file's
    /// row, or the line that says why there is none. A click on the image
    /// shows it full size over the window, from the bytes the window
    /// holds; `note` says it is a note's, whose page carries run beside
    /// Open.
    pub(crate) fn picture(
        &self,
        file: &BlobRef,
        note: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let line = move |text: String| div().text_sm().text_color(palette.faint).child(text);
        if !file.not_taken.is_empty() {
            let name = file.name.clone();
            return line(model::not_taken(file))
                .debug_selector(move || format!("not taken {name}"))
                .into_any_element();
        }
        if model::is_page(file) {
            return self.page_row(file, note, palette).into_any_element();
        }
        if !model::is_image(file) {
            return self.file_row(file, palette).into_any_element();
        }
        match self.images.get(&file.hash) {
            Some(Picture::Here(image)) => {
                let shown = img(image.clone())
                    .h(px(IMAGE_HEIGHT))
                    .max_w_full()
                    .object_fit(ObjectFit::ScaleDown)
                    .with_fallback(move || {
                        line("an image footplate cannot draw".to_string()).into_any_element()
                    });
                // A click shows it full size over the window, from the
                // bytes the window holds: no second fetch, and no
                // browser. The press takes no focus: the window holds
                // the keys once it shows, and the focus goes back where
                // the click found it when it shuts.
                let hash = file.hash.clone();
                let bytes = file.bytes;
                shown
                    .id(SharedString::from(format!("image open {hash}")))
                    .debug_selector({
                        let hash = hash.clone();
                        move || format!("image open {hash}")
                    })
                    .cursor_pointer()
                    .tooltip(|_, cx| cx.new(|_| OpenTip).into())
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_image(hash.clone(), bytes, window, cx)
                    }))
                    .into_any_element()
            }
            Some(Picture::Not(why)) => line(why.clone()).into_any_element(),
            // The bytes are not there yet: a click opens the overlay all
            // the same, with this line, and the image shows when they
            // come.
            Some(Picture::Asked) | None => {
                let hash = file.hash.clone();
                let bytes = file.bytes;
                let text = format!("image, {}…", model::size(file.bytes));
                div()
                    .h(px(IMAGE_HEIGHT))
                    .id(SharedString::from(format!("image open {hash}")))
                    .debug_selector({
                        let hash = hash.clone();
                        move || format!("image open {hash}")
                    })
                    .cursor_pointer()
                    .tooltip(|_, cx| cx.new(|_| OpenTip).into())
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_image(hash.clone(), bytes, window, cx)
                    }))
                    .child(line(text))
                    .into_any_element()
            }
        }
    }

    /// A page of a mail or of a note: a row with its size, and Open
    /// in the browser, through the yard's web view, with run beside it
    /// for a note's, through the run base. Where the yard has no view
    /// the row says it is not set, with the way to set it; where the
    /// view does not answer the row says how to start it instead of
    /// opening nowhere; where the sandbox origin does not answer the row
    /// says so beside Open. A mail's page is the row without run, since
    /// the sandbox origin serves no mail's bytes, and that stays.
    pub(crate) fn page_row(
        &self,
        file: &BlobRef,
        note: bool,
        palette: Palette,
    ) -> impl IntoElement + use<> {
        self.rows_shown.set(true);
        let hash = file.hash.clone();
        let row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_sm()
            .debug_selector({
                let hash = hash.clone();
                move || format!("page row {hash}")
            });
        let label = div()
            .text_color(palette.faint)
            .child(format!("page, {}", model::size(file.bytes)));
        let Some(base) = self.view_base() else {
            let row = row.child(label).child(
                div()
                    .text_color(palette.faint)
                    .debug_selector({
                        let hash = hash.clone();
                        move || format!("page unset {hash}")
                    })
                    .child("this yard's web view is not set: set it in the settings"),
            );
            // A note's page can still run through an explicitly set run
            // base when this yard has no web view. The run base is a
            // separate address; only Open needs the web view.
            if !note {
                return row.into_any_element();
            }
            return match (self.run_address(), self.run_up_now()) {
                (Some(base), Some(true)) => {
                    let Some(url) = base.page_url(&file.hash) else {
                        return row.into_any_element();
                    };
                    let run = self.means.open.clone();
                    row.child(
                        div()
                            .id(SharedString::from(format!("page run {hash}")))
                            .debug_selector(move || format!("page run {hash}"))
                            .text_color(palette.link)
                            .cursor_pointer()
                            .on_click(move |_, _, cx| (run)(&url, cx))
                            .child("run"),
                    )
                    .into_any_element()
                }
                (_, Some(false)) => row
                    .child(
                        div()
                            .text_color(palette.faint)
                            .debug_selector(move || format!("page run down {hash}"))
                            .child(
                                "run is not reached: update the yard's yardr, or set the run base",
                            ),
                    )
                    .into_any_element(),
                _ => row.into_any_element(),
            };
        };
        match self.view_up_now() {
            Some(true) => {
                // The probe answered at this address, so it parses: the
                // same reading, not another. A hash that is no hex names
                // no page, and the row has no Open.
                let Some(url) = Address::parse_view(&base)
                    .expect("the web view answered at this address")
                    .page_url(&file.hash)
                else {
                    return row.child(label).into_any_element();
                };
                let open = self.means.open.clone();
                let row = row.child(label).child(
                    div()
                        .id(SharedString::from(format!("page open {hash}")))
                        .debug_selector({
                            let hash = hash.clone();
                            move || format!("page open {hash}")
                        })
                        .text_color(palette.link)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| (open)(&url, cx))
                        .child("Open"),
                );
                // No run for a mail's page: the sandbox origin
                // serves no mail's bytes, and that stays.
                if !note {
                    return row.into_any_element();
                }
                // Run beside Open, through the run base: the same
                // reading, not another. A hash that is no hex runs
                // nothing, and the row has no run; where the origin
                // does not answer the row says so beside Open, and
                // while it is asked the row is Open alone.
                match (self.run_address(), self.run_up_now()) {
                    (Some(base), Some(true)) => {
                        let Some(url) = base.page_url(&file.hash) else {
                            return row.into_any_element();
                        };
                        let run = self.means.open.clone();
                        row.child(
                            div()
                                .id(SharedString::from(format!("page run {hash}")))
                                .debug_selector(move || format!("page run {hash}"))
                                .text_color(palette.link)
                                .cursor_pointer()
                                .on_click(move |_, _, cx| (run)(&url, cx))
                                .child("run"),
                        )
                        .into_any_element()
                    }
                    (_, Some(false)) => row
                        .child(
                            div()
                                .text_color(palette.faint)
                                .debug_selector(move || format!("page run down {hash}"))
                                .child("run is not reached: update the yard's yardr, or set the run base"),
                        )
                        .into_any_element(),
                    _ => row.into_any_element(),
                }
            }
            Some(false) => row
                .child(label)
                .child(
                    div()
                        .text_color(palette.faint)
                        .debug_selector(move || format!("page start {hash}"))
                        .child("the web view is not reached: start it with yardr web start"),
                )
                .into_any_element(),
            None => row
                .child(
                    div()
                        .text_color(palette.faint)
                        .child(format!("page, {}…", model::size(file.bytes))),
                )
                .into_any_element(),
        }
    }

    /// A file of a mail or of a note, neither an image nor a page: a row
    /// with its name and its size, and Open in the browser, through the
    /// yard's web view, under the name the browser saves it as. Where the
    /// yard has no view the row says it is not set, with the way to set
    /// it; where the view does not answer the row says how to start it
    /// instead of opening nowhere. Never run: only a page runs.
    pub(crate) fn file_row(&self, file: &BlobRef, palette: Palette) -> impl IntoElement + use<> {
        self.rows_shown.set(true);
        let hash = file.hash.clone();
        let row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_sm()
            .debug_selector({
                let hash = hash.clone();
                move || format!("file row {hash}")
            });
        let label = match file.name.is_empty() {
            false => format!("{}, {}", file.name, model::size(file.bytes)),
            true => format!("file, {}", model::size(file.bytes)),
        };
        let label = div().text_color(palette.faint).child(label);
        let Some(base) = self.view_base() else {
            return row
                .child(label)
                .child(
                    div()
                        .text_color(palette.faint)
                        .debug_selector({
                            let hash = hash.clone();
                            move || format!("file unset {hash}")
                        })
                        .child("this yard's web view is not set: set it in the settings"),
                )
                .into_any_element();
        };
        match self.view_up_now() {
            Some(true) => {
                // The probe answered at this address, so it parses: the
                // same reading, not another. A hash that is no hex names
                // no file, and the row has no Open.
                let Some(url) = Address::parse_view(&base)
                    .expect("the web view answered at this address")
                    .file_url(&file.hash, &file.name)
                else {
                    return row.child(label).into_any_element();
                };
                let open = self.means.open.clone();
                row.child(label)
                    .child(
                        div()
                            .id(SharedString::from(format!("file open {hash}")))
                            .debug_selector({
                                let hash = hash.clone();
                                move || format!("file open {hash}")
                            })
                            .text_color(palette.link)
                            .cursor_pointer()
                            .on_click(move |_, _, cx| (open)(&url, cx))
                            .child("Open"),
                    )
                    .into_any_element()
            }
            Some(false) => row
                .child(label)
                .child(
                    div()
                        .text_color(palette.faint)
                        .debug_selector(move || format!("file start {hash}"))
                        .child("the web view is not reached: start it with yardr web start"),
                )
                .into_any_element(),
            None => {
                let pending = match file.name.is_empty() {
                    false => format!("{}, {}…", file.name, model::size(file.bytes)),
                    true => format!("file, {}…", model::size(file.bytes)),
                };
                row.child(div().text_color(palette.faint).child(pending))
                    .into_any_element()
            }
        }
    }

    /// An attachment of the draft: the image small, or its name when it is
    /// none footplate draws, with the mark that removes it. One that cannot
    /// go is pale in a frame of the colour its reason is said in.
    pub(crate) fn attachment(
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

    /// The mail `seq` knows the pointer stands on `piece` of it: while it
    /// does, the bar of reactions a tap puts floats over the mail.
    ///
    /// The pointer can leave one piece for another within a single move,
    /// and the window tells of the two in no fixed order. A piece that is
    /// left clears only the mark it set itself, so that it never undoes
    /// the mark of the piece the pointer came to.
    pub(crate) fn hover_react(
        seq: u64,
        piece: Piece,
        el: gpui::Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let entity = cx.entity();
        el.on_hover(move |hovered, _, cx| {
            entity.update(cx, |this, cx| {
                let here = Some((seq, piece));
                this.react_hover = match hovered {
                    true => here,
                    false if this.react_hover == here => None,
                    false => return,
                };
                // The bar went with the letter: no emoji of it is stood
                // on. Its buttons draw no leave of their own while they
                // are not drawn.
                if this.react_hover.is_none() {
                    this.emoji_hover = None;
                }
                cx.notify();
            });
        })
    }

    /// Whether the bar of reactions floats over the mail `seq`: the
    /// pointer stands on it, the yard takes reactions, and no overlay
    /// stands over the mail. The bar paints after the whole window, so
    /// without this it would float over the switcher, an image, a
    /// screen, the About card or the cheatsheet too.
    pub(crate) fn reacting(&self, seq: u64) -> bool {
        self.react_hover.is_some_and(|(at, _)| at == seq)
            && self.model.as_ref().is_some_and(|m| !m.react_too_old())
            && self.switching.is_none()
            && self.image_open.is_none()
            && self.screen_open.is_none()
            && self.about.is_none()
            && !self.show_keys
    }

    /// The letter's head, with the bar of reactions at its right while the
    /// pointer stands on the letter: the head is the letter's first line,
    /// where it has one. `at` is the time it says: the time alone where a
    /// separator above names the day, with the date where none does.
    /// `item` is the row of the list the head stands in.
    pub(crate) fn entry_head(
        &self,
        entry: &Entry,
        at: String,
        item: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let seq = entry.seq;
        Self::hover_react(
            seq,
            Piece::Head,
            div()
                .id(("head", seq))
                .relative()
                .w_full()
                .flex()
                .gap_2()
                // The breathing room to the body stands inside the head,
                // so the two touch and the pointer goes from one to the
                // other with no strip between them that is neither.
                .pb_1()
                .debug_selector(|| format!("head {seq}")),
            cx,
        )
        .child(
            div()
                .debug_selector(|| format!("clock {seq} {at}"))
                .text_color(palette.faint)
                .child(at),
        )
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                // Their own name draws dim: the accent is kept for
                // what waits on them.
                .when(entry.mine, |el| el.text_color(palette.faint))
                .child(entry.from.clone()),
        )
        .when(self.reacting(seq), |el| {
            el.children(self.react_row(seq, item, &entry.reactions, palette, cx))
        })
        .into_any_element()
    }

    /// The letter's body. A letter that reads under another's head has no
    /// head of its own, so its bar of reactions floats on its top edge,
    /// and its time stands in the gutter while the pointer is on it. The
    /// letter draws no ground, hovered or not: the time appearing is the
    /// whole affordance. `item` is the row of the list the body stands in.
    pub(crate) fn entry_body(
        &self,
        entry: Entry,
        item: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let seq = entry.seq;
        let hovered = !entry.head && self.reacting(seq);
        let clocked = hover_clock(entry.head, self.react_hover, seq);
        let at = model::clock_time(&entry.at, &chrono::Local);
        let body = div()
            .id(("body", seq))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            // The breathing room to the row below stands inside the
            // body, so a body touches the head or body under it: the
            // scale's space-8, 24 pixels, past a blank line within a
            // letter, so one letter reads apart from the next.
            .pb_6();
        Self::hover_react(seq, Piece::Body, body, cx)
            // A correction carries its label above its text, so it reads
            // as no new thought.
            .when(entry.correction, |el| {
                el.child(
                    div()
                        .ml(rems(LETTER_GUTTER))
                        .debug_selector(|| format!("correction {seq}"))
                        .text_sm()
                        .text_color(palette.faint)
                        .child("correction"),
                )
            })
            .when(!entry.text.is_empty(), |el| {
                // The margin is outside the text's bounds, so that these
                // are where the text is.
                let open = cx.entity();
                let on_ref: select::RefClick = Rc::new(move |target, window, cx| {
                    open.update(cx, |this, cx| this.open_reference(target, window, cx));
                });
                el.child(
                    div()
                        .ml(rems(LETTER_GUTTER))
                        .debug_selector(|| format!("text {seq}"))
                        .child(select::text_with_refs(
                            seq,
                            entry.text,
                            entry.refs,
                            on_ref,
                            &self.selection,
                        )),
                )
            })
            // The parties' reactions, one a line, in faint words under
            // the text.
            .when(!entry.reactions.is_empty(), |el| {
                el.child(
                    div()
                        .ml(rems(LETTER_GUTTER))
                        .debug_selector(|| format!("reactions {seq}"))
                        .children(entry.reactions.iter().map(|r| {
                            div()
                                .text_sm()
                                .text_color(palette.faint)
                                .child(model::plain(&format!("{} {}", r.emoji, r.from)))
                        })),
                )
            })
            .children(
                entry
                    .asks
                    .iter()
                    .map(|ask| self.ask_preview(ask, palette, cx)),
            )
            .children(entry.files.iter().map(|file| {
                div()
                    .pl(rems(LETTER_GUTTER))
                    .pt_1()
                    .child(self.picture(file, false, palette, cx))
            }))
            .when_some(entry.standing, |el, standing| {
                el.child(
                    div()
                        .pl(rems(LETTER_GUTTER))
                        .text_sm()
                        .text_color(palette.faint)
                        .child(format!("· {standing}")),
                )
            })
            // The letter's own time in the gutter: absolute, so it takes
            // no room and the letter keeps its place whether it stands
            // there or not.
            .when(clocked, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .debug_selector({
                            let at = at.clone();
                            move || format!("hover clock {seq} {at}")
                        })
                        .text_color(palette.faint)
                        .child(at.clone()),
                )
            })
            .when(hovered, |el| {
                el.children(self.react_row(seq, item, &entry.reactions, palette, cx))
            })
            .into_any_element()
    }

    /// The reactions a tap puts on the mail, in a bar that floats at the
    /// top right of the letter while the pointer stands on it, on its top
    /// edge: it is absolute in the head or body that draws it, so it takes
    /// no room and the letter keeps its place and its height. All of it
    /// stands above the letter, over the blank the letter above leaves
    /// and the bottom of that letter's last line: no line of this one is
    /// under it, nor the copy control of a code block that starts it. A
    /// letter at the list's top, or within the bar's height under it, has
    /// no such room: the bar stops at the list's top instead, over the
    /// letter's own first lines, and never stands over the pane's head,
    /// where a crew member's conversation has its Screen control. `item`
    /// is the row of the list that draws the head or body: a row that the
    /// scroll has carried wholly above the list has no bar (`bar_top`). It
    /// paints after the rest of the window, so no line of the letter or of
    /// the next one paints over it and the list's scroll does not cut it:
    /// the kit paints in tree order, and the letter's body stands after
    /// the head that draws the bar. Its own ground and hairline keep the
    /// text under it from showing through, the kit's popover shadow lifts
    /// it, and it blocks the mouse, so that a tap on it is no press on
    /// that text. The yard's fixed set, each in a square of 26 pixels with
    /// 4 pixels around and between them, so the bar is 36 pixels high at
    /// every text size: closer than the kit's 32-pixel control, since the
    /// bar floats over a letter and the emoji are all it holds. The one
    /// the pointer stands on draws 1.2 times larger in its square,
    /// so neither its neighbours nor the bar move. The user's own current
    /// one stands on the open ground. A tap on that one takes it away, and
    /// a tap on another puts it instead.
    pub(crate) fn react_row(
        &self,
        seq: u64,
        item: usize,
        reactions: &[Reaction],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        // How far under the list's top the row stands, as `keep_place`
        // reckons it: the row's top in the layout, which the scroll has not
        // moved, with the scroll offset and the list's top. The layout is
        // the last frame's, so a change of it shows here a frame late; the
        // bar shows while the pointer rests on a letter, after the scroll
        // has settled, and the offset is the current one, so the wheel's
        // scroll shows at once. With no layout yet there is nothing to stop
        // the bar at.
        let list = &self.conversation;
        let top = match list.bounds_for_item(item) {
            Some(row) => {
                let below = row.top() + list.offset().y - list.bounds().top();
                bar_top(below, row.size.height)?
            }
            None => px(-36.),
        };
        let me = self.model.as_ref().map(Model::me).unwrap_or_default();
        let mine = reactions
            .iter()
            .find(|r| r.from == me)
            .map(|r| r.emoji.as_str());
        // The bar is a piece of the letter of its own: the mouse it blocks
        // is the letter's, so the letter's head or body no longer has the
        // pointer where the bar stands.
        let mut row = Self::hover_react(
            seq,
            Piece::Bar,
            div()
                .id(("react", seq))
                .absolute()
                // On the top edge of the head or body: 36 pixels up, the
                // bar's own height, so its bottom meets that edge (mckean's
                // pick, ask 13848). No line of the letter is under it, nor
                // the copy control of a code block that starts it. It
                // stands over the blank the letter above leaves under its
                // text (`pb_6`, 24 pixels; 32 above a head) and the bottom
                // of that letter's last line. Its gap and padding are in
                // pixels, not rems, so it is 36 high at every text size: a
                // shorter bar would leave a strip between it and the
                // letter, where the pointer going up from the letter lands
                // on the one above and the bar goes. Where the letter is
                // too near the list's top for that, the offset is the
                // clamped one worked out above, so that the bar stops at
                // the list's top (mckean's pick, ask 13938).
                .top(top)
                .right_0()
                .flex()
                .flex_row()
                .gap(px(4.))
                .p(px(4.))
                .rounded_md()
                .border_1()
                .border_color(palette.line)
                .bg(palette.ground)
                // The kit's popover shadow, 6 pixels down and 20 blurred.
                .shadow(vec![gpui::BoxShadow {
                    color: palette.shadow.into(),
                    offset: gpui::point(px(0.), px(6.)),
                    blur_radius: px(20.),
                    spread_radius: px(0.),
                }])
                .occlude()
                .debug_selector(|| format!("react row {seq}")),
            cx,
        );
        for emoji in model::REACTIONS {
            let tapped = format!("react {seq} {emoji}");
            let zoomed = self
                .emoji_hover
                .as_ref()
                .is_some_and(|(at, on)| *at == seq && on == emoji);
            let entity = cx.entity();
            let on = emoji.to_string();
            row = row.child(
                div()
                    .id(SharedString::from(tapped.clone()))
                    .debug_selector({
                        let tapped = tapped.clone();
                        move || tapped.clone()
                    })
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(rems(match zoomed {
                        true => 1.05,
                        false => 0.875,
                    }))
                    .when(zoomed, |el| el.bg(palette.hover))
                    .when(!zoomed && mine == Some(emoji), |el| el.bg(palette.open))
                    .on_hover(move |hovered, _, cx| {
                        entity.update(cx, |this, cx| {
                            let here = Some((seq, on.clone()));
                            this.emoji_hover = match hovered {
                                true => here,
                                false if this.emoji_hover == here => None,
                                false => return,
                            };
                            cx.notify();
                        });
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.react(seq, emoji, cx)))
                    .child(emoji.to_string()),
            );
        }
        // Deferred, so the bar paints after the letter and what stands
        // under it: its layout stays where it is, absolute at the top
        // right of the head or body, raised to stand on its top edge.
        Some(gpui::deferred(row).into_any_element())
    }

    /// A mail among the yard row's notices. No separator names its day
    /// there, so its head says the date with the time, as a notice does.
    /// `item` is the row of the list the mail stands in, head and body
    /// together.
    pub(crate) fn entry(
        &self,
        entry: Entry,
        item: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let head = entry.head;
        div()
            .flex()
            .flex_col()
            .when(head, |el| {
                let at = model::clock(&entry.at, &chrono::Local);
                el.child(self.entry_head(&entry, at, item, palette, cx))
            })
            .child(self.entry_body(entry, item, palette, cx))
            .into_any_element()
    }

    /// An open ask a mail names, under the mail: what it asks with how
    /// long it waits, and the one action, which opens the card in
    /// Decide. The card itself stays there: one source of truth.
    pub(crate) fn ask_preview(
        &self,
        ask: &AskPreview,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bead = ask.bead.clone();
        let ask_seq = ask.ask;
        div()
            .pl(rems(LETTER_GUTTER))
            .pt_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(ask_preview_head(&ask.kind, ask.ask, &ask.waits)),
            )
            .child(div().child(ask.question.clone()))
            .child(
                div().flex().flex_row().child(
                    small_button("Open card".to_string(), palette)
                        .id(SharedString::from(format!("open card {bead} {ask_seq}")))
                        .debug_selector({
                            let bead = bead.clone();
                            move || format!("open card {bead} {ask_seq}")
                        })
                        .text_color(palette.text_accent)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_reference(
                                select::RefTarget::Ask {
                                    bead: bead.clone(),
                                    ask: ask_seq,
                                    open: true,
                                },
                                window,
                                cx,
                            )
                        })),
                ),
            )
            .into_any_element()
    }

    /// The conversation's mail as groups under one head each: a mail that
    /// reads under the sender's name above joins its group, and every other
    /// one starts a group, with the day's separator above it where the day
    /// changes. Each group stands in the reading column. The rows are the
    /// list's own, one for each day break, head and body, from the first
    /// after the line above the oldest mail: each head and body is told
    /// the row it stands in, for the bar of reactions to stop at the
    /// list's top by.
    pub(crate) fn mail_rows(
        &self,
        entries: Vec<Entry>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        for entry in entries {
            // The head and day break are separate scroll rows. An older
            // page can then add mail to this group without moving the
            // message that was already in view inside a growing wrapper.
            if let Some(day) = entry.day.clone() {
                out.push(day_break(day, palette));
            }
            if entry.head {
                // A separator above names the day, over this group or an
                // earlier one of it: the head says the time alone.
                let at = model::clock_time(&entry.at, &chrono::Local);
                let item = ROWS_BEFORE_MAIL + out.len();
                out.push(reading_row(
                    self.entry_head(&entry, at, item, palette, cx),
                    true,
                ));
            }
            let item = ROWS_BEFORE_MAIL + out.len();
            out.push(reading_row(
                self.entry_body(entry, item, palette, cx),
                false,
            ));
        }
        out
    }

    pub(crate) fn talk_conversation(
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
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(with.to_string()),
                            )
                            // What the agent is doing, beside its name, so the
                            // reader looks no further than the header. People and
                            // a linked yard's people have no state.
                            .children(model.crew_state(with).map(|state| {
                                div()
                                    .debug_selector(|| "conversation state".to_string())
                                    .text_color(palette.faint)
                                    .child(format!("· {state}"))
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            // A voice session's minutes and cost, quiet.
                            .children(self.voice_meter(palette))
                            // The crew member's screen, at the header's right
                            // edge. Who is no member of this yard's crew has no
                            // session, and the header carries no icon.
                            .children(model.screen_for_crew(with).map(|session| {
                                screen_icon(session, format!("talk screen {with}"), palette, cx)
                            })),
                    ),
            )
            .children(self.voice_refusal_line("voice refusal", palette))
            .children(about.into_iter().enumerate().map(|(i, line)| {
                // The first says who that is; what follows keeps mail from
                // arriving.
                div()
                    .text_sm()
                    .text_color(if i == 0 { palette.faint } else { palette.warn })
                    .child(line)
            }));

        let entries = model.entries();
        // The mail reads as a column: each row stands full width up to the
        // reading width, centred in the pane.
        let mail = div()
            .id("conversation")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.conversation)
            // Keep the mail's side edges on the composer's 12px margin;
            // the vertical breathing room stays 16px.
            .px_3()
            .py_4()
            .flex()
            .flex_col()
            .items_center()
            // No gap of its own: the breathing room between rows stands
            // inside the head and the body, so a letter's two touch and
            // the pointer never stands between them.
            .gap_0();
        let mail = if entries.is_empty() {
            mail.child(reading_column(
                div()
                    .text_color(palette.faint)
                    .child(if with == model.me() {
                        "No notes to yourself yet. What you write here is mail to yourself."
                            .to_string()
                    } else {
                        format!("No mail between you and {with} yet. What you write here is mail.")
                    })
                    .into_any_element(),
            ))
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
                reading_column(
                    div()
                        .debug_selector(|| "conversation top".to_string())
                        .flex()
                        .justify_center()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(top)
                        .into_any_element(),
                )
                // What the rows' gap carried under a row with no body.
                .mb_1(),
            )
            .children(self.mail_rows(entries, palette, cx))
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
        let button = self.composer_button(model, with, cx);
        pane.children(self.reconnect_strip(palette))
            .child(head)
            .children(self.voice_caption(palette))
            .child(mail)
            // The notice stands in the reading column with the composer
            // under it, and has the side margins the composer's frame has,
            // so that its line starts where the frame does at any width.
            .when(!model.notice.is_empty(), |el| {
                el.child(
                    reading_column(
                        div()
                            .debug_selector(|| "notice".to_string())
                            .py_1()
                            .text_sm()
                            .text_color(palette.warn)
                            .child(format!("! {}", model.notice))
                            .into_any_element(),
                    )
                    .px_3(),
                )
            })
            .child(
                // The composer stands in the reading column with the mail
                // above it: centred, no wider than the mail, its edges
                // lining up with the column's on a wide window. Two divs,
                // not one, as reading_column has it; the side margins stand
                // on the outer one so the frame meets the column's edges,
                // and where the pane is narrower than the reading width the
                // frame keeps today's margins. The notice above it has the
                // same margins, so that the two start at one edge.
                div().mx_3().child(
                    div().max_w(px(READING_WIDTH)).mx_auto().child(
                        div()
                            .debug_selector(|| "composer".to_string())
                            .my_3()
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
                            .child(self.line.clone())
                            // The draft stays as typed while the yard is
                            // reconnecting, with the reason beside the line.
                            .when(self.reconnecting(), |el| el.child(send_held(palette)))
                            // The bottom row: the cheatsheet's hint, where `?`
                            // is the line's text and the keys open it with a
                            // click instead; the session's state in quiet words
                            // while one stands; and the one button, which is
                            // voice, send or stop.
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_none().text_xs().child(keys_hint(
                                        "keys hint line",
                                        palette,
                                        cx,
                                    )))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .justify_end()
                                            .children(self.voice_words(palette)),
                                    )
                                    .child(self.composer_control(button, with, palette, cx)),
                            ),
                    ),
                ),
            )
    }

    /// One travelling bead whole: the title, id, platform and line, who
    /// works it where a session does, the expected arrival and remark of
    /// the row it came from, the labels, the body, then the notes newest
    /// last. Its asks are the Decisions
    /// rows' job: one line counts them and links nothing. A quiet back
    /// link naming where it came from leads there, and Open takes the
    /// bead's page to the browser where the web view answers. Full width
    /// in the reading pane and in the drawer beside it alike; `scroll`
    /// is whose scroll it moves.
    pub(crate) fn talk_bead(
        &self,
        model: &Model,
        bead: &str,
        scroll: &ScrollHandle,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        let Some(detail) = model.arrival_detail(bead) else {
            return pane
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child("Reading the bead.");
        };
        // The probe answered at this address, so it parses: the same
        // reading, not another. An id that is none opens nothing, and
        // the head has no Open, as a page row behaves; with no view for
        // the yard there is none either.
        let url = match (self.view_base(), self.view_up_now()) {
            (Some(base), Some(true)) => Address::parse_view(&base)
                .expect("the web view answered at this address")
                .bead_url(&detail.bead),
            _ => None,
        };
        // The quiet back link names where the bead was opened from.
        let back = model.bead_back();
        let back_name = bead_back_name(&back, model.departures());
        // The bead's route, as the board's rows show it: the strip
        // under the id line says the stage, so the line drops the word.
        // With no route the bead stands in no served stage, and the line
        // keeps it, as it did.
        let route = model.route_for(&detail.bead);
        let sub = bead_sub(&detail, route.as_ref());
        // Who works it, from the bead's flows row, which is where the
        // yard says so; nothing where no session does. Where the window
        // is the operator's the line carries Screen, which opens that
        // session's screen over the window.
        let worked = model
            .flow_bead(&detail.bead)
            .and_then(|(found, _)| worker_words(found).map(|(_, line)| line));
        let screen = model.screen_for_bead(&detail.bead);
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
                    .child(
                        div()
                            .id("bead back")
                            .debug_selector(|| "bead back".to_string())
                            .text_sm()
                            .text_color(palette.faint)
                            .cursor_pointer()
                            .hover(|style| style.text_color(palette.text))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select(back.clone(), window, cx)
                            }))
                            .child(back_name),
                    )
                    .children(url.map(|url| {
                        let open = self.means.open.clone();
                        let bead = detail.bead.clone();
                        div()
                            .id(SharedString::from(format!("bead open {bead}")))
                            .debug_selector({
                                let bead = bead.clone();
                                move || format!("bead open {bead}")
                            })
                            .text_sm()
                            .text_color(palette.link)
                            .cursor_pointer()
                            .on_click(move |_, _, cx| (open)(&url, cx))
                            .child("Open")
                    })),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .debug_selector({
                        let bead = detail.bead.clone();
                        move || format!("bead title {bead}")
                    })
                    .child(detail.title.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .debug_selector({
                        let bead = detail.bead.clone();
                        move || format!("bead sub {bead}")
                    })
                    .child(sub),
            )
            .children(worked.map(|line| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .debug_selector({
                                let bead = detail.bead.clone();
                                move || format!("bead worker {bead}")
                            })
                            .child(line),
                    )
                    .children(screen.clone().map(|session| {
                        screen_control(session, format!("bead screen {}", detail.bead), palette, cx)
                    }))
            }))
            .children(
                route
                    .as_ref()
                    .map(|route| self.route_strip(route, &detail.bead, palette, "bead")),
            )
            .children((!detail.expected.is_empty()).then(|| {
                let remark = remark_color(
                    &detail.remark,
                    &detail.tone,
                    &detail.held_by,
                    model.me(),
                    palette,
                );
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .child(
                        div()
                            .text_color(palette.faint)
                            .child(format!("arrives {}", detail.expected)),
                    )
                    .children((!detail.remark.is_empty()).then(|| {
                        div()
                            .text_color(remark)
                            .debug_selector({
                                let bead = detail.bead.clone();
                                move || format!("bead remark {bead}")
                            })
                            .child(format!("· {}", detail.remark))
                    }))
            }))
            .children((!detail.labels.is_empty()).then(|| {
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .debug_selector({
                        let bead = detail.bead.clone();
                        move || format!("bead labels {bead}")
                    })
                    .child(detail.labels.join(" · "))
            }));
        let mut list = div()
            .id("bead")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(scroll)
            .p_4()
            .flex()
            .flex_col()
            .gap_4();
        if detail.open_asks > 0 {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .debug_selector({
                        let bead = detail.bead.clone();
                        move || format!("bead asks {bead}")
                    })
                    .child(ask_words(detail.open_asks)),
            );
        }
        list =
            list.child(self.bead_body(select::key(("bead", &detail.bead)), &detail, palette, cx));
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

    /// One arrival's bead whole: its body, and its notes newest last,
    /// each with its author and time, a note's files as a mail's.
    pub(crate) fn bead_body(
        &self,
        bead_key: u64,
        detail: &ArrivalDetail,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut out = div().flex().flex_col().gap_2();
        if !detail.body.is_empty() {
            out = out
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.faint)
                        .child("body"),
                )
                .child(select::text(
                    expanded_text_key(bead_key, "body", 0, &detail.body),
                    detail.body.clone(),
                    &self.selection,
                ));
        }
        if !detail.notes.is_empty() {
            out = out.child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.faint)
                    .child("notes"),
            );
            // Who wrote a note stands over it and not before it: a note's
            // first line may be a heading or an item, which begin a line.
            for (ix, note) in detail.notes.iter().enumerate() {
                let mut block = div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(note.author.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(palette.faint)
                                    .child(model::clock(&note.at, &chrono::Local)),
                            ),
                    )
                    .child(select::text(
                        expanded_text_key(bead_key, "note", ix, &note.text),
                        note.text.clone(),
                        &self.selection,
                    ));
                // A note's files as a mail's: no run beside a page here,
                // as on a card.
                for file in &note.files {
                    block = block.child(self.picture(file, false, palette, cx));
                }
                out = out.child(block);
            }
        }
        out
    }

    /// The bead whole, expanded in place: its body, notes and about-beads.
    /// A design bead's document is named by path and branch, not fetched.
    pub(crate) fn expanded_body(
        &self,
        card_key: u64,
        content: Expanded<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let Expanded {
            body,
            notes,
            about,
            doc,
        } = content;
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
                let mut block = div()
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
                    ));
                for file in &note.files {
                    block = block.child(self.picture(file, true, palette, cx));
                }
                out = out.child(block);
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
