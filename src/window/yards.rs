//! The yards' views: the switcher with its rows, each yard's card
//! with its login switch and agents settings, pairing and signing in,
//! and the away screen with the reconnecting strip.

use std::path::Path;

use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, MouseButton, Rgba, SharedString, Window, div,
    img, prelude::*, px, rems,
};

use super::Footplate;
use super::ask::{PAIRING_GIVES, card_button};
use super::keys::SECONDARY;
use super::metrics::{RAIL, SWITCHER_MAX, SWITCHER_MIN};
use super::number_key;
use super::rail::RailTip;
use super::state::{
    Adding, AgentsBusy, Away, Counts, LoginDid, SwitchBusy, SwitchFor, SwitchRead, YardIdentity,
    code_of, pairing_failure,
};
use super::ui::{keys_hint, quiet_button, segment, segmented, small_button};
use crate::api::{self, Address, Client};
use crate::model::{self, Entry, Model};
use crate::session;
use crate::settings::{self, Section};
use crate::setup;
use crate::theme::{Palette, yard_color};
use crate::titlebar;
use crate::yard;

impl Footplate {
    /// Which yard the window shows, in words: the whole telling, as the
    /// settings say it. The rail's tile has its name.
    pub(crate) fn which_yard(&self) -> String {
        let opened = self.settings.opened();
        match (self.env.address(&self.settings), &self.said) {
            (Some(address), Some(me)) => {
                let mut told = format!(
                    "the yard {} at {address}, as its device {}",
                    me.yard, me.said_by
                );
                if self.env.yard.is_none() && opened.is_hosted() && !opened.account.is_empty() {
                    told.push_str(&format!(", signed in as {}", opened.account));
                }
                told
            }
            (Some(address), None) => format!("the yard at {address}"),
            (None, _) if opened.socket.is_empty() => "this machine's yard".to_string(),
            (None, _) => format!("the yard at the socket {}", opened.socket),
        }
    }
    /// Which yard the away screen names, short: the open one by the name
    /// the switcher lists it under, or the whole telling of the one yard
    /// the environment names. The whole telling stands in the settings.
    pub(crate) fn which_yard_short(&self) -> String {
        match self.env.yard {
            Some(_) => self.which_yard(),
            None => self.yard.clone(),
        }
    }
    pub(crate) fn yard_identity(&self, name: &str) -> Option<YardIdentity> {
        match &self.env.yard {
            Some(address) => Some(YardIdentity::Environment(
                address.clone(),
                self.env.token_file.clone(),
            )),
            None => self.settings.yard(name).cloned().map(YardIdentity::Listed),
        }
    }
    /// The switcher's list at the rail's head, or no longer: the yards
    /// the settings list, to open one of. While it shows, the window
    /// holds the keys, on the row of the open yard first. With one yard
    /// named by the environment there is nothing to switch between. A
    /// yard that is away has the list as any other: it is how the person
    /// leaves it.
    pub(crate) fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self.switching.take().is_some() {
            let conversation = self.model.as_ref().and_then(|m| m.selected_conversation());
            let focus = match conversation {
                // A yard that is away draws no line to type in, but one
                // that is reconnecting keeps its own.
                Some(_) if !self.settings_open && (self.down.is_none() || self.reconnecting()) => {
                    self.line.focus_handle(cx)
                }
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
    pub(crate) fn switch_nth(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.settings.yards.get(n).map(|yard| yard.name.clone()) {
            self.switch_to(&name, window, cx);
        }
    }
    /// Shows the yard called `name` in place of the open one, as it stands:
    /// it was followed all along. It is the one to open at the next start.
    /// The line has the focus, as when the window starts. The yard that is
    /// open already stays as it is, unless it could not be opened: that is
    /// tried again.
    pub(crate) fn switch_to(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.switching = None;
        self.shut_menu();
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
    /// What the status row at the list's head is for the open yard: its
    /// state with Start/Stop, or none.
    pub(crate) fn switch_for(&self) -> SwitchFor {
        // Windows has no yard of this machine, and no yardr to start or
        // stop one with: a yard there is another machine's, whatever the
        // settings list.
        if self.env.yard.is_some() || !cfg!(unix) {
            return SwitchFor::Far;
        }
        let Some(entry) = self.settings.yard(&self.yard) else {
            return SwitchFor::Far;
        };
        if entry.is_far() {
            return SwitchFor::Far;
        }
        if entry.socket.is_empty() {
            return SwitchFor::Near(None);
        }
        let socket = Path::new(&entry.socket);
        let home = socket
            .file_name()
            .is_some_and(|name| name == "yard.sock")
            .then(|| socket.parent())
            .flatten()
            .filter(|home| !home.as_os_str().is_empty())
            .map(Path::to_path_buf);
        match home {
            Some(home) => SwitchFor::Near(Some(home)),
            None => SwitchFor::NoHome,
        }
    }
    /// The yardr the switch runs for the open yard: the fetched one, by
    /// its whole path, on the yard's home. None where there is no switch,
    /// or yardr is not there yet.
    pub(crate) fn switch_yardr(&self) -> Option<setup::Yardr> {
        let program = self.switch_program.clone()?;
        let home = match self.switch_for() {
            SwitchFor::Near(home) => home,
            SwitchFor::Far | SwitchFor::NoHome => return None,
        };
        Some(setup::Yardr {
            program,
            home,
            path: self
                .switch_path
                .clone()
                .unwrap_or_else(|| self.means.path.clone()),
            actor: None,
        })
    }
    /// Reads the switch anew: the yardr to run, and what `status` says of
    /// the open yard. Off the window's thread, as the first run's work is;
    /// an answer that comes when another yard was opened since is dropped.
    /// Nothing is read while the first run is open: it owns fetching
    /// then, and the yard it makes is read once it ends.
    pub(crate) fn refresh_switch(&mut self, cx: &mut Context<Self>) {
        if self.first_run.is_some() || self.switch_busy.is_some() || self.switch_reading {
            return;
        }
        let home = match self.switch_for() {
            SwitchFor::Near(home) => home,
            SwitchFor::Far | SwitchFor::NoHome => return,
        };
        let opening = self.opening;
        self.switch_reading = true;
        let (fetch, path, program) = (
            self.means.yardr.clone(),
            self.switch_path.clone(),
            self.switch_program.clone(),
        );
        let inherited = self.means.path.clone();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    let (program, stale) = match program {
                        Some(program) => (program, None),
                        None => match fetch() {
                            Ok((program, stale)) => (program, stale),
                            Err(why) => return SwitchRead::NoYardr(why),
                        },
                    };
                    let mut yardr = setup::Yardr {
                        program: program.clone(),
                        home,
                        path: path.clone().unwrap_or_else(|| inherited.clone()),
                        actor: None,
                    };
                    // Doctor widens the PATH once; where it fails, status
                    // is still asked with what there is.
                    if path.is_none() {
                        let _ = yardr.needs();
                    }
                    let status = yardr.status();
                    SwitchRead::Ready {
                        program,
                        path: yardr.path.clone(),
                        status,
                        stale,
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    // Another yard's from here on: its own reading owns
                    // the flag, if it has one.
                    return;
                }
                this.switch_reading = false;
                this.read_switch(read, cx);
            })
            .ok();
        })
        .detach();
    }
    /// What a reading of the switch came to is the status row's from
    /// here on.
    pub(crate) fn read_switch(&mut self, read: SwitchRead, cx: &mut Context<Self>) {
        match read {
            SwitchRead::NoYardr(why) => {
                self.switch_program = None;
                self.switch_status = None;
                self.switch_noyard = false;
                self.switch_note = Some(why);
            }
            SwitchRead::Ready {
                program,
                path,
                status,
                stale,
            } => {
                self.switch_program = Some(program);
                self.switch_path = Some(path);
                // A reading that went through clears what an earlier one
                // could not read; what there is to know of the yardr
                // stands instead.
                self.switch_note = stale;
                match status {
                    Ok(status) => {
                        self.switch_status = Some(status);
                        self.switch_noyard = false;
                    }
                    Err(setup::StatusError::NoYard) => {
                        self.switch_status = None;
                        self.switch_noyard = true;
                    }
                    Err(setup::StatusError::Failed(why)) => {
                        self.switch_status = None;
                        self.switch_noyard = false;
                        self.switch_note = Some(why);
                    }
                }
            }
        }
        // The settings stand open on the yard this read: its operator
        // settings are read now that yardr is there to run.
        if self.settings_open {
            self.read_agents(cx);
        }
        cx.notify();
    }
    /// The open yard is another: what the switch said of the last is
    /// gone. The yardr and the PATH stay: they are the window's.
    pub(crate) fn switch_reset(&mut self) {
        self.switch_status = None;
        self.switch_noyard = false;
        self.switch_reading = false;
        self.switch_busy = None;
        self.switch_troubles = Vec::new();
        self.switch_note = None;
        self.switch_question = None;
    }
    /// Whether the open yard is proven to be a stopped one: its home
    /// holds a yard, and nothing answers there.
    pub(crate) fn yard_is_stopped(&self) -> bool {
        self.down.is_some() && self.switch_status.is_some() && !self.answers_under_adding()
    }
    /// Whether the open yard answered in this run and is away now, asked
    /// again every few seconds: its rows and its open row stay on screen
    /// as last seen, marked stale by the one quiet line, and its sends
    /// are held until it answers. A yard never reached, one that no
    /// longer lets this device in, and one that could not be opened keep
    /// the away screen, which says what to do. So does one that answered
    /// while a way's fields stood open: it is not away, but its screen
    /// stands until the person leaves the fields.
    pub(crate) fn reconnecting(&self) -> bool {
        self.down.is_some()
            && self.model.is_some()
            && self.away == Away::Trying
            && !self.answered_while_adding
    }
    /// Whether the open yard is a stopped one of this machine that is
    /// reconnecting: the last reading says neither its server nor its
    /// daemon runs. Its row reads Stopped, not Reconnecting: the
    /// reading says what to start.
    pub(crate) fn stopped_while_reconnecting(&self) -> bool {
        self.reconnecting()
            && matches!(self.switch_for(), SwitchFor::Near(_))
            && matches!(
                self.switch_status.as_ref(),
                Some(status) if !status.server_running() && !status.daemon_running()
            )
    }
    /// The quiet line where the list has no status row for it: a yard at
    /// an address, while it is reconnecting. A yard of this machine says
    /// it in the status row instead. A small strip at the top of the
    /// conversation; after the reconnect it goes away with the state.
    pub(crate) fn reconnect_strip(&self, palette: Palette) -> Option<impl IntoElement> {
        (self.reconnecting() && matches!(self.switch_for(), SwitchFor::Far)).then(|| {
            div()
                .debug_selector(|| "reconnecting".to_string())
                .flex_none()
                .px_3()
                .py_1()
                .text_sm()
                .text_color(palette.faint)
                .border_b_1()
                .border_color(palette.line)
                .child("Reconnecting…")
        })
    }
    /// Tries the switch's yardr again, after it could not be had.
    pub(crate) fn switch_retry(&mut self, cx: &mut Context<Self>) {
        self.switch_note = None;
        self.refresh_switch(cx);
        cx.notify();
    }
    /// "Start at login", switched `on`. On is `up --login`: the login
    /// services are installed, and a yard that is stopped is started.
    /// Off removes the daemon's service; the daemon the service ran is
    /// stopped with it, and started again where the yard runs. What went
    /// is the switch's from here on; what could not be is said. A click
    /// on what it already is does nothing.
    pub(crate) fn set_login(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(yardr) = self.switch_yardr() else {
            return;
        };
        if self.switch_busy.is_some() {
            return;
        }
        let is = self
            .switch_status
            .as_ref()
            .is_some_and(|status| status.starts_at_login());
        if is == on {
            return;
        }
        self.switch_busy = Some(SwitchBusy::Login);
        self.switch_note = None;
        cx.notify();
        let (opening, running) = (self.opening, self.down.is_none());
        cx.spawn(async move |this, cx| {
            let did = cx
                .background_executor()
                .spawn(async move {
                    match on {
                        true => LoginDid::Checked(yardr.up_login()),
                        false => LoginDid::Unchecked(yardr.daemon_uninstall()),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    return;
                }
                this.switch_busy = None;
                match did {
                    LoginDid::Checked(Ok(up)) => {
                        let failed: Vec<String> = up
                            .login
                            .iter()
                            .filter(|unit| unit.error.is_some())
                            .map(|unit| unit.said())
                            .collect();
                        if !failed.is_empty() {
                            this.notice(failed.join("\n"), cx);
                        }
                    }
                    LoginDid::Checked(Err(why)) => this.notice(why, cx),
                    LoginDid::Unchecked(Ok(uninstalled)) => {
                        if uninstalled.removed && uninstalled.unloaded && running {
                            this.start_yard(cx);
                        }
                        this.notice(
                            match uninstalled.removed {
                                true => "The daemon's login service is removed. herdr's server still starts at login: its service is shared by every yard on this machine, and yardr removes it only once the yard is stopped.",
                                false => "No login service was installed.",
                            }
                            .to_string(),
                            cx,
                        );
                    }
                    LoginDid::Unchecked(Err(why)) => this.notice(why, cx),
                }
                this.refresh_switch(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    /// Reads "Agents and you" anew: the near yard's operator settings, as
    /// the setup reads doctor. Off the window's thread; an answer that
    /// comes when another yard was opened since is dropped. Nothing is
    /// read while the settings are shut, or for a yard at an address.
    pub(crate) fn read_agents(&mut self, cx: &mut Context<Self>) {
        if !self.settings_open || self.agents_busy.is_some() || self.agents_read {
            return;
        }
        if !matches!(self.switch_for(), SwitchFor::Near(_)) {
            return;
        }
        let Some(yardr) = self.switch_yardr() else {
            return;
        };
        self.agents_busy = Some(AgentsBusy::Reading);
        self.agents_said = None;
        cx.notify();
        let opening = self.opening;
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { yardr.operator() })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    return;
                }
                this.agents_busy = None;
                match read {
                    Ok(operator) => {
                        // Unset, or newer than this footplate: the setup's
                        // default, the most guidance.
                        this.agents_style = setup::style_at(&operator.style).unwrap_or(0);
                        this.agents_about
                            .update(cx, |line, cx| line.set_text(&operator.note, cx));
                        this.agents_read = true;
                    }
                    Err(why) => this.agents_said = Some(why),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    /// Saves "Agents and you": the chosen style and the line about the
    /// operator. An emptied line unsets the note. What came of it stands
    /// under the section: a change reaches the next session.
    pub(crate) fn save_agents(&mut self, cx: &mut Context<Self>) {
        if self.agents_busy.is_some() || !self.agents_read {
            return;
        }
        let Some(yardr) = self.switch_yardr() else {
            return;
        };
        let style = setup::STYLES[self.agents_style].0.to_string();
        let note = self.agents_about.read(cx).text().trim().to_string();
        self.agents_busy = Some(AgentsBusy::Saving);
        self.agents_said = None;
        cx.notify();
        let opening = self.opening;
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move { yardr.set_operator(&style, &note) })
                .await;
            this.update(cx, |this, cx| {
                if this.opening != opening {
                    return;
                }
                this.agents_busy = None;
                this.agents_said = Some(match saved {
                    Ok(()) => "Saved. The next session picks it up.".to_string(),
                    Err(why) => why,
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    /// Tries "Agents and you" again, after it could not be read: yardr
    /// itself first, where it could not be had.
    pub(crate) fn agents_retry(&mut self, cx: &mut Context<Self>) {
        if self.switch_program.is_none() {
            self.switch_retry(cx);
        }
        self.agents_said = None;
        self.read_agents(cx);
        cx.notify();
    }
    /// The open yard is another: what "Agents and you" said of the last
    /// is gone, and the yard's own are read anew once the settings show.
    pub(crate) fn agents_reset(&mut self, cx: &mut Context<Self>) {
        self.agents_style = 0;
        self.agents_about
            .update(cx, |line, cx| line.set_text("", cx));
        self.agents_busy = None;
        self.agents_read = false;
        self.agents_said = None;
    }
    /// The device of the yard called `name` is to be paired again: the
    /// settings, with a code to type for that yard's address. From where a
    /// yard said this device is not let in, it is the open one.
    pub(crate) fn pair_again(
        &mut self,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pairing || self.signing_in {
            return;
        }
        if !self.settings_open {
            self.toggle_settings(window, cx);
        }
        // The code is typed in the yards section, whatever stood open.
        self.pick_section(Section::Yards, cx);
        self.shut_menu();
        self.adding = Some(Adding::Again(name.unwrap_or_else(|| self.yard.clone())));
        self.yard_said = None;
        self.code.update(cx, |line, cx| line.set_text("", cx));
        window.focus(&self.code.focus_handle(cx));
        cx.notify();
    }
    /// The hosted yard called `name` is to be signed in again: the
    /// settings, with the Kratos account's email and password to type for
    /// that yard's address. From where the edge no longer knows the
    /// session, it is the open one. The email stands filled where the yard
    /// names its account.
    pub(crate) fn sign_in_again(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pairing || self.signing_in || self.env.yard.is_some() {
            return;
        }
        if !self.settings_open {
            self.toggle_settings(window, cx);
        }
        // The account is typed in the yards section, whatever stood open.
        self.pick_section(Section::Yards, cx);
        self.shut_menu();
        self.adding = Some(Adding::SignInAgain(name.to_string()));
        self.yard_said = None;
        let account = self
            .settings
            .yard(name)
            .map(|yard| yard.account.clone())
            .unwrap_or_default();
        self.email
            .update(cx, |line, cx| line.set_text(&account, cx));
        self.password.update(cx, |line, cx| line.set_text("", cx));
        window.focus(&self.email.focus_handle(cx));
        cx.notify();
    }
    /// Sends the code that is typed to the yard it is for, and opens that
    /// yard as the device the code was for: a yard that is added, at the
    /// address that is typed, or one of the list whose device is paired
    /// again. The token goes into that yard's own file and no other's. At
    /// a hosted yard of the list the session rides beside the code, and
    /// where it cannot be read that is said instead of pairing.
    /// What keeps a yard out of the list is said before anything is sent,
    /// and so is a token file that cannot be written: a code is used up by
    /// the yard it is sent to.
    pub(crate) fn pair(&mut self, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in {
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
            (_, Adding::Near)
            | (_, Adding::Hosted)
            | (_, Adding::SignInAgain(_))
            | (_, Adding::Rename(_)) => return,
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
        // Pairing a hosted yard of the list again: the code is the edge's
        // to let through, with the session beside it.
        let session: Result<Option<String>, String> = match &adding {
            Adding::Again(name) => self
                .env
                .session_of(&self.settings, name, kept.as_deref())
                .and_then(|file| {
                    file.map(|file| yard::session_for(&file, &address))
                        .transpose()
                }),
            _ => Ok(None),
        };
        let session = match session {
            Ok(session) => session,
            Err(why) => {
                self.yard_said = Some(why);
                return;
            }
        };
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
                    let paired = match &session {
                        Some(session) => {
                            api::pair_hosted(&address, &code, session).map_err(pairing_failure)
                        }
                        None => api::pair(&address, &code).map_err(|err| err.to_string()),
                    }?;
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
                            (None, Adding::Near)
                            | (None, Adding::Hosted)
                            | (None, Adding::SignInAgain(_))
                            | (None, Adding::Rename(_)) => None,
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
    /// Signs the Kratos account in for a hosted yard: one that is added,
    /// at the address that is typed, or one of the list whose session is
    /// signed in again. The session goes into that yard's own file and no
    /// other's, and the account stands with the yard. What keeps a yard
    /// out of the list is said before anything is sent. A yard that is
    /// added is paired next: the code's fields stand open with what the
    /// sign-in said. The password is sent and then stands nowhere: not in
    /// a field, a file or a setting.
    pub(crate) fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.pairing || self.signing_in {
            return;
        }
        let Some(adding) = self.adding.clone() else {
            return;
        };
        cx.notify();
        let kept = self.kept.clone();
        // The yard's name and address, the file its session goes in, and
        // the settings once it is signed in.
        let asked = match (&self.env.yard, &adding) {
            (_, Adding::Near)
            | (_, Adding::Far)
            | (_, Adding::Again(_))
            | (_, Adding::Rename(_)) => return,
            // The environment's yard is reached direct, and is signed in
            // nowhere: its fields are never asked for.
            (Some(_), _) => return,
            (None, Adding::SignInAgain(name)) => {
                let listed = match self.settings.yard(name) {
                    Some(listed) if listed.is_hosted() => listed.clone(),
                    _ => {
                        self.yard_said = Some(format!(
                            "The yard {name} is no longer a hosted yard of the list."
                        ));
                        return;
                    }
                };
                self.env
                    .session_of(&self.settings, name, kept.as_deref())
                    .and_then(|file| {
                        file.map(|file| (listed.address.clone(), file, self.settings.clone()))
                            .ok_or_else(|| {
                                format!("The yard {name} is no longer a hosted yard of the list.")
                            })
                    })
            }
            (None, Adding::Hosted) => {
                let address = self.address.read(cx).text().trim().to_string();
                // Called by its address's host where no name is typed.
                let name = match self.yard_name.read(cx).text().trim() {
                    "" => settings::host_of(&address),
                    name => name.to_string(),
                };
                let token_file = self.settings.token_name(&name, kept.as_deref());
                let session_file = self.settings.session_name(&name, kept.as_deref());
                let yard = settings::Yard::hosted(&name, &address, &token_file, &session_file);
                let settings = self.settings.clone();
                settings
                    .with_yard(None, yard, kept.as_deref())
                    .and_then(|settings| {
                        let file = yard::token_path(&session_file, kept.as_deref())?;
                        Ok((address, file, settings.opening(&name)))
                    })
            }
        };
        let parsed =
            asked.and_then(|(text, file, settings)| Ok((Address::parse(&text)?, file, settings)));
        let (address, file, settings) = match parsed {
            Ok(asked) => asked,
            Err(why) => {
                self.yard_said = Some(why);
                return;
            }
        };
        let email = self.email.read(cx).text().trim().to_string();
        if email.is_empty() {
            self.yard_said =
                Some("An email is wanted: the Kratos account to sign in as.".to_string());
            return;
        }
        // The masked line gives up the password before the request starts,
        // whatever the edge takes to answer.
        let password = self.password.update(cx, |line, cx| line.take_text(cx));
        if password.is_empty() {
            self.yard_said = Some("A password is wanted: the Kratos account's.".to_string());
            return;
        }
        self.signing_in = true;
        self.yard_said = None;
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_executor()
                .spawn(async move {
                    let signed =
                        api::sign_in(&address, &email, &password).map_err(|err| err.to_string())?;
                    // The session goes from the sign-in's answer into its
                    // file with the address it was signed in at, and
                    // nowhere else. The password bought nothing that is
                    // lost: where the file cannot be written, signing in
                    // again is the whole of it.
                    yard::keep_session(&file, &signed.session, &address).map_err(|err| {
                        format!(
                            "Signed in as {}, but {} could not be written ({err}). Sign in again.",
                            signed.account,
                            file.display()
                        )
                    })?;
                    Ok::<_, String>(signed.account)
                })
                .await;
            this.update(cx, |this, cx| {
                this.signing_in = false;
                cx.notify();
                // The password is sent: it stands in no field any longer,
                // whatever the sign-in came to.
                this.password.update(cx, |line, cx| line.set_text("", cx));
                let account = match answer {
                    Ok(account) => account,
                    Err(why) => {
                        this.yard_said = Some(why);
                        return;
                    }
                };
                this.email.update(cx, |line, cx| line.set_text("", cx));
                match &adding {
                    Adding::Hosted => {
                        // The account stands with the added yard, and the
                        // road goes on to pairing: the code's fields stand
                        // open with what the sign-in said. Ordinary
                        // settings may have changed while its answer was
                        // away, as with a pairing's.
                        let next = settings
                            .yard(&settings.opened().name)
                            .cloned()
                            .and_then(|mut yard| {
                                yard.account = account.clone();
                                let name = yard.name.clone();
                                this.settings
                                    .clone()
                                    .with_yard(None, yard, this.kept.as_deref())
                                    .ok()
                                    .map(|settings| (settings.opening(&name), name))
                            });
                        match next {
                            Some((settings, name)) => {
                                this.set(settings, cx);
                                this.reopen(cx);
                                this.adding = Some(Adding::Again(name));
                                this.yard_said = Some(format!(
                                    "Signed in as {account}: now pair this device with the code from the ready page."
                                ));
                                let focus = this.code.focus_handle(cx);
                                let handle = this.handle;
                                // Afterwards: the window is not to be had
                                // while the sign-in's answer is still on
                                // its way.
                                cx.defer(move |cx| {
                                    let _ = handle.update(cx, |_, window, _| {
                                        window.focus(&focus)
                                    });
                                });
                            }
                            None => {
                                this.yard_said = Some(
                                    "Signed in, but its settings entry is no longer there. Add the yard again."
                                        .to_string(),
                                );
                                this.shut_adding(cx);
                            }
                        }
                    }
                    Adding::SignInAgain(name) => {
                        let next = this
                            .settings
                            .yard(name)
                            .cloned()
                            .and_then(|mut yard| match yard.is_hosted() {
                                true => {
                                    yard.account = account.clone();
                                    this.settings
                                        .clone()
                                        .with_yard(Some(name), yard, this.kept.as_deref())
                                        .ok()
                                        .map(|settings| settings.opening(name))
                                }
                                false => None,
                            });
                        match next {
                            Some(settings) => {
                                this.yard_said = Some(format!("Signed in as {account}."));
                                this.shut_adding(cx);
                                this.set(settings, cx);
                                // The session it was opened with is not
                                // the edge's any more: the yard is opened
                                // with the new one.
                                this.reopen_anew(Some(name), cx);
                            }
                            None => {
                                this.yard_said = Some(format!(
                                    "Signed in, but the yard {name} is no longer a hosted yard of the list. Add it again."
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            })
            .ok();
        })
        .detach();
    }
    /// How a yard of the settings is reached, in words: a yard at an
    /// address by it, with the signed-in account beside it where it has
    /// one; a yard of this machine by its socket, this run's where it
    /// names none. The switcher's second line and the settings' rows say it.
    pub(crate) fn yard_reach(yard: &settings::Yard) -> String {
        match (yard.is_far(), yard.socket.is_empty()) {
            (true, _) if yard.is_hosted() && !yard.account.is_empty() => {
                format!("{} · signed in as {}", yard.address, yard.account)
            }
            (true, _) => yard.address.clone(),
            (false, true) => "by this run's socket".to_string(),
            (false, false) => yard.socket.clone(),
        }
    }
    /// What the settings say of a yard of the list after its name and its
    /// kind: that it is not reached, with the short why, that it is the
    /// open one, or what waits in it now. Every yard is followed, so one
    /// that is not reached is asked again by itself, and says so until it
    /// answers.
    pub(crate) fn yard_state(&self, yard: &settings::Yard) -> String {
        match yard.name == self.yard && !self.unreached.contains_key(&yard.name) {
            true => "open".to_string(),
            false => self.yard_waits(&yard.name),
        }
    }
    /// What is said of the yard called `name` wherever its being open goes
    /// without saying, as in the menu bar's menu: that it is not reached,
    /// with the short why, or what waits in it now, the open yard's too.
    pub(crate) fn yard_waits(&self, name: &str) -> String {
        let model = match name == self.yard {
            true => self.model.as_ref(),
            false => self.held.get(name).and_then(|h| h.model.as_ref()),
        };
        match (self.unreached.get(name), model) {
            (Some(why), _) => format!("not reached: {why}"),
            (None, Some(model)) => Counts::of(model).line(),
            (None, None) => "looking for the yard".to_string(),
        }
    }
    /// The yards in the settings: what a pairing is for, every yard of
    /// the list as a card with its name, its kind, its reach and what
    /// waits in it, and the fields of one that is added. With a yard
    /// named by the environment there is that one alone, to pair again.
    pub(crate) fn talk_yards(
        &self,
        setting: impl Fn(String, String) -> Div,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let faint = |text: String| div().text_sm().text_color(palette.faint).child(text);
        let mut yard = setting(
            "yards".to_string(),
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
                    yard = yard.child(self.yard_card(listed, palette, cx));
                }
                yard = yard.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        // A yard of this machine is reached by its socket,
                        // which Windows has not.
                        .children(cfg!(unix).then(|| {
                            small_button("add a yard of this machine".to_string(), palette)
                                .id("add near")
                                .debug_selector(|| "add near".to_string())
                                .when(self.adding == Some(Adding::Near), |el| el.bg(palette.open))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_yard(Adding::Near, window, cx)
                                }))
                        }))
                        .child(
                            small_button("add a yard at an address".to_string(), palette)
                                .id("add far")
                                .debug_selector(|| "add far".to_string())
                                .when(self.adding == Some(Adding::Far), |el| el.bg(palette.open))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_yard(Adding::Far, window, cx)
                                })),
                        )
                        .child(
                            small_button("add a hosted yard".to_string(), palette)
                                .id("add hosted")
                                .debug_selector(|| "add hosted".to_string())
                                .when(self.adding == Some(Adding::Hosted), |el| {
                                    el.bg(palette.open)
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_yard(Adding::Hosted, window, cx)
                                })),
                        ),
                );
            }
        }
        yard.children(self.yard_asked(palette, cx))
    }
    /// One yard of the list as a card: its name, its kind, its reach and
    /// what waits in it, with Open where it is not the open one and a ⋯
    /// menu holding pair again, rename and remove (and the sign-in again
    /// of a hosted yard). Where the yard is renamed, the form stands in
    /// its card, where its menu stood. The open yard's card carries
    /// start at login where it is a yard of this machine.
    pub(crate) fn yard_card(
        &self,
        listed: &settings::Yard,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let name = listed.name.clone();
        let open = name == self.yard;
        let state = self.yard_state(listed);
        let count = (state == "open").then(|| self.yard_waits(&name));
        let reach = match (listed.is_far(), listed.socket.is_empty()) {
            (true, _) if listed.is_hosted() && !listed.account.is_empty() => {
                format!("{} · signed in as {}", listed.address, listed.account)
            }
            (true, _) => listed.address.clone(),
            (false, true) => "by this run's socket".to_string(),
            (false, false) => listed.socket.clone(),
        };
        let mut actions = div().flex_none().flex().items_center().gap_2();
        if !open {
            let at = format!("yard open {name}");
            actions = actions.child(
                small_button("open".to_string(), palette)
                    .id(SharedString::from(at.clone()))
                    .debug_selector(move || at.clone())
                    .on_click(cx.listener({
                        let name = name.clone();
                        move |this, _, window, cx| this.switch_to(&name, window, cx)
                    })),
            );
        }
        let at = format!("yard menu {name}");
        actions = actions.child(
            small_button("…".to_string(), palette)
                .id(SharedString::from(at.clone()))
                .debug_selector(move || at.clone())
                .tooltip(|_, cx| cx.new(|_| RailTip("Yard actions".to_string())).into())
                .on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, _, cx| this.toggle_menu(&name, cx)
                })),
        );
        let mut card = div()
            .debug_selector({
                let name = name.clone();
                move || format!("yard row {name}")
            })
            .rounded_lg()
            .border_1()
            .border_color(palette.line)
            .bg(palette.side)
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_sm()
                                            .text_color(palette.faint)
                                            .child(listed.kind()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_right()
                                            .text_sm()
                                            .text_color(palette.faint)
                                            .child(state),
                                    ),
                            ),
                    )
                    .child(actions),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .truncate()
                    .child(reach),
            );
        if let Some(count) = count {
            card = card.child(
                div()
                    .debug_selector({
                        let name = name.clone();
                        move || format!("yard count {name}")
                    })
                    .text_sm()
                    .text_color(palette.faint)
                    .text_right()
                    .child(count),
            );
        }
        if self.yard_menu.as_deref() == Some(name.as_str()) {
            let item = |what: &str, text: &str| {
                let at = format!("yard {what} {name}");
                div()
                    .id(SharedString::from(at.clone()))
                    .debug_selector(move || at.clone())
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.hover))
                    .child(text.to_string())
            };
            let mut menu = div()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(palette.line)
                .bg(palette.ground)
                .p_1();
            if listed.is_far() {
                menu = menu.child(item("again", "pair again").on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, window, cx| this.pair_again(Some(name.clone()), window, cx)
                })));
            }
            if listed.is_hosted() {
                menu = menu.child(item("sign in", "sign in again").on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, window, cx| this.sign_in_again(&name, window, cx)
                })));
            }
            menu = menu.child(item("rename", "rename").on_click(cx.listener({
                let name = name.clone();
                move |this, _, window, cx| this.rename_start(&name, window, cx)
            })));
            menu = menu.child(item("remove", "remove").on_click(cx.listener({
                let name = name.clone();
                move |this, _, _, cx| this.remove_yard(&name, cx)
            })));
            card = card.child(menu);
        }
        if matches!(&self.adding, Some(Adding::Rename(at)) if *at == name) {
            card = card.children(self.rename_asked(&name, palette, cx));
        }
        if let Some(login) = self.login_card(&name, palette, cx) {
            card = card.child(login);
        }
        card
    }
    /// "Start at login" on the open yard's card, where it is a yard of
    /// this machine: the yard's daemon starts at login, and herdr's
    /// server with it, by the one service every yard on this machine
    /// shares. On is the yard starting at login, both halves. Any other
    /// card carries nothing: a yard at an address starts nothing on this
    /// machine, so the switch shows only on the local yard, where it
    /// applies. Beside the switch, one faint line for what came of a
    /// switch.
    pub(crate) fn login_card(
        &self,
        name: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if name != self.yard || !matches!(self.switch_for(), SwitchFor::Near(_)) {
            return None;
        }
        let faint = |text: String| div().text_sm().text_color(palette.faint).child(text);
        let mut card = div().flex().flex_col().gap_1();
        match self.switch_status.as_ref() {
            Some(status) if status.daemon_service().supported => {
                let on = status.starts_at_login();
                card = card
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().child("start at login"))
                            .child(segmented(
                                [("off", false), ("on", true)]
                                    .map(|(name, want)| {
                                        segment(name.to_string(), want == on, palette)
                                            .id(SharedString::from(format!("login {name}")))
                                            .debug_selector(move || format!("login {name}"))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.set_login(want, cx)
                                            }))
                                    })
                                    .into_iter()
                                    .collect(),
                                palette,
                            ))
                            .when(self.switch_busy.is_some(), |el| el.opacity(0.5)),
                    )
                    .child(faint(self.login_help(status)));
            }
            Some(_) => {
                card = card.child(faint(
                    "this system has no service manager yardr writes to: nothing starts at login"
                        .to_string(),
                ));
            }
            None => {
                card = card.child(faint(match (&self.switch_note, self.switch_reading) {
                    (Some(_), false) => "yardr could not be read.".to_string(),
                    _ => "asking yardr what starts at login…".to_string(),
                }));
            }
        }
        if let Some(note) = self.switch_note.as_ref() {
            card = card.child(
                div()
                    .debug_selector(|| "switch note".to_string())
                    .text_sm()
                    .text_color(palette.faint)
                    .truncate()
                    .child(note.clone()),
            );
        }
        Some(card)
    }
    /// The one line of help under "start at login". Where the yard is
    /// stopped and the switch is off, switching on starts the yard now
    /// as well as at every login (`up --login` starts a stopped yard),
    /// so it says so before the click.
    pub(crate) fn login_help(&self, status: &setup::Status) -> String {
        let help = "the yard's daemon and herdr's shared server start at login";
        // While the switch is busy a click starts nothing, so the help
        // names only login then.
        match !status.starts_at_login() && self.yard_is_stopped() && self.switch_busy.is_none() {
            true => format!("{help}; switching on starts the yard now as well"),
            false => help.to_string(),
        }
    }
    /// "Agents and you" in the settings: how the near yard's agents work
    /// with its operator, and the line they know about them. A yard at
    /// an address keeps its operator's settings where it runs: the
    /// section says whose they are, read-only, and where they are
    /// changed.
    pub(crate) fn agents_setting(
        &self,
        setting: impl Fn(String, String) -> Div,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let focus = self.agents_about.focus_handle(cx);
        let help = "how this yard's agents work with you, and the line they know about you";
        match self.switch_for() {
            SwitchFor::Far => setting("Agents & you".to_string(), help.to_string()).child(
                div()
                    .debug_selector(|| "agents far".to_string())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(format!("Set by the operator on {}.", self.yard))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .child("They are changed on that machine, with yardr settings set."),
                    ),
            ),
            SwitchFor::NoHome => setting(
                "Agents & you".to_string(),
                "this entry names no yard home: nothing is read or changed".to_string(),
            ),
            SwitchFor::Near(_) if !self.agents_read => {
                let mut section = setting("Agents & you".to_string(), help.to_string());
                // What a read came to instead of the yard's own, or why
                // yardr is not there to run: either is tried again.
                let said = match (&self.agents_said, &self.switch_note) {
                    (Some(said), _) => Some(said),
                    (None, Some(note)) if self.switch_program.is_none() && !self.switch_reading => {
                        Some(note)
                    }
                    (None, _) => None,
                };
                match said {
                    Some(said) => {
                        section = section
                            .child(
                                div()
                                    .debug_selector(|| "agents said".to_string())
                                    .child(said.clone()),
                            )
                            .child(
                                div().flex().child(
                                    small_button("try again".to_string(), palette)
                                        .id("agents again")
                                        .debug_selector(|| "agents again".to_string())
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.agents_retry(cx)),
                                        ),
                                ),
                            );
                    }
                    None => {
                        section = section.child(
                            div()
                                .text_sm()
                                .text_color(palette.faint)
                                .child("asking yardr how agents work with you…"),
                        );
                    }
                }
                section
            }
            SwitchFor::Near(_) => {
                let chosen = self.agents_style;
                let busy = self.agents_busy.is_some();
                let mut section = setting("Agents & you".to_string(), help.to_string())
                    .child(
                        div().flex().child(
                            segmented(
                                setup::STYLES
                                    .iter()
                                    .enumerate()
                                    .map(|(at, (key, _, _))| {
                                        let name = setup::style_name(at);
                                        segment(name, at == chosen, palette)
                                            .id(SharedString::from(format!("agents style {key}")))
                                            .debug_selector(move || format!("agents style {key}"))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.agents_style = at;
                                                cx.notify();
                                            }))
                                    })
                                    .collect(),
                                palette,
                            )
                            .when(busy, |el| el.opacity(0.5)),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .child(setup::STYLES[chosen].2.to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(rems(6.)).flex_none().child("about you"))
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
                                    .debug_selector(|| "agents about".to_string())
                                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                                        window.focus(&focus);
                                        window.prevent_default();
                                    })
                                    .child(self.agents_about.clone()),
                            ),
                    )
                    .child(
                        div().flex().flex_wrap().items_center().gap_2().child(
                            small_button("save (enter)".to_string(), palette)
                                .id("agents save")
                                .debug_selector(|| "agents save".to_string())
                                .when(busy, |el| el.opacity(0.5))
                                .on_click(cx.listener(|this, _, _, cx| this.save_agents(cx))),
                        ),
                    );
                if let Some(said) = &self.agents_said {
                    section = section.child(
                        div()
                            .debug_selector(|| "agents said".to_string())
                            .child(said.clone()),
                    );
                }
                section
            }
        }
    }
    /// The status row at each mode's list head. A yard at an address has
    /// none: its processes are elsewhere. "Start at login" is a switch of
    /// the settings, with the faint line for what came of a switch beside
    /// it there.
    pub(crate) fn switch_rows(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.first_run.is_some() {
            return Vec::new();
        }
        match self.switch_for() {
            SwitchFor::Far => Vec::new(),
            SwitchFor::NoHome => vec![
                div()
                    .debug_selector(|| "switch no home".to_string())
                    .text_size(rems(0.8))
                    .text_color(palette.faint)
                    .child("the entry names no yard home")
                    .into_any_element(),
            ],
            SwitchFor::Near(_) => vec![self.switch_state(palette, cx)],
        }
    }
    /// The one status row: a coloured dot, the yard's state, and
    /// Start/Stop as a quiet text button at the right end. Running where
    /// it answers, Starting and Stopping while it is started or stopped,
    /// Stopped where nothing runs, or what the last start could not
    /// start, in yardr's words. A stopped yard reads Stopped while it
    /// reconnects too: the reading says what to start. Where the yard
    /// answered in this run and is away now, and the reading says
    /// something still runs or none says yet, the row is the one quiet
    /// line that says so, Reconnecting with a grey dot, and Start stands
    /// where a reading knows what to start.
    pub(crate) fn switch_state(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let running = self.model.is_some() && (self.down.is_none() || self.answers_under_adding());
        let (state, dot, button): (String, Rgba, Option<(&str, &str)>) = match self.switch_busy {
            Some(SwitchBusy::Starting) => ("Starting…".to_string(), palette.accent, None),
            Some(SwitchBusy::Stopping) => ("Stopping…".to_string(), palette.accent, None),
            _ if self.stopped_while_reconnecting() => (
                "Stopped".to_string(),
                palette.faint,
                Some(("switch start", "Start")),
            ),
            _ if self.reconnecting() => (
                "Reconnecting…".to_string(),
                palette.faint,
                match (self.switch_yardr().is_some(), self.switch_note.is_some()) {
                    (true, _) => Some(("switch start", "Start")),
                    (false, true) => Some(("switch try again", "try again")),
                    (false, false) => None,
                },
            ),
            _ if self.switch_program.is_none() && self.switch_note.is_some() => match running {
                true => (
                    "Running · yardr is not fetched".to_string(),
                    palette.warn,
                    Some(("switch try again", "try again")),
                ),
                false => (
                    "yardr is not fetched".to_string(),
                    palette.warn,
                    Some(("switch try again", "try again")),
                ),
            },
            _ if running => (
                "Running".to_string(),
                palette.ok,
                Some(("switch stop", "Stop")),
            ),
            _ if !self.switch_troubles.is_empty() => (
                self.switch_troubles.join("; "),
                palette.warn,
                Some(("switch start", "Start")),
            ),
            _ if self.switch_program.is_none() => ("…".to_string(), palette.faint, None),
            _ if self.switch_noyard => ("No yard".to_string(), palette.faint, None),
            _ => match self.switch_status.as_ref() {
                None => ("…".to_string(), palette.faint, None),
                // Processes without a socket: half started, or half
                // stopped. Start repairs it, and is safe to repeat.
                Some(status) if status.server_running() || status.daemon_running() => (
                    "Starting…".to_string(),
                    palette.accent,
                    Some(("switch start", "Start")),
                ),
                Some(_) => (
                    "Stopped".to_string(),
                    palette.faint,
                    Some(("switch start", "Start")),
                ),
            },
        };
        let mut row = div()
            .debug_selector(|| "switch state".to_string())
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "switch dot".to_string())
                    .size(rems(0.5))
                    .rounded_full()
                    .bg(dot)
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(rems(0.8))
                    .truncate()
                    .when(state == "Reconnecting…", |el| {
                        el.debug_selector(|| "reconnecting".to_string())
                    })
                    .child(state),
            );
        if let Some((id, text)) = button {
            let button = quiet_button(text.to_string(), palette)
                .id(id)
                .debug_selector(move || id.to_string())
                .flex_none();
            row = row.child(match id {
                "switch start" => {
                    button.on_click(cx.listener(|this, _, _, cx| this.start_yard(cx)))
                }
                "switch stop" => button.on_click(cx.listener(|this, _, _, cx| this.stop_yard(cx))),
                _ => button.on_click(cx.listener(|this, _, _, cx| this.switch_retry(cx))),
            });
        }
        row.into_any_element()
    }
    /// The switcher's list as a popover to the right of the rail's head:
    /// a row for each yard of the settings, on two lines. The first has
    /// the yard's dot in its colour, its name, its kind, its waiting
    /// count right-aligned in one column and its number key; the second
    /// its address, dimmer. A yard that is not reached carries no count:
    /// the second line says why in red, with the button that rights it
    /// beside it. A click opens a yard, as Enter does on the row the
    /// arrows are on; the footer's key opens the settings' yards. It
    /// stands over whatever the window shows.
    pub(crate) fn switcher(
        &self,
        at: usize,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = div().flex().flex_col().flex_none().py_1();
        for (place, yard) in self.settings.yards.iter().enumerate() {
            let name = yard.name.clone();
            let open = name == self.yard;
            let why = self.unreached.get(&yard.name).copied();
            let counts = match open {
                true => self.model.as_ref(),
                false => self.held.get(&name).and_then(|held| held.model.as_ref()),
            }
            .map(Counts::of);
            // A yard that is not reached carries no count, since its
            // second line says why. Every reached yard carries its
            // waiting number, including the open one; its tooltip says
            // the kinds in words. The number keys stand last, so the
            // counts' right edges stand in one column.
            let count = div()
                .id(SharedString::from(format!("switch count {name}")))
                .debug_selector({
                    let name = name.clone();
                    move || format!("switch count {name}")
                })
                .flex_none()
                .text_right()
                .text_color(palette.faint);
            let count = match (why, counts) {
                (None, Some(counts)) => {
                    let (number, words) = counts.switcher();
                    count
                        .tooltip(move |_, cx| cx.new(|_| RailTip(words.clone())).into())
                        // What waits reads in text rust; a yard with
                        // nothing waiting keeps its dim zero.
                        .when(counts.waiting() > 0, |el| {
                            el.text_color(palette.text_accent)
                        })
                        .child(number)
                }
                _ => count,
            };
            let mut second = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                // The dot's room, so the line stands under the name.
                .child(div().flex_none().size(px(8.)));
            match why {
                Some(why) => {
                    second = second.child(
                        div()
                            .debug_selector({
                                let name = yard.name.clone();
                                move || format!("switch why {name}")
                            })
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(palette.warn)
                            .child(format!("not reached: {why}")),
                    );
                    // The button that rights it: signing in again where
                    // the session is what is out, else pairing again for
                    // a yard at an address, as the away screen offers
                    // them. A click rights the yard instead of opening it.
                    if why == session::SIGN_IN_AGAIN {
                        let again = name.clone();
                        second = second.child(
                            small_button("sign in again".to_string(), palette)
                                .id(SharedString::from(format!("switch sign in {name}")))
                                .debug_selector({
                                    let name = name.clone();
                                    move || format!("switch sign in {name}")
                                })
                                .flex_none()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.switching = None;
                                    this.sign_in_again(&again, window, cx);
                                })),
                        );
                    } else if yard.is_far() {
                        let again = name.clone();
                        second = second.child(
                            small_button("pair again".to_string(), palette)
                                .id(SharedString::from(format!("switch again {name}")))
                                .debug_selector({
                                    let name = name.clone();
                                    move || format!("switch again {name}")
                                })
                                .flex_none()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.switching = None;
                                    this.pair_again(Some(again.clone()), window, cx);
                                })),
                        );
                    }
                }
                None => {
                    second = second.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(palette.faint)
                            .child(Self::yard_reach(yard)),
                    );
                }
            }
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
                    .flex()
                    .flex_col()
                    .when(place == at, |el| el.bg(palette.open))
                    .hover(|style| style.bg(palette.hover))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.switch_to(&name, window, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            // The yard's dot, in its colour: the row's
                            // name reads with it, as the rail's chip and
                            // a card's header do.
                            .child(
                                div()
                                    .debug_selector({
                                        let name = yard.name.clone();
                                        move || format!("switch dot {name}")
                                    })
                                    .flex_none()
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(yard_color(&yard.name, palette.dark)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .when(open, |el| el.font_weight(FontWeight::SEMIBOLD))
                                    .child(yard.name.clone()),
                            )
                            .child(
                                div()
                                    .debug_selector({
                                        let name = yard.name.clone();
                                        move || format!("switch kind {name}")
                                    })
                                    .flex_none()
                                    .px_1()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(palette.line)
                                    .text_xs()
                                    .text_color(palette.faint)
                                    .child(yard.kind()),
                            )
                            .child(count)
                            .child(
                                div()
                                    .debug_selector({
                                        let name = yard.name.clone();
                                        move || format!("switch key {name}")
                                    })
                                    .flex_none()
                                    .text_xs()
                                    .text_color(palette.faint)
                                    .child(number_key(place).unwrap_or_default()),
                            ),
                    )
                    .child(second),
            );
        }
        // The footer's hint on one line: the hints elide where the list
        // is narrower than them, and the actions stand last, unshrunk,
        // so "+ add a yard" and "? all keys" are always on it. One
        // line, since a wrapping footer inside a content-sized list
        // mis-measures: on Linux at the largest text size the footer
        // stood one line tall with its last child laid out on a
        // second, past the list's edge.
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(palette.faint)
                .debug_selector(|| "switch hint".to_string())
                .child(
                    // No grow: the hints stand left, as ever, and only
                    // give way where the list is narrower than them.
                    div()
                        .min_w_0()
                        .truncate()
                        .child("↑↓ move · ↵ open · esc close ·"),
                )
                .child(
                    div()
                        .id("switch add")
                        .debug_selector(|| "switch add".to_string())
                        .flex_none()
                        .cursor_pointer()
                        .hover(|style| style.text_color(palette.text))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_yard_from_switcher(window, cx)
                        }))
                        .child("+ add a yard"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .child("·")
                        .child(keys_hint("keys hint switch", palette, cx).flex_none()),
                ),
        );
        // The list between its narrowest and its widest, capped at the
        // window's width less the rail and 8 pixels' margin each side, so
        // it never runs past the window's edge. In rems, with the text.
        let rem = f32::from(window.rem_size());
        let avail = (f32::from(window.viewport_size().width) - (RAIL + 16.)).max(0.) / rem;
        let max = avail.min(SWITCHER_MAX);
        let min = SWITCHER_MIN.min(max);
        // Level with the chip, which stands below the title bar's room
        // where the content reaches up into it.
        let top = titlebar::inset(self.means.titlebar.is_some(), window.is_fullscreen());
        div()
            .absolute()
            .left(px(RAIL + 8.))
            .top(px(8. + top))
            .min_w(rems(min))
            .max_w(rems(max))
            .debug_selector(|| "switcher".to_string())
            .rounded_lg()
            .border_1()
            .border_color(palette.line)
            .bg(palette.ground)
            .child(list)
    }
    /// The away screen's first line where the yard was reached before and
    /// is not now: which yard, and why in a word or two.
    pub(crate) fn away_why(&self) -> Option<String> {
        let short = self.unreached.get(&self.yard).copied()?;
        Some(format!("{} · {short}", self.which_yard_short()))
    }
    /// The away screen's first words: a yard that is only stopped says
    /// that, with where to start it; where no yard was ever made, one
    /// plain sentence says so and what to do, with no socket, no error
    /// and no yardr command in it; anywhere else the telling stands.
    pub(crate) fn away_lead(&self, why: &str) -> String {
        if self.yard_is_stopped() {
            return "This yard is stopped. Start it with Start, at the top of the list."
                .to_string();
        }
        if self.sets_up_here() {
            return "No yard has been made here yet — set one up.".to_string();
        }
        why.to_string()
    }
    /// Enter for the away screen's details: where no yard was made and no
    /// way's fields are open, they open, or shut again. Anywhere else the
    /// key is a field's, the line's or the switcher's, and this takes
    /// nothing: it answers beside those, and never stops the key.
    pub(crate) fn away_details_key(&mut self, cx: &mut Context<Self>) {
        if self.down.is_none()
            || self.adding.is_some()
            || self.settings_open
            || self.first_run.is_some()
            || self.switching.is_some()
            || !self.sets_up_here()
        {
            return;
        }
        self.toggle_away_details(cx);
    }
    /// The away screen's details line, where no yard was ever made: shut
    /// at first, it opens on a click or Enter.
    pub(crate) fn away_details_line(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.away_details;
        div()
            .id("away details")
            .debug_selector(|| "away details".to_string())
            .text_sm()
            .text_color(palette.faint)
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_away_details(cx)))
            .child(format!("{} details", if open { "▾" } else { "▸" }))
    }
    /// What the open details hold: the error and where it was looked for,
    /// as they stood on the screen before.
    pub(crate) fn away_details_body(
        &self,
        why: &str,
        place: Option<String>,
        palette: Palette,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "away details why".to_string())
                    .child(why.to_string()),
            )
            .children(place.map(|place| {
                div()
                    .debug_selector(|| "away details place".to_string())
                    .text_color(palette.faint)
                    .child(place)
            }))
    }
    /// No yard: which, and why in a word or two, then why in words, where
    /// it was looked for, and whether it is tried again. Where no yard
    /// was ever made the words are one plain sentence, and the socket,
    /// the error and the hint at yardr status hide under details, shut at
    /// first; "set up a yard here" is the first way, and the marked one.
    /// A yard that no longer lets this device in is not: the device is
    /// paired again from here. One whose session the edge no longer lets
    /// in says to sign in again, and pairing is not offered. A yard that
    /// is only stopped says that, and points at Start at the top of the
    /// list. It stands beside the rail, whose head opens another yard;
    /// the one yard the environment names has no other. Under the words
    /// are the ways to a yard from here, which are
    /// the settings' own: each asks for what the settings ask for a yard
    /// that is added, in the same fields, and opens the yard it adds.
    /// Where the yard answers under a way's open fields, one line says so
    /// in place of whether it is tried again, with the key that opens it.
    pub(crate) fn away(
        &self,
        why: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let place = self.client.as_ref().map(Client::place);
        let alone = self.env.yard.is_some();
        let setup_here = self.sets_up_here();
        let again = match self.away {
            Away::Trying => format!(
                "footplate tries again every {} seconds.",
                session::RETRY.as_secs()
            ),
            Away::Out => {
                "footplate does not ask again until this device has a token again.".to_string()
            }
            Away::SignedOut => {
                "footplate does not ask again until signed in again.".to_string()
            }
            Away::Stopped if alone => {
                "footplate does not try again: the settings open a yard, and so does the next start."
                    .to_string()
            }
            Away::Stopped => {
                "footplate does not try again: the switcher opens another yard, and so do the settings."
                    .to_string()
            }
        };
        let answers = self.answers_under_adding();
        let pair_again =
            self.client.as_ref().is_some_and(Client::is_remote) && self.away != Away::SignedOut;
        let sign_in_again = self.env.yard.is_none()
            && self
                .settings
                .yard(&self.yard)
                .is_some_and(|yard| yard.is_hosted());
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
                    .children(self.away_why().map(|told| {
                        div()
                            .debug_selector(|| "away why".to_string())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(told)
                    }))
                    .children((!answers).then(|| {
                        div()
                            .debug_selector(|| "away lead".to_string())
                            .child(self.away_lead(why))
                    }))
                    // Where no yard was ever made the socket, the error and
                    // the hint at yardr status hide under details, shut at
                    // first; anywhere else they stand as they did.
                    .children(setup_here.then(|| self.away_details_line(palette, cx)))
                    .children(
                        (setup_here && self.away_details)
                            .then(|| self.away_details_body(why, place.clone(), palette)),
                    )
                    .children(
                        (!setup_here)
                            .then(|| place.clone())
                            .flatten()
                            .map(|place| div().text_color(palette.faint).child(place)),
                    )
                    // Where the yard answers under a way's open fields,
                    // one line says so, with the key that opens it, in
                    // place of whether it is tried again.
                    .children(answers.then(|| {
                        div()
                            .debug_selector(|| "away answers".to_string())
                            .text_color(palette.faint)
                            .child("This yard answers again: esc opens it.")
                    }))
                    .children((!answers).then(|| div().text_color(palette.faint).child(again))),
            )
            // The ways to a yard: one entry each, and a further way is one
            // more. The environment's yard is the one there is, and has
            // none.
            .when(!alone, |el| {
                // A yard on this machine is reached by its socket, which
                // Windows has not: the first way is left out there.
                let ways = [
                    ("way near", "open a yard on this machine", Adding::Near),
                    ("way far", "pair with a yard at an address", Adding::Far),
                    ("way hosted", "open a hosted yard", Adding::Hosted),
                ]
                .into_iter()
                .filter(|(_, _, adding)| cfg!(unix) || *adding != Adding::Near);
                el.child(
                    // Three divs: the max stands on a block of its own, so
                    // the column under it has its width before its texts
                    // measure. A flex column with the max itself measures
                    // them at full width and keeps one line's height once
                    // the max clamps it, and the help spills over the row
                    // under the ways.
                    div().w_full().child(
                        div().max_w(rems(32.5)).mx_auto().child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    // The ways stand one to a line, not beside
                                    // each other: a wrapping row under the
                                    // column's width keeps the height of one line
                                    // and spills the rest over whatever stands
                                    // under it.
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_start()
                                        .gap_2()
                                        // The first way makes the yard that is
                                        // not there: this run's, and no other. It
                                        // stands filled, as the one to take.
                                        .children(setup_here.then(|| {
                                            card_button(
                                                "way here".to_string(),
                                                "set up a yard here".to_string(),
                                                palette,
                                                Some(cx.listener(|this, _, window, cx| {
                                                    this.set_up_here(window, cx)
                                                })),
                                            )
                                        }))
                                        .children(ways.map(|(id, text, adding)| {
                                            small_button(text.to_string(), palette)
                                                .id(id)
                                                .debug_selector(move || id.to_string())
                                                .when(self.adding == Some(adding.clone()), |el| {
                                                    el.bg(palette.open)
                                                })
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.add_yard(adding.clone(), window, cx)
                                                    },
                                                ))
                                        })),
                                )
                                .children(self.yard_asked(palette, cx)),
                        ),
                    ),
                )
            })
            // The actions stand in their own row under the ways, in the
            // ways' own column, so no button ever stands on a way's
            // lines. A yard at an address that is away for whatever
            // reason: the device can be paired again, which is what
            // helps when the yard no longer knows it. Where the session
            // is what is out, pairing again helps nothing, so it is not
            // offered. A hosted yard that is away: its session can be
            // signed in again, which is what helps when the edge no
            // longer knows it. The one yard the environment names is
            // reached direct, and is signed in nowhere.
            .when(pair_again || sign_in_again, |el| {
                el.child(
                    // Two divs, not one, as reading_column has it:
                    // `w_full` with `max_w` keeps the height its text
                    // has at full width once the max clamps it.
                    div().w_full().child(
                        div()
                            .max_w(rems(32.5))
                            .mx_auto()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(pair_again.then(|| {
                                small_button("pair again".to_string(), palette)
                                    .id("pair again")
                                    .debug_selector(|| "pair again".to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pair_again(None, window, cx)
                                    }))
                            }))
                            .children(sign_in_again.then(|| {
                                small_button("sign in again".to_string(), palette)
                                    .id("sign in again")
                                    .debug_selector(|| "sign in again".to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let name = this.yard.clone();
                                        this.sign_in_again(&name, window, cx)
                                    }))
                            })),
                    ),
                )
            })
    }
    /// The yard row open: the notices, oldest first, and no line to type
    /// in. One key marks one read, and with shift every one, as the button
    /// in the row's head does while a notice is unread.
    pub(crate) fn talk_yard(
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
                    Said::Mail(mut entry) => {
                        // The notices stand between the mails, so no mail
                        // reads under another's name here: every one carries
                        // its own.
                        entry.head = true;
                        return self.entry(entry, palette, cx).into_any_element();
                    }
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
                    // A hook delivery's files, as a mail's are drawn.
                    .children(n.files.iter().map(|file| {
                        div()
                            .pl_4()
                            .pt_1()
                            .child(self.picture(file, false, palette, cx))
                    }))
                    .into_any_element()
            }))
        };
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
            .child(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_baseline()
                    .gap_1()
                    .text_sm()
                    .text_color(palette.faint)
                    .child("x marks one notice read, X all of them; mail is read when shown here")
                    .child("·")
                    .child(keys_hint("keys hint yard", palette, cx)),
            )
    }
}
