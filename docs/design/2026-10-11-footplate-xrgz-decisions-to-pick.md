# Decisions as something to pick in footplate, with the beads in view

Design footplate-xrgz, proposed 2026-10-11. A proposal as it was decided on, not a description of what the code does today; the decision is on the bead.

Design for bead footplate-xrgz (flow `design`, stage `draft`). A bead is
one durable piece of work; a depot is a repository beads are worked in; a
flow is the stages a bead goes through, and a stage one step of it; a crew
member is an always-on agent of the yard, and the yardmaster the one that
coordinates; a user is a person of the yard, with an inbox; mail is a
message between parties of the yard; a note is evidence attached to a bead.
See docs/glossary.md in the yardr repository for all of yardr's words.

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
with four or five open questions, a yes or no before something changes the
live yard, 'say build' for a shaped bead, a choice of order. The answers
were a few words each, and several times an answer could have meant two
things. Routine notices arrived in the same stream and buried the
questions. Nothing about this is a mechanism: it is talk, and the talk is
the wall of text.

**Where decisions already have a home: a bead in a manual stage.** A flow
routes a stage to a group; a `manual` group starts nothing, and the bead
waits there until a member moves it (docs/groups.md in the yardr
repository, "Runners"). The depot's `decide` stage is routed to the manual
group `decisions`, whose members are mckean and the yardmaster
(docs/flows.md, "Worked example: depot yardr"). The stage file
`.yardr/flows/default/decide.md` in the yardr repository already says the
rule this design keeps: "The bead carries the question and the options,
written by whoever sent it here", and "A bead leaves once the answer is on
it, as a note." A decision about no one piece of code is a bead created
straight into `decide`, closed with the answer as its reason. So the
question, the options and the answer already live on the bead, as prose in
its body and its notes; what is missing is only that the options are
something to pick.

**The queue already exists: `yardr review`.** `yardr review` walks the open
beads in stages routed to manual groups, longest untouched first, showing
each bead's stage, description, last notes and diff stat, and the person
picks where it goes next: the outcomes the flow names for the stage, else
the stages the flow allows (docs/review.md). `yardr review list` prints the
same queue for scripts, with `--json` for actions. This is each question in
its own place, ordered by waiting longest, and it is what the footplate
view mirrors; no new queue is proposed.

**Notes already carry a kind.** A bead's notes are the evidence on it
(`yardr bead note <id> <text>`), and each note has a `Kind` field, today
`""`, `note`, `journal` or `handoff`
(`internal/store/graph.go` in the yardr repository, the `Note` struct). A
structured question can therefore be a note of a new kind, read by the same
code that reads notes, rather than a new kind of record.

**Mail, conversations and notifications are separate already, in yardr if
not yet in footplate.** A user's inbox has two parts, Mail and
Notifications, each acknowledged apart, so a stream of notifications nobody
reads does not hold back what counts as read mail (docs/users.md, "The
inbox"). `yardr chat` shows the crew's conversations and, above them, the
`yard` row with the notifications, which has no line to type in
(docs/chat.md, "The yard"). Footplate today shows only conversations
(`src/model.rs`: rows of crew and linked parties, the open conversation,
`Effect::Read` and `Effect::Ack`); it has no `yard` row yet (README,
"Not in this slice"). The decisions view must join that separation, not
blur it: what waits for an answer is a third thing, neither talk nor
report.

**What footplate may ask of the yard.** Footplate is an interface over
yardr's socket API and starts nothing of yardr's (README). It speaks only
what docs/api.md of the yardr repository promises a program that is not
yardr: HTTP/1.1 over the unix socket, a call a `POST /call/<Method>` with
its arguments as a JSON object, the events a `GET /events?after=<seq>`
(`src/api.rs`). That page promises the calls a chat client needs — `Head`,
`Crew`, `Peers`, `GetUser`, `Settings`, `UserInbox`, `UserNotifications`,
`Conversation`, `Mail`, `MailPeer`, `PutBlob`, `AckUserInbox`,
`AckUserNotifications` — and the events `mail`, `mail_read`,
`crew_status`, notifications and `notifications_read`. Everything else the
socket serves is yardr's own and changes without notice; a client must not
call it ("What is stable"). Beads, notes, asks and moves are all in that
"everything else" today. Consequence: with yardr as it is today, footplate
can show no bead and record no pick; every bead the decisions view touches
needs a yardr-side socket call first. Section 6 names those as yardr beads,
apart from footplate's own.

**How footplate follows the yard.** On its own thread, `session::follow`
reads the state once (`session::load`: `Head`, `Crew`, `Peers`, the user,
the unread mail) and then applies the stream after the head, re-reading
when the stream ends (`src/session.rs`). The window holds a `Model` of
plain data and renders rows on the left, the open conversation on the
right; anything needing a call comes back as an `Effect` the window runs
(`src/model.rs`, `src/window.rs`: `Footplate::change`, `Footplate::run`).
The tests run without a yard and without a screen, against a fake yard
(`src/fake.rs`). A decisions view follows the same shape: state read once,
events applied, effects for calls, a fake yard extended for tests.

**The evening's cases, as beads.** The design bead with open questions
(compare yardr-me78, decided 11 Oct 2026: its document
docs/design/2026-10-11-yardr-me78-depot-rules.md, its writer's note listing
four open questions with the writer's pick for each, its critic's note, the
decision recorded in a yardmaster note) is a bead in `decide` whose body
and document hold several questions at once. 'Say go', 'say build' and the
choice of order are single questions, each answerable with one pick. 'You
decide for the others' is delegation, and must be a pick like any other.

## 3. The proposal

The smallest version that answers the question has three parts: a question
is data on a bead, written once; every view draws the same data, footplate
as something to pick; the answer goes back as the option picked, so it
cannot be misread.

The existing mechanisms carry it: the bead in a manual stage carries the
question (decide.md already says the bead carries the question and the
options); the `yardr review` queue carries "each question in its own
place"; notes carry the writing, with a new note kind for the structured
part; the flow's outcomes and next stages carry "what picking each one
does" wherever the pick is a move. What is new is only: the structured
shape of a question (an ask), the commands that write and answer one, the
socket calls that let footplate read and write them, and the footplate
view.

### 3.1. An ask: the question as data

An ask is one question on a bead, with enumerable options. Whoever sends a
bead to a manual stage writes one ask per question with:

```
yardr bead ask <id> "Take the depot's rules from .yardr/depot.md?" \
  --option "yes, as proposed: the brief appends the file" \
  --option "no: keep the roles as forks" \
  --recommend 1 --why "one file, read from the base, nothing copied"
```

An ask holds: the question in one line; two to five options, each a label
and what it means in a line; the one recommended and why in a line; and,
per option, what picking it does (below). It is stored as a bead note of a
new kind `ask`, so `yardr prime --bead` and `yardr review` show it as
numbered text with no new plumbing, and the answer is a plain note naming
the option picked. A question with no enumerable options ("what should the
tone of the greeting be?") is an ask with no options: the views show it as
words wanted, and the answer is words (section 3.5).

A design bead in `decide` holds one ask per open question: five questions
are five asks on the one bead, each answered apart. A yes-or-no, a 'say
build' and a choice of order are one ask each. The writer writes them once;
footplate draws them as something to pick, and `yardr chat` and the pane
draw the same thing as numbered text, as the yardmaster's idea has it.

### 3.2. What a pick does, and how it is undone

A pick records an answer, and optionally moves the bead. Each option of an
ask names what picking it does: either nothing but the record, or one move
— an outcome of the flow for the stage (`done`, taking the bead where the
flow says), a named stage, or closing the bead with the pick as its
reason. The default is record only. The writer states the move per option
where the case has one ('say go' closes the decision bead with "go" as its
reason; 'say build' moves the shaped bead to `new`); the five questions of
a design bead record only, and the person moves the bead once, after the
last answer, as today.

Recording is a note: who picked, which ask, which option, and any words
with it. The answer cannot be misread because it is the option, not a
paraphrase of it. Anything the pick sets in motion beyond the move is the
asker's to do on reading the answer ('say go and I move the links': the
yardmaster moves the links when the pick lands); yardr records, the asker
acts. No automation beyond the named move is proposed.

A pick stands until it has taken effect, and until then it can be changed
or undone. "Taken effect" means the bead moved on the answer, or the asker
acted on it. Before that, a newer pick on the same ask supersedes the older
one: the latest pick is the answer, each pick a note, so the history stands
and nothing is silently rewritten. Undo is a retraction, recorded the same
way, which leaves the ask open again. Footplate shows the current pick on
the card and offers "change" while the ask is open; once the bead has moved
or the asker has marked the answer taken (a note saying so), the card says
the answer stands, and changing it is a new conversation — a note, or the
bead moved back by hand.

### 3.3. 'You decide': handing a question back

Handing a question back to the crew member is one of the picks. yardr adds
`you decide` to every ask with options; the writer does not write it and
cannot leave it out. Picking it records that the person left this question
to the asker, with any words ("you decide for the others; keep the paths
short"), and moves the bead as the ask's `you-decide` move says — by
default back to where the asker works it (the stage the bead came from, or
`backlog` for reshaping where the flow has no way back), named on the card
before the person picks it. The asker reads the delegation as it reads any
answer, decides, and notes what it took. A whole evening of "your proposal
is sound" becomes one pick per question; "you decide for the others"
becomes one pick on each of the rest.

### 3.4. What the person sees

The window keeps its shape: rows on the left, the open thing on the right.
Above the conversations stands a third list, the decisions: the beads in
manual stages routed to groups the person is in, longest waiting first, as
`yardr review` walks them. Each bead is one card, and each card shows the
bead's title, its stage, and the two or three facts that matter: who asked,
how long it has waited, and what the writer recommends. The full bead and
its document are one step away, never on the card: a keypress or click
expands the card in place to the whole body and notes, and the document of
a design bead is named by path and branch (`docs/design/…` on
`yard/<bead>`), to open outside footplate. An ask answered shows its pick;
an ask open shows its options as buttons, the recommended one marked, with
the line under for words with the pick.

One question with three options. The card is the whole of the right side;
the left list shows the one decision above the conversations:

```
  decisions (1)                  │ yardr-n8ta · decide · waiting 2h
> yardr-n8ta  go?             1  │ Move the depot links to the new paths
  ─────────────                  │ asked by yardmaster · recommends: go
  yardmaster  idle               │   go and I move the links
  assembly    working            │
                                 │ ○ 1. Go — move the links now
                                 │ ○ 2. Not yet — after Friday's release
                                 │ ○ 3. You decide — leave it to yardmaster
                                 │   recommended: 1, one file already moved by hand
                                 │ ─────────────────────────────
                                 │ > Go, and tell assembly [       ]
                                 │   enter picks 1 · 1-3 pick · esc leaves it
```

A design bead with five open questions. The card lists the five asks, each
with its options folded to one line until selected; the recommended picks
stand, so answering "your proposal is sound" is one key per ask. The critic's
note is named, not quoted; the document is one step away:

```
  decisions (1)                  │ yardr-q2w9 · decide · waiting 1d
> yardr-q2w9  5 questions     5  │ Design: depot working rules in the brief
  ─────────────                  │ asked by yardr-q2w9-draft · doc: docs/design/… on yard/yardr-q2w9
  yardmaster  idle               │ critic's note by yardr-q2w9-critique · e expands all
                                 │
                                 │ 1. Path of the rules file · ★ .yardr/depot.md ✓ picked
                                 │ 2. Pack fallback · ★ depot-only, no fallback
                                 │    ○ depot-only — the file is the depot's alone
                                 │    ○ fall back — read the pack's file too
                                 │    ○ you decide
                                 │    recommended: depot-only, one place to look
                                 │ 3. prime shows it · ★ briefs only
                                 │ 4. Adoption · ★ delete the builder/reviewer forks
                                 │ 5. Merger role · no recommendation — words wanted
                                 │ ─────────────────────────────
                                 │ > [                                                   ]
```

Ten things waiting at once. The left list is the queue, longest waiting
first, each row one bead with its count of open asks; the right side shows
the selected card. Nothing waiting is buried: the conversations stand below
a rule, and the decisions keep their own count:

```
  decisions (10)                 │ yardr-me41 · decide · waiting 3d
> yardr-me41  order?          1  │ Build this design before yardr-me78?
  yardr-n8ta  go?             1  │ asked by yardmaster · recommends: yes, this first
  yardr-q2w9  5 questions     4  │
  yardr-b7zz  build?          1  │ ○ 1. Yes — this design first
  yardr-k1p0  wording         1  │ ○ 2. No — the other one first
  yardr-p4aa  2 questions     2  │ ○ 3. You decide
  … 4 more                       │   recommended: 1, it unblocks two builders
  ─────────────                  │ ─────────────────────────────
  yardmaster  idle           2   │ > [                                   ]
  assembly    working            │
```

### 3.5. What still needs words, and how a pick and a sentence go together

Three things still need words. First, conditions and nuance on a pick
("go, but after Friday"; "option 2, and keep the old path working a
week"). Second, an answer the options did not offer ("none of these: move
only the docs links"). Third, a question with no enumerable options, asked
as an ask with none. The card carries one line under the picks for the
sentence. Enter with an option chosen sends the pick with the words, and
both are recorded in the one answer note; words with no option chosen go as
a plain note on the bead, and the ask stays open. Talk about the question —
why it is asked, what the options assume — stays where talk lives, in the
conversation with the asker; the card's line is for the answer, not the
discussion.

### 3.6. Questions apart from notices

What waits for an answer is never buried by what only informs. Three lists,
three counts, three positions: the decisions (asks open, longest waiting
first), the conversations (mail, answered in turn), and the yard's notices
(the `yard` row, which footplate gains with the notifications calls it
already knows from the socket page). Opening a card marks nothing else
read; reading mail marks no ask answered; a night of landed beads fills the
notices and leaves the decisions where they were. The decisions list is
ordered by waiting longest, like `yardr review`, so the oldest question is
the first row, not the last line of a stream.

### 3.7. What footplate needs from yardr, and what works as yardr is today

Footplate needs five things it has not got, each a yardr bead in section
6: the ask and its rendering in prime and review; the answer, with its
move and its `you decide`; the socket calls to read the queue, read one
bead whole (body, notes, asks, allowed moves) and write an answer, a note
and a move, documented in docs/api.md; the events that let the view follow
asks answered and beads moved without asking in a loop; and the same asks
drawn as numbered text in `yardr chat` and answered by number, so the
terminal draws what the window draws.

With yardr as it is today, the convention works and the interface does
not. A crew member can already put the question and the options on a bead
in `decide`, a person can already answer in a note and move the bead, and
`yardr review` already walks the queue longest-waiting first — all without
new code, and the yard should work that way from now on, so the questions
leave the pane before the interface is built. What no client can do today
is draw it: the socket API promises no bead, no note and no move, and
footplate reads nothing it does not promise. Until the yardr beads land,
the decisions view is `yardr review` in a terminal.

## 4. Left out

**A general forms system.** An ask is one question with up to five options,
not a form with fields, branches or validation. Anything richer stays prose
on the bead.

**Notifications and mobile.** Being told outside the yard of an open ask is
the push mechanism as it is (docs/users.md, "Told outside the yard"):
mail and subscribed notifications, nothing new. A bead waiting long in a
manual stage already raises one `waited` event (docs/groups.md, "A bead
that waits long").

**The asker acting by itself.** Picking 'go' does not move the links; it
records "go", and the yardmaster moves them. yardr takes no action on a
pick beyond the move the option names. A pick that runs a command is left
out until use asks for it.

**Reading the document's bytes.** The card names the design document by
path and branch; footplate does not fetch it. Serving branch files over the
socket is a larger promise (which commit, which access) and is not this
design's to make.

**Editing an ask after it is answered.** A pick superseded is history, kept
as notes; the ask itself is not rewritten. If the question was wrong, the
asker writes a new ask and notes the old one withdrawn.

**signalbox.** Another view of its own; not this.

**Code in this design.** None is proposed; section 6 lists the beads it
would become.

## 5. Open questions

1. **Where the ask lives.** Option A: a bead note of kind `ask`, as section
   3.1 proposes — costs a new kind beside `note`, `journal` and `handoff`,
   and `yardr bead ask` to write one; prime, review and the socket read it
   as they read notes. Option B: a fixed section of the bead body ("Open
   questions", as the design documents write them) — costs a parser, and
   prose that drifts from the shape silently stops being picks. Option C: a
   record of its own beside notes — costs a new concept, new storage and
   new rendering everywhere. I would take A: the kind field exists for this,
   and the ask stays readable as text wherever notes are shown.

2. **Whether a pick may move the bead.** Option A: each option names its
   move, default record-only, as section 3.2 proposes — costs the move on
   the option and the refusal when the flow disallows it. Option B:
   record-only always; every move by hand after — costs nothing new, but
   'say build' stays two acts (answer, then move) where one would do, and
   the evening's shortest answers stay the most cumbersome. I would take A:
   the move is where "what picking each one does" is kept, and the default
   keeps the design bead's five answers as pure record.

3. **What `you decide` does to the bead.** Option A: it moves the bead back
   to the asker along a named edge, as section 3.3 proposes — costs naming
   that edge per ask (with the default the stage the bead came from). Option
   B: it records the delegation and leaves the bead where it is, for the
   person to move by hand — costs nothing, but leaves a delegated question
   sitting in the person's queue until they move it, which buries nothing
   but litters the list. I would take A: a handed-back question should leave
   the person's queue in the same act.

4. **Whether `yardr chat` draws asks now or later.** Option A: with this
   design — the numbered text and answering by number land with the ask
   itself, so the terminal and the window draw the same data from the start;
   costs a yardr bead in this chain. Option B: later — footplate first, chat
   after; costs a window where the terminal still shows the ask as prose to
   be answered in words, and two shapes of answer for one question. I would
   take A: one question, one shape of answer, in every view.

5. **Where the decisions live in the window.** Option A: a list above the
   conversations in the same window, as section 3.4 sketches — costs room in
   the left column, and the window holds three lists at once. Option B: its
   own view, switched with a key — costs a mode, and the conversations are
   hidden while deciding. I would take A: deciding with the asker one row
   away is the point, and ten waiting rows still leave room for the crew.

## 6. The beads it would become

For depot yardr, in order; then for depot footplate, which depend on the
yardr socket bead. Each is small enough that a builder finishes it without
asking.

**Yardr 1. A bead carries structured asks.** Goal: whoever sends a bead to
a manual stage writes each question once, as data. Where to look: the
`Note` struct and its kinds in `internal/store/graph.go`; `yardr bead
note` in `internal/cli`; `yardr prime --bead` and `yardr review` where
notes are printed. Done when: `yardr bead ask <id> <question> --option …
--recommend n --why … [--move …]` stores an ask, prime and review show it
as numbered text with the recommendation marked, and an ask with no options
shows as words wanted.

**Yardr 2. An answer is the option picked.** Goal: answering records the
pick as a note and takes the option's move, and `you decide` is on every
ask. Where to look: Yardr 1; the flow's outcomes and next stages
(docs/flows.md, "Outcomes"); the `decide` stage file. Done when: `yardr
bead answer <id> <ask> <option> [words]` (and the same picks in
`yardr review`) records who picked which option with what words, takes the
move the option names or refuses it with the flow's words, a newer pick
supersedes while the bead has not moved on the answer, and `you decide`
records the delegation and moves the bead along the ask's named edge.

**Yardr 3. The socket API serves the decisions.** Goal: a client that is
not yardr reads the queue and one bead whole, and writes answers, notes
and moves. Where to look: docs/api.md ("The calls", "The stream", "What is
stable"); the store calls behind `yardr review list`, prime and `bead
answer`; `src/api.rs` and `src/fake.rs` in footplate for what a client does
with the page. Depends on Yardr 1 and 2. Done when: new documented calls
read the person's queue (id, title, stage, waits-since, asks open), one
bead (body, notes, asks, allowed moves) and write an answer, a note and a
move; new documented events carry asks answered and beads moved; and the
api.md test holds the page to the server as it does today.

**Yardr 4. `yardr chat` draws an ask as numbered text.** Goal: the terminal
draws what the window draws, and the answer goes back as the option picked.
Where to look: docs/chat.md; the rendering from Yardr 1. Depends on Yardr 1
and 2. Done when: a conversation with an asker shows the bead's open asks
as numbered options with the recommendation marked, and answering by number
records the same answer note as `bead answer`.

**Footplate 5. The decisions list.** Goal: the window shows the beads
waiting on the person, longest waiting first, each its own card with title,
stage, asker, waits-since and recommendation. Where to look: `src/model.rs`
(rows, `Effect`), `src/session.rs` (`load`, `follow`), `src/window.rs`
(`list`, `talk`), and the new socket calls from Yardr 3. Depends on Yardr
3. Done when: the list stands above the conversations with its own count,
   follows the new events without asking in a loop, and the fake yard
answers the new calls in tests.

**Footplate 6. Picks, with a sentence.** Goal: an open ask is answered by
picking, with words alongside, and changed or undone before it takes
effect. Where to look: Footplate 5; `src/input.rs` for the line.
Depends on Footplate 5. Done when: the options are buttons with the
recommendation marked, Enter sends the pick with the line's words, words
alone go as a note and leave the ask open, and the current pick shows with
"change" until the answer stands.

**Footplate 7. The full bead one step away.** Goal: the card stays small;
the whole body, notes and document reference open in place. Where to look:
Footplate 5 and 6. Depends on Footplate 5. Done when: a keypress or click
expands the card to the full body and notes, the document path and branch
show where the bead names one, and nothing waiting is buried by mail or
notices in any of the three sketches' cases.
