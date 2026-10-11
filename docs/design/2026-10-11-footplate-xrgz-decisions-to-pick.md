# Decisions as something to pick in footplate, with the beads in view

Design footplate-xrgz, proposed 2026-10-11. A proposal as it was decided on, not a description of what the code does today; the decision is on the bead.

Design for bead footplate-xrgz (flow `design`, stage `draft`). A bead is
one durable piece of work; a depot is a repository beads are worked in; a
crew member is an always-on agent of the yard, and the yardmaster the one
that coordinates; a user is a person of the yard, with an inbox; mail is a
message between parties of the yard; a note is evidence attached to a bead;
an ask is one question as data, the new thing this design proposes. See
docs/glossary.md in the yardr repository for all of yardr's words.

## 1. The question

mckean, 11 Oct 2026, in the yardmaster's pane after an evening of many
decisions: "i often am caught in a wall of text and mostly i think your
suggestions or options are good. so it would be nice to be able to pick on
an interface, with beads in context. i thought about footplate, maybe you
have an idea or can open a design spec line on footplate and see what they
come up with". Earlier the same evening: "there's a lot happening right
now, its hard to keep up especially since there are no threads here". How
does a person decide in footplate what a crew member asks them, by picking,
with the beads the question is about in view, and each question in its own
place and not in one stream?

## 2. What exists today

**Where decisions are asked today: prose in a pane.** The yardmaster asked
for decisions in its pane about fifteen times in one evening: a design bead
with open questions, a yes or no before something changes the live yard,
'say build' for a shaped bead, a choice of order. The answers were a few
words each, and several times an answer could have meant two things.
Routine notices arrived in the same stream and buried the questions.
Nothing about this is a mechanism: it is talk, and the talk is the wall of
text.

**A decision already lives on a bead, but only in `decide`.** The stage
file `.yardr/flows/default/decide.md` in the yardr repository already says
the rule this design keeps: "The bead carries the question and the
options, written by whoever sent it here", and "A bead leaves once the
answer is on it, as a note." But most of that evening's questions were
about work in flight, on beads in no manual stage: "shall I move the
links", "this design before that one", "close PR #3?". No mechanism holds
those today; they are only talk.

**The queue already exists: `yardr review`.** `yardr review` walks the open
beads in stages routed to manual groups, longest untouched first, showing
each bead's stage, description, last notes and diff stat (docs/review.md).
It is each question in its own place, ordered by waiting longest, and it is
what the footplate view mirrors; no new queue is proposed.

**Notes already carry a kind.** A bead's notes are the evidence on it
(`yardr bead note <id> <text>`), and each note has a `Kind` field, today
`""`, `note`, `journal` or `handoff`, with the text in `Note.Text`
(`internal/store/graph.go` in the yardr repository, the `Note` struct). A
structured question can therefore be a note of a new kind, read by the same
code that reads notes, rather than a new kind of record.

**Mail, notifications and answers are separate already, in yardr if not yet
in footplate.** A user's inbox has two parts, Mail and Notifications, each
acknowledged apart (docs/users.md, "The inbox"). `yardr chat` shows the
crew's conversations and, above them, the `yard` row with the
notifications, which has no line to type in (docs/chat.md, "The yard").
Footplate today shows only conversations: its client implements only
`UserInbox`/`AckUserInbox` (`src/api.rs`, `user_inbox` and
`ack_user_inbox`), not the `UserNotifications`/`AckUserNotifications` the
socket page also promises, and it has no `yard` row yet (README, "Not in
this slice"). What waits for an answer is a third thing, neither talk nor
report, and must never be buried by either.

**Beads already carry labels.** `yardr bead create --label` and `yardr bead
update --add-label` put labels on a bead, and routes already match them
(docs/groups.md, "Example: a crew member reviews the beads with a label").
Grouping questions needs no new concept: the labels are there.

**What footplate may ask of the yard.** Footplate is an interface over
yardr's socket API and starts nothing of yardr's (README). It speaks only
what docs/api.md of the yardr repository promises a program that is not
yardr: HTTP/1.1 over the unix socket, a call a `POST /call/<Method>`, the
events a `GET /events?after=<seq>` (`src/api.rs`). Beads, notes, asks and
moves are all outside that promise today ("What is stable"). Consequence:
with yardr as it is today, footplate can show no bead and record no pick;
section 6 names those yardr beads apart from footplate's own.

**How footplate follows the yard.** On its own thread, `session::follow`
reads the state once and then applies the stream after the head
(`src/session.rs`). The window holds a `Model` of plain data and renders
rows on the left, the open thing on the right; anything needing a call
comes back as an `Effect` the window runs (`src/model.rs`, `src/window.rs`).
The tests run without a yard and without a screen, against a fake yard
(`src/fake.rs`). A decisions view follows the same shape.

## 3. The proposal

The core stands: a question is data, written once, and the answer is the
option picked, so it cannot be misread. What changes from the earlier draft
is what a decision is for the person: plain words first, enough context on
the card to decide on, questions grouped by topic, and everything
dismissable. The first version is small: ask, answer, the list, the pick,
dismiss.

The existing mechanisms carry it: notes carry the writing, with two new
kinds (`ask`, `answer`); `yardr review` carries "each question in its own
place"; bead labels carry topics; the inbox's split between mail and
notifications carries "questions apart from notices". What is new is only:
the shape of an ask, the commands that write and answer one, the socket
calls that let footplate read and write them, and the footplate view. No
pick moves a bead in this version: the asker reads the answer and acts,
moves included.

### 3.1. An ask: the question as data, on any bead

An ask is one question with enumerable options, asked of one person. It
lives as a note of kind `ask` on any bead in any stage — the bead the
work is on, not a bead waiting in `decide`. Whoever asks writes:

```
yardr bead ask yardr-n8ta "Move this yard's links to the declared source now?" \
  --to mckean \
  --context "The links file declares the new source; 64 links still point at the old one." \
  --context "The move was tested on a copy just now and passed; the old source goes away Friday." \
  --option "Move them now: all 64 links point at the new source tonight" \
  --option "After Friday: keep the old source until the release is out" \
  --recommend 1 --why "the test passed; waiting only risks Friday's rush"
```

An ask holds: the question in one line, as a person would ask it; who must
answer (a user; default the yard's operator); two to five lines of context
(section 3.2); one or more options, each a label and its consequence in a
line; the one recommended and why in a line, or no recommendation said
plainly. yardr adds `you decide` to every ask with options; the writer does
not write it and cannot leave it out. Picking it records that the person
left this question to the asker, with any words; the asker decides and
notes what it took.

An ask about several beads names them (`--about yardr-3msx,yardr-vgjp`);
the ask lives on the one bead and the others stand on the card as context,
each one step away. A question with no bead at all is asked on a topic
bead: a standing bead per topic, titled "Questions: \<topic\>", made once
by hand, holding that topic's beadless asks over time. No new concept and
no new command beyond `ask` and `answer`.

`yardr prime --bead` and `yardr review` show an ask as numbered text with
no new plumbing. A question with no enumerable options ("what should the
tone of the greeting be?") is an ask with none: the views show it as words
wanted, and the answer is words.

### 3.2. What the writer must supply, so the card is enough

Between the question and the options stands what it is about, written for
the person and not for a builder: what is being decided, why now, and what
happens with each option to each project it touches — two to five lines,
and for an option the consequence in a line. The command refuses an ask
without context (`--context`, at least one line) and without a consequence
on every option, so the card is never empty. The full bead, document and
notes stay one step away (a keypress expands the card in place; the
document of a design bead is named by path and branch), but the card must
be enough to decide on for most questions.

The card for a real case, whole — the links move, as it stood that
evening after the test passed:

```
Move this yard's 64 links to the declared source?          yardr-n8ta · 2h
asked by yardmaster · recommends: move them now

The links file declares the new source; 64 links still point
at the old one. The move was tested on a copy just now and
passed. The old source goes away Friday, with the release.

○ 1. Move them now — all 64 links point at the new source tonight
○ 2. After Friday — keep the old source until the release is out
○ 3. You decide — leave it to yardmaster
  recommended: 1, the test passed; waiting only risks Friday's rush
─────────────────────────────────────────
> Move them, and say when it is done [        ]
  enter picks 1 · 1-3 pick · x puts it aside · esc leaves it
```

The bead id is small and to the side, never the row's text. No row reads
"\<id\> go?".

### 3.3. What the person sees: that evening, grouped

The window keeps its shape: rows on the left, the open thing on the right.
Above the conversations stands the decisions: the beads with at least one
open ask addressed to the person, longest waiting first, grouped by topic —
the bead's first label, as `yardr review` walks them. A question with no
topic stands under "Everything else". Each row leads with the question in
plain words, then whose it is and how long it waits; the bead id sits
small at the row's end. Answering a bead's last open ask removes its card
even before the bead moves. Ten open questions are three groups, not ten
rows.

That evening as it really was (the depot rules card shows one ask open,
three folded; the full links card is in section 3.2):

```
decisions (6)                        │ Release rule for beads from another yard?
                                     │                       yardr-k1p0 · 3d
  LINEAR OUT OF THE CORE             │ asked by yardmaster · recommends: hold them
▸ Move the 64 links… · yardm · 2h n8ta│
  Send 3msx and vgjp… · yardm · 5h 3msx│ Beads built in another yard arrive with their
  ─────────────────                    │ own release line. Holding them at release keeps
  WEB VIEW                             │ this yard's Friday; releasing with the train
  Cleanup: build? · assembly · 1d 9f2c│ keeps theirs. Two trains a week touch this.
  Close sketch PR #3? · yardm · 1d 7h11│
  ─────────────────                    │ ○ 1. Hold at release — foreign beads wait for Friday
  DESIGN LANE                        │ ○ 2. Release with the train — they go when built
▸ Depot rules: 4 questions · 1d me78 │ ○ 3. You decide
  Release rule… · yardm · 3d k1p0    │   recommended: 1, Friday is the yard's promise
  ─────────────────                    │ ─────────────────────────────────────────
  yardmaster · idle                2   │ > [                                     ]
  assembly · working                  │
  ─────────────────                    │
  yard · 4 unread                     │
```

One question with three options is the same window with one group and one
row; the card is the whole of the right side, as in section 3.2.

The same in `yardr chat` as text, in the conversation with the asker:

```
yardmaster · 3 open for you · depot rules: 4 questions (yardr-me78)
  1. Move the 64 links to the declared source now? (yardr-n8ta, 2h)
     The move was tested on a copy and passed; old source goes Friday.
     [1] Move them now (recommended)  [2] After Friday  [3] You decide
  2. Send yardr-3msx and yardr-vgjp to build? (yardr-3msx, 5h) ...
  3. Release rule for beads from another yard? (yardr-k1p0, 3d) ...
answer with: yardr bead answer <id> <ask> <n> [words] · x puts it aside
```

### 3.4. What a pick does, and what still needs words

A pick records an answer: a note of kind `answer` naming who picked which
option, with any words. The answer cannot be misread because it is the
option, not a paraphrase of it. The asker reads the answer and acts —
moving the links, moving the bead to build; yardr runs nothing on a pick.
'Say build' is one pick: the answer records "build", and the asker moves
the bead there on reading it, as a member moves any bead today.

Three things still need words: conditions on a pick ("go, but after
Friday"); an answer the options did not offer ("none of these: move only
the docs links"); a question with no enumerable options. The card carries
one line under the picks for the sentence. Enter with an option chosen
sends the pick with the words in the one answer note; words with no option
chosen go as a plain note on the bead, and the ask stays open. Talk about
the question stays where talk lives, in the conversation with the asker.

A pick stands, but it is not carved: a newer pick on the same ask
supersedes for display, and every pick is a note, so the history stands and
nothing is silently rewritten. The asker reads the latest when it acts;
changing a pick after the asker has acted is a new conversation — a note,
or talk — not a change to the record.

### 3.5. Dismissable: nothing needs an answer to go away

A person can make a thing go away without answering it. Nothing is
deleted; everything dismissed stays on the bead as notes, and footplate
keeps a folded "Decided" section under the decisions where the answered,
declined and put-aside stand with their picks.

A notice takes one key: `x` on the `yard` row marks it read, as viewing
does; `yardr inbox --ack` does the same in the terminal. A notice never
needs more than one key.

A question put aside (`x` on its card, "Later") leaves the decisions list
and comes back in 24 hours, or sooner when the asker writes a new note on
the bead — whichever first. The asker is not asked again; the question
simply waits its turn.

A question refused ("Decline", with words) records an answer that picks no
option: "mckean declined to decide this: \<words\>". The asker sees it
plainly as what it is — not a pick, not a delegation, the person saying
they will not decide it — and finds another way or lets it lie. It is not
`you decide`: that hands the question back and expects an answer; this
ends it.

### 3.6. What footplate needs from yardr, and what works as yardr is today

Footplate needs two things it has not got, as yardr beads in section 6:
the ask and the answer with their rendering in prime, review and chat; and
the socket calls to read the person's queue, read one bead whole (body,
notes, asks, about-beads) and write an answer, a note and a dismissal,
documented in docs/api.md. The `yard` row needs no new yardr call: the
socket page already promises `UserNotifications` and
`AckUserNotifications`.

With yardr as it is today, the convention works and the interface does
not. A crew member can already put the question, the context and the
options on any bead, a person can already answer in a note, and `yardr
review` already walks the queue longest-waiting first — all without new
code, and the yard should work that way from now on, so the questions leave
the pane before the interface is built. What no client can do today is draw
it: the socket API promises no bead and no note. Until the yardr beads
land, the decisions view is `yardr review` in a terminal.

## 4. Left out

**A pick that moves the bead.** Each option names no move; the asker moves
the bead on reading the answer. 'Build' needs no move: the answer records
"build" and the asker, a member that moves beads today, moves it. A move
returns if a question is asked by no one who can move the bead on reading
the answer, or when the toil of moving by hand is measured and worth the
machinery.

**Claiming an answer by sequence number.** No `taken` note, no expected
version: the latest pick is the answer and the asker reads it when it acts.
A claim returns if an asker ever acts on an answer the person had already
replaced — that mishap, observed, is what would bring it back.

**Withdrawn asks.** With no move-taking pick, nothing takes a bead's other
asks with it; a pick answers one question and leaves the rest open. This
returns with moves, if moves return.

**A third note kind.** Two kinds (`ask`, `answer`) do; `taken` went with
claiming. A kind returns with the behaviour it would carry.

**Snooze choices.** One "Later" (24 hours or the asker's next note) does;
no choice of tomorrow, next week, or a date. Choice returns if people put
questions aside and miss them, or dread their return.

**Topic beads made by themselves.** A topic bead is made once by hand; the
ask command does not create one. Automatic creation returns if the yard
collects topics faster than someone makes beads for them.

**Reading the document's bytes.** The card names the design document by
path and branch; footplate does not fetch it. Serving branch files over the
socket is a larger promise (which commit, which access) and is not this
design's to make.

**A general forms system.** An ask is one question with options, not a form
with fields, branches or validation. Anything richer stays prose.

**Notifications and mobile.** Being told outside the yard of an open ask is
the push mechanism as it is (docs/users.md, "Told outside the yard"). A
bead waiting long in a manual stage already raises one `waited` event
(docs/groups.md, "A bead that waits long").

**The asker acting by itself.** Picking 'go' does not move the links; it
records "go", and the yardmaster moves them. A pick that runs a command is
left out until use asks for it.

**signalbox.** Another view of its own; not this.

**Code in this design.** None is proposed; section 6 lists the beads it
would become.

## 5. Open questions

1. **What a topic is.** Option A: a bead label, as section 3.3 proposes —
costs nothing new; the yard adopts topic labels (`linear-out`, `web-view`,
`design-lane`) and the window groups by the bead's first. Option B: a
train — costs fitting every topic to a parent bead with children, which
suits the design lane and not "Linear out of the core". Option C: the
outside issue the beads track (`--tracks`, a Linear issue) — costs nothing
new where beads already track one, and nothing groups where they do not. I
would take A: labels exist, route already, and group what trains and
tracked issues cannot.
2. **How an ask is encoded.** Option A: JSON in `Note.Text` of a note of
kind `ask` — costs a schema every renderer parses, and prose tools see raw
JSON. Option B: a fixed readable text shape (question, context, numbered
options, recommendation) parsed back strictly — costs a parser, and prose
that drifts from the shape silently stops being picks. I would take A: one
schema, parsed the same everywhere, with prime and review rendering it as
text so nobody reads the raw form.
3. **When a put-aside question returns.** Option A: 24 hours or the asker's
next note, as section 3.5 proposes — costs a clock the queue checks. Option
B: the asker's next note only — costs nothing, but a question the asker
never touches never returns. Option C: a choice per question (tomorrow, a
date) — costs the choice and its interface. I would take A: nothing is
lost, and nothing waits on an asker gone quiet.
4. **Where the decisions live in the window.** Option A: a list above the
conversations in the same window, as section 3.3 sketches — costs room in
the left column. Option B: its own view, switched with a key — costs a
mode, and the conversations are hidden while deciding. I would take A:
deciding with the asker one row away is the point, and three groups still
leave room for the crew.

## 6. The beads it would become

For depot yardr, in order; then for depot footplate, which depend on the
yardr socket bead except the yard row, which needs only calls already
stable. Each is small enough that a builder finishes it without asking.

**Yardr 1. Any bead carries asks, answered by picking.** Goal: whoever asks
writes each question once, as data, on any bead; the person answers by
picking, puts aside, or declines. Where to look: the `Note` struct and its
kinds in `internal/store/graph.go`; `yardr bead note` in `internal/cli`;
`yardr prime --bead` and `yardr review` where notes are printed; docs/chat.md
for the text shape. Done when: `yardr bead ask <id> <question> --to …
--context … --option … [--recommend n --why …] [--about …]` stores an ask
(refusing missing context or a consequence-less option) with `you decide`
added; prime, review and chat show it as numbered text with the
recommendation marked; `yardr bead answer <id> <ask> <option> [words]`
(and Later and Decline) records a note of kind `answer`, a newer pick
supersedes for display, and every declined or put-aside ask says so
plainly.

**Yardr 2. The socket API serves the decisions.** Goal: a client that is
not yardr reads the person's queue and one bead whole, and writes answers,
notes and dismissals. Where to look: docs/api.md ("The calls", "The
stream", "What is stable"); the store calls behind `yardr review list`,
prime and `bead answer`; `src/api.rs` and `src/fake.rs` in footplate for
what a client does with the page. Depends on Yardr 1. Done when: new
documented calls read the queue (question, asker, waits-since, topic, bead
id and title) and one bead (body, notes, asks, about-beads) and write an
answer, a note and a dismissal; new documented events carry asks answered
and beads changed; and the api.md test holds the page to the server as it
does today.

**Footplate 1. The yard row and the decisions list.** Goal: the window
shows the yard's notices apart from the conversations, and above them the
person's open questions grouped by topic, in plain words. Where to look:
`src/api.rs` (`user_inbox` and `ack_user_inbox`, beside which the new calls
sit), `src/model.rs`, `src/session.rs`, `src/fake.rs`; `UserNotifications`
and `AckUserNotifications` in docs/api.md of the yardr repository, already
stable; the new calls from Yardr 2. Depends on Yardr 2 for the list; the
yard row depends on nothing new. Done when: the `yard` row stands below
the conversations with its own unread count and one-key dismiss, the
decisions stand above grouped by topic with rows leading with the question,
both follow the events without asking in a loop, and the fake yard answers
the calls in tests.

**Footplate 2. The card: pick, words, dismiss, one step away.** Goal: an
open ask is answered by picking with words alongside, put aside, or
declined; the full bead is one step away. Where to look: Footplate 1;
`src/input.rs` for the line. Depends on Footplate 1. Done when: the card
shows context, options with consequences and the recommendation marked,
Enter sends the pick with the line's words, words alone go as a note and
leave the ask open, Later and Decline do what section 3.5 says, a keypress
expands the card to the full body and notes, and nothing waiting is buried
by mail or notices.
