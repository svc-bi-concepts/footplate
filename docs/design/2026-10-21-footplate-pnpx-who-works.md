# Who works each bead, and how many are working, in the Lines views

Design footplate-pnpx, proposed 2026-10-21. A proposal as it was decided on, not a description of what the code does today; the decision is on the bead.

Design for bead footplate-pnpx (flow `design.design`, stage `draft`). A yard is one installation of yardr: its beads, its crew and its depots. A bead is one durable piece of work; it travels the stages of a flow, and at each stage it is routed to a group. A group is the yard's pool of workers for a stage: its runner says how its work runs (`herdr` starts an agent in a pane, `exec` runs a command, `manual` is a person's turn, `peer` sends the bead to a linked yard), its limit is its seats, and its config may name a harness kind (`claude`, `muse`, `codex`, `kimi`: the agent program, which is what mckean calls the provider) and a model. A session is one claim of a bead by a group: the yard records which group claimed it, on which runner, and, since the kind and model were recorded, which kind and model it started with. footplate is the yard's native app; its Lines mode (`Mode::Timetable` in src/model.rs) shows the beads still travelling in three zooms: the panel of tracks (the arrivals pane, with the departures board behind a toggle), one line (a depot) as stage columns with a card per bead, and one bead's details. A headcode is a bead's short handle as the panel draws it (`short_bead` in src/model.rs). See docs/glossary.md in the yardr repository for all of yardr's words.

## 1. The question

mckean, in mail 57618 on 21 Oct at 10:49, as the bead quotes it: "the arrivals view, and the line views generally, show what is moving but not who is moving it. Add: on a bead in the views: which session works it, and that session's provider and model (claude / opus-5-5, muse / spark-1.3-contributor), on hover or expansion so the rows stay uncluttered; an overview of how many agent sessions are running (per provider or group would be a natural cut)." The bead asks the design to decide where the session and model show, what the count looks like and where it sits, and whether the yard's API already says enough; where it does not, the API addition is its own bead in the yardr depot (the yardmaster filed it as yardr-8bde and asks this draft to name the fields it wants). The web view is not in this bead.

## 2. What exists today

**Every Lines zoom reads the socket's `Flows` call, and a bead there already says that it is worked and by which session.** `Flows` (yardr docs/api.md under `Flows`; `Store.Flows` in yardr internal/store/flowsboard.go) serves every depot's flows, their stages with the open beads on each, each depot's Elsewhere beads and the beads travelling with no line. Each bead (`FlowBead`, yardr internal/store/flowsboard.go:89-113) carries `working` (a non-manual session claimed it) and `worker`, "who works it, as `yardr session list` shows the claiming session: the runner with the agent's name or pid" (`Session.RunnerShown` in yardr internal/store/routing.go:1795), for example `herdr footplate-pnpx-draft`. footplate mirrors it field for field (`FlowBead` in src/api.rs:403-433) and finds any travelling bead's row by id (`Model::flow_bead` in src/model.rs:2167), which the bead's details already use where the board serves the bead no longer (`Model::arrival_detail`, src/model.rs:2192).

**What a bead's session does not serve: its group, kind and model.** The yard records all three on the session at the claim: `Session` has `Group`, `Runner`, `Kind` and `Model` (yardr internal/store/routing.go:1702-1733), and the claim fills `Kind` from the group (`Group.Kind`, which falls back to the yard's `default_kind` for a herdr group that names none, routing.go:47-55) and `Model` from the group's config `model`, empty where it names none (routing.go:2101-2105); the `started` event carries them too (routing.go:2117-2123). A session from before they were recorded has neither. `Flows` reads the live sessions already (`bySession`, `taken` in flowsboard.go:177-203) but copies only `RunnerShown()` onto the bead. The `Arrivals` call (docs/api.md under `Arrivals`, `Arrival` in src/api.rs:283-306) serves no session field at all. The `Crew` call serves a `model` and `kind` per crew member (docs/api.md under `Crew`), which footplate does not read (`CrewMember` in src/api.rs:223-240); crew sessions are not tied to a bead (yardr-5bd7 was closed on exactly that, citing yardr docs/crew.md), so they never stand on a Lines view.

**What footplate draws with it today.** The line view's card (`bead_card` in src/window.rs:14953) puts "worked by herdr footplate-pnpx-draft" in its faint meta line, beside how long the bead has stood at its stage and "waits for a seat". The line view's stage column head says the routed group and its seats, "opus-builders · 2 of 2 seats" (`stage_group_words`, src/window.rs:737-754, from the stage's `group`, `limit` and `taken`). The panel's headcode (`panel_headcode`, src/window.rs:14359) shows a worked bead with the running green dot and semibold weight, and nothing of who. The departures board's row (`arrival_row`, src/window.rs:11829, under `departures_board` at :13913) has columns Expected, Bead, Platform, Line, Remark and nothing of who. The bead's details (`talk_bead`, src/window.rs:15263) show the back link, title, the id with the line (`bead_sub`, :815), the route strip, body and notes, and nothing of who. The panel's head line (`panel_summary`, src/window.rs:13845) says "3 working · 1 waiting on you · 2 held", where "working" is `Model::working_count` (src/model.rs:1954): the Flows beads with `working` set, which counts agent sessions, command sessions (`exec`, the assembly and release groups) and peer sends alike.

**footplate has a tooltip already.** `RailTip` (src/window.rs:305-321) is a small bordered box in the window's ground colour and text size small, used by the rail's tiles and the yard actions button through gpui's `.tooltip(...)`.

**The design system's rules that bear on it.** yardr-design readme.md, read for this draft as the `eyes` label asks: counts are quiet words at the right, "3 working", never a badge or "(2)" (line 75); keep the railway vocabulary, never "agent list" or "dashboard" (lines 10 and 68); mono is for commands, paths and ports (line 93); hover steps a surface one shade lighter and quiet text to primary (line 105). The yard's own words for the parts are group, runner, kind, model and session; "provider" is mckean's word for kind and is not used in footplate's copy.

**What a running yard looks like.** On this yard today (`yardr group list`): `opus-builders` is herdr, kind `claude`, model `claude-opus-5-5`, 2 seats; `muse-builders`, `muse-light`, `muse-hosted`, `muse-security` and `shapers` are herdr, kind `muse`, model `muse-spark-1.3-contributor`; `kimi-builders` is kind `kimi`, paused; the assembly and release groups are `exec`; `backlog`, `decisions` and `siding` are manual and hold many live sessions (`yardr session list` shows 35 running manual sessions this morning, beside 7 herdr and 1 exec). A count of "running sessions" taken from the session list or from a stage's `taken` would be dominated by those manual claims, which are a person's turn and no work.

## 3. The proposal

The yard serves a worked bead's group, kind and model on `Flows` beside the `worker` it serves already; footplate shows them on hover over a bead wherever the Lines mode draws one, and as one line in the bead's details; and the panel's existing "N working" splits by kind. No new call, no new view, no layout change.

### 3.1. What the yard adds (yardr-8bde)

Three fields on every `FlowBead` (so on stage beads, Elsewhere beads and unassigned beads alike), set exactly where `worker` is set today, from the claiming live session itself:

| field | type | |
|---|---|---|
| `group` | string, may be absent | the group whose session works the bead, as `yardr session list` shows it; absent where `worker` is |
| `kind` | string, may be absent | the harness kind the session started with (`claude`, `muse`, ...); absent where the session has none: a group whose runner is not herdr, or a session from before the yard recorded it |
| `model` | string, may be absent | the model the session started with; absent where its group's config named none, and the harness's own was used |

They come from the session, not the group's current config, because the session keeps what it was started with and the group may have been changed since. The names are `Crew`'s own (`model`, `kind`), so a reader of both reads one vocabulary. `Arrivals` does not change: every surface that draws an arrivals row can find the same bead's Flows row by id (`Model::flow_bead`), and adding the fields to both would serve them twice. Nothing new invalidates the view: a claim, a release and a session's end are events that carry the bead, on which footplate already re-reads `Flows`.

### 3.2. Who works a bead: on hover, and in the details

One tooltip, the same words wherever a worked bead is drawn, built by one plain function in src/window.rs from the bead's Flows row and tested on its own:

```
footplate-pnpx-draft
opus-builders · claude · claude-opus-5-5
```

The first line is the session as the yard names it (`worker` without its runner word where the runner is herdr, since the second line says the rest; whole otherwise, so `exec pid 4127` stays as served). The second is group, kind and model, each where served, joined with a middle dot as the window joins its other meta. A bead whose session has no kind (an `exec` or `peer` claim) shows its group alone on the second line. A bead with no worker has no tooltip.

Where it hangs, in the three zooms:

- **The panel**: on each headcode (`panel_headcode`). Headcodes are the panel's only per-bead mark and are too small to carry words, so hover is the only place.
- **The departures board**: on each row whose bead is worked (`arrival_row`), from the bead's Flows row by id. The board's columns do not change.
- **The line view**: on each card (`bead_card`). The card's meta line keeps "worked by …" as it is.

The bead's details (`talk_bead`) are the expansion: a quiet line under the id line, in the faint colour, "worked by footplate-pnpx-draft · opus-builders · claude · claude-opus-5-5", the same words as the tooltip on one line. A bead with no worker has no such line.

### 3.3. How many are working: the panel's head, split by kind

The panel's head line keeps its place and its first word. Today it reads "5 working · 1 waiting on you · 2 held". With kinds served it reads:

```
5 working: 3 muse, 2 claude · 1 waiting on you · 2 held
```

The split counts the panel's own worked beads by their served `kind`, most first, ties by name; worked beads with no kind (commands, peer sends) count in the total and are not split out, so "6 working: 3 muse, 2 claude" means one of the six is a command or a peer. Where every worked bead has the same kind the split is dropped: "2 working". A tooltip on "N working" breaks the same beads down by group, with the seats the stages already serve:

```
muse-builders · 3 of 5 seats
opus-builders · 2 of 2 seats
footplate-assembly · 1 of 1 seat
```

That is the per-provider cut in the line and the per-group cut on hover, as mckean offered both. It counts sessions on travelling and backlog beads, which is every bead session the yard runs; crew members (yardmaster and the like) are not on any bead and stay out, as they are out of every Lines view. footplate counts the rows it already holds, as `working_count` does today: no yard-side count is needed (section 5, question 2, is the operator's to overrule).

What a person sees, with nothing to type and nothing to set: the panel opens with "5 working: 3 muse, 2 claude" at its head; hovering a green-dotted headcode says which session, group, kind and model; the bead it opens says the same under its id.

## 4. Left out

**A visible session or model on the rows.** mckean asked for hover or expansion so the rows stay uncluttered; the panel's cells hold two headcodes and "+N", the board's columns are fixed (`BOARD_*` widths), and the card already carries one meta line. The tooltip and the details line answer it without moving anything.

**The count beyond the panel.** The rail's Lines tile, the list's line rows and the line view's head could each carry a count; the bead names one overview, and the panel's head already holds "N working". Line rows carrying their own "2 working" is a later bead if wanted.

**Crew in the count.** Crew sessions are agents too, but they are not on a bead (yardr-5bd7), so they are not "moving" anything on a line. The Crew mode already lists each crew member with its state; showing their kind and model there is a small separate change (`CrewMember` would read the `model` and `kind` the `Crew` call serves) and is not this bead's question.

**`Arrivals` serving the fields too.** Every arrivals row footplate draws has its Flows row beside it; serving the fields once keeps one source. signalbox reads the web view, not the socket, so it gains nothing from either.

**The session id.** The yard's session id (`mkcqn2sx…`) is what `yardr session` commands take, but it is not what a person reads; the agent's name in `worker` is. A copy-the-id action is not asked for.

**The web view.** Named out of the bead; a follow-up if mckean wants it there.

## 5. Open questions

1. **What "N working" counts.** (a) Every worked bead, as today, split by kind, with commands and peer sends in the total only (as drawn above): no change to a number people already read. (b) Only beads worked by an agent (a kind served), so "working" means agents and the assembly's merge script drops out: the number is then exactly mckean's "agent sessions", but the head stops counting the commands it counts today, and a bead with a green dot may not be in it. (c) Both, "6 working, 5 agents: 3 muse, 2 claude": exact and longer, and the head line wraps at the narrowest window. I would take (a).
2. **Who counts.** (a) footplate counts its Flows rows (as drawn): no new yard field, and the count can never disagree with the dots on the panel. (b) The yard serves a `sessions` summary on `Flows` (per kind and per group, live non-manual sessions): exact even for a session whose bead Flows does not list (a backlog bead with no depot, which no route sends to an agent on this yard), at the cost of a second field to document and keep. I would take (a).
3. **The split in the line or only on hover.** (a) Kinds in the line, groups on hover (as drawn). (b) "5 working" alone in the line, kinds and groups both on hover: the head stays as short as today, but the overview mckean asked for is then one hover away. (c) Groups in the line: exact, but six groups on this yard make it long. I would take (a).
4. **The model as served or shortened.** mckean wrote "opus-5-5" and "spark-1.3-contributor"; the yard serves `claude-opus-5-5` and `muse-spark-1.3-contributor`. (a) As served: no rule to keep in footplate, and the words match `yardr group list`. (b) Drop a leading "<kind>-" from the model where it repeats the kind: shorter, one small rule. I would take (a).

## 6. The beads it would become

1. **yardr-8bde (exists, yardr depot): Flows beads say the group, kind and model of the session that works them.** Goal: section 3.1. Where: `FlowBead` and the `row` closure in yardr internal/store/flowsboard.go (where `Worker` is set from `bySession`), `Session.Group`, `Session.Kind`, `Session.Model` in internal/store/routing.go; the three `beads[]`, `elsewhere[]` and `unassigned[]` field rows and the example in docs/api.md under `Flows`; the store's flows test. Done when a bead claimed by a herdr group serves `group`, `kind` and `model` as its session recorded them, an `exec` claim serves `group` alone, a manual claim and an unclaimed bead serve none of the three, and docs/api.md says so. `Arrivals` unchanged; any session count left to question 2's answer.
2. **footplate: who works a bead, on hover and in its details.** Depends on 1. Goal: section 3.2. Where: `FlowBead` in src/api.rs (three `#[serde(default)]` fields) and its answers in src/fake.rs; one plain function in src/window.rs for the tooltip's two lines and the details line, with its tests (herdr worker, exec worker, no kind, no model, no worker); `.tooltip` with `RailTip` on `panel_headcode`, `arrival_row` (through `Model::flow_bead`) and `bead_card`; the details line in `talk_bead`. Done when hovering a worked headcode, board row or card shows the two lines, a worked bead's details show the one line, an unworked bead shows neither, and an old yard that serves none of the fields shows the session's name alone.
3. **footplate: the panel's "N working" splits by kind, with groups on hover.** Depends on 1; independent of 2. Goal: section 3.3, as question 1 and 3 are answered. Where: `Model::working_count` in src/model.rs gains a plain function beside it returning the kinds with counts and the groups with their stages' `limit`/`taken`, tested on its own; `panel_summary` in src/window.rs draws the words and the tooltip; the narrow-window test (`the_panel_holds_at_the_narrowest_window`) covers the longer line. Done when the head reads "N working: a kind, b kind" with two or more kinds, "N working" with one, and the tooltip lists each working group with its seats.
