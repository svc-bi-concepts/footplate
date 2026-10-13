# One footplate for several yards, each with its own pairing, and a switcher

Design footplate-5wz5, proposed 2026-10-13. A proposal as it was decided on, not a description of what the code does today; the decision is on the bead.

Design for bead footplate-5wz5 (flow `design`, stage `draft`). A yard is
one installation of yardr: its store, its crew and its depots. A user is a
person of a yard, a name and an inbox; the operator is the user who runs
the yard. A device is one program of a user that calls the yard from
outside it, let in by a token the yard gave it. A pairing is how a device
gets that token: the operator opens a pairing with a code
(`yardr user device pair <user> <device>`), and the device trades the code
for its token. An ask is one question as data on a bead; a run ask
proposes one yardr command of a fixed list that the operator approves, and
the yard runs exactly that command. A peer is another yard this one is
linked with. footplate is the native window in which a person reads mail
and answers asks. See docs/glossary.md in the yardr repository for all of
yardr's words.

## 1. The question

mckean, 13 Oct 2026, by mail: he plans a second yard on the Mac, for
Plantime, beside the yard footplate is paired with, and through a peer
link he would see the second yard's cards but could not approve a run
ask, because an approval never crosses a peer, on purpose. The
yardmaster offered a second footplate window, and his answer was: "we
could have a switcher in footplate, having multiple footplates defeats
the purpose of what a footplate should be and complicates." So: how does
one footplate window hold several yards, each reached by a pairing of
its own, with a switcher between them, so that a card of any of them can
be approved where it belongs?

## 2. What exists today

**One yard for a window.** Which yard the window opens is `Env` in
src/yard.rs: `FOOTPLATE_YARD` names a yard at an address and wins over
the settings, else the settings choose (`Settings::chosen` in
src/settings.rs), else it is this machine's yard by its socket
(`api::socket_path` in src/api.rs: `$YARDR_HOME/yard.sock`, else
`~/.yardr/yard.sock`). `Env::open` builds the one `Client`: `Client::new`
for the socket, `Client::remote` with the token for an address. The
window holds exactly one client (`client` in src/window.rs) and shows
exactly one yard.

**One token in one file.** A device's token is never in the settings. It
is kept in `yard.token` beside the settings file (`TOKEN_FILE` in
src/yard.rs), or in `FOOTPLATE_TOKEN_FILE`, which wins. `read_token`
refuses a file others can read and one that holds no token;
`keep_token` writes a pairing's token for its user alone. A token file
alone, without an address, names no yard: `Env::open` with no address
returns the socket client (src/yard.rs:58-61, proved in test at
src/yard.rs:221-225). The address and code as typed, including a QR
string of the form `<address>#<code>`, are split by `yard::typed`.

**One chosen address in the settings.** src/settings.rs keeps `remote`
and `address`: whether the window shows a yard at an address, and the
last address paired with, kept while this machine's yard is chosen again
to go back by. `Settings::read` answers today's look for a missing or
unreadable file; `Settings::write` keeps every change.

**One Model of one yard.** src/model.rs holds the window's plain data:
rows, unread mail, notices, asks, the open conversation, the news that
arrived since the window started (`News`, taken with `take_news`). One
Model, one yard. src/session.rs follows the yard on a thread of its own
(`follow`): the state once (`load`: head, crew, users, peers, inbox,
notifications, asks), then the stream after it, retrying every 3 seconds
(`RETRY`) while the yard is away, and ending the thread with `Update::Out`
when a device is no longer let in.

**Switching yards today already tears everything down.** Choosing the
other yard in the settings calls `reopen`, which calls `follow` in
src/window.rs, whose comment says it plainly: "nothing of that one is
shown or kept, since a name there is not the same one here." It drops
the model, the drafts, the ask words, the line, the secret, the
notification burst and the approving state, then follows the new yard.
The teardown a switcher needs exists; what is missing is keeping
anything per yard across it.

**The head names the one yard.** `which_yard` in src/window.rs tells the
whole ("the yard south at https://…, as its device laptop", from
`/user/me`, the `Me` struct in src/api.rs, or "this machine's yard");
`which_yard_short` shortens this machine's. The head of the left column
renders `me · which_yard_short` beside the logo. The settings view
(`talk_settings`) says the whole telling again, with the choice of this
machine's yard or a yard at an address, the address and code fields of a
pairing (`choose_far`, `pair`, `show_far`, `choose_near`), and where the
token is kept.

**Approving runs a command in the open yard.** A run ask's card carries
the yard's block (`run_block` in src/window.rs): the command whole, the
yard's sentences for what it changes, the command that takes it back.
Approving (`approve`) calls `ApproveAsk` with the bead, the ask and the
actor, and nothing of the command, on the open yard's client; the yard
runs what it stored. The key is `secondary-enter` (`Approve` in
`bind_keys`), which no other card answers, and Enter alone never
approves. There is exactly one yard it can run in: the open one.

**Drafts and words belong to no yard.** `Drafts` in src/draft.rs is keyed
by conversation name alone, and `ask_words` in src/window.rs by bead and
ask. The line's text moves into the drafts when another row opens and
back out when its conversation does. Switching yards today drops all of
it.

**Notifications and the taskbar count for the open yard.** `notify::wanted`
in src/notify.rs tells of the open yard's news by the settings' kinds;
`Burst` folds a burst into one notification; activating one brings the
window forward on its row (`come_forward`). src/taskbar.rs is the Linux
desktop icon and entry, not a count; the counts live in the window's
rows and the system's notifications, all of the one open yard.

**On the yardr side: the device, the approval that never crosses, and the
gaps.** Design yardr-xpf8 (docs/design/2026-10-12-yardr-xpf8-remote-footplate.md
in the yardr repository) is the remote footplate: the listener serves
the person's calls under `/user/` to a device's token, and footplate
refuses plain http off loopback. Design yardr-x3g8, section 3.5, is the
run ask: `ApproveAsk` runs the stored command once, and section 3.6 is
where an approval may come from, never a link: at the other yard the
copy stands with the line "open mac in this window, or at its terminal".
A peer carries mail as `name@peer` and copied asks, and answering there
has known gaps: yardr-8pmj (mail answered in a trusted name's
conversation stays in this yard and never reaches the other) and
yardr-nw8w (two reach names for one user: only one can answer the copied
ask). The phone, footplate-fwfq, will meet the same question of several
yards once it exists; its design is parked behind the trial.

**The other road, before the proposal: no switcher, a peer that may
approve.** One more alternative, with its costs, as the yardmaster asked.
footplate stays with one yard; the second is a peer with trust and reach
(as airy is today: asks are copied and the answer from there is taken as
the user's), and yardr lets the approval of a run ask cross a peer link
where the operator of the yard that runs the command has allowed that
peer to, off unless allowed, per peer. This is a change in yardr, to the
run ask's design (section 3.5 of yardr-x3g8 in the yardr repository), and
not in footplate. Its costs: the approval path gains a delegation (whose
peer, whose operator, which command list, recorded where, revoked how);
for two yards of one person on one machine the peer link is no real
wall, so it is defensible there, but between machines it means the other
yard's word runs a listed command here, which is exactly what rule 4 of
design yardr-r6jh ("nothing that acts follows") refuses today. And with
it, footplate would still lack the other yard itself: its mail as a
place (an answer stays local, yardr-8pmj), its crew and conversations
entirely (a peer exposes only a roster with role lines, `parties` in
src/model.rs), and copied asks keep their sharp edges (yardr-nw8w).
mckean's own measure is that the switcher is "beneficial if not running
a local yard": wherever footplate is not on the yard's machine, a peer
is not a window on it. So this road is refused as the whole answer. It
stays yardr's to take or leave as its own change; the switcher stands
either way.

## 3. The proposal

One window, one open yard, several yards kept. The settings keep a list
of yards; the head becomes the switcher; switching reuses today's
teardown and loads the newly opened yard, keeping per-yard memory aside.
Each yard is reached by a pairing of its own, as a device of its user,
with its own token; this machine's yard by its socket stays possible as
one of them, and more than one of this machine's yards can be kept, one
per home. Nothing goes from one yard to another through footplate: what
a card approves runs in the yard the card belongs to, with that yard's
token, through that yard's client alone. No yardr change is needed; all
of this is client-side.

### 3.1. Where the yards are kept

The settings keep `yards`, a list of entries in the order the switcher
lists them, and `open`, the name of the yard the window opens. Each
entry has a name to show, unique in the list, and one of two reaches:

- near: `socket`, the yard's socket path. Empty means this run's socket
  (`api::socket_path`, so `YARDR_HOME` still moves it); a path names
  another home's yard on this machine. No token, no pairing: the
  socket's trust is as today.
- far: `address` and `token_file`. The token's file is kept as a bare
  file name, resolved beside the settings file, or as an absolute path
  where the person names one; anything else is refused with its reason
  where it is typed. The default for a new entry is `yard-<slug>.token`
  from the entry's name (lowercased, runs of other characters to one
  `-`, `yard` where nothing is left), with `-2`, `-3` and so on while
  the name is taken. The token is never in the settings, as today.

No two far entries may resolve to the same token file, and no renaming
or edit may make them: the settings refuse a second entry, name or path
that would collide, where it is typed, before anything is paired or
written. Renaming an entry never renames its token file, so a rename
cannot collide with another entry's. Generation skips both names
another entry already resolves to and names a file on disk already
holds, so two entries never share a generated `yard-<slug>.token`.
Where a hand-edited file still holds two entries on one file, the
settings view refuses to pair or open either colliding entry, naming
the shared file, until they differ. Today's single-token resolution
(`Env::token_file` in src/yard.rs:47-52) has no per-entry identity;
each entry carries its own resolved file instead, and `Env` opens per
entry.

An existing settings file with one yard is moved on read, once:
`remote` with an `address` becomes two entries, first a far entry named
for the address's host, keeping `yard.token` as its token file, which
stays the one that opens, then the near entry "this machine", since the
window offered both before the list; anything else becomes the near
entry "this machine" alone. Nothing is added to a file that already has
a list: a person who removed this machine's yard does not get it back.
The deprecated `remote` and
`address` fields are still read, so mckean's own file opens, and the
next write keeps the new shape. Compatibility with anyone else's file is
not needed. Two entries may not share a name; the settings say so where
one is added or renamed.

### 3.2. One open yard

Only the open yard is held: one `Client`, one follower thread, one
`Model`, as today. Switching is today's `reopen` with per-yard memory
(3.3): tear down, open, load. The switcher rows of the yards that are
not open show last-known counts (unread mail, open asks, unread
notices), saved at each switch, dimmed and marked as when last opened.
The system tells only of the open yard. Whether every yard is held open
at once instead is open question 1; the first bead does not wait for it.

### 3.3. The switcher

The switcher stands in the head of the left column, where the yard is
named today. The head's second line shows the open yard's name and, when
it is away, the short why. Clicking the head, or one key from anywhere,
opens the list below it: one row per yard, each with its name, its kind
("on this machine" or "paired"), and its state: open, reached with
last-known counts, or not reached with the short why. Up and down move,
Enter opens, Escape leaves, clicking a row opens it. `secondary-1`
through `secondary-9` open yards in listed order from anywhere; plain
digits stay the cards' picks. The list's own key is the builder's first
free one of `secondary-y` and its neighbours, checked against `bind_keys`
in src/window.rs and src/input.rs; the head always also clicks.

What is kept for each yard across a switch: the open row, both scroll
positions, the drafts with their attachments, the ask words, and the
last-known counts. The secret field is never kept: it is cleared at
every switch, as the settings already clear it when they open. The line
always shows, and sends, the open yard's draft: at a switch the line's
text is stored with the old yard's drafts and the line is filled from
the new yard's, extending what `follow` does today (it clears; it will
save, clear, and restore).

What makes it sure a draft typed for one yard is never sent to another:
drafts and ask words are keyed by yard as well as by conversation, bead
and ask, so the other yard's send path cannot name them; the line holds
only the open yard's text; and every send, answer and approval closes
over the open yard's `Client`, which the switch replaces. A send already
on its way when the switch happens goes to the yard it was composed in,
which is where it belongs. There is no move of a draft between yards;
what was typed for one is retyped for another.

### 3.4. Always plain which yard a card belongs to

Three places, before any approval. The head always names the open yard,
as today. Every card's header gains the yard's name first, beside the
bead and the waits. The run ask's block leads with the yard and the user
it runs as ("in plantime, when you approve, the yard runs this as
mckean, and nothing else:"), and its control names the yard too
("Approve in plantime (⌘↵)"). `secondary-enter` in a yard still runs its
command there; what changes is that no card can be read, and no control
pressed, without the yard on it. No second step before an approval is
proposed here; that is open question 4 of design yardr-x3g8, and stays
its.

### 3.5. Adding and removing a yard

The settings list every yard with its kind, its reach, and its state.
Adding appends: a near yard by name and socket path (empty for this
run's), a far yard by name and address, then the pairing's code for
that yard, in the two fields as today (`yard::typed` still takes a
pasted `<address>#<code>` QR string). Pairing trades the code with that
address (`api::pair`) and keeps the token in that entry's file
(`yard::keep_token`). Each yard pairs on its own; a code is for one
yard and is used up there.

Pairing writes only to the entry's own resolved file, never to another
entry's. Removing an entry deletes its token's file only where the file
is a bare name beside the settings (the default and any name there) and
no other entry resolves to the same file; a token file named by an
absolute path elsewhere is always left alone, and a file still named by
another entry is left and the settings say which entry keeps it. Either
way the device itself stays at the yard until its operator removes it
(`yardr user device rm <user> <device>`), and the settings say that in
one line. Above the list, one sentence says what a pairing gives over a
peer link: "A paired yard is called with this device's own token, so
your picks and approvals there count as yours, where over a peer link
you only see what the link carries and an approval never crosses."

### 3.6. A yard that is not reached

Its switcher row says so, with the short why: stopped or away from the
network ("nothing answers"), or no longer let in ("pair again").
Opening it shows today's away view (`away` in src/window.rs) with the
yard's words, except the head with the switcher stays in place, so the
person goes back without the settings. The others go on: under one open
yard nothing of another is held, so nothing of another can fail; each
yard is retried on its own (`session::follow` per opening, as today),
and a 401 ends only that yard's thread (`Update::Out`).

### 3.7. What the person types and sees

Pairing the second yard, in the settings, under the sentence of 3.5:

```text
yards
  mac · on this machine · open
  plantime · paired · 2 mail, 1 decision when last opened
  [add a yard…]

plantime                                   [pair again] [remove]
address  https://plantime.example.net
code     [                        ]  [pair (enter)]
```

The switcher, open, with the first yard's window behind it:

```text
footplate
mckean · mac ▾
───────────────────────────
▸ mac · on this machine · open
  plantime · paired · 2 mail, 1 decision when last opened
  shop · paired · not reached: nothing answers
───────────────────────────
decisions (1) …
```

And a run ask's card, its yard on the header and on the control:

```text
plantime · yardr-k3vd · 2h
asked by yardmaster · recommends: approve

in plantime, when you approve, the yard runs this as mckean,
and nothing else:
  yardr peer allow airy --mail-limit 300
the yard says: …
[Approve in plantime (⌘↵)]
```

### 3.8. What the phone design takes from this

The phone (footplate-fwfq) takes the shape and the rules, not the
window: the same entries (a name to show, near by socket or far by
address, the credential beside the settings — on a phone a keychain
entry per yard, never the settings file), the same one open yard with
per-yard drafts keyed so a draft can never be sent to another, every
card and the approve control naming its yard, notifications carrying
the yard they open, and pairing by the same address and code, read from
a QR picture where a desktop pastes the string. Whether a phone holds
every yard open or one on demand is its own question; nothing here
forces it either way.

## 4. Left out

**The peer road as the whole answer.** Compared in section 2 and refused:
it is yardr's change, not footplate's; between machines it lets the
other yard's word run a listed command here; and footplate would still
lack the other yard's mail, crew and conversations. It returns only as
yardr's own decision, and the switcher stands either way.

**A second window.** The yardmaster offered it; mckean refused it: several
footplates defeat what a footplate is. One footplate, one window is
fixed.

**One list of every yard's rows.** The decisions of all yards merged
above the conversations would put cards of two yards one key apart,
against "nothing goes from one yard to another". The yards stay one
window apart, and the yard is on every card.

**A second step before an approval.** Design yardr-x3g8's open question 4
owns it. This design makes the yard unmistakable instead.

**What tells per yard.** One switch for all yards, as today. Per-yard
kinds return if someone wants mail of one yard and silence of another.

**Moving a draft between yards.** Never; retype. Copying text across
would be the one path a draft takes into the wrong yard.

**Enrolment beyond address and code.** A QR picture on the desktop adds
nothing over pasting the string `typed` already splits. The phone reads
the picture.

**Scopes on a token, a read-only yard.** Design yardr-xpf8 left them out
for the same reason: one kind of device, one set of calls.

**Finding yards by itself.** No scan of homes or sockets; entries are
added by hand, named by the person. Discovery returns if the list grows
past naming.

**Holding every yard open in the first bead.** Open question 1; the first
bead is one open yard, and is worth having alone.

**Code in this design.** None is proposed; section 6 lists the beads it
would become.

## 5. Open questions

1. **Whether footplate holds every yard open or only the open one.**
   Option A, only the open one, as section 3.2 proposes: one `Client`,
   one follower thread, one `Model`; the switcher shows last-known
   counts for the rest; the system tells only of the open yard. It costs
   the per-yard memory of 3.3 and nothing held: cheap in code, and a
   closed yard's counts go stale, marked as such. Option B, every yard
   held open: one client, thread and model per yard, the switcher live
   (unread mail, cards to decide, notices), cards of closed yards
   reaching the taskbar and notifications, each naming its yard and
   opening it on activation. It costs one model and session per yard in
   code, N streams held, notification routing across yards, and tests
   with several fake yards. I would take A: the window shows one yard,
   the first bead is the valuable one, and staleness is marked; B
   returns as its own bead if living with A misses live counts.
2. **Whether `FOOTPLATE_YARD` stays or goes, with `FOOTPLATE_TOKEN_FILE`
   beside it.** Option A, they stay: with `FOOTPLATE_YARD` set the
   window is of the one yard it names, with no switcher, exactly as
   today — the trial's window, the debugging window, design
   yardr-xpf8's first step; the token is read from
   `FOOTPLATE_TOKEN_FILE` where it is set, else `yard.token` beside the
   settings, and a pairing in that window writes to the same file.
   Without `FOOTPLATE_YARD`, `FOOTPLATE_TOKEN_FILE` is ignored:
   opening and pairing in the switcher use each entry's own resolved
   file and never the environment's. This ignoring is a change from
   today, where the override applies even when the address comes from
   the settings (`Env::token_file` in src/yard.rs:47-52 is used for
   the settings-chosen address in `Env::open`, 58-66, and for pairing
   in src/window.rs:2618). A migrated remote entry whose token lived
   only in that override path therefore opens as not paired ("pair
   again") until it is paired once more, or the person names the old
   path as the entry's absolute token file; the migration keeps
   `yard.token` (3.1) and never copies the environment's path into the
   settings. It costs the branches that exist (`Env::address`,
   per-entry `token_file` with the single-yard fallback). Option B,
   they go: every yard lives in the settings, both variables are
   ignored, and a trial edits them; the window with no home loses the
   fallback the no-home error names today (`Env::open` in
   src/yard.rs:58-67). It costs every use that names a yard for one
   run, and the tests that open a yard by environment. I would take A:
   one run's window stays one run's, and the override cannot silently
   lend one yard's token to another because it only ever applies to
   the single yard it names; the one setup it breaks is named above
   and is fixed by one pairing.
3. **The order of the yards and which one opens at start.** Option A, the
   settings' order, opening the last-open yard (`open` in the settings),
   else the first entry, as proposed: the list is the person's order,
   adding appends, and the window comes back where it was. Option B, the
   settings' order, always opening the first: one rule, and the person
   puts first what should open, but every start leaves where they were.
   Option C, this machine's yard first at start whatever the order: the
   local yard is home, but a person living in a far yard starts away
   from it daily. Each costs one kept name at most. I would take A.

## 6. The beads it would become

No yardr bead: all of this is client-side. The footplate beads land in
order; every one but the last touches src/window.rs, so they are
chained one at a time, each depending on the one before it.

**Footplate 1. Several yards in the settings and a switch between them.**
Goal: the settings keep several yards, each paired on its own, and the
head switches between them, each loaded when opened, with per-yard
drafts, ask words and last-known counts from the start, so the first
bead alone is worth having. Where to look: src/settings.rs (the entries,
`open`, migration of `remote`/`address`), src/yard.rs (`Env` per entry,
per-entry token files, `typed`, `keep_token`), src/window.rs (the
settings list with pair and remove, the switcher in the head, `reopen`
with save and restore), src/draft.rs (drafts keyed by yard),
src/api.rs (`Address::parse`, `Client::remote`, `api::pair`),
src/fake.rs for the tests. Done when: two yards (this machine's by its
socket and one paired, plus a second home's socket in tests) switch from
the head; a draft, its attachments and ask words typed for one are
untouched by the other and send only there; the switcher shows kind and
last-known counts; no two far entries can resolve to one token file, a
rename never collides, generated names skip taken ones, and removing
deletes a managed token file only where no other entry names it and
says the yard-side command; mckean's old settings file opens as one
entry; and `.yardr/test` passes.

**Footplate 2. The open row and the scroll are kept per yard.** Goal: a
switch comes back to the open row and scroll of each yard, not to the
top. Where to look: Footplate 1; the selection and the two scroll
handles in src/window.rs, saved into the per-yard memory at each switch.
Depends on Footplate 1. Done when: each yard reopens on its row, each
conversation and decision view at its scroll, and `.yardr/test` passes.

**Footplate 3. Every card and the approve control name the yard.** Goal:
no card can be read and no approval pressed without the yard on it, a
run ask above all. Where to look: Footplate 2; the card headers, the run
block and its control in src/window.rs, the card structs in src/model.rs
where the yard's name must reach them. Depends on Footplate 2. Done
when: every card header leads with the yard, the run block leads with
yard and user, the control reads "Approve in \<yard\> (⌘↵)", and
`.yardr/test` passes.

**Footplate 4. A yard not reached says so and the others go on.** Goal:
the switcher shows an unreached yard with its short why, and opening it
shows the away view with the switcher still in place. Where to look:
Footplate 3; the switcher rows and `away` in src/window.rs,
`session::follow` and `Update::Down`/`Update::Out` for the whys.
Depends on Footplate 3. Done when: a stopped yard, one away from the
network, and one no longer letting the device in each show their why in
the switcher and in the away view, switching back needs no settings, and
`.yardr/test` passes.

**Footplate 5. Every yard held open (only if open question 1 goes B).**
Goal: the switcher is live and closed yards notify. Where to look:
Footplate 4; one client, follower thread and model per yard in
src/window.rs and src/session.rs, per-yard news in src/model.rs and
src/notify.rs (each notification naming its yard and opening it on
activation). Depends on Footplate 4 and on the decision. Done when: the
switcher shows live unread mail, cards and notices per yard, a card of
a closed yard notifies with its yard and opens it, one yard's outage
touches nothing of the others, and `.yardr/test` passes.
