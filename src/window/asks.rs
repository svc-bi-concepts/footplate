//! The decisions' views: the decisions list with its decided history,
//! each open ask's card with its secret, run and setup blocks, and what
//! the cards do: answer, approve, give, decline, put aside.

use std::path::Path;

use gpui::{
    AnyElement, Context, Div, Entity, Focusable, FontWeight, MouseButton, SharedString, Window,
    div, prelude::*, px, relative, rems, rgba,
};

use super::Expanded;
use super::Footplate;
use super::ask::{
    ANSWER_WORDS, APPROVE_ELSEWHERE, Approval, DECLINE_IS_NO_PICK, RUN_PROPOSED, RunText,
    SECRET_GOES, SEND_WORDS, SETUP_PROPOSED, SETUP_TAKE, WEIGHTY_NO_GATE, answer_label, approval,
    approve_button, asked_age, candidate_choice_line, card_head, carried_texts, decided_keys,
    decision_head, description_line, fenced, give_label, give_unlock_label, recommended_line,
    run_keys, run_runs, run_texts, secret_keys, section_head, send_button, setup_keys, setup_runs,
    show_button, take_back_label, texts_label, words_send,
};
use super::expanded_text_key;
use super::keys::{APPROVE_KEY, SETUP_SECRET_CONTEXT};
use super::rail::{RailTip, UnsentNote, post_note};
use super::state::Adding;
use super::ui::{keys_hint, send_held, small_button};
use super::words::{decided_words, waiting_words};
use super::{AskAbout, Decline, Expand, FocusFilter, FocusWords};
use crate::api::{self};
use crate::draft::{self, Attachment};
use crate::input::Line;
use crate::model::{
    self, AnswerAction, DecidedEntry, DecidedFold, DecidedRow, DecisionAsk, DecisionDetail,
    DecisionRow, Effect, Mode, Model, Selected, Target,
};
use crate::select::{self};
use crate::theme::{Palette, yard_color};
use crate::unlock;

impl Footplate {
    /// The line says whom it writes to, or what words on a card do.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
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
                    "Words wanted: Enter sends the answer (i)".to_string()
                } else if self.current_foreign() {
                    // The yard refuses a plain note on another yard's
                    // bead: words go with a pick, and talking to the asker
                    // is "ask about this" below.
                    "Words with the pick (i)".to_string()
                } else {
                    "Words with the pick, or alone as a note (i)".to_string()
                }
            }
            Some(Selected::Decided(_, _)) => "Words with the pick".to_string(),
            _ => String::new(),
        };
        self.line
            .update(cx, |line, cx| line.set_placeholder(&to, cx));
        // What is typed of a secret is for the ask it was typed on, and is
        // kept for no other and for no return to this one. The toggle
        // stands as the ask says: shown where the yard marks its value
        // no secret, hidden elsewhere.
        let ask = self.open_ask();
        if ask != self.secret_for {
            self.forget_secret(cx);
            self.secret_for = ask.clone();
            let shown = ask.as_ref().is_some_and(|(bead, ask)| {
                self.model
                    .as_ref()
                    .is_some_and(|m| !m.concealed(bead, *ask))
            });
            self.set_secret_shown(shown, cx);
            // The secret shots pose a value in the field, hidden and
            // shown: a debug build's alone, as the settings and the
            // draft of Footplate::opened.
            #[cfg(debug_assertions)]
            if matches!(
                std::env::var("FOOTPLATE_SHOT").as_deref(),
                Ok("secret" | "secret-shown")
            ) {
                self.secret
                    .update(cx, |field, cx| field.set_text("bk-live-3f9a-7Qx", cx));
            }
            // The refused shot poses the yard's refusal of the answer
            // on the card, the field empty as a refused giving leaves
            // it: a debug build's alone, as above.
            #[cfg(debug_assertions)]
            if std::env::var("FOOTPLATE_SHOT").as_deref() == Ok("secret-refused") {
                let refused = api::Error::Refused {
                    code: "error".to_string(),
                    message: "ask 7 asks for BOOKS_TOKEN: BOOKS_TOKEN was not given".to_string(),
                };
                self.refused.insert(
                    ("bead-s".to_string(), 7),
                    model::not_given(&refused, "bead-s", 7),
                );
            }
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
                    || ((m.run(bead, *ask).is_some() || m.setup(bead, *ask).is_some())
                        && self.declining.as_ref() != Some(open))
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
        let wanted = self
            .giving()
            .map(|secret| {
                let at = self.secret_typed.len();
                let name = secret.env.get(at).cloned().unwrap_or_default();
                let of = match secret.env.len() {
                    0 | 1 => String::new(),
                    n => format!(" ({} of {n})", at + 1),
                };
                format!("{name} for the source {}{of}", secret.source)
            })
            .or_else(|| {
                self.setup_account().map(|(names, pack)| {
                    let at = self.secret_typed.len();
                    match names.get(at) {
                        Some(name) => {
                            let of = match names.len() {
                                0 | 1 => String::new(),
                                n => format!(" ({} of {n})", at + 1),
                            };
                            format!("{name} for the setup of {pack}{of}")
                        }
                        None => format!("typed — {APPROVE_KEY} approves"),
                    }
                })
            });
        self.secret.update(cx, |field, cx| {
            field.set_placeholder(&wanted.unwrap_or_default(), cx)
        });
    }

    /// What the open card's field takes, when its ask is a secret ask that
    /// can be given.
    pub(crate) fn giving(&self) -> Option<api::Secret> {
        let (bead, ask) = self.open_ask()?;
        self.model.as_ref()?.giving(&bead, ask).cloned()
    }

    /// The command the control on the open card approves, when its ask is
    /// a run ask that can be approved.
    pub(crate) fn approvable(&self) -> Option<api::Run> {
        let (bead, ask) = self.open_ask()?;
        self.model.as_ref()?.approving(&bead, ask).cloned()
    }

    /// Whether the open card is a run ask's, open or dealt with.
    pub(crate) fn run_card(&self) -> bool {
        self.open_ask().is_some_and(|(bead, ask)| {
            let model = self.model.as_ref();
            model.is_some_and(|m| m.run(&bead, ask).is_some())
        })
    }

    /// The setup the control on the open card approves, when its ask is
    /// a setup ask that can be approved.
    pub(crate) fn setup_approvable(&self) -> Option<api::Setup> {
        let (bead, ask) = self.open_ask()?;
        self.model.as_ref()?.setup_approving(&bead, ask).cloned()
    }

    /// What the open card's field takes beside the preset pick, when its
    /// ask is a setup ask that can be approved: the account names the
    /// answer types — the missing ones where the update newly needs
    /// them — with the pack they set up.
    pub(crate) fn setup_account(&self) -> Option<(Vec<String>, String)> {
        let setup = self.setup_approvable()?;
        Some((model::setup_typed_names(&setup).to_vec(), setup.pack))
    }

    /// Nothing typed of a secret stays in the window: not in the field, and
    /// not among the values of an ask of several names.
    pub(crate) fn forget_secret(&mut self, cx: &mut Context<Self>) {
        self.secret_typed.clear();
        self.secret.update(cx, |field, cx| field.set_text("", cx));
    }

    /// Draws the secret as typed, for a look, or as marks again: the
    /// card's toggle, for the ask the field is for.
    pub(crate) fn set_secret_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.secret_shown = shown;
        self.secret
            .update(cx, |field, cx| field.set_shown(shown, cx));
    }

    /// Whether the open card's bead is another yard's question.
    pub(crate) fn current_foreign(&self) -> bool {
        self.model
            .as_ref()
            .and_then(|m| m.current_ask())
            .is_some_and(|(bead, _)| model::foreign_bead(&bead))
    }

    /// Whether the open card's current ask wants words: it has no options.
    pub(crate) fn current_options_empty(&self) -> bool {
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
    pub(crate) fn schedule_return(&mut self, cx: &mut Context<Self>) {
        self.schedule_return_in(self.session, cx);
    }

    /// Reads the queue of the following `session` again when the next
    /// put-aside ask comes back, which arrives with no event: of a yard
    /// that is held as of the open one, since what waits in it is told of
    /// and counted. One timer waits at a time for a yard; a queue read
    /// since replaces it. The queue is read once for a return: a yard that
    /// still lists the ask as put aside after its time is not asked again
    /// for it, and what the stream brings says when it is back.
    pub(crate) fn schedule_return_in(&mut self, session: u64, cx: &mut Context<Self>) {
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

    /// Says why the ask's answer did not go on its card, in the yard of
    /// the following `session`: on the open one's card at once, and on
    /// one that is held when the window shows it. A yard no longer
    /// followed has no card to say it on.
    pub(crate) fn refused_in(
        &mut self,
        session: u64,
        at: (String, u64),
        why: String,
        cx: &mut Context<Self>,
    ) {
        if session == self.session {
            self.refused.insert(at, why);
        } else if let Some(held) = self.held.values_mut().find(|h| h.session == session) {
            held.refused.insert(at, why);
        }
        cx.notify();
    }

    /// Words for the ask `at` of `yard` that the yard did not take are
    /// back with what was typed since: in the line while that yard is
    /// still the open one, as it was opened then, which is answered, and
    /// else with the ask they were written for, in its own yard. Why
    /// they are back is the caller's to say.
    pub(crate) fn words_back(
        &mut self,
        (yard, opening): (&str, u64),
        at: (String, u64),
        words: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if opening == self.opening {
            self.line.update(cx, |line, cx| {
                let back = draft::rejoin(words, line.text().to_string());
                line.set_text(&back, cx);
            });
            return true;
        }
        let kept = match yard == self.yard {
            true => &mut self.ask_words,
            false => self.kept_words.entry(yard.to_string()).or_default(),
        };
        let typed = kept.remove(&at).unwrap_or_default();
        kept.insert(at, draft::rejoin(words, typed));
        false
    }

    /// The open card's ask: the current one of a decision, or the dealt
    /// with itself.
    pub(crate) fn card(&self) -> Option<(String, u64)> {
        // The settings stand where the card does: its keys are not for a
        // card that is not in view.
        if self.settings_open {
            return None;
        }
        self.open_ask()
    }

    /// The ask of the card that is open, in view or under the settings.
    pub(crate) fn open_ask(&self) -> Option<(String, u64)> {
        match self.model.as_ref()?.selected()? {
            Selected::Decision(_) => self.model.as_ref()?.current_ask(),
            Selected::Decided(bead, ask) => Some((bead.clone(), *ask)),
            _ => None,
        }
    }

    /// Whether the Mac's unlock stands in front of a weighty approval:
    /// where the system offers its gate, or where a test fixed it.
    pub(crate) fn unlock_available(&self) -> bool {
        self.test_unlock
            .map(|(offered, _)| offered)
            .unwrap_or_else(unlock::available)
    }

    /// What the Mac's unlock answers where a test fixed it: the gate is
    /// then answered at once, with no prompt. None asks the system.
    fn unlock_answer(&self) -> Option<bool> {
        self.test_unlock.map(|(_, answer)| answer)
    }

    /// How the ask's card approves: straight to the yard, through the
    /// Mac's unlock first, or nowhere from this window.
    pub(crate) fn card_approval(&self, bead: &str, ask: u64) -> Approval {
        let weighty = self
            .model
            .as_ref()
            .is_some_and(|m| !m.weight(bead, ask).is_empty());
        let elsewhere = self.approve_elsewhere.contains(&(bead.to_string(), ask));
        approval(weighty, self.unlock_available(), elsewhere)
    }

    /// The approval of the following `session` is no longer with the
    /// gate: its control sends again.
    fn clear_approving_in(&mut self, session: u64) {
        if session == self.session {
            self.approving = None;
        } else if let Some(held) = self.held.values_mut().find(|h| h.session == session) {
            held.approving = None;
        }
    }

    /// Opens the Mac's unlock for a weighty approval or giving: `send`
    /// goes where the person passes it, with `weighed`, and `cancel` puts
    /// the card back where they do not, sending nothing. The approval is
    /// the yard's of `session`: what came of it is that yard's, and is
    /// said there, wherever the window is by then.
    fn unlock_then(
        &mut self,
        session: u64,
        reason: String,
        cx: &mut Context<Self>,
        send: impl FnOnce(&mut Self, u64, &mut Context<Self>) + 'static,
        cancel: impl FnOnce(&mut Self, u64, &mut Context<Self>) + 'static,
    ) {
        // A test fixed the gate: answered at once, with no prompt.
        if let Some(passed) = self.unlock_answer() {
            return match passed {
                true => send(self, session, cx),
                false => cancel(self, session, cx),
            };
        }
        let asked = unlock::evaluate(&reason);
        cx.spawn(async move |this, cx| {
            let passed = asked.recv().await.unwrap_or(false);
            this.update(cx, |this, cx| match passed {
                true => send(this, session, cx),
                false => cancel(this, session, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Enter in the field of a secret ask: the value goes to the yard in
    /// its own call and in nothing else. The field is emptied here, before
    /// the yard is asked and whatever it answers, and what the window says
    /// afterwards is the yard's words or footplate's, on the card, never
    /// the value. An ask of several names takes them one after the other
    /// and sends them together. Enter in the field of a setup ask takes
    /// one account name the same way, and the approval sends them with
    /// the preset pick.
    pub(crate) fn give(&mut self, cx: &mut Context<Self>) {
        // Held as a conversation's sends are: the value stays in the
        // field, with the reason beside Give.
        if self.reconnecting() {
            return;
        }
        let Some((bead, ask)) = self.card() else {
            return;
        };
        // A secret ask's names go in their own call; a setup ask's wait
        // for the approval, which sends them with the preset pick.
        let secret = self.giving();
        let setup = self.setup_account();
        let names = match (&secret, &setup) {
            (Some(secret), _) => secret.env.clone(),
            (None, Some((names, _))) => names.clone(),
            (None, None) => return,
        };
        // What is pasted often ends in a line break, which nobody sees
        // among the marks and which the yard refuses at either end.
        let value = self
            .secret
            .update(cx, |field, cx| field.take_text(cx))
            .trim()
            .to_string();
        let Some(name) = names.get(self.secret_typed.len()) else {
            return self.forget_secret(cx);
        };
        if value.is_empty() {
            return self.notice(format!("nothing is typed for {name}"), cx);
        }
        self.secret_typed.push(value);
        self.notice(String::new(), cx);
        self.refused.remove(&(bead.clone(), ask));
        if self.secret_typed.len() < names.len() {
            return self.shown(cx);
        }
        if secret.is_none() {
            return self.shown(cx);
        }
        let secret = secret.unwrap();
        let reason = unlock::give_reason(&secret.env, &self.yard);
        let values: Vec<(String, String)> = secret
            .env
            .into_iter()
            .zip(std::mem::take(&mut self.secret_typed))
            .collect();
        self.shown(cx);
        let session = self.session;
        match self.card_approval(&bead, ask) {
            // No gate: the card shows where to give instead, and offers
            // no control. What was typed is kept, as what was typed since.
            Approval::Nowhere => {
                self.secret_typed = values.into_iter().map(|(_, value)| value).collect();
                self.shown(cx);
            }
            // The Mac's unlock first: its cancel sends nothing, and what
            // was typed is kept for another try: the last value back in
            // the field, the rest typed, so Enter tries again. Where the
            // window left the yard meanwhile, what was typed is dropped
            // instead: it is that yard's, and is not restored into
            // another's field.
            Approval::Unlock => {
                let back: Vec<String> = values.iter().map(|(_, value)| value.clone()).collect();
                self.unlock_then(
                    session,
                    reason,
                    cx,
                    move |this, session, cx| {
                        this.send_give_in(session, bead, ask, values, true, cx)
                    },
                    move |this, session, cx| {
                        if session == this.session {
                            let mut back: Vec<String> = back;
                            if let Some(last) = back.pop() {
                                this.secret
                                    .update(cx, |field, cx| field.set_text(&last, cx));
                            }
                            this.secret_typed = back;
                        }
                        this.shown(cx);
                    },
                );
            }
            Approval::Direct => self.send_give_in(session, bead, ask, values, false, cx),
        }
    }

    /// Gives the typed values to the yard in its own call and in nothing
    /// else. What the window says afterwards is the yard's words or
    /// footplate's, on the card, never the value. `weighed` goes where
    /// the person passed the Mac's unlock for a weighty card.
    pub(crate) fn send_give_in(
        &mut self,
        session: u64,
        bead: String,
        ask: u64,
        values: Vec<(String, String)>,
        weighed: bool,
        cx: &mut Context<Self>,
    ) {
        let call_bead = bead.clone();
        // What the yard answers is of the yard it was given in, and is
        // said there, wherever the window is by then.
        self.ask_in(
            session,
            cx,
            move |client, me| {
                client
                    .give_secret(&call_bead, ask, &values, me, weighed)
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
                Err(why) => this.refused_in(session, (bead.clone(), ask), why, cx),
            },
        );
    }

    /// Approves the open card's run or setup ask: its key and its button.
    /// A run ask's approval names the ask and nothing of the command,
    /// which the yard runs as it stored it; a setup ask's the preset pick
    /// with the account together. A weighty card's approval passes the
    /// Mac's unlock first, and a card that approves nowhere has no
    /// control: its card says where to approve instead.
    pub(crate) fn approve(&mut self, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        let at = (bead.clone(), ask);
        if self.approve_elsewhere.contains(&at)
            || self.approving.is_some()
            || self.client.is_none()
            || self.reconnecting()
            || matches!(self.card_approval(&bead, ask), Approval::Nowhere)
        {
            return;
        }
        if let Some(run) = self.approvable() {
            if matches!(self.card_approval(&bead, ask), Approval::Direct) {
                return self.send_approve(bead, ask, cx);
            }
            // The Mac's unlock first: its cancel sends nothing, and the
            // control sends again.
            let reason = unlock::approve_reason(&run.line(), &self.yard);
            let session = self.session;
            self.declining = None;
            self.approving = Some(at.clone());
            self.shown(cx);
            self.notice(String::new(), cx);
            self.refused.remove(&at);
            self.unlock_then(
                session,
                reason,
                cx,
                move |this, session, cx| this.send_approve_in(session, bead, ask, true, cx),
                move |this, session, cx| {
                    this.clear_approving_in(session);
                    this.shown(cx);
                },
            );
        } else if self.setup_approvable().is_some() {
            self.approve_setup(bead, ask, cx);
        }
    }

    /// Approves the open card's run ask. What the window says afterwards
    /// is the yard's: the command as it ran with what it printed, or the
    /// refusal whole, on the card. A call that ended without an answer is
    /// never said to be approved or not, and a yard without the call for
    /// this window has its card say where to approve.
    pub(crate) fn send_approve(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        self.send_approve_in(self.session, bead, ask, false, cx);
    }

    /// Approves a run ask of the following `session`. `weighed` goes
    /// where the person passed the Mac's unlock for a weighty card.
    fn send_approve_in(
        &mut self,
        session: u64,
        bead: String,
        ask: u64,
        weighed: bool,
        cx: &mut Context<Self>,
    ) {
        let at = (bead.clone(), ask);
        if session == self.session {
            self.declining = None;
            self.approving = Some(at.clone());
            self.shown(cx);
            self.notice(String::new(), cx);
            self.refused.remove(&at);
        } else {
            let Some(held) = self.held.values_mut().find(|h| h.session == session) else {
                return;
            };
            held.approving = Some(at.clone());
            held.refused.remove(&at);
            self.notice_in(session, String::new(), cx);
        }
        // The approval is the yard's it was pressed in: what came of it is
        // that yard's, and is said there, wherever the window is by then.
        self.ask_in(
            session,
            cx,
            move |client, me| client.approve_ask(&bead, ask, me, weighed),
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
                    Err(err) => {
                        this.refused_in(session, (bead.clone(), ask), model::not_approved(&err), cx)
                    }
                }
            },
        );
    }

    /// Approves the open card's setup ask: the preset pick, the option
    /// chosen or the recommended one, with the account together. A filled
    /// secret field types another; else the tap takes its file; else a
    /// drift card whose standing file holds every name is kept with the
    /// bare preset pick. What the window says afterwards is the yard's:
    /// that the server carries the setup out, or the refusal whole, on
    /// the card. A call that ended without an answer is never said to be
    /// approved or not, and a yard without the call for this window has
    /// its card say where to approve.
    pub(crate) fn approve_setup(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        let at = (bead.clone(), ask);
        let Some(option) = self.model.as_ref().and_then(|m| m.setup_pick(&bead, ask)) else {
            let unread = !self.model.as_ref().is_some_and(|m| m.has_bead(&bead));
            return self.notice(
                match unread {
                    true => "reading the bead: approve once its presets show".to_string(),
                    false => "pick a preset first".to_string(),
                },
                cx,
            );
        };
        let Some(setup) = self.setup_approvable() else {
            return;
        };
        // The field's value goes with what Enter took, as the next name;
        // what is pasted often ends in a line break, which nobody sees
        // among the marks and which the yard refuses at either end.
        let field = self.secret.read(cx).text().trim().to_string();
        let mut typed = self.secret_typed.clone();
        if !field.is_empty() {
            typed.push(field);
        }
        let names = model::setup_typed_names(&setup);
        if names.is_empty() {
            return self.notice(
                "the yard asked for no account names: read the card again".to_string(),
                cx,
            );
        }
        if typed.len() > names.len() {
            return self.notice(
                "the account is typed: clear the field to approve it".to_string(),
                cx,
            );
        }
        let have = typed.len();
        let values: Option<Vec<(String, String)>> = match have == names.len() {
            true => Some(names.iter().cloned().zip(typed).collect()),
            false => None,
        };
        let account = match (&values, self.model.as_ref()) {
            (None, Some(model)) => match model.account_choice(&bead, ask) {
                taken if taken.is_empty() => None,
                taken => Some(taken),
            },
            _ => None,
        };
        if values.is_none() && account.is_none() {
            // A drift card whose standing file holds every account name
            // is kept with the bare preset pick: with nothing typed and
            // no candidate taken the approval goes as it stands. What is
            // typed instead changes the account, and is wanted whole.
            let keep = have == 0 && model::setup_keepable(&setup);
            if !keep {
                if have == 0 {
                    return self.notice(
                        match setup.candidates.is_empty() {
                            true => "type the account".to_string(),
                            false => "take a candidate, or type the account".to_string(),
                        },
                        cx,
                    );
                }
                let next = &names[have];
                return self.notice(format!("type {next} for the account"), cx);
            }
        }
        let session = self.session;
        if !matches!(self.card_approval(&bead, ask), Approval::Direct) {
            // The Mac's unlock first: its cancel sends nothing, and the
            // control sends again. `approve` turns a card that approves
            // nowhere away before this.
            let reason = unlock::setup_reason(&setup.pack, &self.yard);
            self.declining = None;
            self.approving = Some(at.clone());
            self.shown(cx);
            self.notice(String::new(), cx);
            self.refused.remove(&at);
            return self.unlock_then(
                session,
                reason,
                cx,
                move |this, session, cx| {
                    this.send_approve_setup_in(
                        session, bead, ask, option, values, account, setup, true, cx,
                    )
                },
                move |this, session, cx| {
                    this.clear_approving_in(session);
                    this.shown(cx);
                },
            );
        }
        self.send_approve_setup_in(
            session, bead, ask, option, values, account, setup, false, cx,
        );
    }

    /// Approves a setup ask of the following `session`: the preset pick
    /// with the account together. `weighed` goes where the person passed
    /// the Mac's unlock for a weighty card. What the window says
    /// afterwards is the yard's: that the server carries the setup out,
    /// or the refusal whole, on the card.
    #[allow(clippy::too_many_arguments)]
    fn send_approve_setup_in(
        &mut self,
        session: u64,
        bead: String,
        ask: u64,
        option: u64,
        values: Option<Vec<(String, String)>>,
        account: Option<String>,
        setup: api::Setup,
        weighed: bool,
        cx: &mut Context<Self>,
    ) {
        let at = (bead.clone(), ask);
        if session == self.session {
            self.declining = None;
            self.approving = Some(at.clone());
            self.shown(cx);
            self.notice(String::new(), cx);
            self.refused.remove(&at);
        } else {
            let Some(held) = self.held.values_mut().find(|h| h.session == session) else {
                return;
            };
            held.approving = Some(at.clone());
            held.refused.remove(&at);
            self.notice_in(session, String::new(), cx);
        }
        // The approval is the yard's it was pressed in: what came of it is
        // that yard's, and is said there, wherever the window is by then.
        self.ask_in(
            session,
            cx,
            move |client, me| {
                let approved = client.approve_setup(
                    &bead,
                    ask,
                    option,
                    values.as_deref(),
                    account.as_deref(),
                    me,
                    weighed,
                );
                (approved, values)
            },
            move |this, (approved, values), cx| {
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
                        let line = model::setup_approved_line(&setup, &approved);
                        this.forget_secret(cx);
                        this.change_in(session, cx, |m| {
                            m.setup_approved(&bead, ask, option, &approved)
                        });
                        this.schedule_return_in(session, cx);
                        this.notice_in(session, line, cx);
                    }
                    Err(err) if model::approves_elsewhere(&err) => {
                        elsewhere.insert(at);
                        this.notice_in(session, String::new(), cx);
                    }
                    Err(err) => {
                        let down = matches!(err, api::Error::Down(_));
                        let why = model::not_approved(&err);
                        // The yard repeats no value in a refusal. Should
                        // anything on the way do so, it is not shown.
                        let echoes = values.as_ref().is_some_and(|values| {
                            values.iter().any(|(_, value)| why.contains(value.as_str()))
                        });
                        let why = match echoes {
                            true => "not approved: the yard refused it".to_string(),
                            false => why,
                        };
                        this.refused_in(session, (bead.clone(), ask), why, cx);
                        // A call that ended without an answer may have
                        // approved all the same: the queue is read again
                        // for what the yard says.
                        if down {
                            this.change_in(session, cx, |_| vec![Effect::Asks]);
                        }
                    }
                }
            },
        );
    }

    /// Takes the setup ask's candidate with a tap: whatever is typed of
    /// another is forgotten, since a filled secret field types another
    /// instead.
    pub(crate) fn take_account_for(
        &mut self,
        bead: String,
        ask: u64,
        source: String,
        cx: &mut Context<Self>,
    ) {
        let approving = self
            .model
            .as_ref()
            .is_some_and(|m| m.setup_approving(&bead, ask).is_some());
        if !approving {
            return;
        }
        self.forget_secret(cx);
        self.change(cx, |m| {
            m.take_account(&bead, ask, &source);
            Vec::new()
        });
        self.shown(cx);
    }

    /// Enter on a card: the pick with the line's words in the one answer,
    /// or the words alone. Enter with nothing chosen and a recommendation
    /// picks the recommended option.
    pub(crate) fn decide(&mut self, cx: &mut Context<Self>) {
        // Answers on a card are held as a conversation's sends are.
        if self.reconnecting() {
            return;
        }
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
            if !self.card_files(&bead, ask).is_empty() {
                return self.notice("a decline carries no file: remove it, then send".into(), cx);
            }
            return self.send_decline(bead, ask, words, cx);
        }
        let words = self.line.read(cx).text().to_string();
        let action = self
            .model
            .as_ref()
            .and_then(|m| m.answer_action(&bead, ask, &words));
        // A file attached to the card's note goes with words alone: an
        // answer carries none, and where the words go nowhere as a note,
        // the file stays until it is removed.
        if !self.card_files(&bead, ask).is_empty() {
            return match action {
                Some(AnswerAction::Note { words }) => {
                    match draft::note_unsendable(&words, self.card_files(&bead, ask)) {
                        Some(why) => self.notice(why, cx),
                        None => self.send_note(bead, ask, words, cx),
                    }
                }
                Some(AnswerAction::Answer { .. }) => {
                    self.notice("an answer carries no file: remove it, then send".into(), cx)
                }
                None => match draft::note_unsendable(&words, self.card_files(&bead, ask)) {
                    Some(why) => self.notice(why, cx),
                    None => self.notice(
                        "words alone go nowhere on this ask: remove the file, then send".into(),
                        cx,
                    ),
                },
            };
        }
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
    /// says. What the yard refuses is back in the line, with its words,
    /// and why is on the card.
    pub(crate) fn send_answer(
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
        self.refused.remove(&(bead.clone(), ask));
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
                    if this.yard_identity(&yard) != identity {
                        return this.notice(
                            format!(
                                "not answered in {yard}: that yard entry was removed or changed, so its words were not restored"
                            ),
                            cx,
                        );
                    }
                    this.words_back((&yard, opening), (bead.clone(), ask), words, cx);
                    this.refused_in(session, (bead, ask), format!("not answered: {err}"), cx);
                }
            },
        );
    }

    /// Words with no option chosen go as a plain note, with the card's
    /// files; the ask stays open. What the yard refuses is back in the
    /// line with its files, and the file that kept it is marked.
    pub(crate) fn send_note(
        &mut self,
        bead: String,
        ask: u64,
        words: String,
        cx: &mut Context<Self>,
    ) {
        let files = self
            .note_files
            .remove(&(bead.clone(), ask))
            .unwrap_or_default();
        self.line.update(cx, |line, cx| line.set_text("", cx));
        self.ask_words.remove(&(bead.clone(), ask));
        self.notice(String::new(), cx);
        let (call_bead, call_words) = (bead.clone(), words.clone());
        let (yard, opening) = (self.yard.clone(), self.opening);
        let identity = self.yard_identity(&yard);
        self.ask_through(
            cx,
            move |client, me| match post_note(client, me, &call_bead, &call_words, &files) {
                Ok(()) => Ok(()),
                Err((at, why)) => Err(UnsentNote {
                    words: call_words,
                    files,
                    at,
                    why,
                }),
            },
            move |this, noted, cx| match noted {
                Ok(()) => {
                    // The note arrives as a `noted` event, which reads the
                    // queue and the bead again.
                }
                Err(_) if this.yard_identity(&yard) != identity => this.notice(
                    format!(
                        "not noted in {yard}: that yard entry was removed or changed, so its words were not restored"
                    ),
                    cx,
                ),
                Err(unsent) => this.note_back(&yard, opening, (bead, ask), unsent, cx),
            },
        );
    }

    /// The files attached to the ask's note, in the order they came.
    pub(crate) fn card_files(&self, bead: &str, ask: u64) -> &[Attachment] {
        self.note_files
            .get(&(bead.to_string(), ask))
            .map_or(&[], Vec::as_slice)
    }

    /// Words for the ask `at` of `yard` that the yard did not take are
    /// back with what was typed since, and their files before what was
    /// attached since, the file that kept them marked: in the line while
    /// that yard is still the open one, as it was opened then, and else
    /// with the ask they were written for, in its own yard.
    pub(crate) fn note_back(
        &mut self,
        yard: &str,
        opening: u64,
        at: (String, u64),
        unsent: UnsentNote,
        cx: &mut Context<Self>,
    ) {
        let kept = match yard == self.yard {
            true => &mut self.note_files,
            false => self.kept_note_files.entry(yard.to_string()).or_default(),
        };
        let files = kept.entry(at.clone()).or_default();
        let mut back = unsent.files;
        back.append(files);
        *files = back;
        if let Some(id) = unsent.at {
            for file in files.iter_mut().filter(|a| a.id == id) {
                file.refused = Some(unsent.why.clone());
            }
        }
        let in_line = self.words_back((yard, opening), at, unsent.words, cx);
        match in_line {
            true => self.notice(format!("not noted: {}", unsent.why), cx),
            false => self.notice(format!("not noted in {yard}: {}", unsent.why), cx),
        }
    }

    /// Puts the ask aside with `x`: it leaves the list and returns as the
    /// yard says.
    pub(crate) fn send_later(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        // Held with the card's answers: the words stay where they are.
        if self.reconnecting() {
            return;
        }
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
    pub(crate) fn send_decline(
        &mut self,
        bead: String,
        ask: u64,
        words: String,
        cx: &mut Context<Self>,
    ) {
        // Held with the card's answers.
        if self.reconnecting() {
            return;
        }
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
                    if this.yard_identity(&yard) != identity {
                        return this.notice(
                            format!(
                                "not declined in {yard}: that yard entry was removed or changed, so its words were not restored"
                            ),
                            cx,
                        );
                    }
                    let in_line =
                        this.words_back((&yard, opening), (bead, ask), words, cx);
                    match in_line {
                        true => this.notice(format!("not declined: {err}"), cx),
                        false => this.notice(format!("not declined in {yard}: {err}"), cx),
                    }
                }
            },
        );
    }

    /// Puts the open card's current ask aside: the `x` key and the Later
    /// button. Later takes no words: only an open ask, never one dealt with.
    pub(crate) fn later_now(&mut self, cx: &mut Context<Self>) {
        let current = self.model.as_ref().and_then(|m| m.current_ask());
        if let Some((bead, ask)) = current {
            self.send_later(bead, ask, cx);
        }
    }

    /// A number key chooses an option of the open card.
    pub(crate) fn pick(&mut self, option: u64, cx: &mut Context<Self>) {
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
    pub(crate) fn choice(&mut self, by: isize, cx: &mut Context<Self>) {
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
    pub(crate) fn decline(&mut self, _: &Decline, window: &mut Window, cx: &mut Context<Self>) {
        self.decline_now(window, cx);
    }

    /// Declines the open card's current ask: the `d` key and the Decline
    /// button.
    pub(crate) fn decline_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.model.as_ref().and_then(|m| m.current_ask());
        let Some((bead, ask)) = current else {
            return;
        };
        // A secret ask is declined with no words: the yard takes none on
        // it, since words there could be the value.
        if self.giving().is_some() {
            // Held with the card's answers: the value stays in the
            // field.
            if self.reconnecting() {
                return;
            }
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
    pub(crate) fn ask_about(&mut self, _: &AskAbout, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_about_now(window, cx);
    }

    /// Opens the conversation with the open card's asker, with the card's
    /// reference first in its draft: `re <bead> "<question>"`, then an
    /// empty line with the caret on it. Nothing is sent; what the draft
    /// held is kept after the reference, which is put once. The ask stays
    /// open among the decisions: its row is one step back. An asker with
    /// no conversation says so instead.
    pub(crate) fn ask_about_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // The asker is a chat row. Save the decision row in its own mode
        // and move the rail with the conversation before opening it.
        self.set_mode(Mode::Chat, window, cx);
        self.select(Selected::Conversation(asker), window, cx);
        self.line.update(cx, |line, cx| line.set_cursor(caret, cx));
    }

    /// One key expands the card in place to the bead's body, notes and
    /// about-beads, or folds it again.
    pub(crate) fn expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        let Some((bead, ask)) = self.card() else {
            return;
        };
        self.change(cx, |m| {
            m.toggle_expand(&bead, ask);
            Vec::new()
        });
    }

    /// The line has the words: `i` types them in, where a number key picks.
    pub(crate) fn focus_words(
        &mut self,
        _: &FocusWords,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        // A weighty card without the gate draws no field to type in: its
        // approval goes in a terminal, and a decline's words take the
        // line.
        if self.declining.is_none() && matches!(self.card_approval(&bead, ask), Approval::Nowhere) {
            return;
        }
        // A setup ask's card has the secret field for its account, as a
        // secret ask's has it for its value.
        let field = match self.giving().is_some() || self.setup_approvable().is_some() {
            true => &self.secret,
            false => &self.line,
        };
        window.focus(&field.focus_handle(cx));
    }

    /// `/` types in the history's filter: the folded history stands open
    /// for it. Elsewhere than decisions the key is nothing.
    pub(crate) fn focus_filter(
        &mut self,
        _: &FocusFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode() != Mode::Decisions
            || !self
                .model
                .as_ref()
                .is_some_and(|model| model.decided_count() > 0)
        {
            return;
        }
        self.change(cx, |m| {
            if !m.decided_expanded() {
                m.toggle_decided();
            }
            Vec::new()
        });
        window.focus(&self.filter.focus_handle(cx));
    }

    /// Makes the ask the bead's current one: the keys act on it. What is
    /// typed stays with the ask it was typed in.
    pub(crate) fn select_ask(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
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
    pub(crate) fn pick_for(&mut self, bead: String, ask: u64, option: u64, cx: &mut Context<Self>) {
        if self.model.as_ref().and_then(|m| m.current_for(&bead)) != Some(ask) {
            self.select_ask(bead.clone(), ask, cx);
        }
        self.pick(option, cx);
    }

    /// Clicks a card's expand: the bead's body, notes and about-beads.
    pub(crate) fn expand_for(&mut self, bead: String, ask: u64, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_expand(&bead, ask);
            Vec::new()
        });
    }

    /// The fields of the yard that is being added, signed in or
    /// paired again, with the button under them, what came of the last
    /// change to the yards under that, and the help under it: under the
    /// yards in the settings, and under the ways to a yard where there
    /// is none. Renaming stands in the renamed card instead, said beside
    /// its form.
    pub(crate) fn yard_asked(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut asked: Vec<AnyElement> = Vec::new();
        let busy = self.pairing || self.signing_in;
        let pair = match self.pairing {
            true => "asking the yard…",
            false => "pair (enter)",
        };
        let sign = match self.signing_in {
            true => "signing in…",
            false => "sign in (enter)",
        };
        const OPERATOR: &str = "The yard's operator opens a pairing for this device and gives you the address and the code: yardr user device pair <user> <device>. A code is for one yard, and is used up there.";
        // What came of the last change stands directly under the button,
        // where a person looks the moment it is pressed: the help stands
        // under that. Below the help, a refusal falls off a short window
        // while the button stays in it, and the press looks silent.
        let mut help: Option<AnyElement> = None;
        match &self.adding {
            None => {}
            Some(Adding::Near) => {
                asked.extend([
                    yard_field("name", &self.yard_name, palette, cx).into_any_element(),
                    yard_field("socket", &self.socket, palette, cx).into_any_element(),
                    yard_submit("add (enter)", "yard pair", busy, palette, cx).into_any_element(),
                ]);
                help = Some(
                    yard_help(
                        "A yard of this machine is reached by its socket: no token or pairing. This field starts at this run's socket; change it to another home's yard socket (<its home>/.yardr/yard.sock)."
                            .to_string(),
                        palette,
                    )
                    .into_any_element(),
                );
            }
            Some(Adding::Far) => {
                asked.extend([
                    yard_field("name", &self.yard_name, palette, cx).into_any_element(),
                    yard_field("address", &self.address, palette, cx).into_any_element(),
                    yard_field("code", &self.code, palette, cx).into_any_element(),
                    yard_submit(pair, "yard pair", busy, palette, cx).into_any_element(),
                ]);
                help = Some(
                    yard_help(format!(
                        "{OPERATOR} With no name it is called by its address's host. The device's token is kept in a file of this yard's own beside the settings, for you alone, and never with the settings."
                    ), palette)
                    .into_any_element(),
                );
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
                asked.extend([
                    div()
                        .debug_selector(|| "yard again for".to_string())
                        .child(format!("a code for {whose}, at {address}"))
                        .into_any_element(),
                    yard_field("code", &self.code, palette, cx).into_any_element(),
                    yard_submit(pair, "yard pair", busy, palette, cx).into_any_element(),
                ]);
                help = Some(yard_help(format!("{OPERATOR} {token}"), palette).into_any_element());
            }
            Some(Adding::Hosted) => {
                asked.extend([
                    yard_field("name", &self.yard_name, palette, cx).into_any_element(),
                    yard_field("address", &self.address, palette, cx).into_any_element(),
                    yard_field("email", &self.email, palette, cx).into_any_element(),
                    yard_field("password", &self.password, palette, cx).into_any_element(),
                    yard_submit(sign, "yard sign in", busy, palette, cx).into_any_element(),
                ]);
                help = Some(
                    yard_help(
                        "The edge's one origin, as the ready page shows it, and the Kratos account's email and password. Signing in keeps the session in a file of this yard's own beside the settings, for you alone, and never with the settings; pairing the device comes next, with the code from the ready page."
                            .to_string(),
                        palette,
                    )
                    .into_any_element(),
                );
            }
            Some(Adding::SignInAgain(name)) => {
                let listed = self.settings.yard(name);
                let address = listed.map(|yard| yard.address.clone()).unwrap_or_default();
                let session = match self
                    .env
                    .session_of(&self.settings, name, self.kept.as_deref())
                {
                    Ok(Some(file)) => format!(
                        "The session is kept in {}, for you alone, and never with the settings.",
                        file.display()
                    ),
                    Ok(None) => format!("The yard {name} is no hosted yard."),
                    Err(why) => why,
                };
                asked.extend([
                    div()
                        .debug_selector(|| "yard sign in for".to_string())
                        .child(format!("a sign-in for {name}, at {address}"))
                        .into_any_element(),
                    yard_field("email", &self.email, palette, cx).into_any_element(),
                    yard_field("password", &self.password, palette, cx).into_any_element(),
                    yard_submit(sign, "yard sign in", busy, palette, cx).into_any_element(),
                ]);
                help = Some(
                    yard_help(
                        format!("The Kratos account's email and password. {session}"),
                        palette,
                    )
                    .into_any_element(),
                );
            }
            // Renaming stands in the renamed card, where its menu stood,
            // not here under the cards: nothing of it is asked for here.
            Some(Adding::Rename(_)) => {}
        }
        // What came of the last change, except while a yard is renamed:
        // that is said in its card, beside the form.
        if !matches!(self.adding, Some(Adding::Rename(_)))
            && let Some(said) = &self.yard_said
        {
            asked.push(
                div()
                    .debug_selector(|| "yard said".to_string())
                    .child(said.clone())
                    .into_any_element(),
            );
        }
        asked.extend(help);
        asked
    }

    pub(crate) fn toggle_decided(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |m| {
            m.toggle_decided();
            Vec::new()
        });
    }

    /// Decisions' list: the open questions, each with what it wants,
    /// grouped by topic where a topic has two asks or more, with the
    /// Decided toggle at the list's end: decided asks are unbounded
    /// history, and without the toggle they would push open decisions
    /// off a short list. Open, the history has its filter with its chips,
    /// and its days with their rows, look-alikes folded.
    pub(crate) fn list_decisions(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let now = chrono::Utc::now();
        let mut body = self.list_body();
        let decisions = model.decisions_count();
        let decided = model.decided_count();
        body = body.child(section_head("Decisions", waiting_words(decisions), palette));
        for group in model.decision_groups(&now) {
            // A topic above a single ask is noise: the head stands only
            // where two asks or more share the topic.
            let asks: usize = group.rows.iter().map(|row| row.asks.len().max(1)).sum();
            if asks > 1 {
                body = body.child(
                    div()
                        .debug_selector({
                            let topic = group.topic.clone();
                            move || format!("decisions topic {topic}")
                        })
                        .px_3()
                        .pt_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.faint)
                        .child(group.topic),
                );
            }
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
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_decided(cx)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .child(format!("{} Decided", if open { "▾" } else { "▸" })),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .text_color(palette.faint)
                            .child(decided_words(decided)),
                    ),
            );
            if open {
                body = body.child(self.decided_filter(palette, cx));
                body = body.child(self.decided_chips(model, palette, cx));
                let groups = model.decided_groups(&chrono::Local::now());
                if groups.iter().all(|group| group.rows.is_empty()) {
                    body = body.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_sm()
                            .text_color(palette.faint)
                            .child("Nothing decided matches."),
                    );
                }
                for group in groups {
                    if group.rows.is_empty() {
                        continue;
                    }
                    body = body.child(
                        div()
                            .debug_selector({
                                let day = group.day.clone();
                                move || format!("decided day {day}")
                            })
                            .px_3()
                            .pt_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette.faint)
                            .child(group.day),
                    );
                    for row in group.rows {
                        match row {
                            DecidedEntry::One(row) => {
                                body = body.child(self.decided_row(row, palette, cx));
                            }
                            DecidedEntry::Fold(fold) => {
                                body = body.child(self.decided_fold(fold.clone(), palette, cx));
                                if fold.open {
                                    for row in fold.rows {
                                        body = body.child(self.decided_row(row, palette, cx));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if model.is_operator() {
            body = body.child(self.grants_row(model, palette, cx));
        }
        self.column(body, palette, cx)
    }

    /// The history's filter field: `/` types in it, and every word typed
    /// matches a row's question, asker or outcome.
    pub(crate) fn decided_filter(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field = self.filter.clone();
        let focus = field.focus_handle(cx);
        div()
            .id("decided filter")
            .debug_selector(|| "decided filter".to_string())
            .mx_1()
            .mt_1()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(palette.line)
            .bg(palette.side)
            .text_sm()
            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                window.focus(&focus);
                window.prevent_default();
            })
            .child(field)
    }

    /// The history's chips: All with the four outcomes after it, each with
    /// how many rows choosing it would show.
    pub(crate) fn decided_chips(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut chips = div().mx_1().px_2().py_1().flex().flex_wrap().gap_1();
        for (chip, count) in model.decided_chip_counts() {
            let word = match chip {
                None => "All".to_string(),
                Some(outcome) => outcome.chip().to_string(),
            };
            let current = model.decided_chip() == chip;
            let id = match chip {
                None => "decided chip all".to_string(),
                Some(outcome) => format!("decided chip {}", outcome.word()),
            };
            chips = chips.child(
                div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector({
                        let id = id.clone();
                        move || id.clone()
                    })
                    .px_2()
                    .rounded_full()
                    .border_1()
                    .text_xs()
                    .cursor_pointer()
                    .when(current, |el| {
                        el.bg(palette.accent)
                            .border_color(palette.accent)
                            .text_color(palette.on_accent)
                    })
                    .when(!current, |el| {
                        el.border_color(palette.line)
                            .text_color(palette.faint)
                            .hover(|style| style.bg(palette.hover))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |m| {
                            m.set_decided_chip(chip);
                            Vec::new()
                        })
                    }))
                    .child(format!("{word} {count}")),
            );
        }
        chips
    }

    /// One bead's open row: the question wrapped to two lines, then
    /// whose it is, what it wants and how long it waits. A bead with
    /// several open asks carries its title instead, with each ask beneath
    /// it in the same shape.
    pub(crate) fn decision_row(
        &self,
        row: DecisionRow,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let sel = Selected::Decision(row.bead.clone());
        let mut col = div()
            .flex()
            .flex_col()
            // Resolve the available row width before measuring the
            // head, so it wraps within the row.
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .line_clamp(2)
                    .debug_selector(|| format!("decision head {}", row.bead))
                    .child(row.head),
            );
        for ask in &row.asks {
            let seq = ask.ask;
            col = col
                .child(
                    div()
                        .line_clamp(2)
                        .debug_selector(|| format!("decision ask {seq} {}", row.bead))
                        .child(ask.head.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.faint)
                        .truncate()
                        .debug_selector(|| format!("decision meta {seq} {}", row.bead))
                        .child(ask.meta.clone()),
                );
        }
        if !row.sub.is_empty() {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(palette.faint)
                    .truncate()
                    .debug_selector(|| format!("decision sub {}", row.bead))
                    .child(row.sub),
            );
        }
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
            .border_1()
            .border_color(rgba(0x00000000))
            .when(row.selected, |el| el.bg(palette.open))
            .when(!row.selected, |el| {
                el.hover(|style| style.border_color(palette.line))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.select(sel.clone(), window, cx)))
            .child(col)
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(palette.faint)
                    .debug_selector(|| format!("decision id {}", row.bead))
                    .child(row.short),
            )
    }

    /// The grants row, last of decisions where the window is the
    /// operator's: what is granted, in so many calls, or that none is,
    /// or that the yard is too old to say.
    pub(crate) fn grants_row(
        &self,
        model: &Model,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let selected = model.selected() == Some(&Selected::Grants);
        let sub = match model.grants_too_old() {
            true => "yard too old".to_string(),
            false => {
                let calls: usize = model.grant_records().iter().map(|g| g.calls.len()).sum();
                match calls {
                    0 => "none granted".to_string(),
                    1 => "1 call granted".to_string(),
                    n => format!("{n} calls granted"),
                }
            }
        };
        div()
            .id("row grants")
            .debug_selector(|| "row grants".to_string())
            .mx_1()
            .px_2()
            .py_1p5()
            .rounded_md()
            .flex()
            .flex_col()
            .cursor_pointer()
            .border_1()
            .border_color(rgba(0x00000000))
            .when(selected, |el| el.bg(palette.open))
            .when(!selected, |el| {
                el.hover(|style| style.border_color(palette.line))
            })
            .on_click(cx.listener(|this, _, window, cx| this.select(Selected::Grants, window, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .debug_selector(|| "grants head".to_string())
                    .child(div().flex_1().truncate().child("Grants")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette.faint)
                    .truncate()
                    .debug_selector(|| "grants sub".to_string())
                    .child(sub),
            )
    }

    /// One dealt with, with its outcome's icon first, then whose it was
    /// and when it was asked. The outcome stands in the text colour, the
    /// rest quiet, each cut with an ellipsis where it does not fit.
    pub(crate) fn decided_row(
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
                            .flex()
                            .flex_row()
                            .items_baseline()
                            .gap_1()
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(relative(0.6))
                                    .text_xs()
                                    .truncate()
                                    .debug_selector(|| {
                                        format!("decided outcome {} {}", row.bead, row.ask)
                                    })
                                    .child(row.outcome),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(palette.faint)
                                    .truncate()
                                    .child(format!("· {}", row.detail)),
                            ),
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

    /// Several look-alikes folded into one row: the first's question with
    /// the shared outcome and asker. A click opens the fold to show them,
    /// or folds it shut again; the count with ▸ and ▾ stands where one
    /// row's bead id does, past what the head's cut would hide.
    pub(crate) fn decided_fold(
        &self,
        fold: DecidedFold,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bead = fold.bead.clone();
        let ask = fold.ask;
        let mark = if fold.open { "▾" } else { "▸" };
        let count = format!("×{} {mark}", fold.rows.len());
        div()
            .id(SharedString::from(format!(
                "decided fold {} {}",
                fold.bead, fold.ask
            )))
            .debug_selector(|| format!("decided fold {} {}", fold.bead, fold.ask))
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
            .hover(|style| style.border_color(palette.line))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.change(cx, |m| {
                    m.toggle_decided_fold(&bead, ask);
                    Vec::new()
                })
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .debug_selector(|| {
                                format!("decided fold head {} {}", fold.bead, fold.ask)
                            })
                            .child(fold.head),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_baseline()
                            .gap_1()
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .debug_selector(|| {
                                        format!("decided fold outcome {} {}", fold.bead, fold.ask)
                                    })
                                    .child(fold.outcome),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(palette.faint)
                                    .truncate()
                                    .child(format!("· {}", fold.detail)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(palette.faint)
                    .child(count),
            )
    }

    /// A bead's open asks, each as its card: the yard, the bead and the
    /// topic once in the head, then the question, the asker, the context
    /// with the bead whole one disclosure away, the numbered options with
    /// the recommended one preselected, a line for words, and the answer
    /// button with Later, ask-about-this and Decline beside it.
    pub(crate) fn talk_decision(
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
            .debug_selector(|| format!("in {} {}", self.yard, detail.bead))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
                div()
                    .debug_selector(|| format!("in {} {} dot", self.yard, detail.bead))
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .bg(yard_color(&self.yard, palette.dark)),
            )
            .child(decision_head(&self.yard, &detail.bead, &detail.topic));
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

    /// One card of an open bead: the question, the largest text; the asker
    /// and how long it waits; the context with the bead whole one
    /// disclosure away; the numbered options, the recommended one
    /// preselected and marked with its why under it; a line for words; the
    /// answer button with Later, ask-about-this and Decline beside it, each
    /// key on its control. Only the current one takes the keys, the line
    /// and the buttons.
    pub(crate) fn decision_card(
        &self,
        detail: &DecisionDetail,
        ask: &DecisionAsk,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bead_id = detail.bead.clone();
        let ask_seq = ask.ask;
        let card_key = select::key((bead_id.clone(), ask_seq));
        let current = ask.current;
        // The option Enter and the button answer: the pick, else the
        // recommendation, which stands preselected.
        let answered = if ask.chosen == 0 {
            ask.recommend
        } else {
            ask.chosen
        };
        let mut card = div().flex().flex_col().gap_2();
        // How the card approves: nowhere where the yard takes no approval
        // from this window, or where a weighty card has no gate.
        let elsewhere = self.approve_elsewhere.contains(&(bead_id.clone(), ask_seq));
        let approval = approval(!ask.weight.is_empty(), self.unlock_available(), elsewhere);
        let sends = !matches!(approval, Approval::Nowhere);
        // The yard's weight first, each class's line as the terminal
        // prints it, in rust: a weighty card reads apart from a routine
        // one, and never in red.
        for (n, weight) in ask.weight.iter().enumerate() {
            let line = model::weight_line(&weight.class, &weight.line);
            card = card.child(
                div()
                    .debug_selector(|| format!("weight {bead_id} {ask_seq} {n}"))
                    .text_color(palette.text_accent)
                    .child(select::text(
                        expanded_text_key(card_key, "weight", n, &line),
                        line,
                        &self.selection,
                    )),
            );
        }
        // The question, with how long it waits beside it. The yard and the
        // bead stand once in the head above, not again on every card.
        let asker_marked = ask.run.is_some() || ask.setup.is_some();
        let question = if asker_marked {
            self.marked_question(
                card_key,
                format!("run question {bead_id} {ask_seq}"),
                &ask.question,
                &ask.asker,
            )
            .into_any_element()
        } else {
            select::text(
                expanded_text_key(card_key, "question", 0, &ask.question),
                ask.question.clone(),
                &self.selection,
            )
            .into_any_element()
        };
        card = card.child(
            div()
                .flex()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .debug_selector(|| format!("question {bead_id} {ask_seq}"))
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(question),
                )
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(asked_age(&ask.waits)),
                ),
        );
        // The asker. The recommendation stands on its own option, below.
        card = card.child(
            div()
                .text_sm()
                .text_color(palette.faint)
                .child(format!("asked by {}", ask.asker)),
        );
        // The context lines: text as it was written, and no block in it.
        // A run or setup ask's each begin with who says them, and wrap
        // under the words.
        for (line, text) in ask.context.iter().enumerate() {
            if asker_marked {
                card = card.child(self.marked_asker_line(
                    card_key,
                    "context",
                    line,
                    format!("run context {bead_id} {ask_seq} {line}"),
                    text,
                    &ask.asker,
                ));
            } else {
                card = card.child(
                    div()
                        .debug_selector(|| format!("context {bead_id} {ask_seq} {line}"))
                        .child(select::text(
                            expanded_text_key(card_key, "context", line, text),
                            text.clone(),
                            &self.selection,
                        )),
                );
            }
        }
        // The bead whole, one disclosure away: beside the context it opens
        // from, not at the card's end.
        let mark = if ask.expanded { "▾" } else { "▸" };
        let bead_click = bead_id.clone();
        card = card.child(
            div()
                .id(SharedString::from(format!("expand {bead_id} {ask_seq}")))
                .debug_selector(|| format!("expand {bead_id} {ask_seq}"))
                .text_sm()
                .text_color(palette.faint)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.expand_for(bead_click.clone(), ask_seq, cx)
                }))
                .child(format!("{mark} bead details (e)")),
        );
        if ask.expanded {
            card = card.child(self.expanded_body(
                card_key,
                Expanded {
                    body: &detail.body,
                    notes: &detail.notes,
                    about: &ask.about,
                    doc: &detail.doc,
                },
                palette,
                cx,
            ));
        }
        // The options numbered, each with its consequence, `you decide`
        // last as the yard lists it. A secret ask has none and wants no
        // words: what it asks for is said in the yard's line, as it is.
        // Nor has a run ask: the yard's block is the last of its card, with
        // the control that approves directly under it. A setup ask has the
        // yard's block with its account choice first, and its presets to
        // pick from under it, as an ordinary ask's options.
        if let Some(secret) = &ask.secret {
            card = card.child(self.secret_lines(&bead_id, ask_seq, secret, palette));
        } else if let Some(run) = &ask.run {
            card = card.child(self.run_block(&bead_id, ask_seq, run, true, palette, cx));
        } else {
            if let Some(setup) = &ask.setup {
                card = card.child(self.setup_block(&bead_id, ask_seq, setup, true, palette, cx));
                card = card.child(self.setup_candidates(
                    &bead_id,
                    ask_seq,
                    setup,
                    &ask.account,
                    palette,
                    cx,
                ));
            }
            if ask.options.is_empty() {
                if ask.setup.is_none() {
                    card = card.child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("words wanted"),
                    );
                }
            } else {
                let mut marked = false;
                for option in &ask.options {
                    let marker = if option.number == answered {
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
                            .when(n == answered, |el| el.bg(palette.open))
                            .when(n != answered, |el| {
                                el.hover(|style| style.bg(palette.hover))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pick_for(bead_click.clone(), ask_seq, n, cx)
                            }))
                            .child(text),
                    );
                    // The recommended one with its why, under its own row: the
                    // one place the card names the recommendation.
                    if ask.recommend != 0 && n == ask.recommend {
                        marked = true;
                        card = card.child(div().text_sm().child(recommended_line(&ask.why)));
                    }
                }
                // A recommendation the options do not hold stands past them, as
                // nothing else names it.
                if ask.recommend != 0 && !marked {
                    card = card.child(div().text_sm().child(recommended_line(&ask.why)));
                }
            }
        }
        if current {
            // A run or setup ask takes words only with a decline: its card
            // has the line while one is asked for. A setup ask's field
            // takes its account otherwise, as a secret ask's takes its
            // value.
            let declining = self.declining.as_ref() == Some(&(bead_id.clone(), ask_seq));
            // A line for words, or the field of a secret in its place. A
            // setup ask's account is typed in the secret field too. A card
            // that sends nowhere draws no field: its approval goes in a
            // terminal, and a decline's words take the line.
            let secret_field = (ask.secret.is_some() || ask.setup.is_some()) && !declining && sends;
            let field = match secret_field {
                true => self.secret.clone(),
                false => self.line.clone(),
            };
            let focus = field.focus_handle(cx);
            let kind = if secret_field { "secret" } else { "words" };
            // Where this yard takes no approval from this window, or a
            // weighty card has no gate, the card says where it does, in
            // place of the control.
            if (ask.run.is_some() || ask.setup.is_some()) && elsewhere {
                let command = match &ask.setup {
                    Some(setup) => model::setup_approve_command(&bead_id, ask_seq, setup),
                    None => model::approve_command(&bead_id, ask_seq),
                };
                card =
                    card.child(self.approve_elsewhere_lines(&bead_id, ask_seq, command, palette));
            } else if !sends && (ask.run.is_some() || ask.setup.is_some() || ask.secret.is_some()) {
                let command = match (&ask.secret, &ask.setup) {
                    (Some(_), _) => format!("yardr bead answer {bead_id} {ask_seq} --secret"),
                    (_, Some(setup)) => model::setup_approve_command(&bead_id, ask_seq, setup),
                    _ => model::approve_command(&bead_id, ask_seq),
                };
                card = card.child(self.weighty_decline_lines(&bead_id, ask_seq, command));
            }
            // A secret's field stands beside its toggle: Show draws what
            // is typed as typed, for a look, Hide draws marks again.
            let toggle = secret_field.then(|| {
                show_button(
                    format!("show {bead_id} {ask_seq}"),
                    self.secret_shown,
                    palette,
                    cx,
                )
            });
            card = card.when(
                (ask.run.is_none() || ask.setup.is_some() || declining) && (sends || declining),
                |card| {
                    let boxed = div()
                        .debug_selector(|| format!("{kind} {bead_id} {ask_seq}"))
                        // A setup ask's account field takes its approval's key
                        // too, with what the field holds.
                        .when(ask.setup.is_some() && secret_field, |el| {
                            el.key_context(SETUP_SECRET_CONTEXT)
                        })
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
                        .child(field);
                    match toggle {
                        Some(toggle) => card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(boxed.flex_1())
                                .child(toggle),
                        ),
                        None => card.child(boxed),
                    }
                },
            );
            // Under the line the files attached to the ask's note, with
            // why a marked one cannot go: what Enter sends with the
            // words. A file is drawn by its name: footplate shows no
            // preview of one, and an image small.
            let files = self.card_files(&bead_id, ask_seq);
            if !files.is_empty() {
                card = card
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .children(files.iter().map(|a| self.attachment(a, palette, cx))),
                    )
                    .children(draft::reasons(files).into_iter().map(|why| {
                        div()
                            .text_sm()
                            .text_color(palette.warn)
                            .child(format!("! {why}"))
                    }));
            }
            // Why the yard did not take the card's answer, in place: its
            // refusal or footplate's words for what else went wrong, as
            // other refusals are said. A new answer clears it.
            if let Some(why) = self.refused.get(&(bead_id.clone(), ask_seq)) {
                card = card.child(
                    div()
                        .debug_selector(|| format!("refused {bead_id} {ask_seq}"))
                        .text_sm()
                        .text_color(palette.warn)
                        .child(format!("! {why}")),
                );
            }
            // Under it the button that answers, exactly as Enter does,
            // with ask-about-this, Later and Decline beside it, each key on
            // its control. An asker with no conversation has the reason in
            // place of that button.
            let foreign = model::foreign_bead(&detail.bead);
            let send = match (&ask.secret, self.giving()) {
                (Some(_), Some(secret)) if sends => Some((
                    format!("give {bead_id} {ask_seq}"),
                    match approval {
                        Approval::Unlock => give_unlock_label(&secret, self.secret_typed.len()),
                        _ => give_label(&secret, self.secret_typed.len()),
                    },
                    !self.secret.read(cx).text().is_empty(),
                )),
                (Some(_), _) => None,
                // A run or setup ask's one button approves, and sends no
                // words.
                (None, _) if ask.run.is_some() || ask.setup.is_some() => None,
                (None, _) => ask
                    .options
                    .iter()
                    .find(|o| o.number == answered)
                    .map(|o| {
                        (
                            format!("send {bead_id} {ask_seq}"),
                            answer_label(o.number, &o.label),
                            true,
                        )
                    })
                    .or_else(|| {
                        words_send(ask, foreign).then(|| {
                            let words_answer = ask.options.is_empty();
                            (
                                format!("send {bead_id} {ask_seq}"),
                                if words_answer {
                                    ANSWER_WORDS.to_string()
                                } else {
                                    SEND_WORDS.to_string()
                                },
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
            // While the yard is reconnecting the card's answers are held
            // as the conversation's sends are: the buttons stand
            // disabled, with the reason beside them, and the keys hold
            // the words where they are.
            let held = self.reconnecting();
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(send, |el, (id, text, enabled)| {
                        el.child(send_button(id, text, enabled && !held, palette, cx))
                    })
                    .when((ask.run.is_some() || ask.setup.is_some()) && sends, |el| {
                        el.child(approve_button(
                            format!("approve {bead_id} {ask_seq}"),
                            &self.yard,
                            matches!(approval, Approval::Unlock),
                            self.approving.is_none() && !held,
                            palette,
                            cx,
                        ))
                    })
                    .when(held, |el| el.child(send_held(palette)))
                    .child(ask_about)
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
                            // Declining is not a pick, and a run ask has no
                            // pick to take it for: only another ask's button
                            // says so.
                            .when(ask.run.is_none(), |el| {
                                el.tooltip(move |_, cx| {
                                    cx.new(|_| RailTip(DECLINE_IS_NO_PICK.to_string())).into()
                                })
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.decline_now(window, cx)),
                            ),
                    ),
            );
            // The keys of a secret, run or setup ask's card: an
            // ordinary card carries each key on its control, and lists
            // none. Every card carries the cheatsheet's hint.
            let keys = match (&ask.secret, &ask.run, &ask.setup) {
                (Some(_), _, _) => secret_keys(&bead_id, ask_seq, approval, unwritable.is_none()),
                (None, Some(_), _) => run_keys(approval, unwritable.is_none()),
                (None, None, Some(setup)) => {
                    setup_keys(&bead_id, ask_seq, setup, approval, unwritable.is_none())
                }
                (None, None, None) => String::new(),
            };
            let prose = !keys.is_empty();
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_baseline()
                    .gap_1()
                    .text_sm()
                    .text_color(palette.faint)
                    .children(prose.then(|| div().child(keys)))
                    .children(prose.then(|| div().child("·")))
                    .child(keys_hint("keys hint card", palette, cx)),
            );
        } else {
            let bead_click = bead_id.clone();
            card = card.child(
                div()
                    .id(SharedString::from(format!("current {bead_id} {ask_seq}")))
                    .text_sm()
                    .text_color(palette.text_accent)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_ask(bead_click.clone(), ask_seq, cx)
                    }))
                    .child("decide this one"),
            );
        }
        card
    }

    /// A run ask's question lines, each with the asker's label beside its
    /// words. When a line wraps, its continuation stays under the words,
    /// where no line of the yard's card begins.
    pub(crate) fn marked_question(
        &self,
        card_key: u64,
        id: String,
        question: &str,
        asker: &str,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .children(question.lines().enumerate().map(|(line, text)| {
                self.marked_asker_line(
                    card_key,
                    "question",
                    line,
                    format!("{id} {line}"),
                    text,
                    asker,
                )
            }))
    }

    /// One run asker's line with the trusted label in its own column, so
    /// soft-wrapped words remain to the right of the label. The words
    /// select and copy, drawn as they were written with no block in
    /// them; the label stays plain, so a copy of the words does not
    /// carry it.
    pub(crate) fn marked_asker_line(
        &self,
        card_key: u64,
        section: &str,
        index: usize,
        id: String,
        line: &str,
        asker: &str,
    ) -> impl IntoElement + use<> {
        let prefix = format!("{asker} says: ");
        let words = line.strip_prefix(&prefix).unwrap_or(line).to_string();
        let label = prefix;
        let label_id = format!("asker label {id}");
        let words_id = format!("asker words {id}");
        let key = expanded_text_key(card_key, section, index, &words);
        div()
            .flex()
            .items_start()
            .child(
                div()
                    .flex_none()
                    .debug_selector(move || label_id.clone())
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(move || words_id.clone())
                    .child(select::verbatim(key, words, &self.selection)),
            )
    }

    /// What a secret ask asks for: the yard's line, which is made from the
    /// source's declaration and never from the asker's words, and where
    /// the value goes.
    pub(crate) fn secret_lines(
        &self,
        bead: &str,
        ask: u64,
        secret: &api::Secret,
        palette: Palette,
    ) -> impl IntoElement + use<> {
        let card_key = select::key((bead, ask));
        let key = expanded_text_key(card_key, "secret", 0, &secret.line);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| secret.line.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(select::text(key, secret.line.clone(), &self.selection)),
            )
            .child(div().text_sm().text_color(palette.faint).child(SECRET_GOES))
    }

    /// What a run ask proposes, in the one block of its card, which is all
    /// the yard's and none of it the asker's: the command whole, in a code
    /// block that wraps where the window is narrower and is never cut; the
    /// command's one line from its help; the yard's sentences for what it
    /// changes, each on its own line; the texts the ask holds behind one
    /// tap, for checking the yard's reading against them; and the command
    /// that takes it back, with who may run it. `open` is an ask that can
    /// still be approved. The approval is live whether or not the tap was
    /// opened.
    pub(crate) fn run_block(
        &self,
        bead: &str,
        ask: u64,
        run: &api::Run,
        open: bool,
        palette: Palette,
        cx: &mut Context<Self>,
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
        let description = model::plain(&run.description);
        if !description.is_empty() {
            let line = description_line(&description);
            let key = expanded_text_key(card_key, "description", 0, &line);
            block = block.child(
                div()
                    .debug_selector(|| format!("run description {bead} {ask}"))
                    .child(select::text(key, line, &self.selection)),
            );
        }
        for (ix, does) in run.does.iter().enumerate() {
            let line = format!("the yard says: {}", model::plain(does));
            let key = expanded_text_key(card_key, "does", ix, &line);
            block = block.child(
                div()
                    .debug_selector(|| format!("run does {bead} {ask} {ix}"))
                    .child(select::text(key, line, &self.selection)),
            );
        }
        block = block.child(self.block_end(
            "run",
            (bead, ask),
            &run_texts(run),
            (&run.back, &run.back_by),
            palette,
            cx,
        ));
        block
    }

    /// The end of a run or setup ask's block: the ask's own texts behind
    /// one tap, for checking the yard's reading against them, and the
    /// commands that take it back, with who may run them. `kind` is whose
    /// block it is, for the texts' selectors. The approval is live whether
    /// or not the tap was opened.
    pub(crate) fn block_end(
        &self,
        kind: &str,
        at: (&str, u64),
        texts: &[RunText],
        back: (&str, &str),
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (bead, ask) = at;
        let (back, back_by) = back;
        let card_key = select::key((bead, ask));
        let mut end = div().flex().flex_col().gap_1();
        // The ask's own texts, behind one tap: the approval is live
        // whether or not it was opened.
        let texts_shown = self
            .model
            .as_ref()
            .is_some_and(|m| m.texts_shown(bead, ask));
        if !texts.is_empty() {
            let bead_click = bead.to_string();
            end = end.child(
                div()
                    .id(SharedString::from(format!("texts {bead} {ask}")))
                    .debug_selector(|| format!("texts {bead} {ask}"))
                    .text_sm()
                    .text_color(palette.faint)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.texts_for(bead_click.clone(), ask, cx)
                    }))
                    .child(texts_label(texts.len(), texts_shown)),
            );
        }
        if texts_shown {
            for (ix, text) in texts.iter().enumerate() {
                end = end
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.faint)
                            .child(text.heading.clone()),
                    )
                    .child(
                        div()
                            .debug_selector(|| format!("{kind} text {bead} {ask} {ix}"))
                            .child(select::text_with_code_copy(
                                expanded_text_key(card_key, text.section, ix, &text.text),
                                fenced(&model::plain(&text.text)),
                                text.text.clone(),
                                &self.selection,
                            )),
                    );
            }
        }
        let back = model::plain(back);
        if !back.is_empty() {
            end = end
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.faint)
                        .child(take_back_label(&model::plain(back_by))),
                )
                .child(
                    div()
                        .debug_selector(|| format!("{kind} back {bead} {ask}"))
                        .child(select::text(
                            expanded_text_key(card_key, "back", 0, &back),
                            fenced(&back),
                            &self.selection,
                        )),
                );
        }
        end
    }

    /// What a setup ask proposes, in the one block of its card, which is
    /// all the yard's and none of it the asker's: what approving does, in
    /// one line; the yard's sentences for what it changes, each on its own
    /// line; the texts the ask holds behind one tap, for checking the
    /// yard's reading against them; and the commands that take it back,
    /// with who may run them. `open` is an ask that can still be approved.
    /// The approval is live whether or not the tap was opened.
    pub(crate) fn setup_block(
        &self,
        bead: &str,
        ask: u64,
        setup: &api::Setup,
        open: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let card_key = select::key((bead, ask));
        let over = match open {
            true => setup_runs(&self.yard),
            false => SETUP_PROPOSED.to_string(),
        };
        let summary = model::setup_summary(setup);
        let summary_key = expanded_text_key(card_key, "summary", 0, &summary);
        let mut block = div()
            .debug_selector(|| format!("setup {bead} {ask}"))
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
                    .debug_selector(|| format!("setup summary {bead} {ask}"))
                    .child(select::text(summary_key, summary, &self.selection)),
            );
        for (ix, does) in setup.does.iter().enumerate() {
            let line = format!("the yard says: {}", model::plain(does));
            let key = expanded_text_key(card_key, "does", ix, &line);
            block = block.child(
                div()
                    .debug_selector(|| format!("setup does {bead} {ask} {ix}"))
                    .child(select::text(key, line, &self.selection)),
            );
        }
        block = block.child(self.block_end(
            "setup",
            (bead, ask),
            &carried_texts(&setup.carried),
            (&setup.back, &setup.back_by),
            palette,
            cx,
        ));
        block
    }

    /// The setup ask's account choice: each candidate the yard lists with
    /// the account its file holds, to take with a tap. `taken` is the one
    /// taken, by source name, empty where none is.
    pub(crate) fn setup_candidates(
        &self,
        bead: &str,
        ask: u64,
        setup: &api::Setup,
        taken: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut choice = div().flex().flex_col().gap_1();
        if setup.candidates.is_empty() {
            return choice.into_any_element();
        }
        choice = choice.child(div().text_sm().text_color(palette.faint).child(SETUP_TAKE));
        for candidate in &setup.candidates {
            let is_taken = candidate.source == taken;
            let text = candidate_choice_line(candidate, taken);
            let source = candidate.source.clone();
            let bead_click = bead.to_string();
            choice = choice.child(
                div()
                    .id(SharedString::from(format!(
                        "account {bead} {ask} {}",
                        candidate.source
                    )))
                    .debug_selector(|| format!("account {bead} {ask} {}", candidate.source))
                    .cursor_pointer()
                    .rounded_md()
                    .when(is_taken, |el| el.bg(palette.open))
                    .when(!is_taken, |el| el.hover(|style| style.bg(palette.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.take_account_for(bead_click.clone(), ask, source.clone(), cx)
                    }))
                    .child(text),
            );
        }
        choice.into_any_element()
    }

    /// Where a run or setup ask is approved when this yard takes no
    /// approval from this window: a terminal on the yard's machine, with
    /// the command in a block that copies it.
    pub(crate) fn approve_elsewhere_lines(
        &self,
        bead: &str,
        ask: u64,
        command: String,
        palette: Palette,
    ) -> impl IntoElement + use<> {
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

    /// Where a weighty ask is approved when this window has no gate: the
    /// decline path, with the command in a block that copies it. Plain
    /// words, never in red: nothing went wrong, and the card stays open.
    pub(crate) fn weighty_decline_lines(
        &self,
        bead: &str,
        ask: u64,
        command: String,
    ) -> impl IntoElement + use<> {
        div()
            .debug_selector(|| format!("weighty decline {bead} {ask}"))
            .flex()
            .flex_col()
            .gap_1()
            .child(div().child(WEIGHTY_NO_GATE))
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
    pub(crate) fn talk_decided(
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
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| format!("in {} {bead} {ask} dot", self.yard))
                            .flex_none()
                            .size(px(8.))
                            .rounded_full()
                            .bg(yard_color(&self.yard, palette.dark)),
                    )
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
        let card_key = select::key((bead, ask));
        let asker_marked = detail.run.is_some() || detail.setup.is_some();
        let question = if asker_marked {
            self.marked_question(
                card_key,
                format!("run decided question {bead} {ask}"),
                &detail.question,
                &detail.asker,
            )
            .into_any_element()
        } else {
            select::text(
                expanded_text_key(card_key, "question", 0, &detail.question),
                detail.question.clone(),
                &self.selection,
            )
            .into_any_element()
        };
        // The yard's weight first, each class's line as the terminal
        // prints it, in rust: a weighty card reads apart from a routine
        // one, and never in red.
        for (n, weight) in detail.weight.iter().enumerate() {
            let line = model::weight_line(&weight.class, &weight.line);
            body = body.child(
                div()
                    .debug_selector(|| format!("weight {bead} {ask} {n}"))
                    .text_color(palette.text_accent)
                    .child(select::text(
                        expanded_text_key(card_key, "weight", n, &line),
                        line,
                        &self.selection,
                    )),
            );
        }
        body = body
            .child(
                div()
                    .debug_selector(|| format!("question {bead} {ask}"))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(question),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.state_line.clone()),
            );
        // Under an approved run ask's command, what it printed. One
        // the yard has not carried out yet printed nothing: its state
        // line says it is being carried out, and the pending words are
        // not what it printed.
        let printed = detail
            .ran
            .as_ref()
            .and_then(|ran| match ran.result.as_str() {
                model::CARRY_PENDING => None,
                _ => Some(model::plain(&ran.result)),
            });
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
            for (line, text) in detail.context.iter().enumerate() {
                if asker_marked {
                    body = body.child(self.marked_asker_line(
                        card_key,
                        "context",
                        line,
                        format!("run decided context {bead} {ask} {line}"),
                        text,
                        &detail.asker,
                    ));
                } else {
                    body = body.child(
                        div()
                            .debug_selector(|| format!("context {bead} {ask} {line}"))
                            .child(select::text(
                                expanded_text_key(card_key, "context", line, text),
                                text.clone(),
                                &self.selection,
                            )),
                    );
                }
            }
            // Whether a put-aside run or setup ask can still be approved
            // from here: one approved, declined or taken back takes no
            // approval.
            let approving = model.approving(bead, ask).is_some();
            let setup_approving = model.setup_approving(bead, ask).is_some();
            let elsewhere = self.approve_elsewhere.contains(&(bead.to_string(), ask));
            let approval = approval(
                !detail.weight.is_empty(),
                self.unlock_available(),
                elsewhere,
            );
            let sends = !matches!(approval, Approval::Nowhere);
            let giving = model.giving(bead, ask);
            // An ask with no options shows none: the answer's words are in
            // the state line, and nothing is wanted. Nor does one taken
            // back: there is nothing to pick. Nor a setup ask past its
            // approval: its presets are picked with the account, which a
            // put-aside one still takes below.
            let taken_back = detail.state == "taken back";
            if !detail.options.is_empty()
                && !taken_back
                && (detail.setup.is_none() || setup_approving)
            {
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
                                    .text_color(palette.text_accent)
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
                body = body.child(self.secret_lines(bead, ask, secret, palette));
                if giving.is_some() && !sends {
                    body = body.child(self.weighty_decline_lines(
                        bead,
                        ask,
                        format!("yardr bead answer {bead} {ask} --secret"),
                    ));
                }
            }
            // A run ask has the yard's block, and under it the control
            // that approves while it is only put aside: one approved,
            // declined or taken back takes no approval.
            if let Some(run) = &detail.run {
                body = body.child(self.run_block(bead, ask, run, approving, palette, cx));
                if approving && elsewhere {
                    body = body.child(self.approve_elsewhere_lines(
                        bead,
                        ask,
                        model::approve_command(bead, ask),
                        palette,
                    ));
                } else if approving && !sends {
                    body = body.child(self.weighty_decline_lines(
                        bead,
                        ask,
                        model::approve_command(bead, ask),
                    ));
                } else if approving {
                    body = body.child(div().flex().child(approve_button(
                        format!("approve {bead} {ask}"),
                        &self.yard,
                        matches!(approval, Approval::Unlock),
                        self.approving.is_none() && !self.reconnecting(),
                        palette,
                        cx,
                    )));
                }
            }
            // A setup ask has the yard's block with its account choice,
            // and under it the control that approves while it is only put
            // aside. Its presets stand to pick above.
            if let Some(setup) = &detail.setup {
                body = body.child(self.setup_block(bead, ask, setup, setup_approving, palette, cx));
                if setup_approving {
                    body = body.child(self.setup_candidates(
                        bead,
                        ask,
                        setup,
                        &detail.account,
                        palette,
                        cx,
                    ));
                    if elsewhere {
                        body = body.child(self.approve_elsewhere_lines(
                            bead,
                            ask,
                            model::setup_approve_command(bead, ask, setup),
                            palette,
                        ));
                    } else if !sends {
                        body = body.child(self.weighty_decline_lines(
                            bead,
                            ask,
                            model::setup_approve_command(bead, ask, setup),
                        ));
                    } else {
                        body = body.child(div().flex().child(approve_button(
                            format!("approve {bead} {ask}"),
                            &self.yard,
                            matches!(approval, Approval::Unlock),
                            self.approving.is_none() && !self.reconnecting(),
                            palette,
                            cx,
                        )));
                    }
                }
            }
            // A line for words with the later pick, the field of a secret
            // to give it again, or the field of a put-aside setup for its
            // account: a declined one takes none, one taken back has
            // neither, and no line for words stands where a value could be
            // typed in sight. A card that sends nowhere draws no field.
            let setup_field = setup_approving && detail.setup.is_some() && sends;
            let secret_field = (detail.secret.is_some() && sends) || setup_field;
            let field = match secret_field {
                true => self.secret.clone(),
                false => self.line.clone(),
            };
            let focus = field.focus_handle(cx);
            let kind = if secret_field { "secret" } else { "words" };
            let words = detail.secret.is_none() && detail.run.is_none() && detail.setup.is_none();
            let fielded = !taken_back && sends && (words || giving.is_some() || setup_field);
            // A secret's field stands beside its toggle, as on an open
            // card.
            let toggle = secret_field
                .then(|| show_button(format!("show {bead} {ask}"), self.secret_shown, palette, cx));
            body = body.when(fielded, |body| {
                let boxed = div()
                    .debug_selector(|| format!("{kind} {bead} {ask}"))
                    // A setup ask's account field takes its approval's key
                    // too, with what the field holds.
                    .when(setup_field, |el| el.key_context(SETUP_SECRET_CONTEXT))
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
                    .child(field);
                match toggle {
                    Some(toggle) => body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(boxed.flex_1())
                            .child(toggle),
                    ),
                    None => body.child(boxed),
                }
            });
            // Under it the button that sends the field, exactly as Enter
            // does: the value again on a secret, the words where they are
            // the answer, the new pick where one is chosen.
            // Why the yard did not take the card's answer, in place, as on
            // an open card: an approval refused from here is shown whole.
            if let Some(why) = self.refused.get(&(bead.to_string(), ask)) {
                body = body.child(
                    div()
                        .debug_selector(|| format!("refused {bead} {ask}"))
                        .text_sm()
                        .text_color(palette.warn)
                        .child(format!("! {why}")),
                );
            }
            let send = match (&detail.secret, giving) {
                _ if taken_back => None,
                (Some(_), Some(secret)) if sends => Some((
                    format!("give {bead} {ask}"),
                    match approval {
                        Approval::Unlock => give_unlock_label(secret, self.secret_typed.len()),
                        _ => give_label(secret, self.secret_typed.len()),
                    },
                    !self.secret.read(cx).text().is_empty(),
                )),
                (Some(_), _) => None,
                (None, _) if detail.run.is_some() || detail.setup.is_some() => None,
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
            // As an open card's: held while the yard is reconnecting,
            // with the reason beside the button that would send.
            let held = self.reconnecting();
            let sendable = send.is_some()
                || (detail.run.is_some() && approving && sends)
                || (detail.setup.is_some() && setup_approving && sends);
            body = body.when_some(send, |body, (id, text, enabled)| {
                body.child(send_button(id, text, enabled && !held, palette, cx))
            });
            body = body.when(held && sendable, |body| body.child(send_held(palette)));
            let run_approval = match approving {
                true => approval,
                false => Approval::Nowhere,
            };
            let setup_approval = match setup_approving {
                true => approval,
                false => Approval::Nowhere,
            };
            let keys = match detail.secret {
                Some(_) if giving.is_some() => secret_keys(bead, ask, approval, false),
                Some(_) => String::new(),
                None if detail.run.is_some() => run_keys(run_approval, false),
                None => match &detail.setup {
                    Some(setup) => setup_keys(bead, ask, setup, setup_approval, false),
                    None => decided_keys(&detail),
                },
            };
            let prose = !keys.is_empty();
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_baseline()
                    .gap_1()
                    .text_sm()
                    .text_color(palette.faint)
                    .children(prose.then(|| div().child(keys)))
                    .children(prose.then(|| div().child("·")))
                    .child(keys_hint("keys hint decided", palette, cx)),
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
                    select::key((bead, ask)),
                    Expanded {
                        body: &detail.body,
                        notes: &detail.notes,
                        about: &detail.about,
                        doc: &detail.doc,
                    },
                    palette,
                    cx,
                ));
            }
        }
        pane.children(self.reconnect_strip(palette))
            .child(head)
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

    /// The grants pane: every source × holder × call the yard serves,
    /// by source, with each call's own time and approver, the weighty
    /// words for update and `--as` rows in rust as on a card, and each
    /// row's revoke line to copy. It changes nothing itself. A yard too
    /// old for the call says so, in plain words.
    pub(crate) fn talk_grants(
        &self,
        model: &Model,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pane = div().flex_1().min_w_0().h_full().flex().flex_col();
        if model.grants_too_old() {
            return pane
                .children(self.reconnect_strip(palette))
                .child(
                    div()
                        .p_3()
                        .border_b_1()
                        .border_color(palette.line)
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Grants")),
                )
                .child(
                    div().flex_1().min_h_0().p_4().child(
                        div()
                            .debug_selector(|| "grants too old".to_string())
                            .text_color(palette.faint)
                            .child(model::GRANTS_TOO_OLD),
                    ),
                );
        }
        let grants = model.grant_records();
        if grants.is_empty() {
            return pane
                .children(self.reconnect_strip(palette))
                .child(
                    div()
                        .p_3()
                        .border_b_1()
                        .border_color(palette.line)
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Grants")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(palette.faint)
                        .debug_selector(|| "grants empty".to_string())
                        .child("no call of a source is granted"),
                );
        }
        let head = div()
            .p_3()
            .border_b_1()
            .border_color(palette.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .debug_selector(|| "grants head".to_string())
                    .child("Grants"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .debug_selector(|| "grants summary".to_string())
                    .child(model::grant_summary(grants)),
            );
        let mut body = div()
            .id("grants")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.decision)
            .p_4()
            .flex()
            .flex_col()
            .gap_4();
        let mut last_source = "";
        for grant in model::ordered_grants(grants) {
            if grant.source.as_str() != last_source {
                last_source = grant.source.as_str();
                body = body.child(
                    div()
                        .debug_selector({
                            let source = grant.source.clone();
                            move || format!("grants source {source}")
                        })
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.faint)
                        .child(model::plain(&grant.source)),
                );
            }
            for call in &grant.calls {
                body = body.child(self.grant_row(grant, call, palette));
            }
        }
        for line in model::grant_unkept(grants) {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .debug_selector(|| "grants unkept".to_string())
                    .child(line),
            );
        }
        pane.children(self.reconnect_strip(palette))
            .child(head)
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

    /// One grant row: who holds what call, the weighty lines where the
    /// call is an update or the grant is as a user, when it was granted
    /// and by whom, and the line that takes it back, to copy.
    fn grant_row(
        &self,
        grant: &api::SourceGrant,
        call: &str,
        palette: Palette,
    ) -> impl IntoElement + use<> {
        let holder = model::grant_holder(grant);
        let (granted, by) = model::grant_given(grant.given.get(call));
        let granted_words = match granted.as_str() {
            "not kept" => "granted not kept".to_string(),
            granted => format!("granted {granted} by {by}"),
        };
        let revoke = model::grant_revoke(&grant.source, call, grant);
        let key = select::key((
            &grant.source,
            &grant.group,
            &grant.crew,
            &grant.as_user,
            call,
        ));
        let mut row = div()
            .debug_selector({
                let source = grant.source.clone();
                let call = call.to_string();
                move || format!("grants row {source} {call}")
            })
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .debug_selector({
                        let source = grant.source.clone();
                        let call = call.to_string();
                        move || format!("grants holder {source} {call}")
                    })
                    .child(format!("{holder} — {}", model::plain(call))),
            );
        for (n, line) in model::grant_weight_lines(&grant.source, call, grant)
            .iter()
            .enumerate()
        {
            let line = line.clone();
            row = row.child(
                div()
                    .debug_selector({
                        let source = grant.source.clone();
                        let call = call.to_string();
                        move || format!("grants weight {source} {call} {n}")
                    })
                    .text_color(palette.text_accent)
                    .child(select::text(
                        expanded_text_key(key, "weight", n, &line),
                        line,
                        &self.selection,
                    )),
            );
        }
        row = row.child(
            div()
                .text_sm()
                .text_color(palette.faint)
                .debug_selector({
                    let source = grant.source.clone();
                    let call = call.to_string();
                    move || format!("grants given {source} {call}")
                })
                .child(granted_words),
        );
        row.child(select::text(
            expanded_text_key(key, "revoke", 0, &revoke),
            fenced(&revoke),
            &self.selection,
        ))
    }
}

/// A field of a yard that is asked for, with its name beside it: the
/// focus moves to it on a click. Under the yards with the other
/// fields, or in the renamed card with its form.
pub(crate) fn yard_field(
    name: &'static str,
    line: &Entity<Line>,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> Div {
    let focus = line.focus_handle(cx);
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(rems(5.)).flex_none().child(name))
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
}

/// The button under a yard's fields: Enter in a field presses it too.
/// Dimmed while a pairing or a sign-in is away.
pub(crate) fn yard_submit(
    text: &str,
    id: &'static str,
    busy: bool,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> Div {
    div().flex().child(
        small_button(text.to_string(), palette)
            .id(id)
            .debug_selector(move || id.to_string())
            .when(busy, |el| el.opacity(0.5))
            .on_click(cx.listener(|this, _, _, cx| this.submit_yard(cx))),
    )
}

/// The help under a yard's fields: one faint line.
pub(crate) fn yard_help(text: String, palette: Palette) -> Div {
    div()
        .debug_selector(|| "yard help".to_string())
        .text_sm()
        .text_color(palette.faint)
        .child(text)
}

/// The directory `dir` as a `file:` URL, for the settings footer's
/// reveal: what the opener is handed. A space in it (macOS's
/// "Application Support") is encoded, so it still opens.
pub(crate) fn dir_url(dir: &Path) -> String {
    let mut url = "file://".to_string();
    for byte in dir.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                url.push(byte as char)
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}
