//! What an ask's card says and takes: its heads and counts, its keys,
//! its buttons, and the texts a run or a setup holds.

use gpui::{App, ClickEvent, Context, FontWeight, SharedString, Window, div, prelude::*};

use super::{APPROVE_KEY, Footplate, small_button};
use crate::api;
use crate::model::{self, DecidedDetail, DecisionAsk};
use crate::theme::Palette;

/// A section's head in the left list: its title on the left, its count
/// on the right, quiet. "Decisions" counts the open ones, "Crew" the
/// mail that is not read.
pub(crate) fn section_head(
    title: &'static str,
    count: String,
    palette: Palette,
) -> impl IntoElement {
    div()
        .px_3()
        .pt_2()
        .pb_1()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .debug_selector(move || format!("section {title}"))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .debug_selector(move || format!("section {title} count"))
                .text_sm()
                .text_color(palette.faint)
                .flex_none()
                .child(count),
        )
}

/// One of chat's groups: its title in the section heads' style, with no
/// count. Each row carries its own.
pub(crate) fn group_head(title: &'static str) -> impl IntoElement {
    div().px_3().pt_2().pb_1().child(
        div()
            .debug_selector(move || format!("group {title}"))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .child(title),
    )
}

/// What the card's button reads with an option to answer with: the
/// option it answers, and its key. The option is the pick, else the
/// recommendation, which stands preselected: Enter does what the button
/// says.
pub(crate) fn answer_label(number: u64, label: &str) -> String {
    format!("Answer: {number} · {label} ↵")
}

/// What the card's button reads where words alone go: `Answer ↵` where
/// they are the answer, `Send ↵` where they go as a note and the ask
/// stays open.
pub(crate) const ANSWER_WORDS: &str = "Answer ↵";
pub(crate) const SEND_WORDS: &str = "Send ↵";

/// The recommended option's mark under its own row, with its why: the one
/// place the card names the recommendation.
pub(crate) fn recommended_line(why: &str) -> String {
    match why.is_empty() {
        true => "recommended".to_string(),
        false => format!("recommended: {why}"),
    }
}

/// How long the ask waits, beside its question: `asked 3d`, `asked now`.
/// Nothing where the yard's time cannot be read.
pub(crate) fn asked_age(waits: &str) -> String {
    match waits.is_empty() {
        true => String::new(),
        false => format!("asked {waits}"),
    }
}

/// What an inline ask's preview says over its question: that it waits on
/// the person reading, which ask, and how long it waits.
pub(crate) fn ask_preview_head(kind: &str, ask: u64, waits: &str) -> String {
    [
        "Waiting on you".to_string(),
        format!("{kind} {ask}"),
        asked_age(waits),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// What the line on an open card says, beside its disclosure: the bead's id
/// and the ask's number, the two that `yardr bead answer <bead> <ask> <n>`
/// takes. The number is the yard's, from the one counter of every note, so
/// a decision named by it in mail, chat or a terminal is found by it here.
pub(crate) fn ask_line(bead: &str, ask: u64) -> String {
    format!("{bead} · ask {ask}")
}

/// The head of a bead's open asks: the yard, the bead and the topic, once
/// for every ask of the bead. The yard's dot stands before it, in its
/// colour. No card is read without its yard.
pub(crate) fn decision_head(yard: &str, bead: &str, topic: &str) -> String {
    format!("{yard} · bead {bead} · {topic}")
}

/// What Decline's tooltip says: saying no to every option, so the
/// asker finds another way.
pub(crate) const DECLINE_IS_NO_PICK: &str = "Say no to all options. The asker finds another way.";

/// What the settings say over the list of yards: what a pairing gives that
/// a link between two yards does not.
pub(crate) const PAIRING_GIVES: &str = "A paired yard is called with this device's own token, so your picks and approvals there count as yours, where over a peer link you only see what the link carries and an approval never crosses.";

/// What a secret ask's card says under the yard's line of what is asked
/// for: where the value goes, in the words `yardr chat` has for it.
pub(crate) const SECRET_GOES: &str =
    "it goes to a file only the yard's server reads; no agent is shown it";

/// The keys of a secret ask's card, with the way for a person without the
/// field: the same answer in a terminal on the yard's machine. Where the
/// Mac's unlock stands in front, Enter opens it first; where the card
/// takes its number back, `i` types the number before the value.
pub(crate) fn secret_keys(bead: &str, ask: u64, approval: Approval, writable: bool) -> String {
    let keys = match approval {
        Approval::Direct => format!(
            "i types it · enter sends it to the yard · esc leaves, and nothing is sent · or in a terminal: yardr bead answer {bead} {ask} --secret"
        ),
        Approval::Unlock => format!(
            "i types it · enter sends it to the yard after unlocking · esc leaves, and nothing is sent · or in a terminal: yardr bead answer {bead} {ask} --secret"
        ),
        Approval::TypeBack => format!(
            "i types the number, then it · enter sends it to the yard with its number · esc leaves, and nothing is sent · or in a terminal: yardr bead answer {bead} {ask} --secret"
        ),
        Approval::Nowhere => format!("in a terminal: yardr bead answer {bead} {ask} --secret"),
    };
    if writable {
        format!("{keys} · a ask about this")
    } else {
        keys
    }
}

/// What a run ask's card says over the command, while it can be approved:
/// the words `yardr chat` has for it, led by the yard the command runs in
/// and with the user it runs as, who is the one the approval is called as.
/// With several yards in one window, the sentence over a command is where
/// it must be plain which of them runs it.
pub(crate) fn run_runs(yard: &str, user: &str) -> String {
    format!("in {yard}, when you approve, the yard runs this as {user}, and nothing else:")
}

/// The header of a card: the yard it is of first, then the bead and how
/// long it waits or which ask it is. No card is read without its yard.
/// The yard's dot stands before it, in its colour.
pub(crate) fn card_head(yard: &str, bead: &str, rest: &str) -> String {
    format!("{yard} · {bead} · {rest}")
}

/// What the control that approves a run ask reads: the yard its command
/// runs in, and its key.
pub(crate) fn approve_label(yard: &str) -> String {
    format!("Approve in {yard} ({APPROVE_KEY})")
}

/// What the control that approves a weighty run or setup ask reads where
/// the Mac's unlock stands in front of it: the yard, and that unlocking
/// comes first.
pub(crate) fn approve_unlock_label(yard: &str) -> String {
    format!("Approve in {yard} after unlocking")
}

/// How the card's approval goes: straight to the yard, through the Mac's
/// unlock first, after the ask's number is typed back where no unlock is
/// offered, or nowhere from this window, where the card shows where to
/// approve instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Approval {
    Direct,
    Unlock,
    TypeBack,
    Nowhere,
}

/// How the ask's card approves: nowhere where the yard takes no approval
/// from this window; through the Mac's unlock where a weighty card has
/// one; after the ask's number is typed back where it has none, as a
/// terminal asks it back; straight to the yard otherwise.
pub(crate) fn approval(weighty: bool, gate: bool, elsewhere: bool) -> Approval {
    match (weighty, gate, elsewhere) {
        (_, _, true) => Approval::Nowhere,
        (true, true, false) => Approval::Unlock,
        (true, false, false) => Approval::TypeBack,
        (false, _, false) => Approval::Direct,
    }
}

/// What a run ask's card says over what the command changes: the
/// command's one line from its help, as a terminal says it.
pub(crate) fn description_line(description: &str) -> String {
    format!("what the command does: {description}")
}

/// Who may run the command that takes the change back, as a terminal
/// says it: the yard's words, or anyone where it sends none.
pub(crate) fn take_back_label(back_by: &str) -> String {
    match back_by.is_empty() {
        true => "anyone can take it back:".to_string(),
        false => format!("{back_by} can take it back:"),
    }
}

/// What it says over the command of one that takes no approval any more.
pub(crate) const RUN_PROPOSED: &str = "the command this ask proposed:";

/// What a setup ask's card says over what approving does, while it can be
/// approved: the words `yardr chat` has for it, led by the yard the setup
/// runs in. With several yards in one window, the sentence over a setup
/// is where it must be plain which of them runs it.
pub(crate) fn setup_runs(yard: &str) -> String {
    format!(
        "in {yard}, when you approve, the yard applies the pack, takes the account, and grants the preset you pick, and nothing else:"
    )
}

/// What it says over the setup of one that takes no approval any more.
pub(crate) const SETUP_PROPOSED: &str = "the setup this ask proposed:";

/// What a setup ask's card says over its candidates: a tap takes one, and
/// a filled secret field types another.
pub(crate) const SETUP_TAKE: &str = "take one with a tap, or type another below:";

/// What a run ask's card says in place of the control that approves, where
/// the yard does not have the call or does not serve it to this window.
pub(crate) const APPROVE_ELSEWHERE: &str =
    "this yard takes no approval from footplate. Approve in a terminal on the yard's machine:";

/// What a weighty card says over its number field where this window has
/// no gate: what to type and why, in the terminal's words. `verb` is what
/// the number lets through: approving a run or setup ask, giving a secret.
pub(crate) fn typeback_line(ask: u64, verb: &str) -> String {
    format!("type the ask's number, {ask}, to {verb} this weighty card")
}

/// What the window says where the number field holds anything but the
/// ask's number: nothing went, and the card stays as it is, in the
/// terminal's words.
pub(crate) fn typeback_mismatch(ask: u64) -> String {
    format!("ask {ask}'s number was not typed back, so nothing was done")
}

/// The keys of a run ask's card. Later, Decline and expand have their
/// buttons; Enter is named because it does nothing here. Where the Mac's
/// unlock stands in front, the key opens it first; where the card takes
/// its number back, `i` types it.
pub(crate) fn run_keys(approval: Approval, writable: bool) -> String {
    let mut keys: Vec<String> = Vec::new();
    match approval {
        Approval::Direct => keys.push(format!("{APPROVE_KEY} approves")),
        Approval::Unlock => keys.push(format!("{APPROVE_KEY} approves after unlocking")),
        Approval::TypeBack => {
            keys.push(format!("{APPROVE_KEY} approves after its number is typed"))
        }
        Approval::Nowhere => {}
    }
    if !matches!(approval, Approval::Nowhere) {
        keys.push("enter alone approves nothing".to_string());
    }
    if matches!(approval, Approval::TypeBack) {
        keys.push("i types the number".to_string());
    }
    if writable {
        keys.push("a ask about this".to_string());
    }
    keys.join(" · ")
}

/// The keys of a setup ask's card: the approval with its key, the field
/// that types the account, and the same answer in a terminal on the
/// yard's machine. Where the yard takes no approval from this window, the
/// card says where it does in place of the control, and lists no keys.
pub(crate) fn setup_keys(
    bead: &str,
    ask: u64,
    setup: &api::Setup,
    approval: Approval,
    writable: bool,
) -> String {
    let mut keys: Vec<String> = Vec::new();
    match approval {
        Approval::Direct => keys.push(format!("{APPROVE_KEY} approves")),
        Approval::Unlock => keys.push(format!("{APPROVE_KEY} approves after unlocking")),
        Approval::TypeBack => {
            keys.push(format!("{APPROVE_KEY} approves after its number is typed"))
        }
        Approval::Nowhere => {}
    }
    if !matches!(approval, Approval::Nowhere) {
        keys.push("enter alone approves nothing".to_string());
        match approval {
            Approval::TypeBack => keys.push("i types the number, then the account".to_string()),
            _ => keys.push("i types the account".to_string()),
        }
        keys.push(format!(
            "or in a terminal: {}",
            model::setup_approve_command(bead, ask, setup)
        ));
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
pub(crate) fn fenced(command: &str) -> String {
    let (mut longest, mut run) = (0, 0);
    for c in command.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{command}\n{fence}")
}

/// One text a run ask holds behind the tap: its heading, its bytes, and
/// the section its copy control is keyed under.
pub(crate) struct RunText {
    pub(crate) heading: String,
    pub(crate) text: String,
    pub(crate) section: &'static str,
}

/// Every text carried sources hold, in the order a terminal prints them:
/// each declaration, command files, file changes and removed entries'
/// texts. A run ask and a setup ask hold the same: the headings are the
/// terminal's words, so the readings agree. An empty text is no text: a
/// removed entry that is not text stands in the yard's sentences alone.
pub(crate) fn carried_texts(carried: &[api::RunSource]) -> Vec<RunText> {
    let mut texts = Vec::new();
    for carried in carried {
        texts.push(RunText {
            heading: format!(
                "the declaration of source {}, whole as carried:",
                model::plain(&carried.name)
            ),
            text: carried.text.clone(),
            section: "declaration",
        });
        for file in &carried.files {
            texts.push(RunText {
                heading: format!(
                    "the command file {} (sha256 {}), whole as carried:",
                    model::plain(&file.program),
                    model::plain(&file.sha256),
                ),
                text: file.text.clone(),
                section: "command file",
            });
        }
        for change in &carried.old {
            texts.push(RunText {
                heading: format!(
                    "the change to command file {} (sha256 {} -> {}), whole as carried:",
                    model::plain(&change.program),
                    or_none(&change.old_sha256),
                    or_none(&change.new_sha256),
                ),
                text: change.diff.clone(),
                section: "file change",
            });
        }
        for removed in carried.removed.iter().filter(|r| !r.text.is_empty()) {
            texts.push(RunText {
                heading: format!(
                    "the entry {} the copy would remove (sha256 {}), whole as carried:",
                    model::plain(&removed.path),
                    model::plain(&removed.sha256),
                ),
                text: removed.text.clone(),
                section: "removed entry",
            });
        }
    }
    texts
}

/// Every text a run ask holds, in the order a terminal prints them: its
/// carried sources' texts, then each plan file's change and new text. A
/// plan object with neither a change nor a new text stands in the yard's
/// sentences alone.
pub(crate) fn run_texts(run: &api::Run) -> Vec<RunText> {
    let mut texts = carried_texts(&run.carried);
    if let Some(pack) = &run.pack
        && let Some(plan) = &pack.plan
    {
        for object in &plan.objects {
            if !object.diff.is_empty() {
                texts.push(RunText {
                    heading: format!(
                        "the change to file {} (sha256 {} -> {}), whole as carried:",
                        model::plain(&object.name),
                        or_none(&object.file_old_sha256),
                        or_none(&object.file_new_sha256),
                    ),
                    text: object.diff.clone(),
                    section: "plan change",
                });
            }
            if !object.text.is_empty() {
                texts.push(RunText {
                    heading: format!(
                        "the file {} as the pack would install it, whole as carried:",
                        model::plain(&object.name),
                    ),
                    text: object.text.clone(),
                    section: "plan file",
                });
            }
        }
    }
    texts
}

/// A hash the card names, or that there is none, as the terminal says it.
pub(crate) fn or_none(sha: &str) -> String {
    match sha.is_empty() {
        true => "(none)".to_string(),
        false => model::plain(sha),
    }
}

/// What the tap behind a run ask's sentences reads: how many texts it
/// holds, and whether they stand shown.
pub(crate) fn texts_label(count: usize, shown: bool) -> String {
    let texts = match count {
        1 => "1 text".to_string(),
        count => format!("{count} texts"),
    };
    match shown {
        true => format!("hide {texts}"),
        false => format!("show {texts}"),
    }
}

/// The keys of a dealt with: nothing is chosen by default and Enter alone
/// sends nothing; a number then Enter replaces the pick. One taken back has
/// no keys: nothing is sent from its card.
pub(crate) fn decided_keys(detail: &DecidedDetail) -> String {
    if detail.state == "taken back" {
        return String::new();
    }
    if detail.options.is_empty() {
        return "i words · enter sends".to_string();
    }
    format!("1-{} change your pick", detail.options.len())
}

/// The button that sends a card, exactly as Enter does: `Give NAME` on a
/// secret ask, `Answer ↵` or `Send ↵` where words alone go, `Answer: 1 ·
/// Move them now ↵` with an option (`Change to: 2 · …` on a dealt with).
/// Disabled while its field is empty: drawn faint, and no click. A pick
/// button is never disabled: a pick alone sends. The option is the pick,
/// else the recommendation, which stands preselected.
pub(crate) fn send_button(
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
/// Where the Mac's unlock stands in front, it says so instead:
/// `Approve in plantime after unlocking`. Disabled while an approval is
/// with the yard.
pub(crate) fn approve_button(
    id: String,
    yard: &str,
    unlock: bool,
    enabled: bool,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> impl IntoElement {
    let click = enabled.then(|| cx.listener(|this, _, _, cx| this.approve(cx)));
    let label = match unlock {
        true => approve_unlock_label(yard),
        false => approve_label(yard),
    };
    card_button(id, label, palette, click)
}

/// One filled button, standing as the one to take, with what a click on
/// it does: a card's that sends, or the away screen's that sets a yard up.
/// Without that it is drawn faint, and takes no click.
pub(crate) fn card_button(
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
pub(crate) fn give_label(secret: &api::Secret, typed: usize) -> String {
    match secret.env.get(typed) {
        Some(name) => format!("Give {name}"),
        None => "Give".to_string(),
    }
}

/// What the button on a weighty secret ask's card says where the Mac's
/// unlock stands in front of it: the name, and that unlocking comes
/// first.
pub(crate) fn give_unlock_label(secret: &api::Secret, typed: usize) -> String {
    match secret.env.get(typed) {
        Some(name) => format!("Give {name} after unlocking"),
        None => "Give after unlocking".to_string(),
    }
}

/// One candidate of a setup ask's account choice: the source with the
/// account its file holds, in the yard's own words (`setupAccountLine` in
/// internal/store/runask_setup.go of the yardr repository).
pub(crate) fn candidate_line(candidate: &api::SetupCandidate) -> String {
    let pairs: Vec<String> = candidate
        .values
        .iter()
        .map(|(name, value)| format!("{}={}", model::plain(name), model::plain(value)))
        .collect();
    format!(
        "{} — {} (in {})",
        model::plain(&candidate.source),
        pairs.join(", "),
        model::plain(&candidate.file)
    )
}

/// A setup candidate the card took, named in words beside its account.
/// Selection is also shown by the raised row; it needs no glyph.
pub(crate) fn candidate_choice_line(candidate: &api::SetupCandidate, taken: &str) -> String {
    let line = candidate_line(candidate);
    if candidate.source == taken {
        format!("taken: {line}")
    } else {
        line
    }
}

/// The toggle beside a secret ask's field: `Show` draws what is typed as
/// typed, for a look, `Hide` draws marks again. The value is unchanged
/// either way: it goes in its call as ever.
pub(crate) fn show_button(
    id: String,
    shown: bool,
    palette: Palette,
    cx: &mut Context<Footplate>,
) -> impl IntoElement {
    let text = if shown { "Hide" } else { "Show" }.to_string();
    small_button(text, palette)
        .id(SharedString::from(id.clone()))
        .debug_selector(|| id.clone())
        .on_click(cx.listener(|this, _, _, cx| this.toggle_secret(cx)))
}

/// Whether the words of an open ordinary card want their own button:
/// they are the answer where there are no options, or a note where there
/// is no pick and none recommended. With a pick the pick's button sends
/// them, and with a recommendation and no pick the recommendation's does;
/// on another yard's bead words alone go as no note.
pub(crate) fn words_send(ask: &DecisionAsk, foreign: bool) -> bool {
    if ask.secret.is_some() || ask.run.is_some() || ask.setup.is_some() || ask.chosen != 0 {
        return false;
    }
    if ask.options.is_empty() {
        return true;
    }
    ask.recommend == 0 && !foreign
}
