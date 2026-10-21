//! One bead's views: the bead whole, the conversation thread with
//! its mail, and the pictures and attachments of both.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, Image, ImageFormat, MouseButton, ObjectFit,
    ScrollHandle, SharedString, Window, div, img, prelude::*, px, rems, rgba, white,
};

use super::Expanded;
use super::Footplate;
use super::ask::ask_preview_head;
use super::expanded_text_key;
use super::metrics::{ATTACHMENT, IMAGE_HEIGHT, IMAGE_MARGIN, READING_WIDTH, drawer_fits};
use super::panel::{day_break, reading_column, reading_row};
use super::rail::{ImageOpen, OpenTip, Picture, Unsent, post};
use super::state::Piece;
use super::ui::{keys_hint, send_held, small_button};
use super::words::{ask_words, bead_back_name, bead_sub, remark_color};
use crate::api::{self, Address, BlobRef, Reaction};
use crate::draft::{self, Attachment, Found};
use crate::model::{self, ArrivalDetail, AskPreview, Entry, Model, Selected, Target, Top};
use crate::select;
use crate::theme::Palette;

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
            return line(model::not_taken(file)).into_any_element();
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
    /// without this it would float over the switcher, an image or the
    /// cheatsheet too.
    pub(crate) fn reacting(&self, seq: u64) -> bool {
        self.react_hover.is_some_and(|(at, _)| at == seq)
            && self.model.as_ref().is_some_and(|m| !m.react_too_old())
            && self.switching.is_none()
            && self.image_open.is_none()
            && !self.show_keys
    }

    /// The letter's head, with the bar of reactions at its right while the
    /// pointer stands on the letter: the head is the letter's first line,
    /// where it has one. `at` is the time it says: the time alone where a
    /// separator above names the day, with the date where none does.
    pub(crate) fn entry_head(
        &self,
        entry: &Entry,
        at: String,
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
            el.child(self.react_row(seq, &entry.reactions, palette, cx))
        })
        .into_any_element()
    }

    /// The letter's body. A letter that reads under another's head has no
    /// head of its own, so its bar of reactions floats over its first line.
    pub(crate) fn entry_body(
        &self,
        entry: Entry,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let seq = entry.seq;
        let hovered = !entry.head && self.reacting(seq);
        let body = div()
            .id(("body", seq))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            // The breathing room to the row below stands inside the
            // body, so a body touches the head or body under it.
            .pb_1();
        Self::hover_react(seq, Piece::Body, body, cx)
            // A correction carries its label above its text, so it reads
            // as no new thought.
            .when(entry.correction, |el| {
                el.child(
                    div()
                        .ml_4()
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
                el.child(div().ml_4().debug_selector(|| format!("text {seq}")).child(
                    select::text_with_refs(seq, entry.text, entry.refs, on_ref, &self.selection),
                ))
            })
            // The parties' reactions, one a line, in faint words under
            // the text.
            .when(!entry.reactions.is_empty(), |el| {
                el.child(
                    div()
                        .ml_4()
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
                    .pl_4()
                    .pt_1()
                    .child(self.picture(file, false, palette, cx))
            }))
            .when_some(entry.standing, |el, standing| {
                el.child(
                    div()
                        .pl_4()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(format!("· {standing}")),
                )
            })
            .when(hovered, |el| {
                el.child(self.react_row(seq, &entry.reactions, palette, cx))
            })
            .into_any_element()
    }

    /// The reactions a tap puts on the mail, in a bar that floats over the
    /// top right of the letter while the pointer stands on it: it is
    /// absolute in the head or body that draws it, so it takes no room
    /// and the letter keeps its place and its height. It paints after the
    /// rest of the window, so no line of the letter or of the next one
    /// paints over it: the kit paints in tree order, and the letter's
    /// body stands after the head that draws the bar. Its own ground and
    /// hairline keep the text under it from showing through, the kit's
    /// popover shadow lifts it, and it blocks the mouse, so that a tap on
    /// it is no press on that text. The yard's fixed set, each in a square
    /// of 26 pixels with 4 pixels around and between them: closer than the
    /// kit's 32-pixel control, since the bar floats over a letter and the
    /// emoji are all it holds. The one the pointer stands on draws 1.2
    /// times larger in its square, so neither its neighbours nor the bar
    /// move. The user's own current one stands on the open ground. A tap
    /// on that one takes it away, and a tap on another puts it instead.
    pub(crate) fn react_row(
        &self,
        seq: u64,
        reactions: &[Reaction],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                .top_0()
                .right_0()
                .flex()
                .flex_row()
                .gap_1()
                .p_1()
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
        // right of the head or body.
        gpui::deferred(row).into_any_element()
    }

    /// A mail among the yard row's notices. No separator names its day
    /// there, so its head says the date with the time, as a notice does.
    pub(crate) fn entry(
        &self,
        entry: Entry,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let head = entry.head;
        div()
            .flex()
            .flex_col()
            .when(head, |el| {
                let at = model::clock(&entry.at, &chrono::Local);
                el.child(self.entry_head(&entry, at, palette, cx))
            })
            .child(self.entry_body(entry, palette, cx))
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
            .pl_4()
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
    /// changes. Each group stands in the reading column.
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
                out.push(reading_row(self.entry_head(&entry, at, palette, cx), true));
            }
            out.push(reading_row(self.entry_body(entry, palette, cx), false));
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
        pane.children(self.reconnect_strip(palette))
            .child(head)
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
                            // The cheatsheet's hint, where `?` is the line's
                            // text: the keys open it with a click instead.
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .justify_end()
                                    .text_xs()
                                    .child(keys_hint("keys hint line", palette, cx)),
                            ),
                    ),
                ),
            )
    }

    /// One travelling bead whole: the title, id, platform and line, the
    /// expected arrival and remark of the row it came from, the labels,
    /// the body, then the notes newest last. Its asks are the Decisions
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
