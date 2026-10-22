//! The shell the views stand in: the rail, the list and the open
//! conversation, the settings and the keys, the tray and the badge,
//! and what answers the yard and the person: told and run, open and
//! follow, jump and scroll, dismiss and leave.

// A name only a screenshot state uses (shot.rs, a debug build's alone) is
// named by its path where it is used, not imported here: a release build
// has no such state, an import nothing else uses warns there, and the
// gate builds no release.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::thread;

use gpui::{
    Action, AnyElement, App, ClipboardItem, Context, Div, Focusable, FontWeight, MouseButton,
    MouseDownEvent, MouseMoveEvent, Rgba, ScrollHandle, SharedString, Stateful, Task, Window, div,
    img, prelude::*, px, relative, rems, rgba,
};

use crate::api::{self, Address, Client};
use crate::draft;
use crate::fetch::{self, Kept};
use crate::input;
use crate::model::{
    self, Effect, Kind, Mode, Model, News, Read, Route, Row, Selected, Target, Top,
};
use crate::notify::{self, Permission};
use crate::select;
use crate::session::{self, Update};
use crate::settings::{self, Notify, Section, Settings};
use crate::setup;
use crate::theme::{Look, Palette, crew_dot, yard_color, yard_ink};
use crate::titlebar::Gesture;
use crate::yard::Env;

use super::Footplate;
use super::ask::group_head;
use super::asks::{dir_url, yard_field, yard_help, yard_submit};
use super::firstrun::{Ended, FirstRun};
use super::keys::{KeysView, SECONDARY, keys_for};
use super::metrics::{DRAWER_WIDTH, HEAD_INSET, RAIL, RAIL_LABEL, RAIL_TILE, grown};
use super::rail::{PROBE_AGAIN, Picture, Probe, RailTip, Unsent, ViewUp, run_address_of};
use super::state::{
    About, Adding, Anchor, Away, Counts, Held, NOT_OPENED, Place, SwitchBusy, rail_counts, waiting,
};
use super::ui::{
    band, checkbox, gear_glyph, keys_hint, lamp_glyph, segment, segmented, semaphore_glyph,
    small_button, track_glyph,
};
use super::words::{footplate_words, unread_words, yardr_words};
use super::{Dismiss, JumpNewest, JumpOldest, Leave};

impl Footplate {
    /// As [`Footplate::opened`], with each yard's stream watched as
    /// `watch` says: the tests wait less for a stuck one.
    pub(crate) fn watching(
        env: Env,
        name: String,
        file: Option<PathBuf>,
        watch: session::Watch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::bare(env, name, window, cx).kept_in(file, cx);
        this.watch = watch;
        this.reopen(cx);
        // A screenshot state that opens the settings (shot.rs): a debug
        // build's alone, on the section the state names.
        #[cfg(debug_assertions)]
        if let Some(section) = shot_section() {
            this.settings_open = true;
            this.settings.section = section;
        }
        // The notifications shot shows the kind checkboxes as well as
        // the master control, in a state where they apply.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("settings-notifications") {
            this.settings.notifications.on = true;
        }
        // The reconnecting shot has a draft in the composer: the
        // yardmaster's conversation's, which opens first, in the line
        // once the yard is read.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("reconnecting") {
            this.drafts.put_text(
                "yardmaster",
                "Land it once the last bead is green.".to_string(),
            );
        }
        // The refusal shot pairs with a yard at an http address off this
        // machine: the way's fields stand open and filled, and pair was
        // pressed, so the refusal is the line under them. A debug build's
        // alone, as the settings and the draft above.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("refusal") {
            this.adding = Some(Adding::Far);
            this.yard_name
                .update(cx, |line, cx| line.set_text("droplet", cx));
            this.address.update(cx, |line, cx| {
                line.set_text("http://droplet-yard.tail5cf2e8.ts.net:8790", cx)
            });
            this.code
                .update(cx, |line, cx| line.set_text("K7QM-2XPD", cx));
            this.pair(cx);
        }
        // The answered and away shots stand with a way's fields open:
        // the first screen's when the yard answers under them, and the
        // away screen's of a yard that no longer lets the device in. A
        // debug build's alone, as above.
        #[cfg(debug_assertions)]
        if matches!(
            std::env::var("FOOTPLATE_SHOT").as_deref(),
            Ok("answered") | Ok("away")
        ) {
            this.adding = Some(Adding::Near);
            this.yard_name
                .update(cx, |line, cx| line.set_text("second home", cx));
            let socket = this.env.socket.to_string_lossy().into_owned();
            this.socket
                .update(cx, |line, cx| line.set_text(&socket, cx));
        }
        // The hosted shot is the first screen with the hosted way's fields
        // open and filled: the origin, and the Kratos account's email and
        // password, the password showing marks. Nothing is sent. A debug
        // build's alone, as above.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("hosted") {
            this.adding = Some(Adding::Hosted);
            this.yard_name
                .update(cx, |line, cx| line.set_text("trial", cx));
            this.address
                .update(cx, |line, cx| line.set_text("https://my.yardr.dev", cx));
            this.email
                .update(cx, |line, cx| line.set_text("jo@example.net", cx));
            this.password
                .update(cx, |line, cx| line.set_text("joes-password", cx));
        }
        // The rename shot is the settings' yards with the far yard's
        // rename form open in its card, with the old name standing. A
        // debug build's alone, as above.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("settings-yards-rename") {
            this.adding = Some(Adding::Rename("far".to_string()));
            this.yard_name
                .update(cx, |line, cx| line.set_text("far", cx));
        }
        // The setup shot is step 2 of setting up a yard here, posed: no
        // yard answers, and the first run stands at the yard's form. A
        // debug build's alone, as above, and one with a socket: the shots
        // are taken against a fake yard at one.
        #[cfg(all(debug_assertions, unix))]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("setup") {
            let first_run = cx.new(|cx| {
                FirstRun::shot_step2(
                    this.means.clone(),
                    &this.env.socket,
                    &this.name,
                    this.handle,
                    cx,
                )
            });
            this.first_run = Some(first_run);
        }
        // The switcher shot opens with the switcher's list standing, on
        // the open yard's row: four yards are paired, one of them away,
        // and the shot is the list. A debug build's alone, as above.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("switcher") {
            let open = this
                .settings
                .yards
                .iter()
                .position(|yard| yard.name == this.yard)
                .unwrap_or(0);
            this.switching = Some(open);
        }
        // The keys shot opens with the cheatsheet standing over the
        // card, as `?` leaves it. A debug build's alone, as above.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("keys") {
            this.show_keys = true;
        }
        // The About shots open with the card standing over the yardmaster's
        // conversation: the first with the yardr the shot script keeps, the
        // second with the card pointed at a directory that keeps none. A
        // debug build's alone, as above.
        #[cfg(debug_assertions)]
        match std::env::var("FOOTPLATE_SHOT").as_deref() {
            Ok("about") => this.show_about(window, cx),
            Ok("about-unfetched") => {
                this.means.own = Some(std::env::temp_dir().join(format!(
                    "footplate-shot-{}-about-unfetched",
                    std::process::id()
                )));
                this.show_about(window, cx);
            }
            _ => {}
        }
        // The reactions-hover shot poses the pointer on the last mail,
        // which reads under the one above's head: its bar of reactions
        // to tap floats over the end of its first line. A debug build's
        // alone, as above.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("reactions-hover") {
            this.react_hover = Some((15, super::state::Piece::Body));
        }
        // The reactions-zoom shot poses it on one emoji of that bar, so
        // the emoji draws larger in its square.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("reactions-zoom") {
            this.react_hover = Some((15, super::state::Piece::Bar));
            this.emoji_hover = Some((15, "❤️".to_string()));
        }
        this
    }

    /// Says on the dock's icon how many things wait, where that is another
    /// number than it says now.
    pub(crate) fn badge(&mut self) {
        let open = std::iter::once(self.model.as_ref());
        let held = self.held.values().map(|held| held.model.as_ref());
        let waiting = waiting(open.chain(held).map(|model| model.map(Counts::of)));
        if waiting != self.waiting {
            self.waiting = waiting;
            #[cfg(all(target_os = "macos", not(test)))]
            crate::dock::badge(waiting);
        }
    }

    /// Says in the menu bar, or the tray, what waits, while the settings
    /// have the item there: the number the dock's icon was just told
    /// ([`Footplate::badge`] goes first), what it is of in words, and each
    /// yard that is followed with what the switcher says of it. With the
    /// item switched off there is none. The mark is the settings' choice
    /// either way, drawn without a restart.
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    pub(crate) fn tray(&mut self) {
        // Before what it says: an item that is yet to be has its mark
        // from the start.
        self.tray.set_mark(self.settings.tray);
        let said = self.settings.menu_bar.then(|| {
            // A yard the environment names is the one that is followed.
            let named = self.env.yard.as_deref().map(|_| self.yard.as_str());
            let listed = self.settings.yards.iter().map(|yard| yard.name.as_str());
            let yards: Vec<&str> = match named {
                Some(named) => vec![named],
                None => listed.collect(),
            };
            // What waits over every yard, as the switcher says it of one;
            // the notices are not of the number, so not of its words.
            let open = std::iter::once(self.model.as_ref());
            let held = self.held.values().map(|held| held.model.as_ref());
            let waits = open.chain(held).flatten().map(Counts::of).fold(
                Counts::default(),
                |waits, yard| Counts {
                    mail: waits.mail + yard.mail,
                    decisions: waits.decisions + yard.decisions,
                    notices: 0,
                    changed: 0,
                },
            );
            crate::tray::said(
                self.waiting,
                &waits.line(),
                yards.into_iter().map(|yard| (yard, self.yard_waits(yard))),
            )
        });
        self.tray.say(said);
    }

    /// Does what a person chooses in the menu of the menu bar's item, as
    /// they choose it.
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    pub(crate) fn heed_tray(
        chosen: async_channel::Receiver<crate::tray::Chosen>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        use crate::tray::Chosen;

        cx.spawn(async move |this, cx| {
            while let Ok(chosen) = chosen.recv().await {
                let done = this.update(cx, |this, cx| match chosen {
                    Chosen::Yard(yard) => this.forward(Some(yard), None, cx),
                    Chosen::Show => this.forward(None, None, cx),
                    Chosen::Quit => cx.quit(),
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Brings the window forward on the row of each notification of
    /// `system` a person activates, in the yard it is a row of.
    pub(crate) fn heed(system: &dyn notify::System, cx: &mut Context<Self>) -> Task<()> {
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
    pub(crate) fn notified_by(
        mut self,
        system: Box<dyn notify::System>,
        cx: &mut Context<Self>,
    ) -> Self {
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
    pub(crate) fn ask_permission(&mut self, cx: &mut Context<Self>) {
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
    pub(crate) fn news(&mut self, cx: &mut Context<Self>) {
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
    pub(crate) fn tell(
        &mut self,
        yard: &str,
        news: Vec<News>,
        in_view: bool,
        cx: &mut Context<Self>,
    ) {
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
    pub(crate) fn come_forward(&mut self, yard: String, row: Selected, cx: &mut Context<Self>) {
        self.forward(Some(yard), Some(row), cx);
    }

    /// Brings the window forward, out of hiding where closing hid it. With
    /// a `yard` and a `row` it is on that row of that yard, as a
    /// notification asks. With a yard alone, as its row in the menu bar
    /// asks, it is on the yard as the switcher opens it
    /// ([`Footplate::switch_to`]): on the row it was left on, or the yard's
    /// own where that is gone. With neither it is on whatever is open.
    pub(crate) fn forward(
        &mut self,
        yard: Option<String>,
        row: Option<Selected>,
        cx: &mut Context<Self>,
    ) {
        // A test hides nothing, and has nothing to show again.
        #[cfg(not(test))]
        crate::unhide();
        cx.activate(true);
        let (this, handle) = (cx.weak_entity(), self.handle);
        // Afterwards: the window is not to be had from here.
        cx.defer(move |cx| {
            // A window that is closed has nothing to bring forward.
            let _ = handle.update(cx, |_, window, cx| {
                window.activate_window();
                this.update(cx, |this, cx| {
                    let Some(yard) = yard else {
                        return;
                    };
                    if row.is_none() || this.yard != yard {
                        this.switch_to(&yard, window, cx);
                    }
                    if let Some(row) = row
                        && this.yard == yard
                    {
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
    pub(crate) fn reopen(&mut self, cx: &mut Context<Self>) {
        self.reopen_anew(None, cx);
    }

    /// As [`Footplate::reopen`], with the yard called `anew` opened anew
    /// whatever is followed of it: its device was paired again, and the
    /// token it was opened with is not the yard's any more.
    pub(crate) fn reopen_anew(&mut self, anew: Option<&str>, cx: &mut Context<Self>) {
        if let Some(named) = self.env.yard.clone() {
            let opening = self.env.open(&self.settings, self.kept.as_deref());
            return self.follow(named, opening, cx);
        }
        let listed = self.settings.yards.clone();
        let show = self.settings.opened().name.clone();
        // Whether what is followed of a yard is followed further: it is
        // listed as it was opened, it is not to be opened anew, and it is
        // not the yard to show without a client to show it by. A change
        // of its web view or run base alone is the same yard, and goes
        // on.
        let goes_on = |name: &str, entry: &Option<settings::Yard>, client: &Option<Client>| {
            entry.as_ref().is_some_and(|entry| {
                listed
                    .iter()
                    .any(|yard| yard.name == name && yard.same_yard(entry))
            }) && anew != Some(name)
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
    pub(crate) fn follow(
        &mut self,
        yard: String,
        opening: Result<Client, String>,
        cx: &mut Context<Self>,
    ) {
        self.held.remove(&yard);
        let keep = self.yard != yard;
        self.leave_yard(&yard, keep, cx);
        let held = self.begin(&yard, None, opening, cx);
        self.put(held);
        self.switch_reset();
        self.agents_reset(cx);
        self.refresh_switch(cx);
    }

    /// Shows the yard called `yard`, which is held, as it stands: on the
    /// row it was left on, with what was typed there. The yard that was
    /// open is held aside in its turn where `keep` says so, and is
    /// followed no longer where not.
    pub(crate) fn show(&mut self, yard: &str, keep: bool, cx: &mut Context<Self>) {
        let Some(held) = self.held.remove(yard) else {
            return;
        };
        self.leave_yard(yard, keep, cx);
        self.put(held);
        self.arrive(cx);
        self.retarget_addresses(cx);
        self.switch_reset();
        self.agents_reset(cx);
        self.refresh_switch(cx);
    }

    /// The addresses' fields hold the shown yard's set ones, and what a
    /// typed one said is gone with the yard it was typed for: the yard
    /// was switched. Its pages go by probes asked for it, which ask
    /// again.
    pub(crate) fn retarget_addresses(&mut self, cx: &mut Context<Self>) {
        let (view, run, usual) = match self.settings.yard(&self.yard) {
            Some(yard) => (
                yard.web_view.clone(),
                yard.run_base.clone(),
                yard.view(&self.env.socket).unwrap_or("").to_string(),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        self.view.update(cx, |line, cx| line.set_text(&view, cx));
        self.run.update(cx, |line, cx| line.set_text(&run, cx));
        self.view
            .update(cx, |line, cx| line.set_placeholder(&usual, cx));
        let run_usual = self.run_usual();
        self.run
            .update(cx, |line, cx| line.set_placeholder(&run_usual, cx));
        self.view_said = None;
        self.run_said = None;
    }

    /// Starts following a yard behind the client, by a follower of its
    /// own, and gives what is held of it from here on: `yard` is its name,
    /// and `entry` the yard of the settings it is. Without a client it
    /// says why there is none, and nothing is asked.
    pub(crate) fn begin(
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
            refused: HashMap::new(),
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
            let (client, name, watch) = (client.clone(), self.name.clone(), self.watch);
            move || {
                session::follow_with_control(
                    &client,
                    &name,
                    session::RETRY,
                    watch,
                    stop,
                    |update| tell.send_blocking(update).is_ok(),
                )
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
    pub(crate) fn leave_yard(&mut self, to: &str, keep: bool, cx: &mut Context<Self>) {
        // The line's text is the open row's of the yard that is left: it
        // stays with that, and is in the line again when the row is open.
        if let Some(from) = self.model.as_ref().and_then(|m| m.selected().cloned()) {
            // A yard left before it was put back where it stood has
            // nothing newer to say of that: its place stays the kept
            // one.
            if self.returning.is_none() {
                let mode = self.mode();
                let kept = self.places.get(&self.yard).cloned().unwrap_or_default();
                let place = self.place(mode, from.clone(), kept);
                self.places.insert(self.yard.clone(), place);
            }
            self.keep_typed(&from, cx);
        }
        self.returning = None;
        // The pointer's mark on a letter is of the yard that is left: a
        // seq alone names a letter, so the yard shown next would have its
        // own mail of that number under the bar. Its emoji's mark goes
        // with it, as where the open row changes. The bead drawer shuts
        // with them: its bead is a bead of the yard that is left.
        self.react_hover = None;
        self.emoji_hover = None;
        self.drawer = None;
        let words = std::mem::take(&mut self.ask_words);
        if !words.is_empty() {
            self.kept_words.insert(self.yard.clone(), words);
        }
        self.ask_words = self.kept_words.remove(to).unwrap_or_default();
        let files = std::mem::take(&mut self.note_files);
        if !files.is_empty() {
            self.kept_note_files.insert(self.yard.clone(), files);
        }
        self.note_files = self.kept_note_files.remove(to).unwrap_or_default();
        self.drafts.turn_to(to);
        // A yard that answered under a way's fields is held as up: it
        // answered, and goes on being followed.
        if self.answered_while_adding {
            self.answered_while_adding = false;
            if !self.unreached.contains_key(&self.yard) {
                self.down = None;
            }
        }
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
            refused: std::mem::take(&mut self.refused),
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
        self.away_details = false;
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
    pub(crate) fn put(&mut self, mut held: Held) {
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
        self.refused = std::mem::take(&mut held.refused);
        self.ask_generation = held.ask_generation;
        self.returns_read = held.returns_read.take();
    }

    /// The yard that was held is shown: in its remembered mode, on the
    /// row that mode was left on, where that is still there, and the line
    /// has what was typed in it. A yard that has not answered yet has
    /// none, and is shown when it does ([`Footplate::told`]).
    pub(crate) fn arrive(&mut self, cx: &mut Context<Self>) {
        let mode = self.mode();
        let kept = self.places.get(&self.yard).cloned().unwrap_or_default();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let effects = model.back_to(
            mode,
            kept.row(mode),
            kept.decided_expanded,
            kept.quiet_expanded,
            kept.departures,
        );
        model.expand(&kept.expanded);
        model.show_texts(&kept.texts);
        model.set_decided_chip(kept.decided_chip);
        model.show_folds(&kept.decided_folds);
        // The filter's field is the window's, not the yard's: what is
        // typed is this yard's filter again.
        let typed = self.filter.read(cx).text().to_string();
        model.set_decided_filter(&typed);
        // Back on Timetable: it is looked at again, and the dot clears.
        let looked = (mode == Mode::Timetable).then(|| model.timetable_visited(true));
        if let Some(late) = self.late.take() {
            model.notice = late;
        }
        let to = model.selected().cloned();
        if let Some(looked) = looked {
            let yard = self.yard.clone();
            self.set(self.settings.clone().with_timetable_seen(&yard, looked), cx);
        }
        self.returning = kept
            .row(mode)
            .filter(|row| to.as_ref() == Some(row))
            .map(|row| ((row, kept.off), false));
        // The mode's list stands where it stood.
        let mut at = self.rows.offset();
        at.y = px(0.) - kept.list(mode);
        self.rows.set_offset(at);
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
    pub(crate) fn on_app_action<A: Action>(
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

    /// The rail's mode in the open yard: the remembered one, chat where
    /// none was.
    pub(crate) fn mode(&self) -> Mode {
        self.settings.mode_of(&self.yard)
    }

    /// What the follower of the following `session` learned: of the open
    /// yard, or of one that is held, which has it without showing it. Its
    /// mail, its asks and its notices are told of as the open yard's are,
    /// and nothing of it is seen. One that is followed no longer says
    /// nothing.
    pub(crate) fn told_of(&mut self, session: u64, update: Update, cx: &mut Context<Self>) {
        if session == self.session {
            return self.told(update, cx);
        }
        let Some((yard, held)) = self.held.iter_mut().find(|(_, h)| h.session == session) else {
            return;
        };
        let yard = yard.clone();
        let mut baseline_to_save = None;
        match update {
            Update::Up(state) => {
                let mut state = *state;
                let head = state.head;
                let fresh = held.model.is_none();
                held.said = state.yard.take();
                let arrivals_notice = state.arrivals_notice.take();
                let flows_notice = state.flows_notice.take();
                let grants_notice = state.grants_notice.take();
                let (mut model, mut effects) = match held.model.as_ref() {
                    Some(before) => before.resumed(state, self.settings.mode_of(&yard)),
                    None => (Model::new_aside(state), Vec::new()),
                };
                let notices: Vec<String> = [arrivals_notice, flows_notice, grants_notice]
                    .into_iter()
                    .flatten()
                    .collect();
                if !notices.is_empty() {
                    model.notice = notices.join("; ");
                }
                if fresh {
                    // A held yard is never looked at: the dot starts from
                    // when Timetable was last open, caught up to the head.
                    let seen = self.settings.timetable_seen_of(&yard);
                    let looked = model.adopt_timetable_seen(seen, head);
                    if seen.is_none() {
                        // Keep the first-run baseline so a later restart
                        // can still find changes that arrive while held.
                        baseline_to_save = Some((yard.clone(), looked));
                    }
                    if looked < head {
                        effects.push(Effect::Timetable {
                            from: looked,
                            until: head,
                        });
                    }
                }
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
            Update::SignedOut(why) => {
                held.down = Some(why);
                held.away = Away::SignedOut;
                self.unreached.insert(yard, session::SIGN_IN_AGAIN);
            }
        }
        if let Some((yard, looked)) = baseline_to_save {
            self.set(self.settings.clone().with_timetable_seen(&yard, looked), cx);
        }
        cx.notify();
    }

    /// What the thread that follows the open yard learned.
    pub(crate) fn told(&mut self, update: Update, cx: &mut Context<Self>) {
        let was_down = self.down.is_some();
        match update {
            Update::Up(state) => {
                let mut state = *state;
                let head = state.head;
                self.said = state.yard.take();
                let arrivals_notice = state.arrivals_notice.take();
                let flows_notice = state.flows_notice.take();
                let grants_notice = state.grants_notice.take();
                let open = self.model.as_ref().and_then(|m| m.selected().cloned());
                let fresh = self.model.is_none();
                // The yard was opened: in its remembered mode, on the row
                // that mode was left on, where that is still there. A
                // screenshot state opens its own row's mode instead.
                let shot = fresh.then(shot_selected).flatten();
                let mode = shot
                    .as_ref()
                    .map(Selected::mode)
                    .unwrap_or_else(|| self.mode());
                if fresh && mode != self.mode() {
                    let yard = self.yard.clone();
                    self.set(self.settings.clone().with_mode(&yard, mode), cx);
                }
                let kept = self
                    .places
                    .get(&self.yard)
                    .filter(|_| fresh)
                    .cloned()
                    .unwrap_or_default();
                let (mut model, mut effects) = match self.model.as_ref() {
                    Some(before) => {
                        let (mut model, mut effects) = before.resumed(state, mode);
                        // The drawer's bead was read into the model that is
                        // gone: read it into this one, while the drawer
                        // stands open on it. A shut drawer reads nothing.
                        if let Some(bead) = self.drawer.clone() {
                            let drawer_read = model.read_bead(&bead);
                            // `resumed` may already ask for it as the
                            // selected decision; one answer serves both.
                            if !effects.iter().any(
                                |effect| matches!(effect, Effect::Bead { bead: queued } if queued == &bead),
                            ) {
                                effects.extend(drawer_read);
                            }
                        }
                        (model, effects)
                    }
                    None => Model::new_returning(
                        state,
                        mode,
                        kept.row(mode).or(shot),
                        kept.decided_expanded,
                        kept.quiet_expanded,
                        kept.departures,
                    ),
                };
                // The departures shot opens the arrivals pane under the
                // toggle: a debug build's alone, as the row above.
                #[cfg(debug_assertions)]
                if fresh && shot_departures() {
                    model.set_departures(true);
                }
                let notices: Vec<String> = [arrivals_notice, flows_notice, grants_notice]
                    .into_iter()
                    .flatten()
                    .collect();
                if !notices.is_empty() {
                    model.notice = notices.join("; ");
                }
                // The model read again has no filter yet: what the field
                // holds is its filter, as on a yard switched to.
                let typed = self.filter.read(cx).text().to_string();
                model.set_decided_filter(&typed);
                if fresh {
                    // The dot starts from when Timetable was last open,
                    // caught up to the head. Opened looking, what is
                    // read is seen.
                    if mode == Mode::Timetable {
                        let looked = model.timetable_visited(true);
                        let yard = self.yard.clone();
                        self.set(self.settings.clone().with_timetable_seen(&yard, looked), cx);
                    } else {
                        let yard = self.yard.clone();
                        let seen = self.settings.timetable_seen_of(&yard);
                        let looked = model.adopt_timetable_seen(seen, head);
                        if seen.is_none() {
                            // The first-run baseline must survive a restart
                            // before the person first opens Timetable.
                            self.set(self.settings.clone().with_timetable_seen(&yard, looked), cx);
                        }
                        if looked < head {
                            effects.push(Effect::Timetable {
                                from: looked,
                                until: head,
                            });
                        }
                    }
                    model.expand(&kept.expanded);
                    model.show_texts(&kept.texts);
                    model.set_decided_chip(kept.decided_chip);
                    model.show_folds(&kept.decided_folds);
                    // The page shot opens the card, so the note's page row
                    // and image show: a debug build's alone, as the settings
                    // and the draft of Footplate::opened.
                    #[cfg(debug_assertions)]
                    if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("page")
                        && let Some((bead, ask)) = model.current_ask()
                    {
                        model.toggle_expand(&bead, ask);
                    }
                    self.returning = kept
                        .row(mode)
                        .filter(|row| model.selected() == Some(row))
                        .map(|row| ((row, kept.off), false));
                    // The mode's list stands where it stood.
                    let mut at = self.rows.offset();
                    at.y = px(0.) - kept.list(mode);
                    self.rows.set_offset(at);
                    // The page shot stands at the card's end, so the note's
                    // page row and image show: past its end, where the
                    // layout stops, as with a yard returned to. A debug
                    // build's alone.
                    #[cfg(debug_assertions)]
                    if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("page")
                        && let Some(Selected::Decision(bead)) = model.selected()
                    {
                        self.returning =
                            Some(((Selected::Decision(bead.clone()), px(1_000_000.)), false));
                    }
                    // The image shot opens the mail's image over the
                    // window, as a click on its thumbnail does: the yard
                    // holds the shot's bytes, and they show when they
                    // come. A debug build's alone, as above, and a unix one's.
                    #[cfg(all(debug_assertions, unix))]
                    if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("image") {
                        let (hash, bytes) = crate::shot::IMAGE_OPEN;
                        self.image_open = Some(super::rail::ImageOpen {
                            hash: hash.to_string(),
                            bytes,
                            back: None,
                        });
                    }
                    // The drawer shot opens the bead beside the
                    // yardmaster's conversation, as a click on its chip
                    // does: the bead is read whole, and the drawer shows
                    // it once the call answers. A debug build's alone,
                    // as above.
                    #[cfg(debug_assertions)]
                    if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("drawer") {
                        self.drawer = Some("yd-zulu".into());
                        effects.extend(model.read_bead("yd-zulu"));
                    }
                    // The decisions shots stand with the history open; the
                    // filtered one with `docs` typed in its filter, as a
                    // `/` and the word would. A debug build's alone.
                    #[cfg(debug_assertions)]
                    if matches!(
                        std::env::var("FOOTPLATE_SHOT").as_deref(),
                        Ok("decisions") | Ok("decisions-filtered")
                    ) {
                        model.toggle_decided();
                        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("decisions-filtered") {
                            model.set_decided_filter("docs");
                            self.filter.update(cx, |line, cx| line.set_text("docs", cx));
                        }
                    }
                }
                self.model = Some(model);
                // The yard answers: the away screen's details are shut,
                // for the next time it is away.
                self.away_details = false;
                // Where a way's fields stand open on the away screen, they
                // and the screen stand: what is typed stays in the field,
                // and the screen says the yard answers again. Shutting
                // them would give the keys to the line, and the rest of a
                // pairing's code to the yard that came back.
                if self.down.is_some() && self.adding.is_some() && !self.settings_open {
                    self.answered_while_adding = true;
                } else {
                    // The fields of a way to a yard stood where this yard is
                    // drawn now: they are asked for no longer, and the line
                    // has the keys, not a field that is not in view.
                    if self.down.take().is_some() && self.adding.is_some() && !self.settings_open {
                        self.shut_adding(cx);
                    }
                }
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
                // The yard answers: what a start could not start is
                // healed, whatever turn of the follower this is.
                self.switch_troubles = Vec::new();
            }
            Update::Event(event) => self.change(cx, |m| m.event(&event)),
            Update::Down(why, short) => {
                self.down = Some(why);
                self.away = Away::Trying;
                self.unreached.insert(self.yard.clone(), short);
                // The question before stopping is for a yard that
                // answers; where it went away, it is asked anew.
                self.switch_question = None;
            }
            Update::Out(why) => {
                self.down = Some(why);
                self.away = Away::Out;
                self.unreached
                    .insert(self.yard.clone(), session::PAIR_AGAIN);
                self.switch_question = None;
            }
            Update::SignedOut(why) => {
                self.down = Some(why);
                self.away = Away::SignedOut;
                self.unreached
                    .insert(self.yard.clone(), session::SIGN_IN_AGAIN);
                self.switch_question = None;
            }
        }
        // The follower asks a yard that is away every few seconds: the
        // switch is read where that changed what answers, and not again
        // for every asking.
        if was_down != self.down.is_some() {
            self.refresh_switch(cx);
        }
        cx.notify();
    }

    /// Changes the model of the following `session`, wherever its yard is
    /// by now: the open yard's as [`Footplate::change`] does, and that of
    /// a yard that is held with nothing shown, its calls made by its own
    /// client and what arrived in it told of as its. A yard followed no
    /// longer has none to change.
    pub(crate) fn change_in(
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
    pub(crate) fn model_in(&mut self, session: u64) -> Option<&mut Model> {
        if session == self.session {
            return self.model.as_mut();
        }
        let held = self.held.values_mut().find(|h| h.session == session)?;
        held.model.as_mut()
    }

    /// Changes the open yard's model and does what the change asks for. A
    /// conversation shows its newest, and the yard its notices; the open
    /// card never moves for mail or notices arriving.
    pub(crate) fn change(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Model) -> Vec<Effect>,
    ) {
        let before_first_mail = self.first_mail_item();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let before_sel = model.selected().cloned();
        let before_position = matches!(&before_sel, Some(Selected::Conversation(_)))
            .then(|| self.conversation.offset());
        let before_span = model.span();
        let before_notices = model.notices().len() + model.yard_mail().len();
        let before_files = model.delivered_count();
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
            // A bead open full width shuts the drawer: one bead view
            // shows.
            if matches!(&after_sel, Some(Selected::Bead(_))) {
                self.drawer = None;
            }
            // The pointer's mark on a letter goes with the row. A letter
            // that is not drawn is told of no leave, so a pointer that
            // goes while the row is away would leave the mark standing,
            // and the bar would be over that letter when the row is open
            // again. Its emoji's mark goes with it, for the same reason.
            self.react_hover = None;
            self.emoji_hover = None;
            match &after_sel {
                Some(Selected::Decision(_))
                | Some(Selected::Decided(_, _))
                | Some(Selected::Grants)
                | Some(Selected::Arrivals)
                | Some(Selected::Bead(_)) => {
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
                            model.rows_above(oldest),
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
                                self.keep_place(above, before_first_mail);
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
                // A delivery's files that came after its notice make the
                // row taller where the view stands: a view at its newest
                // stays there, and one scrolled up is the reader's.
                Some(Selected::Yard)
                    if model.delivered_count() != before_files
                        && (self.bottom_pending || self.at_bottom()) =>
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
        self.turned(before_sel.clone(), cx);
        // The selection leads and the mode follows: a row of another mode
        // opens its mode, and the yard remembers it. The row left is the
        // old mode's again when the person steps back.
        if let (Some(before), Some(after)) = (&before_sel, &after_sel) {
            let (from, to) = (before.mode(), after.mode());
            if from != to {
                let mut kept = self.places.get(&self.yard).cloned().unwrap_or_default();
                kept.rows[from.index()] = Some(before.clone());
                kept.lists[from.index()] = (px(0.) - self.rows.offset().y).max(px(0.));
                self.places.insert(self.yard.clone(), kept);
                if to != self.mode() {
                    let yard = self.yard.clone();
                    self.set(self.settings.clone().with_mode(&yard, to), cx);
                }
            }
        }
        self.shown(cx);
        self.pictures(cx);
        self.run(effects, cx);
        self.news(cx);
        cx.notify();
    }

    /// Whether the conversation stands at its newest: its scroll offset
    /// is at the bottom, within a pixel.
    pub(crate) fn at_bottom(&self) -> bool {
        let offset = self.conversation.offset();
        let max = self.conversation.max_offset();
        offset.y + max.height <= px(1.)
    }

    /// Whether the jump control stands over the open conversation: one
    /// is open, the settings are not over it, and its view is above its
    /// newest.
    pub(crate) fn shows_jump(&self) -> bool {
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
    pub(crate) fn scroll_to_newest(&mut self) {
        self.conversation.scroll_to_bottom();
        self.bottom_pending = true;
        self.anchor = None;
    }

    /// `above` rows were put in above the view of the open conversation:
    /// the mail at the top of the view stays where it stood. The scroll
    /// offset is counted from the top of the conversation, and how high
    /// the new rows are is known only once they are laid out. So the
    /// layout of the next frame puts that mail at the top of the view, and
    /// the frame after it moves the view by what the mail stood off the
    /// top ([`Footplate::reconcile_scroll`]).
    /// The oldest shown mail's body among the conversation's children:
    /// past the line above it, its day's break, and its group's head.
    pub(crate) fn first_mail_item(&self) -> usize {
        let breaks = self
            .model
            .as_ref()
            .map(|m| m.entries().first().is_some_and(|e| e.day.is_some()))
            .unwrap_or(false);
        2 + usize::from(breaks)
    }

    pub(crate) fn keep_place(&mut self, above: usize, first_mail: usize) {
        let anchor = match self.anchor.take() {
            // More was put in before the last was laid out.
            Some((kept, _)) => Anchor {
                item: kept.item + above,
                off: kept.off,
            },
            None => {
                // The line, day break and group head stay at the front
                // when older mail joins the first group. Anchor its first
                // body instead, which moves by the new mail rows.
                let top = self.conversation.logical_scroll_top().0;
                let item = if top < first_mail { first_mail } else { top };
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
    pub(crate) fn reconcile_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // A resize keeps a conversation that stood at its newest there:
        // the offset counts from the top, so a smaller view ends above
        // it. One scrolled up is not moved. A jump the layout has not
        // put there yet counts as standing there.
        let size = self.conversation.bounds().size;
        if let Some((stood_size, stood_bottom)) = self.stood
            && size != stood_size
            && stood_bottom
        {
            self.scroll_to_newest();
        }
        self.stood = Some((size, self.bottom_pending || self.at_bottom()));
        if self.bottom_pending {
            self.bottom_pending = false;
            return;
        }
        if self.at_bottom() {
            self.new_below = 0;
        }
        self.reach_top(cx);
        // The arrivals shots show the list's end, where the arrivals row
        // stands: a debug build's alone, as the page shot's stand at its
        // card's end. The layout says how far the end is, so the frame
        // that first has it whole stands there; past that the list is
        // the reader's.
        #[cfg(debug_assertions)]
        if matches!(
            std::env::var("FOOTPLATE_SHOT").as_deref(),
            Ok("yardmaster") | Ok("arrivals") | Ok("arrival")
        ) {
            let max = self.rows.max_offset().height;
            if max > px(0.) && self.rows.offset().y == px(0.) {
                let mut at = self.rows.offset();
                at.y = px(0.) - max;
                self.rows.set_offset(at);
            }
        }
    }

    /// Where the open yard stands, on `row`: what is kept of it when
    /// another yard is opened.
    /// Where the open yard stands, on `row` in `mode`: what is kept of it
    /// when another yard is opened. The row and the list's scroll are the
    /// mode's own; the pane's scroll and the expanded cards are the yard's,
    /// for the row that was open. What other modes kept stands.
    pub(crate) fn place(&self, mode: Mode, row: Selected, mut kept: Place) -> Place {
        let off = match &row {
            // A jump to the newest that the layout has not applied yet
            // is where it stands all the same.
            Selected::Conversation(_) | Selected::Yard if self.bottom_pending => px(0.),
            Selected::Conversation(_) | Selected::Yard => {
                self.conversation.max_offset().height + self.conversation.offset().y
            }
            // The board, the lines, No line, the grants and the bead
            // read from the top, as a card does.
            Selected::Decision(_)
            | Selected::Decided(_, _)
            | Selected::Grants
            | Selected::Arrivals
            | Selected::Line(_)
            | Selected::NoLine
            | Selected::Bead(_) => px(0.) - self.decision.offset().y,
        };
        let model = self.model.as_ref();
        kept.rows[mode.index()] = Some(row);
        kept.lists[mode.index()] = (px(0.) - self.rows.offset().y).max(px(0.));
        kept.off = off.max(px(0.));
        kept.expanded = model.map(|m| m.expanded().clone()).unwrap_or_default();
        kept.texts = model.map(|m| m.texts().clone()).unwrap_or_default();
        kept.decided_expanded = model.is_some_and(Model::decided_expanded);
        kept.quiet_expanded = model.is_some_and(Model::quiet_expanded);
        kept.departures = model.is_some_and(Model::departures);
        kept.decided_chip = model.and_then(|m| m.decided_chip());
        kept.decided_folds = model.map(|m| m.decided_folds().clone()).unwrap_or_default();
        kept
    }

    /// Puts what is shown where it stood when its yard was left for
    /// another. Not before the yard has answered for what the row shows:
    /// until then the row is shorter than it was, and the layout would
    /// hold the view within that. A conversation's place is counted from
    /// its newest, and how long it is the layout says: so the frame that
    /// first has it whole lays it out, and the one after moves the view.
    pub(crate) fn come_back(&mut self, window: &mut Window) {
        let Some(((row, off), laid)) = self.returning.take() else {
            return;
        };
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let here = match &row {
            Selected::Conversation(_) | Selected::Yard => model.answered(),
            Selected::Decision(bead) | Selected::Decided(bead, _) | Selected::Bead(bead) => {
                model.has_bead(bead)
            }
            // The board's, a line's, No line's and the grants' rows came
            // with the state: they are whole at once.
            Selected::Arrivals | Selected::Line(_) | Selected::NoLine | Selected::Grants => true,
        };
        // Under the settings nothing of the row is laid out.
        if !here || self.settings_open {
            self.returning = Some(((row, off), false));
            return;
        }
        // A jump to the newest asked for since is applied by this frame's
        // layout, over any offset set here: the next one has the place.
        if !laid || self.bottom_pending {
            self.returning = Some(((row, off), true));
            window.request_animation_frame();
            return;
        }
        if matches!(&row, Selected::Conversation(_))
            && off > self.conversation.max_offset().height
            && model.top() == Top::Older
        {
            // The saved position is beyond the newest page. Reach the
            // oldest loaded mail to fetch older pages, then try the saved
            // distance again after each page is laid out.
            let mut at = self.conversation.offset();
            at.y = px(0.);
            self.conversation.set_offset(at);
            self.returning = Some(((row, off), true));
            return;
        }
        match row {
            Selected::Conversation(_) | Selected::Yard => {
                let mut at = self.conversation.offset();
                at.y = (off - self.conversation.max_offset().height).min(px(0.));
                self.conversation.set_offset(at);
            }
            // Where the card is shorter now, the layout stops at its end.
            Selected::Decision(_)
            | Selected::Decided(_, _)
            | Selected::Grants
            | Selected::Arrivals
            | Selected::Line(_)
            | Selected::NoLine
            | Selected::Bead(_) => {
                let mut at = self.decision.offset();
                at.y = px(0.) - off;
                self.decision.set_offset(at);
            }
        }
    }

    /// Asks for the older mail of the open conversation when its view
    /// stands within half its height of the top, and the yard has older:
    /// before the top is reached, so that reading upwards rarely waits
    /// there.
    pub(crate) fn reach_top(&mut self, cx: &mut Context<Self>) {
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
    pub(crate) fn to_newest(shown: &ScrollHandle) {
        let max = shown.max_offset();
        let mut at = shown.offset();
        at.y = px(0.) - max.height;
        shown.set_offset(at);
    }

    /// Jumps the open conversation to its newest message: the control
    /// and its key. The offset is set at once, so the control is gone in
    /// the frame that follows, and the line keeps the keys.
    pub(crate) fn jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    /// card's, the board's or the bead's, or the yard row's, which is the
    /// conversation's. None under the settings, where none of those is
    /// shown, and none with nothing open.
    pub(crate) fn scrolled(&self) -> Option<&ScrollHandle> {
        if self.settings_open {
            return None;
        }
        match self.model.as_ref()?.selected() {
            Some(Selected::Conversation(_) | Selected::Yard) => Some(&self.conversation),
            Some(
                Selected::Decision(_)
                | Selected::Decided(_, _)
                | Selected::Grants
                | Selected::Arrivals
                | Selected::Line(_)
                | Selected::NoLine
                | Selected::Bead(_),
            ) => Some(&self.decision),
            None => None,
        }
    }

    /// Moves what is shown by a page: down towards the newest, or up
    /// towards the oldest. A page is the height of the view less two
    /// lines, so the eye keeps its place; at an end the view stops.
    pub(crate) fn page(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(crate) fn jump_oldest(&mut self, _: &JumpOldest, _: &mut Window, cx: &mut Context<Self>) {
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
    pub(crate) fn jump_newest(
        &mut self,
        _: &JumpNewest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    /// The web view's base address of the yard the window shows: its
    /// set one, else its usual. None is no web view: the yard's files
    /// say it is not set, and nothing is probed for it.
    pub(crate) fn view_base(&self) -> Option<String> {
        self.settings
            .yard(&self.yard)?
            .view(&self.env.socket)
            .map(str::to_string)
    }

    /// The run base the shown yard uses where none is set: its web
    /// view's host on the sandbox port, or none where it has no view.
    pub(crate) fn run_usual(&self) -> String {
        self.view_base()
            .and_then(|base| Address::parse_view(&base).ok())
            .map(|view| view.run_base().to_string())
            .unwrap_or_default()
    }

    /// The run base the yard the window shows has set: empty where
    /// unset, and of the one yard the environment names, which keeps
    /// none.
    pub(crate) fn run_set(&self) -> String {
        self.settings
            .yard(&self.yard)
            .map(|yard| yard.run_base.clone())
            .unwrap_or_default()
    }

    /// Asks whether the web view answers at the shown yard's base
    /// address: a page carries Open when it does, and how to start it
    /// when it does not. Once per yard and address while it answers; a
    /// "no" is asked again on a timer while a file's or page's row is
    /// drawn. Off the window's thread, as a call. Where the yard has no
    /// view nothing is asked, and nowhere is probed for it.
    pub(crate) fn view_probe(&mut self, cx: &mut Context<Self>) {
        let Some(base) = self.view_base() else {
            self.view_up = ViewUp::Unknown;
            return;
        };
        let Some(yard) = self.yard_identity(&self.yard) else {
            self.view_up = ViewUp::Unknown;
            return;
        };
        let asked = Probe { yard, base };
        match &self.view_up {
            ViewUp::Asked(asked_) | ViewUp::Answered(asked_, _) if asked_ == &asked => return,
            _ => {}
        }
        // The page shot shows the row with Open and run, and the mail
        // and image shots the mail's row with Open and its image: the
        // view is taken as answering at its address, so the shot is the
        // row and not the probe. A debug build's alone; where the
        // address reads as none, the probe below says so.
        #[cfg(debug_assertions)]
        if matches!(
            std::env::var("FOOTPLATE_SHOT").as_deref(),
            Ok("page") | Ok("mail") | Ok("image")
        ) && api::Address::parse_view(&asked.base).is_ok()
        {
            self.view_up = ViewUp::Answered(asked, true);
            return;
        }
        self.view_up = ViewUp::Asked(asked.clone());
        cx.spawn(async move |this, cx| {
            let base = asked.base.clone();
            let up = cx
                .background_executor()
                .spawn(async move {
                    api::Address::parse_view(&base).is_ok_and(|address| address.answers())
                })
                .await;
            this.update(cx, |this, cx| {
                if matches!(&this.view_up, ViewUp::Asked(now) if now == &asked) {
                    this.view_up = ViewUp::Answered(asked.clone(), up);
                    if !up {
                        this.view_retry(asked, cx);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks the web view again after a "no" at `asked`, in PROBE_AGAIN:
    /// a view started after the first probe is found without a restart.
    /// Only while a file's or page's row is drawn; a "yes" is never
    /// asked again, and neither is another yard or address.
    pub(crate) fn view_retry(&mut self, asked: Probe, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PROBE_AGAIN).await;
            this.update(cx, |this, cx| {
                if !matches!(&this.view_up, ViewUp::Answered(now, false) if now == &asked) {
                    return;
                }
                // The timer ticks on while the answer is "no"; the asking
                // waits for a row that shows it.
                if this.rows_shown.get() {
                    this.view_up = ViewUp::Unknown;
                    this.view_probe(cx);
                } else {
                    this.view_retry(asked, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the web view answers at the shown yard's address, if that
    /// was asked: what a page is drawn with.
    pub(crate) fn view_up_now(&self) -> Option<bool> {
        let base = self.view_base()?;
        let yard = self.yard_identity(&self.yard)?;
        match &self.view_up {
            ViewUp::Answered(asked, up) if asked.yard == yard && asked.base == base => Some(*up),
            _ => None,
        }
    }

    /// Asks whether the sandbox origin answers at the shown yard's run
    /// base: a note's page carries run beside Open when it does, and why
    /// not when it does not. Once per yard and base while it answers; a
    /// "no" is asked again on a timer while a file's or page's row is
    /// drawn. Off the window's thread, as a call.
    pub(crate) fn run_probe(&mut self, cx: &mut Context<Self>) {
        if self.view_base().is_none() && self.run_set().trim().is_empty() {
            self.run_up = ViewUp::Unknown;
            return;
        }
        let Some(yard) = self.yard_identity(&self.yard) else {
            self.run_up = ViewUp::Unknown;
            return;
        };
        let asked = Probe {
            yard,
            base: self.run_asked(),
        };
        match &self.run_up {
            ViewUp::Asked(asked_) | ViewUp::Answered(asked_, _) if asked_ == &asked => return,
            _ => {}
        }
        // The page shot shows the row with Open and run: both are taken
        // as answering at their addresses, so the shot is the row and
        // not the probes. A debug build's alone; where an address reads
        // as none, the probe below says so.
        #[cfg(debug_assertions)]
        if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("page") && self.run_address().is_some()
        {
            self.run_up = ViewUp::Answered(asked, true);
            return;
        }
        self.run_up = ViewUp::Asked(asked.clone());
        let run_setting = self.run_set();
        let view_setting = self.view_base().unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let up = cx
                .background_executor()
                .spawn(async move {
                    run_address_of(&run_setting, &view_setting)
                        .is_some_and(|address| address.answers())
                })
                .await;
            this.update(cx, |this, cx| {
                if matches!(&this.run_up, ViewUp::Asked(now) if now == &asked) {
                    this.run_up = ViewUp::Answered(asked.clone(), up);
                    if !up {
                        this.run_retry(asked, cx);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks the sandbox origin again after a "no" at `asked`, in
    /// PROBE_AGAIN: an origin started after the first probe is found
    /// without a restart. Only while a file's or page's row is drawn; a
    /// "yes" is never asked again, and neither is another yard or base.
    pub(crate) fn run_retry(&mut self, asked: Probe, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PROBE_AGAIN).await;
            this.update(cx, |this, cx| {
                if !matches!(&this.run_up, ViewUp::Answered(now, false) if now == &asked) {
                    return;
                }
                // The timer ticks on while the answer is "no"; the asking
                // waits for a row that shows it.
                if this.rows_shown.get() {
                    this.run_up = ViewUp::Unknown;
                    this.run_probe(cx);
                } else {
                    this.run_retry(asked, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the sandbox origin answers at the shown yard's run base,
    /// if that was asked: what a note's page is drawn with.
    pub(crate) fn run_up_now(&self) -> Option<bool> {
        let yard = self.yard_identity(&self.yard)?;
        let asked = Probe {
            yard,
            base: self.run_asked(),
        };
        match &self.run_up {
            ViewUp::Answered(now, up) if now == &asked => Some(*up),
            _ => None,
        }
    }

    /// The run base as probed: the setting and the yard's view address,
    /// since the effective base is derived from the view's where none is
    /// set. An address holds no whitespace, so the join tells settings
    /// apart; the probe's yard tells the yards apart.
    pub(crate) fn run_asked(&self) -> String {
        format!(
            "{}\n{}",
            self.run_set(),
            self.view_base().unwrap_or_default()
        )
    }

    /// The sandbox origin's base address of the yard the window shows:
    /// the run base setting where one is set, else the yard's view,
    /// where it has one, on the sandbox port. None where neither reads
    /// as an address.
    pub(crate) fn run_address(&self) -> Option<api::Address> {
        run_address_of(&self.run_set(), &self.view_base().unwrap_or_default())
    }

    /// What `from` was open. When another is now, what is typed stays
    /// with the conversation or the ask it was typed in, and the line has
    /// what was typed in the one that is open, if one is.
    pub(crate) fn turned(&mut self, from: Option<Selected>, cx: &mut Context<Self>) {
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
    pub(crate) fn keep_typed(&mut self, from: &Selected, cx: &mut Context<Self>) {
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
            // Nothing is typed on the yard row, the grants, the board,
            // a line, No line or the bead.
            Selected::Yard
            | Selected::Grants
            | Selected::Arrivals
            | Selected::Line(_)
            | Selected::NoLine
            | Selected::Bead(_) => {}
        }
    }

    /// What was typed in the row `to`, for the line: the row is open.
    pub(crate) fn typed_for(&mut self, to: Option<&Selected>) -> String {
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

    /// The card's toggle is pressed: what is typed is drawn as typed,
    /// for a look, or as marks again. The value is unchanged: it goes
    /// in its call as ever.
    pub(crate) fn toggle_secret(&mut self, cx: &mut Context<Self>) {
        let shown = !self.secret_shown;
        self.set_secret_shown(shown, cx);
    }

    /// Makes a call off the window's thread, as the user the window is for,
    /// and hands its answer back to the window. An answer that comes when
    /// another yard was opened since is dropped: it says nothing of what
    /// the window shows now.
    pub(crate) fn ask<R: Send + 'static>(
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
    pub(crate) fn ask_through<R: Send + 'static>(
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
    pub(crate) fn ask_in<R: Send + 'static>(
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

    /// The stamp of a read made now of the yard of the following
    /// `session`.
    pub(crate) fn stamp_in(&mut self, session: u64) -> u64 {
        self.model_in(session).map_or(0, |m| m.reads().make())
    }

    /// Changes the model of the following `session` as `f` does with the
    /// answer of the read of `read` stamped `stamp`, where that answer is
    /// newer than what the model holds of it: whether it was. An older one
    /// that answered later says nothing the model does not have.
    pub(crate) fn fresh_in(
        &mut self,
        session: u64,
        read: Read,
        stamp: u64,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Model) -> Vec<Effect>,
    ) -> bool {
        let fresh = self
            .model_in(session)
            .is_some_and(|m| m.reads().fresh(read, stamp));
        if fresh {
            self.change_in(session, cx, f);
        }
        fresh
    }

    /// The read of `read` stamped `stamp` failed, in the yard of the
    /// following `session`: if it is still current, says so and, where
    /// `retryable`, makes it again after [`session::RETRY`] while the yard
    /// is up. A yard that is down is its follower's: what it reads when it
    /// is up again has all of it.
    pub(crate) fn read_failed_in(
        &mut self,
        session: u64,
        read: Read,
        stamp: u64,
        retryable: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(model) = self.model_in(session) else {
            return false;
        };
        let accepted = model.reads().accepts_failure(read, stamp);
        let retry = retryable && model.reads().failed(read, stamp);
        if !accepted {
            return false;
        }
        if !retry {
            return true;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(session::RETRY).await;
            this.update(cx, |this, cx| {
                let up = this.up_in(session);
                let again = this
                    .model_in(session)
                    .is_some_and(|m| m.reads().again(read));
                if again && up {
                    this.run_in(session, vec![read.effect()], cx);
                }
            })
            .ok();
        })
        .detach();
        true
    }

    /// Whether the yard of the following `session` answers, as its
    /// follower last said.
    pub(crate) fn up_in(&self, session: u64) -> bool {
        if session == self.session {
            return self.down.is_none();
        }
        self.held
            .values()
            .find(|h| h.session == session)
            .is_some_and(|h| h.down.is_none())
    }

    /// Says `notice` in the yard of the following `session`: at once in
    /// the open one, and in one that is held when the window shows it.
    pub(crate) fn notice_in(&mut self, session: u64, notice: String, cx: &mut Context<Self>) {
        if session == self.session {
            return self.notice(notice, cx);
        }
        if let Some(model) = self.model_in(session) {
            model.notice = notice;
        }
    }

    pub(crate) fn notice(&mut self, notice: String, cx: &mut Context<Self>) {
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
    pub(crate) fn run(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        self.run_in(self.session, effects, cx);
    }

    /// Does what a change of the model of the following `session` asks
    /// for: each call by that yard's client, and its answer to that yard's
    /// model, wherever the yard is by then.
    pub(crate) fn run_in(&mut self, session: u64, effects: Vec<Effect>, cx: &mut Context<Self>) {
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
                Effect::Crew => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, _| Ok::<_, api::Error>((client.head()?, client.crew()?)),
                        move |this, crew, cx| match crew {
                            Ok((head, crew)) => {
                                this.fresh_in(session, Read::Crew, stamp, cx, |m| {
                                    m.listed(head, crew)
                                });
                            }
                            Err(err) => {
                                if this.read_failed_in(session, Read::Crew, stamp, true, cx) {
                                    this.notice_in(
                                        session,
                                        format!("the crew could not be read: {err}"),
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                }
                Effect::Peers => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, _| client.peers(),
                        move |this, peers, cx| match peers {
                            Ok(peers) => {
                                this.fresh_in(session, Read::Peers, stamp, cx, |m| {
                                    m.linked(&peers)
                                });
                            }
                            Err(err) => {
                                if this.read_failed_in(session, Read::Peers, stamp, true, cx) {
                                    this.notice_in(
                                        session,
                                        format!("the linked yards could not be read: {err}"),
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                }
                Effect::Users => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, _| client.users(),
                        move |this, users, cx| match users {
                            Ok(users) => {
                                this.fresh_in(session, Read::Users, stamp, cx, |m| m.users(users));
                            }
                            Err(err) => {
                                if this.read_failed_in(session, Read::Users, stamp, true, cx) {
                                    this.notice_in(
                                        session,
                                        format!("the users could not be read: {err}"),
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                }
                Effect::Notifications => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, me| client.user_notifications(me, 0),
                        move |this, notifications, cx| match notifications {
                            Ok(notifications) => {
                                this.fresh_in(session, Read::Notifications, stamp, cx, |m| {
                                    m.notifications(notifications.notifications)
                                });
                            }
                            Err(err) => {
                                if this.read_failed_in(
                                    session,
                                    Read::Notifications,
                                    stamp,
                                    true,
                                    cx,
                                ) {
                                    this.notice_in(
                                        session,
                                        format!("the notices could not be read: {err}"),
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                }
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
                // A yard too old for the Lines has no call to make again.
                Effect::Arrivals => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, _| client.arrivals(),
                        move |this, arrivals, cx| match arrivals {
                            Ok(board) => {
                                this.fresh_in(session, Read::Arrivals, stamp, cx, |m| {
                                    m.arrivals(board);
                                    Vec::new()
                                });
                            }
                            Err(err) => {
                                if this.read_failed_in(
                                    session,
                                    Read::Arrivals,
                                    stamp,
                                    !model::lines_too_old(&err),
                                    cx,
                                ) {
                                    this.change_in(session, cx, |m| {
                                        m.arrivals_failed(&err);
                                        Vec::new()
                                    });
                                }
                            }
                        },
                    )
                }
                Effect::Flows => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, _| client.flows(),
                        move |this, flows, cx| match flows {
                            Ok(board) => {
                                this.fresh_in(session, Read::Flows, stamp, cx, |m| m.flows(board));
                            }
                            Err(err) => {
                                if this.read_failed_in(
                                    session,
                                    Read::Flows,
                                    stamp,
                                    !model::lines_too_old(&err),
                                    cx,
                                ) {
                                    this.change_in(session, cx, |m| m.flows_failed(&err));
                                }
                            }
                        },
                    )
                }
                // A yard too old for the grants view has no call to make again.
                Effect::Grants => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, _| client.source_grants(),
                        move |this, grants, cx| match grants {
                            Ok(grants) => {
                                this.fresh_in(session, Read::Grants, stamp, cx, |m| {
                                    m.grants(grants);
                                    Vec::new()
                                });
                            }
                            Err(err) => {
                                if this.read_failed_in(
                                    session,
                                    Read::Grants,
                                    stamp,
                                    !model::grants_too_old(&err),
                                    cx,
                                ) {
                                    this.change_in(session, cx, |m| {
                                        m.grants_failed(&err);
                                        Vec::new()
                                    });
                                }
                            }
                        },
                    )
                }
                Effect::Timetable { from, until } => self.ask_in(
                    session,
                    cx,
                    move |client, me| client.timetable_news(me, from, until),
                    move |this, news, cx| match news {
                        Ok(news) => this.change_in(session, cx, |m| {
                            m.timetable_seeded(news, until);
                            Vec::new()
                        }),
                        Err(err) => this.notice_in(
                            session,
                            format!("the timetable's changes could not be read: {err}"),
                            cx,
                        ),
                    },
                ),
                Effect::Asks => {
                    let stamp = self.stamp_in(session);
                    self.ask_in(
                        session,
                        cx,
                        |client, me| client.user_asks(me),
                        move |this, asks, cx| match asks {
                            Ok(asks) => {
                                if this.fresh_in(session, Read::Asks, stamp, cx, |m| m.asks(asks)) {
                                    this.schedule_return_in(session, cx);
                                }
                            }
                            Err(err) => {
                                if this.read_failed_in(session, Read::Asks, stamp, true, cx) {
                                    this.notice_in(
                                        session,
                                        format!("the decisions could not be read: {err}"),
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                }
                Effect::Bead { bead } => self.ask_in(
                    session,
                    cx,
                    {
                        let bead = bead.clone();
                        move |client, _| client.bead(&bead)
                    },
                    move |this, answered, cx| match answered {
                        Ok(bead) => {
                            this.change_in(session, cx, |m| {
                                m.bead_loaded(bead);
                                Vec::new()
                            });
                            // The plain-select shot poses a selection over
                            // the open card's first context line, as a drag
                            // over it would: a debug build's alone.
                            #[cfg(debug_assertions)]
                            if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("plain-select")
                                && let Some(model) = this.model.as_ref()
                            {
                                pose_shot_selection(&this.selection, model, cx);
                            }
                        }
                        Err(err) => this.change_in(session, cx, |m| {
                            m.bead_failed(&bead, &err.to_string());
                            Vec::new()
                        }),
                    },
                ),
                // The ids go one after another on one thread, a connection
                // at a time: a long mail names many. The first the yard
                // does not answer ends the batch, since a yard that fails
                // one fails the rest, each after its own wait.
                Effect::Beads { beads } => {
                    let asked = beads.clone();
                    self.ask_in(
                        session,
                        cx,
                        move |client, _| {
                            let mut answers = Vec::new();
                            for bead in beads {
                                let answer = client.bead(&bead).map(|found| found.id);
                                let answered =
                                    matches!(answer, Ok(_) | Err(api::Error::Refused { .. }));
                                answers.push((bead, answer));
                                if !answered {
                                    break;
                                }
                            }
                            answers
                        },
                        move |this, answers, cx| {
                            this.change_in(session, cx, |m| {
                                m.beads_looked(&asked, answers);
                                Vec::new()
                            })
                        },
                    )
                }
                // As the ids above: the yard row may show many deliveries,
                // so they go one connection at a time, and the first call
                // that fails ends the batch.
                Effect::DeliveryFiles { seqs } => {
                    let asked = seqs.clone();
                    self.ask_in(
                        session,
                        cx,
                        move |client, _| {
                            let mut answers = Vec::new();
                            for seq in seqs {
                                let answer = client.delivery_files(seq);
                                let failed = answer.is_err();
                                answers.push((seq, answer));
                                if failed {
                                    break;
                                }
                            }
                            answers
                        },
                        move |this, answers, cx| {
                            this.change_in(session, cx, |m| {
                                m.delivery_files(&asked, answers);
                                Vec::new()
                            })
                        },
                    )
                }
            }
        }
    }

    /// Whom the open conversation is with, and where what is typed goes.
    pub(crate) fn open(&self) -> Option<(String, Target)> {
        let model = self.model.as_ref()?;
        Some((model.selected_conversation()?.to_string(), model.target()?))
    }

    /// Nothing of a draft is lost to a failed send: what did not go is back
    /// where it was, in the conversation `to` of `yard`, before what was
    /// typed and attached since, the attachment that kept it is marked,
    /// and the window says why `at`, whom it was written to, did not get
    /// it. It is written to nobody else instead, and where another yard is
    /// open by now it is in no line of that one: it waits in its own.
    pub(crate) fn unsent(
        &mut self,
        yard: &str,
        to: &str,
        at: &str,
        unsent: Unsent,
        cx: &mut Context<Self>,
    ) {
        let open = self.drafts.turn_to(yard);
        let here = open == yard;
        self.drafts.restore(to, unsent.files);
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

    pub(crate) fn select(&mut self, sel: Selected, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Opens what an ask or bead id in mail names: an open ask's card in
    /// Decide, with the ask current, a dealt-with ask where it stands,
    /// or the bead whole. The selection leads and the mode follows, as
    /// anywhere.
    pub(crate) fn open_reference(
        &mut self,
        target: select::RefTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            select::RefTarget::Ask {
                bead,
                ask,
                open: true,
            } => {
                self.select(Selected::Decision(bead.clone()), window, cx);
                self.select_ask(bead, ask, cx);
            }
            select::RefTarget::Ask {
                bead,
                ask,
                open: false,
            } => {
                // A folded "Decided" has no row to select: unfold it, as
                // opening the row from the list does.
                if self.model.as_ref().is_some_and(|m| !m.decided_expanded()) {
                    self.toggle_decided(cx);
                }
                self.select(Selected::Decided(bead, ask), window, cx);
            }
            select::RefTarget::Bead { bead } => {
                self.open_bead(bead, window, cx);
            }
        }
    }

    /// Copies what is selected of the messages and the ask cards. The
    /// line copies its own selection while the focus is in it.
    pub(crate) fn copy(&mut self, _: &input::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selection.read(cx).text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
    }

    pub(crate) fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        let mode = self.mode();
        self.change(cx, |m| m.step_in(mode, by));
        let focus = match self.model.as_ref().and_then(|m| m.selected_conversation()) {
            Some(_) => self.line.focus_handle(cx),
            None => self.focus.clone(),
        };
        window.focus(&focus);
    }

    /// Steps the rail to the mode `by` on from the open one, around the
    /// ends: the brackets, from anywhere.
    pub(crate) fn step_mode(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode().step(by);
        self.set_mode(mode, window, cx);
    }

    /// Opens `mode`: the row it was left on, where that is still there,
    /// else its own default. The open row and the list's scroll stay the
    /// old mode's. Without a model there is no list to show, and its icons
    /// are inert.
    pub(crate) fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.mode();
        if from == mode || self.model.is_none() {
            return;
        }
        if let Some(open) = self.model.as_ref().and_then(|m| m.selected().cloned()) {
            let mut kept = self.places.get(&self.yard).cloned().unwrap_or_default();
            kept.rows[from.index()] = Some(open);
            kept.lists[from.index()] = (px(0.) - self.rows.offset().y).max(px(0.));
            self.places.insert(self.yard.clone(), kept);
        }
        let yard = self.yard.clone();
        let mut settings = self.settings.clone().with_mode(&yard, mode);
        // Timetable opened or left: what was seen is kept, per yard.
        // Opening clears the dot.
        if (from == Mode::Timetable) != (mode == Mode::Timetable)
            && let Some(model) = self.model.as_mut()
        {
            let looked = model.timetable_visited(mode == Mode::Timetable);
            settings = settings.with_timetable_seen(&yard, looked);
        }
        self.set(settings, cx);
        let row = self.places.get(&yard).and_then(|kept| kept.row(mode));
        self.settings_open = false;
        self.change(cx, |m| m.show(mode, row));
        let mut at = self.rows.offset();
        at.y = px(0.)
            - self
                .places
                .get(&yard)
                .map(|kept| kept.list(mode))
                .unwrap_or(px(0.));
        self.rows.set_offset(at);
        let focus = match self.model.as_ref().and_then(|m| m.selected_conversation()) {
            Some(_) => self.line.focus_handle(cx),
            None => self.focus.clone(),
        };
        window.focus(&focus);
    }

    /// One key marks one notice read, on the open yard row; on a card `x`
    /// puts the ask aside.
    pub(crate) fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.model.as_ref().and_then(|m| m.selected()) == Some(&Selected::Yard) {
            self.change(cx, |m| m.dismiss());
            return;
        }
        self.later_now(cx);
    }

    /// Marks every notice read, on the open yard row: shift-x and the
    /// button in the row's head. Elsewhere it does nothing.
    pub(crate) fn dismiss_all(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| m.dismiss_all());
    }

    /// Esc leaves what it can: the cheatsheet, the About card, the image
    /// over the window, a decline asked for words, the words for the
    /// card's keys, the pick, then the words.
    pub(crate) fn leave(&mut self, _: &Leave, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_keys {
            self.show_keys = false;
            cx.notify();
            return;
        }
        if self.about.is_some() {
            return self.shut_about(cx);
        }
        if self.image_open.is_some() {
            return self.shut_image(window, cx);
        }
        if self.switch_question.is_some() {
            return self.stop_left(cx);
        }
        if self.switching.is_some() {
            return self.toggle_switcher(window, cx);
        }
        if self.yard_menu.is_some() {
            self.shut_menu();
            cx.notify();
            return;
        }
        if self.settings_open {
            return self.toggle_settings(window, cx);
        }
        if let Some(first_run) = self.first_run.clone() {
            return first_run.update(cx, |first_run, cx| first_run.leave(cx));
        }
        // The fields of a way to a yard, where there is none. Where
        // the yard answers under them, it opens, and the line has the
        // keys, as when a yard is switched to.
        if self.down.is_some() && self.adding.is_some() {
            self.yard_said = None;
            self.adding = None;
            let answers = self.answers_under_adding();
            self.answered_while_adding = false;
            if answers {
                self.down = None;
                self.refresh_switch(cx);
                window.focus(&self.line.focus_handle(cx));
            } else {
                window.focus(&self.focus.clone());
            }
            cx.notify();
            return;
        }
        if self.declining.take().is_some() {
            self.shown(cx);
            window.focus(&self.focus.clone());
            cx.notify();
            return;
        }
        // Esc in the history's filter leaves it, and what is typed stays.
        if self.filter.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus.clone());
            return;
        }
        // Esc shuts the bead drawer: the focus is in it or the reading
        // pane from here on, past the overlays, the away fields, the
        // decline and the filter above, and ahead of the card's own
        // leavings below.
        if self.drawer.take().is_some() {
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
        // Esc in a weighty card's number field leaves it, and what is
        // typed stays: the number is no secret.
        if self.typeback.focus_handle(cx).is_focused(window) {
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

    /// Shows the settings where the conversation is, or what was open
    /// again. While they show, the window holds the keys: nothing typed
    /// goes to a line that is not in view.
    pub(crate) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shut_menu();
        self.settings_open = !self.settings_open;
        // The sheet takes the whole window: no letter is drawn under it, and
        // a pointer that leaves one is told of no leave. The mark goes with
        // the sheet, or the bar would wait over that letter when the
        // conversation shows again. Its emoji's mark goes with it, as where
        // the open row changes. The bead drawer shuts with them: the sheet
        // stands in place of the reading pane it stands beside.
        self.react_hover = None;
        self.emoji_hover = None;
        self.drawer = None;
        // A card under the settings is not in view, and neither is its
        // field: nothing typed of a secret waits there.
        self.forget_secret(cx);
        self.shown(cx);
        // "Agents and you" reads once the settings show.
        self.read_agents(cx);
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
    pub(crate) fn set(&mut self, settings: Settings, cx: &mut Context<Self>) {
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
        // The yards moved where another is to open, or one is listed no
        // longer as it was reached: a change of a web view or a run base
        // alone moves neither the switch nor what the agents' section
        // holds.
        let yards_moved = settings.open != self.settings.open
            || settings.yards.len() != self.settings.yards.len()
            || settings
                .yards
                .iter()
                .zip(self.settings.yards.iter())
                .any(|(set, was)| !set.same_yard(was));
        let shown = self.yard.clone();
        let view_of = |set: &Settings| {
            set.yard(&shown)
                .and_then(|yard| yard.view(&self.env.socket))
                .map(str::to_string)
        };
        let view_moved = view_of(&settings) != view_of(&self.settings);
        // The run base moves with its setting and with the view's, which
        // it is derived from where none is set.
        let run_set_of = |set: &Settings| set.yard(&shown).map(|yard| yard.run_base.clone());
        let run_moved = run_set_of(&settings) != run_set_of(&self.settings) || view_moved;
        self.settings = settings;
        if switched_on {
            self.ask_permission(cx);
        }
        if yards_moved {
            // The open yard may be another, or another home's: what the
            // switch said is gone, and read anew.
            self.switch_reset();
            self.agents_reset(cx);
            self.refresh_switch(cx);
        }
        if view_moved {
            // The address the field holds is the yard's again, its hint
            // the address in effect, and a note's pages go by whether
            // the view answers at it.
            let base = self
                .settings
                .yard(&shown)
                .map(|yard| yard.web_view.clone())
                .unwrap_or_default();
            self.view.update(cx, |line, cx| line.set_text(&base, cx));
            let effect = self
                .settings
                .yard(&shown)
                .and_then(|yard| yard.view(&self.env.socket))
                .unwrap_or("")
                .to_string();
            self.view
                .update(cx, |line, cx| line.set_placeholder(&effect, cx));
            self.view_probe(cx);
        }
        if run_moved {
            let base = self.run_set();
            let usual = self.run_usual();
            self.run.update(cx, |line, cx| {
                line.set_text(&base, cx);
                line.set_placeholder(&usual, cx);
            });
            self.run_probe(cx);
        }
        cx.notify();
    }

    /// Whether closing the window hides footplate where it would else
    /// quit, on this system and by the settings.
    pub(crate) fn closing_hides(&self) -> bool {
        self.settings.closing_hides(cfg!(target_os = "macos"))
    }

    /// footplate has its item in the menu bar, or no longer.
    pub(crate) fn menu_bar_on(&mut self, on: bool, cx: &mut Context<Self>) {
        self.set(
            Settings {
                menu_bar: on,
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// The system tells of what arrives, or no longer.
    pub(crate) fn notify_on(&mut self, on: bool, cx: &mut Context<Self>) {
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
    pub(crate) fn notify_of(&mut self, kind: Kind, cx: &mut Context<Self>) {
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
    pub(crate) fn resize(&mut self, by: i8, cx: &mut Context<Self>) {
        self.set(self.settings.clone().sized(by), cx);
    }

    pub(crate) fn usual_size(&mut self, cx: &mut Context<Self>) {
        self.set(
            Settings {
                text: settings::TEXT,
                ..self.settings.clone()
            },
            cx,
        );
    }

    pub(crate) fn look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.set(
            Settings {
                look,
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// The item's mark in the menu bar or the tray is the choice's:
    /// white, black, or as the system draws it. The item draws it
    /// without a restart.
    pub(crate) fn tray_mark(&mut self, mark: Look, cx: &mut Context<Self>) {
        self.set(
            Settings {
                tray: mark,
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// Keeps the web view's base address that is typed, on the yard
    /// the window shows: its pages open in the browser through it. What
    /// is no address is said, and nothing changed.
    pub(crate) fn submit_view(&mut self, cx: &mut Context<Self>) {
        let address = self.view.read(cx).text().to_string();
        cx.notify();
        match self.settings.clone().with_yard_view(&self.yard, &address) {
            Ok(settings) => {
                self.view_said = None;
                self.set(settings, cx);
            }
            Err(why) => self.view_said = Some(why),
        }
    }

    /// The shown yard's web view is unset again: its usual.
    pub(crate) fn usual_view(&mut self, cx: &mut Context<Self>) {
        self.view_said = None;
        self.view.update(cx, |line, cx| line.set_text("", cx));
        let yard = self.yard.clone();
        if let Ok(settings) = self.settings.clone().with_yard_view(&yard, "") {
            self.set(settings, cx);
        }
    }

    /// Keeps the run base that is typed, on the yard the window shows:
    /// its pages run in the browser through it. What is no address is
    /// said, and nothing changed; empty is the yard's view, where it has
    /// one, on the sandbox port again.
    pub(crate) fn submit_run(&mut self, cx: &mut Context<Self>) {
        let address = self.run.read(cx).text().to_string();
        cx.notify();
        match self.settings.clone().with_yard_run(&self.yard, &address) {
            Ok(settings) => {
                self.run_said = None;
                self.set(settings, cx);
            }
            Err(why) => self.run_said = Some(why),
        }
    }

    /// The shown yard's run base is empty again: its view, where it has
    /// one, on the sandbox port.
    pub(crate) fn usual_run(&mut self, cx: &mut Context<Self>) {
        self.run_said = None;
        self.run.update(cx, |line, cx| line.set_text("", cx));
        let yard = self.yard.clone();
        if let Ok(settings) = self.settings.clone().with_yard_run(&yard, "") {
            self.set(settings, cx);
        }
        cx.notify();
    }

    /// Shows the cheatsheet of the open view's keys, or shuts it again.
    /// The window keeps the focus it had: the keys still answer under
    /// the sheet, and esc shuts it first.
    pub(crate) fn toggle_keys(&mut self, cx: &mut Context<Self>) {
        self.show_keys = !self.show_keys;
        cx.notify();
    }

    /// Shows the About card: the version of this footplate, and the one the
    /// yardr it keeps gives to `--version`, which is asked now, off the
    /// window's thread; the card stands meanwhile, saying so. One that
    /// stands already is left as it is. The window keeps the focus it had,
    /// as under the cheatsheet, and esc shuts it.
    pub(crate) fn show_about(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.about.is_some() {
            return;
        }
        self.about = Some(About { kept: None });
        cx.notify();
        let own = self.means.own.clone();
        cx.spawn(async move |this, cx| {
            let kept = cx
                .background_executor()
                .spawn(async move { fetch::kept(own.as_deref()) })
                .await;
            this.update(cx, |this, cx| {
                // A card that was shut meanwhile has nothing to be told.
                if let Some(about) = this.about.as_mut() {
                    about.kept = Some(kept);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shuts the About card.
    pub(crate) fn shut_about(&mut self, cx: &mut Context<Self>) {
        self.about = None;
        cx.notify();
    }

    /// The open view, as the cheatsheet names its keys: the switcher
    /// and the settings over whatever they stand on, the away screen
    /// and the first run as one screen, else the open row, else the
    /// mode.
    pub(crate) fn keys_view(&self) -> KeysView {
        if self.switching.is_some() {
            return KeysView::Switcher;
        }
        if self.settings_open {
            return KeysView::Settings;
        }
        if self.image_open.is_some() || self.first_run.is_some() || self.switch_question.is_some() {
            return KeysView::Away;
        }
        if self.down.is_some() && !self.reconnecting() {
            return KeysView::Away;
        }
        match self.model.as_ref().and_then(|m| m.selected().cloned()) {
            Some(Selected::Conversation(_)) => KeysView::Conversation,
            Some(Selected::Decision(_) | Selected::Decided(_, _)) => KeysView::Card,
            Some(Selected::Grants) => KeysView::Grants,
            Some(Selected::Yard) => KeysView::Yard,
            Some(_) => KeysView::Timetable,
            None => match self.mode() {
                Mode::Decisions => KeysView::Card,
                Mode::Timetable => KeysView::Timetable,
                Mode::Chat => KeysView::Conversation,
            },
        }
    }

    /// From the switcher's list, by its key or its footer's button: the
    /// list shuts, and the settings stand at their yards, where a further
    /// yard is added.
    pub(crate) fn add_yard_from_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switching = None;
        if !self.settings_open {
            self.toggle_settings(window, cx);
        }
        // The yards section, whatever stood open.
        self.pick_section(Section::Yards, cx);
        cx.notify();
    }

    /// Whether a yard can be set up from here: the yard that is away is
    /// this run's own, at its socket, and nothing answers there. Another
    /// home's yard, and one at an address, are not footplate's to make.
    /// Nor is one that is only stopped: its home holds a yard already.
    pub(crate) fn sets_up_here(&self) -> bool {
        let here = self.entry.as_ref().is_some_and(|yard| {
            !yard.is_far()
                && (yard.socket.is_empty() || std::path::Path::new(&yard.socket) == self.env.socket)
        });
        here && self.env.yard.is_none()
            && self.away == Away::Trying
            && !self.yard_is_stopped()
            && !self.answers_under_adding()
    }

    /// "Set up a yard here": the first run begins, where the window said
    /// there is no yard. The yard goes on being asked for meanwhile, and
    /// is followed as soon as it answers, under the first run.
    pub(crate) fn set_up_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pairing || self.first_run.is_some() || !self.sets_up_here() {
            return;
        }
        // A yard that answered under the fields is left with them: where
        // it answers, it opens under the first run.
        if self.answered_while_adding {
            self.answered_while_adding = false;
            if !self.unreached.contains_key(&self.yard) {
                self.down = None;
                self.refresh_switch(cx);
            }
        }
        self.adding = None;
        self.yard_said = None;
        let (means, socket, name) = (
            self.means.clone(),
            self.env.socket.clone(),
            self.name.clone(),
        );
        let handle = self.handle;
        let first_run = cx.new(|cx| FirstRun::new(means, &socket, &name, handle, cx));
        cx.subscribe(&first_run, |this, _, ended: &Ended, cx| {
            this.first_run = None;
            // The switch stood aside while the first run owned fetching:
            // what it made is read now.
            this.refresh_switch(cx);
            // The line has the keys, as when a yard is switched to.
            let (handle, focus) = (this.handle, this.line.focus_handle(cx));
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.focus(&focus));
            });
            if let Ended::Talking { yardmaster } = ended {
                this.wanted_row = Some(Selected::Conversation(yardmaster.clone()));
                this.seek_wanted(cx);
            }
            cx.notify();
        })
        .detach();
        window.focus(&first_run.focus_handle(cx));
        self.first_run = Some(first_run);
        cx.notify();
    }

    /// Opens the row that is wanted, once the yard has it.
    pub(crate) fn seek_wanted(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.wanted_row.clone() else {
            return;
        };
        if self.first_run.is_some() || self.down.is_some() {
            return;
        }
        if !self.model.as_ref().is_some_and(|m| m.has_row(&row)) {
            return;
        }
        self.wanted_row = None;
        self.settings_open = false;
        self.change(cx, |m| m.select(row));
    }

    /// Whether the yard answers under a way's open fields: it answered
    /// while they stood open, and is not away again since. The screen
    /// says so, and leaving the fields opens it.
    pub(crate) fn answers_under_adding(&self) -> bool {
        self.answered_while_adding && !self.unreached.contains_key(&self.yard)
    }

    /// Start in the status row: `yardr up`. Safe to repeat, so starting
    /// a yard that runs, or twice, changes nothing.
    pub(crate) fn start_yard(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.switch_yardr() else {
            return;
        };
        if self.switch_busy.is_some() {
            return;
        }
        self.switch_busy = Some(SwitchBusy::Starting);
        self.switch_troubles = Vec::new();
        self.switch_note = None;
        cx.notify();
        let opening = self.opening;
        cx.spawn(async move |this, cx| {
            let up = cx
                .background_executor()
                .spawn(async move { yardr.up() })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    return;
                }
                this.switch_busy = None;
                match up {
                    Ok(up) if up.up => {}
                    Ok(up) => {
                        this.switch_troubles = up.troubles();
                        if this.switch_troubles.is_empty() {
                            this.switch_troubles = vec![
                                "The yard did not start, and yardr did not say why.".to_string(),
                            ];
                        }
                    }
                    Err(why) => this.notice(why, cx),
                }
                this.refresh_switch(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stop in the status row: `down` without the flag first, which on a
    /// yard with agents stops nothing and asks what it would stop. With
    /// no agent it stops the yard at once, and asks nothing.
    pub(crate) fn stop_yard(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.switch_yardr() else {
            return;
        };
        if self.switch_busy.is_some() || self.switch_question.is_some() {
            return;
        }
        self.switch_busy = Some(SwitchBusy::Asking);
        cx.notify();
        let opening = self.opening;
        cx.spawn(async move |this, cx| {
            let down = cx
                .background_executor()
                .spawn(async move { yardr.down(false) })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    return;
                }
                this.switch_busy = None;
                match down {
                    Ok(down) if down.confirm => {
                        // The question takes the conversation's place: no
                        // letter is drawn under it, and the pointer that
                        // leaves one is told of no leave. The mark goes
                        // with it, as with the settings, and its emoji's.
                        this.react_hover = None;
                        this.emoji_hover = None;
                        this.switch_question = Some(down);
                    }
                    Ok(down) => {
                        let troubles = down.troubles();
                        if !troubles.is_empty() {
                            this.notice(troubles.join("\n"), cx);
                        }
                    }
                    Err(why) => this.notice(why, cx),
                }
                this.refresh_switch(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Stop them": `down` again, with the flag. The turn each agent is
    /// in is lost; what they have not committed stays in their worktrees.
    pub(crate) fn stop_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.switch_yardr() else {
            return;
        };
        if self.switch_busy.is_some() {
            return;
        }
        self.switch_question = None;
        self.switch_busy = Some(SwitchBusy::Stopping);
        cx.notify();
        let opening = self.opening;
        cx.spawn(async move |this, cx| {
            let down = cx
                .background_executor()
                .spawn(async move { yardr.down(true) })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    return;
                }
                this.switch_busy = None;
                match down {
                    Ok(down) => {
                        let troubles = down.troubles();
                        if !troubles.is_empty() {
                            this.notice(troubles.join("\n"), cx);
                        }
                    }
                    Err(why) => this.notice(why, cx),
                }
                this.refresh_switch(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Leave running": the question is gone, and nothing was stopped.
    pub(crate) fn stop_left(&mut self, cx: &mut Context<Self>) {
        self.switch_question = None;
        cx.notify();
    }

    /// The settings show `section` alone, and keep it the open one. A
    /// yard's menu shuts: it is of the yards section.
    pub(crate) fn pick_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.shut_menu();
        self.set(
            Settings {
                section,
                ..self.settings.clone()
            },
            cx,
        );
    }

    /// A yard is to be added, in the settings or from where there is no
    /// yard: its fields are asked for, with the focus in its name. They
    /// are empty but for the socket of a yard of this machine, where this
    /// run's stands: the path to change for another home's yard.
    pub(crate) fn add_yard(&mut self, adding: Adding, window: &mut Window, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in || self.env.yard.is_some() {
            return;
        }
        self.shut_menu();
        let near = adding == Adding::Near;
        self.adding = Some(adding);
        self.yard_said = None;
        for field in [
            &self.yard_name,
            &self.socket,
            &self.address,
            &self.code,
            &self.email,
            &self.password,
        ] {
            field.update(cx, |line, cx| line.set_text("", cx));
        }
        if near {
            let socket = self.env.socket.to_string_lossy().into_owned();
            self.socket
                .update(cx, |line, cx| line.set_text(&socket, cx));
        }
        window.focus(&self.yard_name.focus_handle(cx));
        cx.notify();
    }

    /// The ⋯ menu of the yard called `name` stands open on its card, or
    /// shuts again: one menu at a time.
    pub(crate) fn toggle_menu(&mut self, name: &str, cx: &mut Context<Self>) {
        match self.yard_menu.as_deref() == Some(name) {
            true => self.yard_menu = None,
            false => self.yard_menu = Some(name.to_string()),
        }
        cx.notify();
    }

    /// No yard's ⋯ menu stands open.
    pub(crate) fn shut_menu(&mut self) {
        self.yard_menu = None;
    }

    /// The yard called `name` is to be renamed: its card's menu shuts,
    /// and its new name is typed in its card, where the menu stood,
    /// with the old one standing.
    pub(crate) fn rename_start(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in || self.env.yard.is_some() {
            return;
        }
        self.shut_menu();
        self.adding = Some(Adding::Rename(name.to_string()));
        self.yard_said = None;
        self.yard_name
            .update(cx, |line, cx| line.set_text(name, cx));
        window.focus(&self.yard_name.focus_handle(cx));
        cx.notify();
    }

    /// Enter in a field of a yard, and the button under them: the yard of
    /// this machine is added, the code goes to the yard at the address, or
    /// the Kratos account is signed in at the edge's origin.
    pub(crate) fn submit_yard(&mut self, cx: &mut Context<Self>) {
        match self.adding {
            Some(Adding::Near) => self.add_near(cx),
            Some(Adding::Far) | Some(Adding::Again(_)) => self.pair(cx),
            Some(Adding::Hosted) | Some(Adding::SignInAgain(_)) => self.sign_in(cx),
            Some(Adding::Rename(_)) => self.rename_yard(cx),
            None => {}
        }
    }

    /// Adds the yard of this machine that is typed, and opens it. What
    /// keeps it out of the list is said, and nothing changed.
    pub(crate) fn add_near(&mut self, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in {
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

    /// Gives the yard the typed name, and keeps it: its reach and its files
    /// stay, what was written for it stays written, and where it was the
    /// open one it still is. A name another yard has is said, and nothing
    /// changed.
    pub(crate) fn rename_yard(&mut self, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in || self.env.yard.is_some() {
            return;
        }
        let Some(Adding::Rename(from)) = self.adding.clone() else {
            return;
        };
        let name = self.yard_name.read(cx).text().trim().to_string();
        cx.notify();
        let Some(mut yard) = self.settings.yard(&from).cloned() else {
            self.yard_said = Some(format!("The yard {from} is no longer one of the list."));
            return;
        };
        yard.name = name.clone();
        match self
            .settings
            .clone()
            .with_yard(Some(&from), yard, self.kept.as_deref())
        {
            Ok(settings) => {
                self.yard_said = None;
                self.shut_adding(cx);
                // What the window kept under the old name is the new
                // name's: the followed yard, which goes on as it is, the
                // drafts, the places, the kept words and files, the
                // unreached mark and the waiting notifications.
                if let Some(mut held) = self.held.remove(&from) {
                    if let Some(entry) = held.entry.as_mut() {
                        entry.name = name.clone();
                    }
                    self.held.insert(name.clone(), held);
                }
                if self.yard == from {
                    self.yard = name.clone();
                    match self.entry.as_mut() {
                        Some(entry) => entry.name = name.clone(),
                        // Opened as this yard from the start, never left:
                        // it goes on as the yard it is listed as now.
                        None => self.entry = settings.yard(&name).cloned(),
                    }
                }
                self.drafts.rename(&from, &name);
                if let Some(place) = self.places.remove(&from) {
                    self.places.insert(name.clone(), place);
                }
                if let Some(words) = self.kept_words.remove(&from) {
                    self.kept_words.insert(name.clone(), words);
                }
                if let Some(files) = self.kept_note_files.remove(&from) {
                    self.kept_note_files.insert(name.clone(), files);
                }
                if let Some(short) = self.unreached.remove(&from) {
                    self.unreached.insert(name.clone(), short);
                }
                self.burst.rename(&from, &name);
                self.set(settings, cx);
                self.reopen(cx);
                // Only the name moved, so the switch reads for the yard
                // again: the read in set() ran while the settings still
                // kept the old name.
                self.refresh_switch(cx);
            }
            Err(why) => self.yard_said = Some(why),
        }
    }

    /// The fields of a yard are no longer asked for. The window holds the
    /// keys again: a field that is not drawn would take them nowhere.
    /// Where they stood in place of a yard that is away, the line has
    /// them, as when a yard is switched to: the yard they added is opened.
    pub(crate) fn shut_adding(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        let focus = match self.settings_open {
            true => self.focus.clone(),
            false => self.line.focus_handle(cx),
        };
        let handle = self.handle;
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
    pub(crate) fn remove_yard(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in || self.env.yard.is_some() {
            return;
        }
        self.shut_menu();
        // Who the device is there is what the yard said when it was read.
        let (model, said) = match self.held.get(name) {
            _ if name == self.yard => (self.model.as_ref(), self.said.as_ref()),
            Some(held) => (held.model.as_ref(), held.said.as_ref()),
            None => (None, None),
        };
        let device = model
            .zip(said)
            .map(|(model, said)| (model.me().to_string(), said.said_by.clone()));
        let (settings, left, session) = self.settings.clone().without(name, self.kept.as_deref());
        let device = device
            .as_ref()
            .map(|(user, device)| (user.as_str(), device.as_str()));
        let mut said = left.said(name, device);
        if let Some(sessions) = session.said_session() {
            said.push(' ');
            said.push_str(&sessions);
        }
        self.yard_said = Some(said);
        if matches!(&self.adding, Some(Adding::Again(at) | Adding::SignInAgain(at) | Adding::Rename(at)) if at == name)
        {
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
            self.kept_note_files.remove(name);
            self.burst.forget(name);
            self.unreached.remove(name);
            self.places.remove(name);
        }
        cx.notify();
    }

    /// The settings' sheet, over whatever stood open: this window, how
    /// it looks, what the system tells of, the yards it shows, how this
    /// yard's agents work with the operator, and the addresses a note's
    /// pages go through, each with its controls, with its own sections
    /// down the left side and the file they are kept in as a footer.
    /// It stands at the rail's edge with no list beside it, so where the
    /// content reaches up into the title bar (`top` pixels) it starts under
    /// an empty band, as the list's head is one: the rail's line runs on
    /// along its foot, and the sheet's own lines down its sides begin under
    /// it, clear of the traffic lights.
    pub(crate) fn talk_settings(
        &self,
        palette: Palette,
        top: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Settings"))
            .child(
                small_button("back (esc)".to_string(), palette)
                    .id("settings back")
                    .debug_selector(|| "settings back".to_string())
                    .flex_none()
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx))),
            );
        // What a setting is called, over its buttons and the keys for it.
        let setting = |name: String, keys: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                .child(div().text_sm().text_color(palette.faint).child(keys))
        };
        let text = self.settings.text;
        let size = setting(
            "text size".to_string(),
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
        let picked = self.settings.look;
        let look = setting(
            "light or dark".to_string(),
            format!("{SECONDARY}+shift+l the next of auto, light, dark · auto is the desktop's"),
        )
        .child(
            div().flex().child(segmented(
                Look::ALL
                    .into_iter()
                    .map(|look| {
                        segment(look.name().to_string(), look == picked, palette)
                            .id(SharedString::from(format!("look {}", look.name())))
                            .debug_selector(move || format!("look {}", look.name()))
                            .on_click(cx.listener(move |this, _, _, cx| this.look(look, cx)))
                    })
                    .collect(),
                palette,
            )),
        );
        let yard = self.talk_yards(setting, palette, cx);
        let view = self.view_setting(setting, palette, cx);
        let run = self.run_setting(setting, palette, cx);
        let agents = self.agents_setting(setting, palette, cx);
        // What the system tells of: nothing, or the kinds that are chosen,
        // and what the system said if it does not let footplate notify.
        let notify = self.settings.notifications;
        let mut notifications = setting(
            "notifications".to_string(),
            "Notify me about things I can't see right now.".to_string(),
        )
        .child(
            div().flex().child(segmented(
                [("off", false), ("on", true)]
                    .map(|(name, on)| {
                        segment(name.to_string(), on == notify.on, palette)
                            .id(SharedString::from(format!("notify {name}")))
                            .debug_selector(move || format!("notify {name}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.notify_on(on, cx)))
                    })
                    .into_iter()
                    .collect(),
                palette,
            )),
        );
        if notify.on {
            notifications = notifications.child(
                div().flex().flex_col().gap_2().children(
                    [
                        ("mail", Kind::Mail),
                        ("decisions", Kind::Decision),
                        ("the yard's notices", Kind::Notice),
                    ]
                    .map(|(name, kind)| {
                        checkbox(name.to_string(), notify.chosen(kind), palette)
                            .id(SharedString::from(format!("notify {name}")))
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
        // The item is in a Mac's menu bar and in a Linux desktop's tray.
        // Closing hides on a Mac alone: elsewhere it quits either way.
        let menu_bar = {
            setting(
                "menu bar or tray item".to_string(),
                match cfg!(target_os = "macos") {
                    true => format!(
                        "while it is on, closing the window hides footplate, which goes on following its yards: its item in the menu bar says what waits and shows it again, as a click on its icon in the dock does, {SECONDARY}+q quits · off, there is no item and closing quits"
                    ),
                    false => "while it is on, footplate has an item in the desktop's tray, where a bar or a panel draws one: its icon says how many things wait, a click on it shows the window, and its menu lists the yards · off, there is no item".to_string(),
                },
            )
            .child(div().flex().child(segmented(
                [("off", false), ("on", true)]
                    .map(|(name, on)| {
                        segment(name.to_string(), on == self.settings.menu_bar, palette)
                            .id(SharedString::from(format!("menu bar {name}")))
                            .debug_selector(move || format!("menu bar {name}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.menu_bar_on(on, cx)))
                    })
                    .into_iter()
                    .collect(),
                palette,
            )))
        };
        // The item's mark, beside its switch: white, black, or as the
        // system draws it.
        let picked = self.settings.tray;
        let mark = setting(
            "tray mark".to_string(),
            "Auto matches the menu bar (Mac) or uses black (Linux tray).".to_string(),
        )
        .child(
            div().flex().child(segmented(
                Look::ALL
                    .into_iter()
                    .map(|mark| {
                        segment(mark.name().to_string(), mark == picked, palette)
                            .id(SharedString::from(format!("tray {}", mark.name())))
                            .debug_selector(move || format!("tray {}", mark.name()))
                            .on_click(cx.listener(move |this, _, _, cx| this.tray_mark(mark, cx)))
                    })
                    .collect(),
                palette,
            )),
        );
        // The About card's second door: the application's menu has the
        // first, where a desktop shows one, and a Linux desktop shows none.
        let about = setting(
            "about footplate".to_string(),
            "which footplate this is, and which yardr it keeps".to_string(),
        )
        .child(
            div().flex().child(
                small_button("show".to_string(), palette)
                    .id("about show")
                    .debug_selector(|| "about show".to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.show_about(window, cx))),
            ),
        );
        // The sheet's own sections down the left side, as chat's rows are:
        // the open one stands out, and a click shows that section alone.
        let open = self.settings.section;
        let mut list = div()
            .id("settings sections")
            .flex_none()
            .w(rems(9.))
            .h_full()
            .overflow_y_scroll()
            .bg(palette.side)
            .border_r_1()
            .border_color(palette.line)
            .py_2();
        for section in Section::ALL {
            let at = format!("settings section {}", section.name());
            list = list.child(
                div()
                    .id(SharedString::from(at.clone()))
                    .debug_selector(move || at.clone())
                    .mx_1()
                    .px_2()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(section == open, |el| el.bg(palette.open))
                    .when(section != open, |el| {
                        el.hover(|style| style.bg(palette.hover))
                    })
                    .child(section.title())
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_section(section, cx))),
            );
        }
        // The file they are kept in, as the footer's line, with what
        // opens its folder: no longer the head's subtitle.
        let saved = match &self.kept {
            Some(file) => format!("Saved to {}", file.display()),
            None => "Not kept: there is no home directory to keep them in.".to_string(),
        };
        let reveal = self
            .kept
            .as_ref()
            .and_then(|file| file.parent())
            .map(dir_url);
        let foot = div()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(palette.line)
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            // Resolve the available footer width before
                            // measuring the path, so truncate() can draw
                            // its ellipsis.
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .debug_selector(|| "settings saved".to_string())
                            .text_sm()
                            .text_color(palette.faint)
                            .child(saved),
                    )
                    .children(reveal.map(|url| {
                        let open = self.means.open.clone();
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().text_sm().text_color(palette.faint).child("·"))
                            .child(
                                div()
                                    .id("settings reveal")
                                    .debug_selector(|| "settings reveal".to_string())
                                    .text_sm()
                                    .text_color(palette.link)
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| (open)(&url, cx))
                                    .child("reveal"),
                            )
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap_1()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(format!("{SECONDARY}+, or esc goes back"))
                    .child("·")
                    .child(keys_hint("keys hint settings", palette, cx)),
            );
        // The sheet stands centred in the window, over whatever stood
        // open: its own sections, not the timetable's rows beside them.
        let sheet = div()
            .debug_selector(|| "settings sheet".to_string())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .justify_center()
            .bg(palette.side)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .max_w(rems(56.))
                    .flex()
                    .flex_col()
                    .bg(palette.ground)
                    .border_l_1()
                    .border_r_1()
                    .border_color(palette.line)
                    .children(self.reconnect_strip(palette))
                    .child(head)
                    .child(
                        div().flex().flex_1().min_h_0().child(list).child(
                            div()
                                .id("settings pane")
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .overflow_y_scroll()
                                .p_4()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .children(match open {
                                    // Windows draws no such item: its controls are
                                    // left out there, as they would do nothing.
                                    Section::ThisWindow if cfg!(windows) => vec![size, about],
                                    Section::ThisWindow => vec![size, menu_bar, mark, about],
                                    Section::Appearance => vec![look],
                                    Section::Notifications => vec![notifications],
                                    Section::Yards => vec![yard],
                                    Section::Agents => vec![agents],
                                    Section::Connection => vec![view, run],
                                }),
                        ),
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
                    .child(foot),
            );
        match top > 0. {
            false => sheet.into_any_element(),
            true => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .bg(palette.side)
                .child(band(top, palette).debug_selector(|| "settings band".to_string()))
                .child(sheet.min_h_0())
                .into_any_element(),
        }
    }

    /// Another name for the yard of this card, typed in it: the form
    /// stands where its ⋯ menu renamed it, with the old name standing,
    /// what came of the last try under the button, and the help under
    /// that.
    pub(crate) fn rename_asked(
        &self,
        name: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let busy = self.pairing || self.signing_in;
        let mut asked = vec![
            div()
                .debug_selector(|| "yard rename for".to_string())
                .child(format!("another name for {name}"))
                .into_any_element(),
            yard_field("name", &self.yard_name, palette, cx).into_any_element(),
            yard_submit("rename (enter)", "yard rename save", busy, palette, cx).into_any_element(),
        ];
        if let Some(said) = &self.yard_said {
            asked.push(
                div()
                    .debug_selector(|| "yard said".to_string())
                    .child(said.clone())
                    .into_any_element(),
            );
        }
        asked.push(
            yard_help(
                "Its reach and its files stay, and what was written for it stays written. No two yards share a name."
                    .to_string(),
                palette,
            )
            .into_any_element(),
        );
        asked
    }

    /// The shown yard's web view in the settings: the field its base
    /// address is typed in, the address in effect under it, what keeps
    /// it, and what a typed one that is none says. A yard reached by
    /// pairing opens its pages once this names the address that yard's
    /// view is reached by. Of the one yard the environment names no
    /// address is kept.
    pub(crate) fn view_setting(
        &self,
        setting: impl Fn(String, String) -> Div,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(entry) = self.settings.yard(&self.yard).cloned() else {
            return setting("web view".to_string(), String::new()).child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("FOOTPLATE_YARD is set, and this window is of the one yard it names: its web view is not kept"),
            );
        };
        let help = match entry.usual_view(&self.env.socket) {
            Some(_) => "Where a note's pages open. Default: the usual address.",
            None => "Where a note's pages open. Empty means no web view.",
        };
        let focus = self.view.focus_handle(cx);
        let overridden = !entry.web_view.trim().is_empty();
        let effect = match entry.view(&self.env.socket) {
            Some(base) => format!("in effect: {base}"),
            None => "in effect: no web view".to_string(),
        };
        let effect = match overridden {
            true => effect,
            false => format!("{effect} · default"),
        };
        let mut section = setting(format!("web view of {}", entry.name), help.to_string())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(rems(4.)).flex_none().child("address"))
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
                            .debug_selector(|| "view address".to_string())
                            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                                window.focus(&focus);
                                window.prevent_default();
                            })
                            .child(self.view.clone()),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "view effect".to_string())
                    .text_sm()
                    .text_color(palette.faint)
                    .child(effect),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        small_button("save (enter)".to_string(), palette)
                            .id("view save")
                            .debug_selector(|| "view save".to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.submit_view(cx))),
                    )
                    .when_some(overridden.then_some(()), |el, _| {
                        el.child(
                            div()
                                .id("view reset")
                                .debug_selector(|| "view reset".to_string())
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("reset")
                                .on_click(cx.listener(|this, _, _, cx| this.usual_view(cx))),
                        )
                    }),
            );
        if let Some(said) = &self.view_said {
            section = section.child(
                div()
                    .debug_selector(|| "view said".to_string())
                    .child(said.clone()),
            );
        }
        section
    }

    /// The shown yard's run base in the settings: the field it is
    /// typed in, the address in effect under it, what keeps it, and what
    /// a typed one that is none says. Of the one yard the environment
    /// names none is kept.
    pub(crate) fn run_setting(
        &self,
        setting: impl Fn(String, String) -> Div,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(entry) = self.settings.yard(&self.yard).cloned() else {
            return setting("run base".to_string(), String::new()).child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("FOOTPLATE_YARD is set, and this window is of the one yard it names: its run base is not kept"),
            );
        };
        let focus = self.run.focus_handle(cx);
        let overridden = !entry.run_base.trim().is_empty();
        let usual = self.run_usual();
        let base = match overridden {
            true => entry.run_base.trim().to_string(),
            false => usual,
        };
        let effect = match base.is_empty() {
            true => "in effect: no run base".to_string(),
            false => format!("in effect: {base}"),
        };
        let effect = match overridden {
            true => effect,
            false => format!("{effect} · default"),
        };
        let mut section = setting(
            format!("run base of {}", entry.name),
            "Where a note's live pages run. Default: the web view's host.".to_string(),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(rems(4.)).flex_none().child("address"))
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
                        .debug_selector(|| "run address".to_string())
                        .on_mouse_down(MouseButton::Left, move |_, window, _| {
                            window.focus(&focus);
                            window.prevent_default();
                        })
                        .child(self.run.clone()),
                ),
        )
        .child(
            div()
                .debug_selector(|| "run effect".to_string())
                .text_sm()
                .text_color(palette.faint)
                .child(effect),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    small_button("save (enter)".to_string(), palette)
                        .id("run save")
                        .debug_selector(|| "run save".to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.submit_run(cx))),
                )
                .when_some(overridden.then_some(()), |el, _| {
                    el.child(
                        div()
                            .id("run reset")
                            .debug_selector(|| "run reset".to_string())
                            .text_sm()
                            .text_color(palette.link)
                            .cursor_pointer()
                            .child("reset")
                            .on_click(cx.listener(|this, _, _, cx| this.usual_run(cx))),
                    )
                }),
        );
        if let Some(said) = &self.run_said {
            section = section.child(
                div()
                    .debug_selector(|| "run said".to_string())
                    .child(said.clone()),
            );
        }
        section
    }

    /// The question before stopping: the agents in the middle of
    /// something by name, the idle ones as a count, and that their work
    /// stays in worktrees. Nothing is stopped until "Stop them".
    pub(crate) fn stop_question(
        &self,
        down: &setup::Down,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let named = down.named();
        let idle = match (named.is_empty(), down.idle) {
            (true, 0) => "No agent runs.".to_string(),
            (true, 1) => "1 idle agent.".to_string(),
            (true, idle) => format!("{idle} idle agents."),
            (false, 0) => String::new(),
            (false, 1) => "1 more is idle.".to_string(),
            (false, idle) => format!("{idle} more are idle."),
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
            .child(
                div()
                    .max_w(rems(32.5))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| "stop question".to_string())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Stop the yard?"),
                    )
                    .children((!named.is_empty()).then(|| {
                        div()
                            .debug_selector(|| "stop working".to_string())
                            .child("These agents are in the middle of something:")
                    }))
                    .children(named.iter().map(|agent| div().ml_4().child(agent.said())))
                    .children((!idle.is_empty()).then(|| {
                        div()
                            .debug_selector(|| "stop idle".to_string())
                            .child(idle)
                    }))
                    .child(
                        div()
                            .debug_selector(|| "stop worktrees".to_string())
                            .child(
                                "What they have not committed stays in their worktrees; the turn they are in is lost.",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                small_button("Stop them".to_string(), palette)
                                    .id("stop them")
                                    .debug_selector(|| "stop them".to_string())
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_confirmed(cx))),
                            )
                            .child(
                                small_button("Leave running".to_string(), palette)
                                    .id("leave running")
                                    .debug_selector(|| "leave running".to_string())
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_left(cx))),
                            ),
                    ),
            )
    }

    /// The cheatsheet of the open view's keys, over the window: every
    /// key the view answers to, each with what it does. A click
    /// outside it shuts it, as esc and `?` again do.
    pub(crate) fn keys_layer(
        &self,
        view: KeysView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap_1();
        for (key, does) in keys_for(view, self.drawer.is_some()) {
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .debug_selector({
                        let key = key.clone();
                        move || format!("keys row {key}")
                    })
                    .child(
                        div()
                            .w(rems(13.))
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(key),
                    )
                    .child(div().flex_1().min_w_0().child(does)),
            );
        }
        div()
            .id("keys overlay")
            .debug_selector(|| "keys".to_string())
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .right(px(0.))
            .bottom(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_keys(cx)))
            .child(
                div()
                    .id("keys card")
                    .debug_selector(|| "keys card".to_string())
                    .max_w(rems(38.))
                    .max_h(relative(0.9))
                    .overflow_y_scroll()
                    .rounded_lg()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.side)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("keys · {}", view.name())),
                    )
                    .child(list)
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .child("esc closes"),
                    ),
            )
            .child(
                div()
                    .id("keys close")
                    .debug_selector(|| "keys close".to_string())
                    .absolute()
                    .top(rems(0.75))
                    .right(rems(1.))
                    .text_xl()
                    .text_color(palette.faint)
                    .cursor_pointer()
                    .child("×")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_keys(cx))),
            )
    }

    /// The About card over the window: which footplate this is and which
    /// yardr it keeps, each with its version, beside footplate's icon. A
    /// click outside it shuts it, as esc and its button do. Where the
    /// content reaches up into the title bar (`top` pixels) the overlay
    /// starts under that strip, which stays the window's handle: a drag
    /// there still moves it.
    pub(crate) fn about_layer(
        &self,
        about: &About,
        top: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The yardr's line is the colour of what it says: the program's own
        // words are text, there being none is quiet, and its not saying
        // warns.
        let kept_color = match &about.kept {
            Some(Kept::Says(_)) => palette.text,
            Some(Kept::Silent(_)) => palette.warn,
            Some(Kept::Nothing) | None => palette.faint,
        };
        let row = |name: &'static str, selector: &'static str, words: String, color: Rgba| {
            div()
                .flex()
                .flex_row()
                .gap_3()
                .child(
                    div()
                        .w(rems(5.))
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(
                    div()
                        .debug_selector(move || selector.to_string())
                        .flex_1()
                        .min_w_0()
                        .text_color(color)
                        .child(words),
                )
        };
        div()
            .id("about overlay")
            .debug_selector(|| "about".to_string())
            // A click outside the card shuts it and is no click on what
            // stands under it.
            .block_mouse_except_scroll()
            .absolute()
            .top(px(top))
            .left(px(0.))
            .right(px(0.))
            .bottom(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.shut_about(cx)))
            .child(
                div()
                    .id("about card")
                    .debug_selector(|| "about card".to_string())
                    .w(rems(28.))
                    .max_w(relative(0.9))
                    .rounded_lg()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.side)
                    // The kit's popover shadow, as the bar of reactions
                    // has it: the card floats over the conversation's
                    // lines, which it does not cover.
                    .shadow(vec![gpui::BoxShadow {
                        color: palette.shadow.into(),
                        offset: gpui::point(px(0.), px(6.)),
                        blur_radius: px(20.),
                        spread_radius: px(0.),
                    }])
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(img(self.icon.clone()).size(rems(4.5)).flex_none())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("About footplate"),
                                    )
                                    .child(row(
                                        "footplate",
                                        "about footplate",
                                        footplate_words(),
                                        palette.text,
                                    ))
                                    // Windows fetches no yardr, so "not
                                    // fetched yet" would promise one.
                                    .when(!cfg!(windows), |lines| {
                                        lines.child(row(
                                            "yardr",
                                            "about yardr",
                                            yardr_words(about.kept.as_ref()),
                                            kept_color,
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        div().flex().justify_end().child(
                            small_button("close (esc)".to_string(), palette)
                                .id("about close")
                                .debug_selector(|| "about close".to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.shut_about(cx))),
                        ),
                    ),
            )
    }

    /// Opens the away screen's details, or shuts them again.
    pub(crate) fn toggle_away_details(&mut self, cx: &mut Context<Self>) {
        self.away_details = !self.away_details;
        cx.notify();
    }

    /// The top strip of the window, where the content reaches up into the
    /// title bar, as on a Mac: a press that drags there moves the window,
    /// and a double-click does what the system's setting gives a title bar,
    /// as the bar the content has taken the place of did. The press goes to
    /// what is under it as ever, and only a drag moves the window, so a
    /// control in the strip is still a click. Where the window has no such
    /// title bar, or is in full screen, the root is as it was.
    pub(crate) fn title_strip(&self, root: Div, window: &Window, cx: &mut Context<Self>) -> Div {
        let Some(strip) = self
            .means
            .titlebar
            .clone()
            .filter(|_| !window.is_fullscreen())
        else {
            return root;
        };
        let (pressed, dragged) = (strip.clone(), strip);
        root.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                match this.title_press.down(event.position, event.click_count) {
                    Gesture::Nothing => {}
                    gesture => pressed(gesture, window, cx),
                }
            }),
        )
        .on_mouse_move(
            cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                match this.title_press.moved(event.position, event.dragging()) {
                    Gesture::Nothing => {}
                    gesture => dragged(gesture, window, cx),
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.title_press.up()),
        )
    }

    /// The rail on the window's far left: the open yard's chip on top,
    /// the three mode icons with their labels, and at the foot the
    /// settings' gear with its. Where the content reaches up into the
    /// title bar it starts with the band, `top` pixels of nothing for the
    /// traffic lights to stand over, with its hairline at the foot; the
    /// rail's own line down its right edge starts under the band, where
    /// the last light would otherwise stand on it. The chip is the yard's
    /// first two letters on its colour, so the yard picks and approvals
    /// count in is unmistakable; a click opens the switcher. The open
    /// mode's tile stands in the accent on the open ground, as the open
    /// row does; the others stand quiet and brighten on hover, as quiet
    /// text does. Each mode's tile carries what waits in it as a badge,
    /// and in words in its tooltip: the unread mail, the open asks, the
    /// person's beads added or closed since they last had Lines open.
    /// While the yard is away the modes stand dimmed and inert, and the
    /// chip and the foot stay alive, because leaving is their job.
    pub(crate) fn rail(
        &self,
        palette: Palette,
        top: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mode = self.mode();
        // The modes answer where a yard's rows are there to show.
        let live = self.model.is_some();
        // What waits in the open yard, for the tiles' badges. A
        // reconnecting yard keeps its model for its rows, but it is away
        // and has nothing current to show.
        let away = self.down.is_some() && !self.answers_under_adding();
        let counts = rail_counts(self.model.as_ref(), away);
        let banded = top > 0.;
        let mut rail = div()
            .debug_selector(|| "rail".to_string())
            .w(px(RAIL))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .pb_2()
            .bg(palette.side)
            .when(!banded, |rail| {
                rail.pt_2().border_r_1().border_color(palette.line)
            })
            // The band is in pixels, as the title bar is: it keeps its
            // height whatever the text, and the chip's margin under it
            // goes with the text, as the padding it stands in for did.
            // The right line is its own element, from the band's foot down,
            // since a border would run up through the band.
            .when(banded, |rail| {
                rail.relative()
                    .child(band(top, palette).debug_selector(|| "rail band".to_string()))
                    .child(
                        div()
                            .debug_selector(|| "rail rule".to_string())
                            .absolute()
                            .top(px(top))
                            .bottom_0()
                            .right_0()
                            .w(px(1.))
                            .bg(palette.line),
                    )
            });
        // The open yard's chip: its first two letters on its colour.
        // The one yard the environment names has no switcher, but it
        // still has its chip, with its name alone for its words.
        let letters: String = self.yard.chars().take(2).collect();
        let tip = match self.env.yard.is_some() {
            true => self.yard.clone(),
            false => format!("{} — switch yards ({}+y)", self.yard, SECONDARY),
        };
        rail = rail.child(
            div()
                .id("yard chip")
                .debug_selector(|| "yard chip".to_string())
                .size(px(40.))
                .when(banded, |chip| chip.mt_2())
                .rounded_md()
                .bg(yard_color(&self.yard, palette.dark))
                .text_color(yard_ink(&self.yard, palette.dark))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .when(self.env.yard.is_none(), |el| {
                    el.cursor_pointer().on_click(
                        cx.listener(|this, _, window, cx| this.toggle_switcher(window, cx)),
                    )
                })
                .tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
                .child(letters),
        );
        for (n, m) in Mode::ALL.into_iter().enumerate() {
            let open = live && m == mode;
            // The open tile reads in the text colour on the open
            // ground: the accent is kept for what waits on the person
            // reading, and the open mode is not that.
            let color = match (open, live && self.rail_hover == Some(m), live) {
                (true, _, _) => palette.text,
                (_, _, false) => palette.faint,
                (_, true, _) => palette.text,
                _ => palette.faint,
            };
            let glyph: AnyElement = match m {
                Mode::Chat => lamp_glyph(color).into_any_element(),
                Mode::Decisions => semaphore_glyph(color).into_any_element(),
                Mode::Timetable => track_glyph(color).into_any_element(),
            };
            // The tile's badge: the count's label at its top right,
            // and the count in words in its tooltip.
            let badge = counts.and_then(|counts| counts.rail(m));
            let tip = match live {
                true => format!("{} ({}+[ or {}+])", m.label(), SECONDARY, SECONDARY),
                false => "the yard is away".to_string(),
            };
            let tip = match badge.as_ref() {
                Some((_, words)) => format!("{tip} — {words}"),
                None => tip,
            };
            let id = format!("rail {}", m.name());
            let label = format!("{} label", id);
            rail =
                rail.child(
                    div()
                        .id(SharedString::from(id.clone()))
                        .debug_selector(move || id.clone())
                        .w(px(RAIL_TILE))
                        .rounded_md()
                        .relative()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .py_1()
                        .gap_1()
                        .when(n == 0, |el| el.mt_2())
                        .when(n > 0, |el| el.mt_1())
                        .when(open, |el| el.bg(palette.open))
                        .when(live, |el| {
                            let entity = cx.entity();
                            el.cursor_pointer()
                                .on_hover(move |hovered, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.rail_hover = match hovered {
                                            true => Some(m),
                                            false => None,
                                        };
                                        cx.notify();
                                    });
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_mode(m, window, cx)
                                }))
                        })
                        .tooltip(move |_, cx| cx.new(|_| RailTip(tip.clone())).into())
                        .child(glyph)
                        .child(
                            div()
                                .debug_selector(move || label.clone())
                                .flex_none()
                                .text_size(px(RAIL_LABEL))
                                .text_color(color)
                                .child(m.label()),
                        )
                        // The badge stands at the tile's top right. Mail
                        // and decisions wait on the person reading, so
                        // theirs read in the accent, as the jump button's
                        // count does; changed beads ask nothing of them,
                        // so Lines' badge stays muted, as the yard row's.
                        .children(badge.map(|(label, _)| {
                            let waiting = m != Mode::Timetable;
                            div()
                                .debug_selector(move || format!("rail badge {}", m.name()))
                                .absolute()
                                .top(px(0.))
                                .right(px(0.))
                                .h(px(16.))
                                .min_w(px(16.))
                                .px(px(4.))
                                .rounded_full()
                                .when(waiting, |el| {
                                    el.bg(palette.accent).text_color(palette.on_accent)
                                })
                                .when(!waiting, |el| {
                                    el.bg(Rgba {
                                        a: 0.25,
                                        ..palette.faint
                                    })
                                    .text_color(palette.text)
                                })
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(label)
                        })),
                );
        }
        rail = rail.child(div().flex_1().min_h_0());
        // The foot: the settings' gear with its label. The yards are
        // switched from the chip at the rail's head.
        rail.child(
            div()
                .id("settings")
                .debug_selector(|| "settings".to_string())
                .w(px(RAIL_TILE))
                .rounded_md()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .py_1()
                .gap_1()
                .mt_1()
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .when(self.settings_open, |el| el.bg(palette.open))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx)))
                .tooltip(|_, cx| {
                    cx.new(|_| RailTip(format!("Settings ({SECONDARY}+,)")))
                        .into()
                })
                .child(gear_glyph(palette.faint))
                .child(
                    div()
                        .debug_selector(|| "settings label".to_string())
                        .flex_none()
                        .text_size(px(RAIL_LABEL))
                        .text_color(palette.faint)
                        .child("Settings"),
                ),
        )
    }

    /// The left column: the status row at its head, outside what scrolls
    /// so it stays in place, and `body` beneath it. Where the content
    /// reaches up into the title bar (`top` pixels) the head is the band:
    /// the rail's top continues into it, its row is centred on the traffic
    /// lights and begins clear of the last, and its line runs on from the
    /// rail's. It stands there with no status row too, empty, as a yard at
    /// an address has none, so that the first row never stands under the
    /// lights.
    pub(crate) fn column(
        &self,
        body: impl IntoElement,
        palette: Palette,
        top: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let status = self.switch_rows(palette, cx);
        let banded = top > 0.;
        div()
            // 270 pixels at the usual text size. Its rows are cut to it,
            // so it grows with the text, but never over the conversation:
            // the smallest window still has the larger part for that.
            .w(rems(16.875))
            .max_w(relative(0.48))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.side)
            .border_r_1()
            .border_color(palette.line)
            .children(match (banded, status.is_empty()) {
                (true, _) => Some(
                    band(top, palette)
                        .debug_selector(|| "list band".to_string())
                        .pl(px(HEAD_INSET))
                        .pr_3()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .children(status),
                ),
                (false, false) => Some(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2p5()
                        .border_b_1()
                        .border_color(palette.line)
                        .children(status),
                ),
                (false, true) => None,
            })
            .child(body)
    }

    /// The open mode's list: chat's crew with the yard row, decisions'
    /// expanded rows, or the timetable's arrivals row.
    pub(crate) fn list(
        &self,
        model: &Model,
        palette: Palette,
        top: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.mode() {
            Mode::Chat => self.list_chat(model, palette, top, cx).into_any_element(),
            Mode::Decisions => self
                .list_decisions(model, palette, top, cx)
                .into_any_element(),
            Mode::Timetable => self
                .list_timetable(model, palette, top, cx)
                .into_any_element(),
        }
    }

    /// What the mode's rows scroll in: the window never moves it, but each
    /// mode stands where it stood.
    pub(crate) fn list_body(&self) -> Stateful<Div> {
        div()
            .id("rows")
            .debug_selector(|| "rows".to_string())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.rows)
            .py_1()
    }

    /// Chat's list: this yard's crew, the linked yards' people, this
    /// yard's people, each under its head, then the yard row under its.
    /// A group with no rows stands without its head. The yard's
    /// notifications row lives here because notifications are mail the
    /// yard wrote to the person, and chat is where the person reads mail.
    pub(crate) fn list_chat(
        &self,
        model: &Model,
        palette: Palette,
        top: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let groups = [
            ("This yard's crew", model.crew_rows()),
            ("Linked yards", model.linked_rows()),
            ("People", model.people_rows()),
        ];
        let mut body = self.list_body();
        if groups.iter().all(|(_, rows)| rows.is_empty()) {
            body =
                body.child(
                    div().p_3().text_sm().text_color(palette.faint).child(
                        "This yard has no crew yet, and no linked yard exposes anyone to it.",
                    ),
                )
        } else {
            for (title, rows) in groups {
                if rows.is_empty() {
                    continue;
                }
                body = body.child(group_head(title));
                for row in rows {
                    let sel = Selected::Conversation(row.name.clone());
                    body = body.child(self.row(row, sel, palette, cx));
                }
            }
        }
        body = body.child(group_head("Yard"));
        let yard = model.yard_row();
        body = body.child(self.row(yard, Selected::Yard, palette, cx));
        self.column(body, palette, top, cx)
    }

    /// One bead's route as a strip: one dot per stage of its flow in
    /// travel order, each with the stage's name. The stages behind the
    /// bead read done, the bead's own reads current in the text colour,
    /// and the rest read ahead, a person's turn among them drawing no
    /// colour: the accent is kept for what waits on the person reading
    /// now, and the strip is not that. `what` names where the strip
    /// stands (`bead`), for its selectors.
    pub(crate) fn route_strip(
        &self,
        route: &Route,
        bead: &str,
        palette: Palette,
        what: &str,
    ) -> impl IntoElement + use<> {
        let mut strip = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1()
            .debug_selector({
                let (what, bead) = (what.to_string(), bead.to_string());
                move || format!("{what} strip {bead}")
            });
        for (n, stage) in route.stages.iter().enumerate() {
            if n > 0 {
                strip = strip.child(div().h_px().w_3().flex_none().bg(palette.line));
            }
            let (fill, edge, label) = match n == route.current {
                true => (palette.text, palette.text, palette.text),
                false if n < route.current => (palette.faint, palette.faint, palette.faint),
                false => (rgba(0x00000000), palette.faint, palette.faint),
            };
            strip = strip.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector({
                                let (what, bead, stage) =
                                    (what.to_string(), bead.to_string(), stage.name.clone());
                                move || format!("{what} stop {bead} {stage}")
                            })
                            .size(rems(0.5))
                            .rounded_full()
                            .flex_none()
                            .bg(fill)
                            .border_1()
                            .border_color(edge),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(label)
                            .when(n == route.current, |el| {
                                el.font_weight(FontWeight::SEMIBOLD)
                            })
                            .child(stage.name.clone()),
                    ),
            );
        }
        strip
    }

    /// One conversation's row, or the yard's: the name with its state
    /// as a dot beside it where it has one, the words under it, the
    /// unread at the right. A linked name's peer suffix stands in its
    /// own colour, past a name that still ends in an ellipsis before it.
    /// The yard's notices are FYI, not waiting, so their count is a grey
    /// badge; every other count is quiet words, never a numeral badge.
    /// The open row keeps its fill, and a row under the pointer a
    /// hairline instead: hover drawn apart from selection, so exactly
    /// one row ever reads open.
    pub(crate) fn row(
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
        let is_yard = matches!(sel, Selected::Yard);
        let name = row.name.clone();
        // A linked name's peer suffix, past the name: `@airy` in its own
        // colour, always shown whole, while the name ends in an ellipsis
        // before it.
        let (who, peer) = match name.split_once('@') {
            Some((who, peer)) => (who.to_string(), Some(format!("@{peer}"))),
            None => (name.clone(), None),
        };
        let dot = row.dot.map(|state| {
            div()
                .debug_selector(|| format!("row dot {}", name))
                .size(px(8.))
                .rounded_full()
                .flex_none()
                .bg(crew_dot(state, palette))
        });
        div()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
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
            .when(row.selected, |el| el.bg(palette.open))
            .when(!row.selected, |el| {
                el.hover(|style| style.border_color(palette.line))
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
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .children(dot)
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .debug_selector(|| format!("row name {}", name))
                                    .when(row.unread > 0, |el| el.font_weight(FontWeight::SEMIBOLD))
                                    .child(div().min_w_0().truncate().child(who))
                                    .children(peer.map(|peer| {
                                        div()
                                            .debug_selector(|| format!("row peer {}", name))
                                            .flex_none()
                                            .text_color(palette.peer)
                                            .child(peer)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.faint)
                            .truncate()
                            // Under the name, past the dot and its gap.
                            .when(row.dot.is_some(), |el| el.ml(px(16.)))
                            .child(row.line),
                    ),
            )
            .when(row.unread > 0, |el| match is_yard {
                true => el.child(
                    div()
                        .flex_none()
                        .debug_selector(|| format!("row unread {}", name))
                        .h(px(16.))
                        .min_w(px(16.))
                        .px(px(7.))
                        .rounded_full()
                        .bg(Rgba {
                            a: 0.25,
                            ..palette.faint
                        })
                        .text_color(palette.text)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(row.unread.to_string()),
                ),
                // Unread mail waits on the person reading, so it
                // reads in text rust; the yard's notices are FYI, and
                // keep their muted badge above.
                false => el.child(
                    div()
                        .flex_none()
                        .debug_selector(|| format!("row unread {}", name))
                        .text_sm()
                        .text_color(palette.text_accent)
                        .child(unread_words(row.unread)),
                ),
            })
    }

    pub(crate) fn talk(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match model.selected() {
            Some(Selected::Conversation(with)) => self
                .talk_conversation(model, with, palette, cx)
                .into_any_element(),
            Some(Selected::Yard) => self.talk_yard(model, palette, cx).into_any_element(),
            Some(Selected::Arrivals) => self.talk_arrivals(model, palette, cx).into_any_element(),
            Some(Selected::Line(line)) => {
                self.talk_flow(model, line, palette, cx).into_any_element()
            }
            Some(Selected::NoLine) => self.talk_no_line(model, palette, cx).into_any_element(),
            Some(Selected::Bead(bead)) => self
                .talk_bead(model, bead, &self.decision, palette, cx)
                .into_any_element(),
            Some(Selected::Decision(bead)) => self
                .talk_decision(model, bead, palette, cx)
                .into_any_element(),
            Some(Selected::Decided(bead, ask)) => self
                .talk_decided(model, bead, *ask, palette, cx)
                .into_any_element(),
            Some(Selected::Grants) => self.talk_grants(model, palette, cx).into_any_element(),
            // A mode with no row open: decisions with nothing to decide.
            // Chat always has the yard row, and timetable the arrivals.
            None => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .text_color(palette.faint)
                .child(match self.mode() {
                    Mode::Chat => "Nobody to write to yet.",
                    Mode::Decisions => "Nothing to decide.",
                    Mode::Timetable => "None in transit.",
                })
                .into_any_element(),
        }
    }

    /// The control that jumps to the newest message: an arrow over the
    /// mail, with how many mails arrived below while the view stood
    /// scrolled up, marked in the accent when there are any.
    pub(crate) fn jump_button(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
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

    /// The bead drawer at the window's right: one bead beside what is
    /// open, as [`Footplate::talk_bead`] draws it, with its own head
    /// above — the bead's id, Open for the bead full width, and its × —
    /// and the cheatsheet's hint below. None where no bead is open in
    /// it. The reading pane narrows to make room; nothing overlaps it.
    pub(crate) fn talk_drawer(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let bead = self.drawer.clone()?;
        let full = bead.clone();
        Some(
            grown(DRAWER_WIDTH)
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .bg(palette.side)
                .border_l_1()
                .border_color(palette.line)
                .id("drawer")
                .debug_selector(|| "drawer".to_string())
                .child(
                    div()
                        .flex_none()
                        .p_3()
                        .border_b_1()
                        .border_color(palette.line)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(bead.clone()),
                        )
                        .child(
                            small_button("Open".to_string(), palette)
                                .id("drawer open")
                                .debug_selector(|| "drawer open".to_string())
                                .text_color(palette.text_accent)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select(Selected::Bead(full.clone()), window, cx)
                                })),
                        )
                        .child(
                            div()
                                .id("drawer close")
                                .debug_selector(|| "drawer close".to_string())
                                .text_xl()
                                .text_color(palette.faint)
                                .cursor_pointer()
                                .child("×")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.drawer = None;
                                    cx.notify();
                                })),
                        ),
                )
                // The bead fills what the head and the hint leave: its pane
                // is full height, so it stands in a wrapper that flexes.
                .child(div().flex_1().min_h_0().child(self.talk_bead(
                    model,
                    &bead,
                    &self.drawer_scroll,
                    palette,
                    cx,
                )))
                .child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .text_xs()
                        .child(keys_hint("keys hint drawer", palette, cx)),
                ),
        )
    }
}

/// The settings' section a screenshot state opens on: this window for
/// the `settings` shot, the named one for a `settings-<section>` shot,
/// the yards one for the rename shot too, else none. A debug build's
/// alone.
#[cfg(debug_assertions)]
fn shot_section() -> Option<Section> {
    match std::env::var("FOOTPLATE_SHOT").as_deref() {
        Ok("settings") => Some(Section::ThisWindow),
        Ok("settings-appearance") => Some(Section::Appearance),
        Ok("settings-yards") => Some(Section::Yards),
        Ok("settings-yards-rename") => Some(Section::Yards),
        Ok("settings-agents") => Some(Section::Agents),
        Ok("settings-notifications") => Some(Section::Notifications),
        Ok("settings-connection") => Some(Section::Connection),
        _ => None,
    }
}

/// The row a screenshot state opens on instead of the yard's own: the
/// refused mail's conversation for the undelivered shot, the open card
/// for the plain, plain-select, page, run, weighty-mapping and secret
/// shots (the refused shot's with its refusal), the board for the
/// arrivals and departures shots, one arrival's bead for the arrival
/// shot, yardr's line for the flow shot and the yard row for the
/// delivery shot, else none. A debug build's alone.
fn shot_selected() -> Option<Selected> {
    // The card shots open their cards, in decisions: a debug build's
    // alone, as the settings and the draft of Footplate::opened.
    #[cfg(debug_assertions)]
    match std::env::var("FOOTPLATE_SHOT").as_deref() {
        Ok("undelivered") => return Some(Selected::Conversation("clerk@north".into())),
        Ok("plain") | Ok("plain-select") | Ok("page") | Ok("lines") | Ok("keys") => {
            return Some(Selected::Decision("bead-a".into()));
        }
        Ok("decisions") | Ok("decisions-filtered") => {
            return Some(Selected::Decision("bead-a".into()));
        }
        Ok("run")
        | Ok("weighty")
        | Ok("weighty-mapping")
        | Ok("secret")
        | Ok("secret-shown")
        | Ok("secret-refused") => {
            return Some(Selected::Decision("bead-s".into()));
        }
        Ok("arrivals") | Ok("departures") | Ok("old-lines") => return Some(Selected::Arrivals),
        Ok("arrival") => return Some(Selected::Bead("yd-zulu".into())),
        Ok("flow") => return Some(Selected::Line("yardr".into())),
        Ok("grants") => return Some(Selected::Grants),
        Ok("delivery") => return Some(Selected::Yard),
        _ => {}
    }
    None
}

/// Whether a screenshot state opens the arrivals pane on the
/// departures board: the departures shot, under the toggle. A debug
/// build's alone.
#[cfg(debug_assertions)]
fn shot_departures() -> bool {
    std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("departures")
}

/// The selection the plain-select shot poses: the open card's first
/// context line past its first word, as a drag over it would. A debug
/// build's alone.
#[cfg(debug_assertions)]
fn pose_shot_selection(
    selection: &gpui::Entity<select::Selection>,
    model: &Model,
    cx: &mut Context<Footplate>,
) {
    let Some((bead, ask)) = model.current_ask() else {
        return;
    };
    let detail = model.decision_detail(&bead, &chrono::Local::now());
    let Some(line) = detail
        .as_ref()
        .and_then(|detail| detail.asks.iter().find(|asked| asked.ask == ask))
        .and_then(|asked| asked.context.first())
    else {
        return;
    };
    // Past the first word, so the shot shows a selection and not the
    // whole line.
    let start = line.find(' ').map(|at| at + 1).unwrap_or(0);
    let key = super::expanded_text_key(select::key((bead, ask)), "context", 0, line);
    selection.update(cx, |selection, cx| {
        selection.pose(key, line.clone().into(), start..line.len(), cx)
    });
}
