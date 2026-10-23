//! The window's words for its rows: what heads and counts say stands
//! where, and the colours that say what waits and for whom.

use gpui::{Div, Rgba, div, prelude::*, rems};

use crate::api::{FlowBead, FlowStage};
use crate::fetch::Kept;
use crate::model::{
    self, ArrivalDetail, Route, Selected, WorkingGroup, held_by_you, is_held, waits_on_you,
};
use crate::theme::Palette;

/// What a head says waits in it, the Decisions head's and a backlog
/// column's: the number with "waiting", or that none does.
pub(crate) fn waiting_words(waiting: usize) -> String {
    plural(waiting, "none waiting", "waiting", "waiting")
}

/// What a stage's head says stands on it: the number with "bead".
pub(crate) fn bead_words(beads: usize) -> String {
    plural(beads, "", "bead", "beads")
}

/// What a stage column's head says under its name: the routed group's
/// name, that it waits on a person where the group is manual, and its
/// seats where the yard serves them. A group with no route says why,
/// as does a paused one: a stage nothing can move through is the one a
/// person most needs named.
pub(crate) fn stage_group_words(stage: &FlowStage) -> String {
    if stage.group.is_empty() {
        return "no route matches".to_string();
    }
    let mut words = stage.group.clone();
    if stage.human {
        words.push_str(" · waiting on a person");
    }
    match (stage.limit, stage.taken) {
        (Some(0), _) => words.push_str(" · paused"),
        (Some(limit), Some(taken)) => {
            words.push_str(&format!(" · {taken} of {limit} seats"));
        }
        _ => {}
    }
    words
}

/// Whether the bead waits for a seat at its stage: unclaimed, its
/// remark naming no held, blocked or waiting state, and its group's
/// seats all taken across the yard. Only a routed group with a limit
/// above 0 can be full; a late mark names no state that explains the
/// bead standing still, so it waits for a seat too.
pub(crate) fn waits_for_seat(bead: &FlowBead, stage: &FlowStage) -> bool {
    if bead.working {
        return false;
    }
    if bead.tone == "red"
        || is_held(&bead.remark)
        || bead.remark.starts_with("blocked by ")
        || bead.remark.starts_with("waits for ")
    {
        return false;
    }
    matches!(
        (stage.limit, stage.taken),
        (Some(limit), Some(taken)) if limit > 0 && taken >= limit
    )
}

/// A stage's lamp as drawn: the served fault or turn, and nothing where
/// no bead stands on the stage. An empty stage stands unlit whatever it
/// serves; the panel's cells and the line view's columns share the rule.
pub(crate) fn stage_lamp(stage: &FlowStage) -> &str {
    if stage.beads.is_empty() {
        return "none";
    }
    match stage.lamp.as_str() {
        "fault" | "turn" => stage.lamp.as_str(),
        _ => "none",
    }
}

/// What a line's head says it carries: the number with "flow".
pub(crate) fn flow_words(flows: usize) -> String {
    plural(flows, "", "flow", "flows")
}

/// What the bead's quiet back link names: where it was opened from —
/// the panel or the departures board, the line, or No line.
pub(crate) fn bead_back_name(back: &Selected, departures: bool) -> String {
    match back {
        Selected::Arrivals => match departures {
            true => "departures".to_string(),
            false => "panel".to_string(),
        },
        Selected::Line(line) => line.clone(),
        Selected::NoLine => "no line".to_string(),
        _ => model::ARRIVALS.to_string(),
    }
}

/// What a bead's details head says under its title: the id with the
/// line, and the stage too where no route strip under it says where
/// the bead stands.
pub(crate) fn bead_sub(detail: &ArrivalDetail, route: Option<&Route>) -> String {
    let mut parts = vec![detail.bead.as_str()];
    if route.is_none() {
        parts.push(detail.stage.as_str());
    }
    parts.push(detail.line.as_str());
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Who works the bead, for the hover over a worked bead and the
/// one line in its details: the tooltip's words with the details
/// line's. The first line names the session as the yard names it,
/// without its runner word where the runner is herdr, since the
/// second line says the rest; any other worker stays whole. The
/// second line is the group, kind and model, each where served: an
/// exec or peer claim shows its group alone. None where no session
/// works the bead, and the session's name alone where the yard
/// serves none of the three.
pub(crate) fn worker_words(bead: &FlowBead) -> Option<(String, String)> {
    if bead.worker.is_empty() {
        return None;
    }
    let session = model::worker_session(&bead.worker);
    let rest = [bead.group.as_str(), bead.kind.as_str(), bead.model.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if rest.is_empty() {
        return Some((session.to_string(), format!("worked by {session}")));
    }
    Some((
        format!("{session}\n{rest}"),
        format!("worked by {session} · {rest}"),
    ))
}

/// What the panel's head says is being worked: the number with
/// "working", split by served kind where two or more kinds work, most
/// first. One kind alone drops the split, as nothing is split: "5
/// working: 3 muse, 2 claude", "2 working".
pub(crate) fn working_words(working: usize, kinds: &[(String, usize)]) -> String {
    if kinds.len() < 2 {
        return format!("{working} working");
    }
    let split = kinds
        .iter()
        .map(|(kind, n)| format!("{n} {kind}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{working} working: {split}")
}

/// What the panel's "N working" tooltip says: each working group with
/// its seats, one per line. None where no worked bead names a group,
/// so an old yard shows no tooltip.
pub(crate) fn working_tip(groups: &[WorkingGroup]) -> Option<String> {
    if groups.is_empty() {
        return None;
    }
    Some(
        groups
            .iter()
            .map(working_group_words)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// What the tooltip says for one working group: its name with its
/// seats where a current stage names them, the name alone where none
/// does. Paused reads as paused, as a stage column's head does.
pub(crate) fn working_group_words(group: &WorkingGroup) -> String {
    match (group.limit, group.taken) {
        (Some(0), _) => format!("{} · paused", group.name),
        (Some(limit), Some(taken)) => {
            let seats = match limit {
                1 => "seat",
                _ => "seats",
            };
            format!("{} · {taken} of {limit} {seats}", group.name)
        }
        _ => group.name.clone(),
    }
}

/// What the folded Decided says was dealt with: the number with
/// "decided".
pub(crate) fn decided_words(decided: usize) -> String {
    plural(decided, "", "decided", "decided")
}

/// The colour of a timetable remark with the yard's `tone` and
/// holder, for the person `me` reads as: a stalled bead reads the
/// error red, and a held bead only where they hold it; a wait that
/// names them reads the waiting text rust, and the rest dim — a hold
/// by anyone else or no one known, a blocked bead, a late mark and a
/// wait for someone else. Lateness is words, never red; a hold or a
/// wait for someone else is not theirs to answer, so it never takes
/// the accent.
pub(crate) fn remark_color(
    remark: &str,
    tone: &str,
    held_by: &str,
    me: &str,
    palette: Palette,
) -> Rgba {
    if is_held(remark) {
        if held_by_you(held_by, me) {
            palette.warn
        } else {
            palette.faint
        }
    } else if tone == "red" {
        palette.warn
    } else if waits_on_you(remark, me) {
        palette.text_accent
    } else {
        palette.faint
    }
}

/// A stage's wait light as a dot: red for fault, rust for turn, and no
/// dot where neither burns. Drawn from [`stage_lamp`], so an empty
/// stage stands unlit.
pub(crate) fn lamp_dot(lamp: &str, selector: String, palette: Palette) -> Option<Div> {
    let color = match lamp {
        "fault" => palette.warn,
        "turn" => palette.accent,
        _ => return None,
    };
    Some(
        div()
            .debug_selector(move || selector.clone())
            .size(rems(0.5))
            .rounded_full()
            .bg(color)
            .flex_none(),
    )
}

/// A headcode's colour on the panel, for the person `me` reads as: a
/// bead being worked reads in the text colour (with the running dot
/// beside it), a stalled bead in the error red, a held bead in red
/// only where they hold it, and a wait on the reader in the waiting
/// rust. Working stands first, whatever the stage's lamp says; a hold
/// by anyone else or no one known stands plain in the text colour as
/// a blocked or late bead does, its reason in words one zoom in. A
/// hold or a wait for someone else is not theirs to answer, so it
/// never takes the accent.
pub(crate) fn headcode_color(
    working: bool,
    remark: &str,
    tone: &str,
    held_by: &str,
    me: &str,
    palette: Palette,
) -> Rgba {
    if working {
        palette.text
    } else if is_held(remark) {
        if held_by_you(held_by, me) {
            palette.warn
        } else {
            palette.text
        }
    } else if tone == "red" {
        palette.warn
    } else if waits_on_you(remark, me) {
        palette.text_accent
    } else {
        palette.text
    }
}

/// What the Crew head says is unread in it: the number with "unread",
/// or that none is. "Waiting" beside Crew would read as crew that
/// waits, where it is mail that is not read.
pub(crate) fn unread_words(unread: usize) -> String {
    plural(unread, "none unread", "unread", "unread")
}

const ASK_ONE: &str = "question, among your decisions";
const ASK_MANY: &str = "questions, among your decisions";

/// What an arrival's asks line says: how many questions the bead has
/// open, which are the Decisions rows' job.
pub(crate) fn ask_words(asks: usize) -> String {
    plural(asks, "", ASK_ONE, ASK_MANY)
}

/// What the About card says of this footplate: its release as the tag
/// names it, as yardr's own `--version` names a release.
pub(crate) fn footplate_words() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// What the About card says of the yardr footplate keeps: its own answer
/// to `--version`, that none is fetched yet, or why it would not say.
/// None is what it says until the program has been asked.
pub(crate) fn yardr_words(kept: Option<&Kept>) -> String {
    match kept {
        None => "reading…".to_string(),
        Some(Kept::Nothing) => "not fetched yet".to_string(),
        Some(Kept::Says(line)) => line.clone(),
        Some(Kept::Silent(why)) => format!("could not say its version ({why})"),
    }
}

/// A count in words: the zero words where none stand, else the
/// number with the one word for one and the many for more. A count
/// with no custom none passes an empty zero, and none counts with
/// the many words, as "0 beads"; where none standing says nothing
/// at all, that is [`count_words`]' job instead.
pub(crate) fn plural(n: usize, zero: &str, one: &str, many: &str) -> String {
    match n {
        0 if !zero.is_empty() => zero.to_string(),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// How many things wait, in words: none where none do, one numbered
/// with its single word, more with their many. What [`Counts::line`]
/// and the rail's tooltips say.
pub(crate) fn count_words(n: usize, one: &str, more: &str) -> Option<String> {
    match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {more}")),
    }
}
