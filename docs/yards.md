# Yards

What footplate talks to and how: the yard's socket and its API, a yard at an
address, a hosted yard, the yardr it depends on, setting a yard up here, and
starting and stopping it. Part of footplate's manual; the
[README](../README.md) is the front door.

## What it talks to

The yard's server, over its unix socket: `$YARDR_HOME/yard.sock` when
`YARDR_HOME` is set, `~/.yardr/yard.sock` otherwise. It speaks what
`docs/api.md` of the yardr repository promises a program that is not yardr,
and nothing else: HTTP/1.1 on the socket, a call a `POST /call/<Method>` with
its arguments as a JSON object, the events a `GET /events?after=<seq>` of one
JSON object a line. `src/api.rs` is the whole client.

Or a yard that is not on this machine, at the address of its listener: see
[A yard at an address](#a-yard-at-an-address). The calls are the same ones.

Beside the yard it speaks to four sites that are no yard's: yardr.dev, for
yardr's installer when you ask for it ([yardr, a dependency](#yardr-a-dependency)), PostHog's EU
capture, to which [the Feedback form](window.md#feedback) sends a message when you
press Send, and never otherwise; and OpenAI's Live API or ElevenLabs, for the
selected voice provider. [Voice settings](window.md#settings) describes what
an explicitly started session sends and keeps.

| It reads | For |
|---|---|
| `Head`, then `GET /events` after it | what happens from then on |
| `Crew`, `Peers`, `Users` | whom there is to talk with: the crew, `<name>@<peer>` for each one a linked yard exposes, and the users of this yard, yourself among them. A peer's `trust` says which of its names are a user of this yard speaking. `Crew` is also read for the one key of a session's `handle` that says it sits at its harness's login screen, and for the one that says the yard leads that login: docs/api.md names no key of the handle, and the crew is read again on the events that tell of it (`nudge_held`, `login_led`) |
| `GET /user/me` | who the device's token is for: read with the state, so when a yard at an address is opened and again each time it is read whole; never on the socket, which has no such path, and no event asks for it again |
| `GetUser`, `Settings` | who the window is for; `Settings` also for the yard's cap on a file and how many a letter carries, before one is sent |
| `UserInbox` | the unread mail, counted on each row |
| `UserNotifications` | what the yard reports, counted on the `yard` row |
| `DeliveryFiles` | the files of each hook delivery the open `yard` row shows, once for each when its notice first shows, and once more when the yard's `hook_files` for it says they were fetched (they come after the delivery and its notice): shown under the notice as a mail's files are, a line for each one the yard did not take |
| `UserAsks` | the open questions above the conversations, grouped by topic, with the decided folded under |
| `Arrivals` | the arrivals pane of Lines: the open beads still travelling, each with its expected arrival and remark, on the panel of tracks and the departures board. Read with the state, and again on every event about a bead and on the other changes docs/api.md names with the call |
| `Flows` | each line of Lines, a depot with its flows, their stages in travel order and the beads on each; the beads that travel with no line; and the travelling total the arrivals row shows. Read as the arrivals are, and again when a depot is removed |
| `SourceGrants` | the Grants row's pane, where the window runs as the yard's operator: every source × holder × call. Read with the state, when the row is opened, and on a `source_granted` event or a change of operator; a revoke tells no event. Not made for a hosted yard, which serves the list to no request that comes down the link from its edge, nor again once a yard has refused it so: the row and the pane say the grants are read on the yard's machine |
| `Bead` | the open bead whole: each ask's options, recommendation and words, the body, notes and about-beads. Also once for each id the open mail names that looks like a bead's and that no list holds, to learn whether the yard has it: a bead that closed still chips |
| `SessionScreen` | the Screen control, where the window runs as the yard's operator (a crew member's conversation, a bead a session works): what that session's pane shows, read when the screen opens and at its Refresh. Not made for a hosted yard, which reads a screen to no request that comes down the link from its edge, nor again once a yard has refused it so: the card says the screen is read on the yard's machine |
| `TelemetryShow` | the settings' Telemetry section, where the window runs as the yard's operator: the switch, the install id and the next payload. Read with the state, and again when the switch is turned and on the event that says it changed. Not made for a hosted yard, which serves the preview to no request that comes down the link from its edge, nor again once a yard has refused it so or said it has no such call: the section is then not offered |
| `Teammates` | the Teammates row's pane, where the window runs as the yard's operator: one row per teammate, with their yard name, email, role and paired-or-waiting state. Read with the state, when the row is opened, and on a `user_*` or `device_*` event or a change of operator |
| `DevicePairState` | the Pair a phone pane, where the window runs as the yard's operator on its machine: the open pairing again — its device, code, expiry and QR string — with the state, when the pane opens, at its expiry and on a `user_*` or `device_*` event. Not made for a yard at an address, nor where the window is not the operator's: the pane is not offered |
| `Conversation` | the mail both ways with the one whose row is open, and with each name trusted as them; on the `yard` row with the yard and with each sender who has no row |
| `GET /blobs/<hash>` | an image a mail of the open conversation or of the open `yard` row carries, or a hook delivery of the open `yard` row, once for each hash |

| It writes | When |
|---|---|
| `Mail` | Enter or the Send button, or a [voice send](window.md#settings), in a conversation with a crew member or a user of this yard, your own too |
| `PutBlob` for each file, then `Mail` with their hashes and names | Enter or the Send button, when files are attached to the draft |
| `MailPeer` | Enter or the Send button, or a [voice send](window.md#settings), in a conversation with one of a linked yard, or in a user's row when a name trusted as them wrote last; after `PutBlob` for each attached file, with their hashes and names |
| `React` | a tap on one of the bar of reactions floating over the hovered mail: the emoji, or nothing on the user's own current one, which takes it away |
| `AnswerAsk` | Enter, or the Answer button, on a card: the option chosen — the recommendation until another is picked — with the line's words in the one answer; an ask with no options is answered with words. A mapping pick sends `weighed` after the weighty gate |
| `AddNote` | Enter, with words and no option chosen and none recommended: a plain note, the ask stays open |
| `PutBlob` for each file, then `AddNote` with their hashes and names | Enter, on a card whose note has files attached |
| `DeferAsk` | `x`, on an open card: the ask put aside, to come back as the yard says |
| `DeclineAsk` | `d`, on an open card: the ask declined with words, not a pick and not "you decide"; a secret ask with none |
| `GiveSecret` | Enter, in the field of a secret ask's card: each name the ask asks for with the value typed |
| `ApproveAsk` | `cmd+enter` (`ctrl+enter` on Linux) or the Approve button, on a run ask's card: the ask named, and nothing of the command |
| `ApproveSetup` | the same key and button, on a setup ask's card: the preset pick with the account together — the typed values, or the candidate taken. A value is in the call's body and in nothing else of the window |
| `TelemetrySet` | the Telemetry section's Turn on or Turn off button, and nothing else: the switch named, with the operator. Where the yard refuses the change the section says the switch is changed in the yard's web view, and the call is not made again by another path |
| `UninviteTeammate` | a teammate row's Remove button, and nothing else: the teammate named, with the operator. Files the removal as a run ask on a new bead, and says the bead and the ask; the run-ask card in the queue is where approval happens |
| `DevicePairOpen` | the Pair a phone pane's Open button, and nothing else: the device name typed, for the operator's user, with the operator. Where the call ends without a reply the pane says the yard did not answer and reads the pairing again |
| `DevicePairClose` | the pane's End button, and nothing else, with the operator. Where the call ends without a reply the pane says the yard did not answer and reads the pairing again |
| `AckUserInbox` | a conversation or the `yard` row showed mail that was unread |
| `AckUserNotifications` | `x`, on the open `yard` row: one notice read; shift-`x` or the head's button: every notice read |

It follows the stream and asks nothing of the yard in a loop: `mail`,
`reaction`, `mail_read`, `notifications_read` and `crew_status` are applied
as they come, the crew is read again on the other `crew_*` events, the linked
yards on `peer_*`, the users on mail from a name of this yard that has no
row, the notices on an event that notified the user, a hook
delivery's files on its `hook_files`, the
queue on every `noted` event and once more at each put-aside ask's
return, and a bead shown on every event whose bead it is. Mail and
notices arriving never cover or move the open card.

Those reads may answer out of the order they were made in: an answer older
than one already applied of its kind is dropped. A failed read is made
again 3 seconds later while the yard answers, until it does; the two Lines
calls of an older yard stay marked as too old, and so does `DeliveryFiles`: its
notices show no files. A yard that is not reached
is read whole when it is back. A read that gave up for its timeout says the
yard is slow to answer, since it changed nothing; a write that did says the
action may or may not have happened, and is not asked again. A write that
gave up while connecting sent nothing, and says it is safe to send again.

A stream says nothing while nothing happens, and so does one that a proxy
or a tunnel holds open after the yard behind it went away. After a minute
of silence the yard's head is asked beside the stream, on a connection of
its own, and once a minute from then on: a head level with the last event
read leaves the stream as it is, and one still beyond it about a minute
later replaces the stream, as when a stream ends, so a stuck stream is
replaced within about two minutes and a few seconds. A link
that went away is found by the system's keepalive within about a minute.

A stream that ends, whether the server replaces itself or a proxy or the edge
cuts it, is opened again after the last event read, as the API says, and the
yard sends what came after it: nothing is read again and no view is drawn
again for it. The state is read whole only where the stream cannot be taken
up there: it cannot be opened, the yard does not say its head or its head is
behind the last event read (a yard rewound), or an event cannot be read. A
state read again that is the one the window has, with nothing heard since,
changes nothing in the window.

**Who you are** is not typed. The window acts as `yardr chat` does: as the
user named in `$YARDR_ACTOR`, else as your login name, when the yard has a
user of that name; else as the yard's operator. A yard that has neither says
how one is added (`yardr user add <name>`). At a yard's address none of
that is read: you are the user the device's token is of, as the yard's
`GET /user/me` says.

**Unread** is the yard's, so `yardr chat`, `yardr inbox` and the window
agree. Opening a conversation marks its mail read, and so does mail that
arrives in the open one. A user's mail has one position for all senders, so
mail of a conversation you have not opened holds back what came after it:
nothing is marked read that the window did not show. Every mail has a row
that shows it and counts it until then: its sender's, the row of the user
a linked yard's name is trusted as, and the `yard` row for the yard's own
mail and for anyone who has no row.

**Files.** Mail can carry files, and the window shows those of a mail
under its text: an image in a space 240 points high, scaled down to fit it,
never up; a page or any other file as a row with Open. A file's row names
it, with its size; Open takes it to the browser through the yard's web view, under
the name the browser saves it as. Each yard has its own, set in the settings. An image is read from the yard once and
kept in memory by its hash, for as long as the window runs. One the yard no
longer has (`yardr store prune` removes a file no mail names) is a line that
says so. Footplate never opens a file itself.

**Sending files.** Paste a file (`cmd+v`, `ctrl+v` on Linux) or drop files
on the window, anywhere: each is attached to the draft of the open
conversation, an image shown small above the line, anything else by its
name. A zip drops to its files, each an attachment under its base name,
and a folder to the files one level deep; the zip itself never travels.
Where a card is open instead, a dropped file is for its note, as below.
Nothing is sent yet. Write something or not, and Enter sends one mail with
the text and every file attached, each with its base name, in the order
they came; a file alone is a mail. A draft belongs to its conversation:
open another, and the text and the files wait where they were. The `×` on
an attachment removes it, and so does `backspace` in an empty line, the
last one first.

An attachment that cannot go is marked, pale in a frame of the warning
colour, with the reason above the line:

- one past what a mail carries (`files_max` of `yardr settings`, 64 when
  the yard answers none), counting what is already attached: a zip of more
  files than that is one marked row, and none of its files attaches;
- one over the yard's cap (`blob_max_bytes` of `yardr settings`, 8 MiB when
  unset), told by its bytes, whatever they are;
- one with no name the yard takes, or none that fits its 255 bytes;
- one of a zip that will not unpack: an entry the read refuses, a zip in a
  zip, or one that takes the drop past the 256 MB one drop unpacks;
- one the yard refused when it was sent, with the yard's words.

In a conversation with `<name>@<peer>` a file is attached and sent the
same way, and whether it crosses is the yard's to say: when the link does
not carry files (`yardr peer allow <peer> --send file`), the linked yard's
yardr is too old for them, or the file is over `peer_file_max_bytes`, the
yard refuses the mail, and the line says its reason. Nothing is marked then,
since no one file is at fault: the draft is back as it was. A file of
mail from a linked yard is shown like any other; one this yard did not take
is a line with the yard's reason.

While an attachment is marked, Enter sends nothing and says so: the mail
that goes is the draft as it stands, never the draft less a file. Remove
the marked one and Enter sends the rest. A send that fails puts the text and
the files back in the draft. A file the yard took before the send failed
stays in its store, named by no mail, until `yardr store prune`.

**A long text.** Enter sends what is typed or pasted as it is, however long.
The window splits nothing. The yard does: a text past one mail (4096 bytes)
goes as numbered mails, up to 16 (about 64 KB), each starting with a line like
`(1/3)`, and any files go with the first. They show in the conversation as
separate mails, in the order the yard stored them. The window does not join
them.

Past 16 mails the yard refuses the text. The window shows its words, as for
any send that did not go
(`not sent to yardmaster: a text is sent as at most 16 mails …`), and leaves
the text in the line. Save it as a file, empty the line and drop the file on
the window instead.

If the yard takes some of the mails and then fails, it says how many went
(`2 of 5 mails went to yardmaster: …`). The whole text is back in the line,
because the window does not know where the yard cut it. The mails that went
show in the conversation. Take their text out of the line before you send the
rest.

A mail to `<name>@<peer>` of a linked yard is not split. The yard takes one of
up to 16 KB and refuses a longer one the same way.

**Files on a note.** Drop files on an open card: each is attached to the
ask's note, an image shown small under the line, anything else by its name.
A note takes files as a conversation does, not pages alone. Nothing is sent
yet. Write what they show, and Enter sends one note with the words and
every file attached, in the order they came; the ask stays open. The note
is then like an agent's: its page opens in the browser from the note, and
stands on the bead's board in the yard's web view. A file belongs to its ask: open
another card, and the words and the files wait where they were. The `×` on
an attachment removes it, and so does `backspace` in an empty line, the
last one first.

An attachment that cannot go is marked, pale in a frame of the warning
colour, with the reason under the line, for the same reasons as on a draft.

While an attachment is marked, Enter sends nothing and says so: the note
that goes is the draft as it stands, never the draft less a file. Remove
the marked one and Enter sends the rest. A note with a file needs words:
without them Enter says so and sends nothing. An answer carries no file:
with a pick or a recommendation Enter says so and sends nothing — remove
the file, pick, and send the answer. A decline carries no file either, and
a secret, a run, or a taken-back ask takes no note, so a file dropped on
its card is not attached. A send that fails puts the words and the files
back with the ask. A file the yard took before the send failed stays in
its store, named by no note, until `yardr store prune`. A yard whose yardr
is too old for a note with files says so, and the note waits where it was.

**No yard.** Where no yard was ever made at this run's place, the window
says so in plain words — "No yard has been made here yet — set one up."
— and the socket, the error and the hint at `yardr status` hide under a
"details" line, shut at first, which a click or Enter opens. "Set up a
yard here" is the first of the ways below, and the marked one. Where the
home holds a yard that is only stopped, it says that instead, and Start
in the head starts it. Anywhere else it says what is wrong and names
where it looked. While it is trying, the window tries again, waiting
longer between tries: trying again starts nothing, and what the head
reads starts nothing either: `status` starts no server. When the server
comes back, the conversation that was open is open again.

Under those words the window offers the ways to a yard from there, and
goes on trying the one that is away meanwhile:

- **set up a yard here** makes the yard that is not there and starts it:
  [Setting up a yard here](#setting-up-a-yard-here). It stands under this
  run's own yard alone, while nothing answers at its socket and no yard is
  there yet, first of the ways and marked as the one to take: another
  home's yard and one at an address are not footplate's to make, and one
  that is only stopped is started from the head instead.
- **open a yard on this machine** asks for a name and a socket path. This
  run's socket is filled in; leave it to try this machine's yard, or change
  it to another home's yard socket (`<its home>/.yardr/yard.sock`). There
  is no pairing.
- **pair with a yard at an address** asks for a name, the address and the
  code its operator reads you. This pairs footplate as a *device* of one
  user of that yard, as "[A yard at an address](#a-yard-at-an-address)"
  has it. It is not yardr's peer pairing, which links two yards to each
  other and stays a command of the terminal.
- **open a hosted yard** asks for the edge's one origin, which of the
  account's yards there, and the Kratos account's email and password, in
  the window. Signing in keeps the session as the device's token is kept,
  and the road goes on to pairing the device with the code from the ready
  page, as "[A hosted yard](#a-hosted-yard)" has it.

The last three are the settings' own entries, "add a yard of this machine",
"add a yard at an address" and "add a hosted yard", in the same fields with
the same rules: Enter or the
button adds the yard to the settings' list and opens it, what keeps it out
of the list is said under the fields, and Esc leaves them. The ways stand
under any yard that is away, also one that no longer lets this device in,
but not in a window of the one yard `FOOTPLATE_YARD` names, which has no
list to add to. None of these three creates a yard or starts one. Where
the yard that is away is one at an address, "pair again" stands under the
ways on their edge, apart from them: it rights the yard that is away,
where a way opens another. Where
the yard that is away answers while a way's fields are open, the fields
and the screen stand until `esc` leaves them, and the screen says the
yard answers again: what is typed stays in the field.

## A yard at an address

The window shows one yard at a time, of the several its settings keep:
one of this machine, by its socket, as above, or one
on another machine, at the address of its listener (the yard's setting
`hook_addr`, or what stands in front of it). There it is a *device* of one
user of that yard, as `docs/api.md` ("Over the network") and
`docs/users.md` ("Devices") of the yardr repository have it: the same
calls, stream and images under `<address>/user/`, each request with
`Authorization: Bearer <the device's token>`, and `GET /user/me` for who
the token is. Under your name the head names the open yard as the settings
list it, and the settings say the whole of it: `this machine's yard`, or
`the yard south at https://mac.example.net, as its device laptop`.

**The yards in the settings.** The settings keep the yards the window
shows, each as a card with its kind ("on this machine" or "paired"), how
it is reached, whether it is the open one, and what waits in it, with
"open" and a … menu holding "pair again", "rename" and "remove" (and the
sign-in again of a hosted yard). A file that names none has one: `this
machine`, the yard at this run's socket. A rename keeps the yard's reach,
its files and what was written for it. "add a yard of this machine" takes
a name and a
socket path, for another home's yard (`<its home>/.yardr/yard.sock`), and
needs no pairing; this run's socket stands in the field to begin with. That socket is trusted as that yard: footplate accepts
what it says and sends there what you type for that entry, including secret
answers. It uses no token for a socket. Drafts, ask words and approvals stay
with their entry, so material typed for another yard does not go there.
"add a yard at an address" takes a name, the address and a code. The yard's
operator opens a pairing for the device and reads you both:

    yardr user device pair <user> <device> --url https://mac.example.net

Type them and press Enter, or "pair". What the pairing's QR code holds,
`<address>#<code>`, can be pasted into the address as it is; with no name
the yard is called by its address's host. footplate sends the code once
(`POST <address>/user/pair`), is answered with the device's token, keeps
it, and opens that yard. What the yard says of a code is shown under the
fields: that it is not the code (the yard counts a wrong one, and its fifth
ends the pairing), that no pairing is open, that the code was used up and
the device could not be added. Nothing changes then: the window shows the
yard it showed. A name another yard has is refused before the code is sent,
since the yard uses a code up.

A paired yard is called with this device's own token, so your picks and
approvals there count as yours, where over a peer link you only see what
the link carries and an approval never crosses. Each yard is paired on its
own, with a token of its own.

**Switching.** The line under your name in the head is the switcher: a
click on it, or `cmd+y` (`ctrl+y` on Linux), lists the yards under it, each
with its kind and its state: `open`, what waits in it now (`2 mail, 1
decision`, or `nothing waiting`), or that it is not reached and why.
The arrows move, Enter opens, esc leaves, a click on a row opens it, a click
outside the list shuts it and goes no further, and `cmd+1` to `cmd+9` open
the yards in the list's order from anywhere. Every
yard of the list is followed from the start, each by a connection of its
own: its counts change as its mail, asks and notices arrive, and a switch
shows it as it stands, with nothing read anew. Nothing of a yard that is
not open is shown or marked read. One yard that is away, or that no longer
lets this device in, changes nothing of the others, and one that is away is
asked again every few seconds by itself. The yard that was open is the one
the next start opens.

What you wrote and did not send stays with the yard it was written for: the
line's text, what is attached, and the words typed on a card are kept for
each yard and are there again when it is open, and none of it can be sent
from another yard. A mail that is on its way when you switch goes to the
yard it was written in; if that yard refuses it, it is back in that yard's
draft and the open one says so. Nothing typed of a secret is kept across a
switch.

**Where they are kept.** The yards are in the settings file (`yards`, each
with its `name` and its `socket`, or its `address` and `token_file`; `open`
is the one to open). A token never is: each paired yard has a file of its
own, by default `yard-<name>.token` beside the settings, written with mode
0600 and replaced by the next pairing of that yard. `token_file` is a bare
name, for a file beside the settings, or a whole path; no two yards may
have the same one, and two of a file edited by hand that do are refused,
both, until they differ; the settings file itself is no yard's token file,
and two yards of one name in a file edited by hand are told apart when it is
read (`mac`, `mac 2`). Removing a yard deletes a bare token file only when
footplate wrote it for a pairing and no other yard names it. An existing
file it did not write, a link, and a file you named by a whole path are left
alone. The device itself stays at the yard until its operator removes it
(`yardr user device rm <user> <device>`), and the settings say so. It is a
file on macOS too, not the Keychain.

A token is for the yard that gave it. The first time a token file is opened
for a yard, footplate writes that yard's address under the token, and from
then on the token is sent to that address alone: with another address, in
the settings or in `FOOTPLATE_YARD`, the yard is refused and nothing is
sent, until the device is paired there. A pairing writes a private marker
and replaces the file. If footplate cannot save an address for an older
token file, it refuses to send that token until the file can be written or
the device is paired again. Reading follows a link and requires its target
to be a regular, private file; pairing replaces a link itself and leaves
its target alone. A direct non-regular file, such as a directory or socket,
is refused before pairing.

A settings file from before there were several yards is read as it was:
`remote` with an `address` is a paired yard, called by the address's
host, with `yard.token` beside the settings as its token file, and it
opens as before; `this machine` is listed after it. Anything else is
`this machine` alone. The next change writes the list.

**By the environment.** Two variables name the one yard of a run:

    FOOTPLATE_YARD=https://mac.example.net FOOTPLATE_TOKEN_FILE=~/.config/footplate/mac.token footplate

`FOOTPLATE_YARD` is the address. The window is of that yard alone then: it
has no switcher, and the settings say so and list no yards.
`FOOTPLATE_TOKEN_FILE` is the file its token is read from, a token as
`yardr user device add` printed it on one line, or the file a pairing wrote;
without it that is `yard.token` beside the settings, and a pairing in that
window writes the same file. If that path is also used by a listed yard at
another address, footplate refuses to use it for both; name a separate file
with `FOOTPLATE_TOKEN_FILE`. It names no yard by itself, and without
`FOOTPLATE_YARD` it says nothing: a yard of the settings is opened and
paired with its own token file, never the environment's. (Before there were
several yards it also applied to the yard the settings chose. A yard whose
token lived only in that file opens as not paired now: pair it again, or
name the file as the yard's `token_file`, whole.)

**What is refused, with the reason, before anything is sent:**

- an address that is `http://` and whose host is not this machine
  (`127.0.0.1`, `::1`, `localhost`). yardr does no TLS: a proxy or a tunnel
  of the operator's stands in front of the listener, and the yard cannot
  tell from the inside whether one does, so the rule is the client's. The
  token, and a pairing's code, would cross the network for anyone to read.
  Over `https://` the certificate is checked against the roots this system
  trusts, for the address's host;
- a token file that anyone but you can read or write (any bit of mode 077):
  whoever reads it calls the yard as you. `chmod 600 <file>` and start
  again. A file that is missing, or holds no token, is said too. Windows
  has no mode to read: see [Windows](windows.md#windows);
- a token whose file names another yard's address than the one it would be
  sent to: pair the device with this yard;
- a token file that cannot be bound to an address, or is not a regular file;
- a socket of a yard of this machine that is no whole path, in a settings
  file edited by hand.

A yard that is refused is not tried again: the window says why, and the
settings, or `cmd+1` to `cmd+9`, open another.

**"This device is no longer let in."** The yard answers 401 with
`WWW-Authenticate: Bearer` when the token is not a device's: the operator
removed the device (`yardr user device rm`), or the token is another
yard's. That does not pass by itself, so footplate says so and stops
asking: no call, no stream, until the device has a token again. "pair
again" under the ways opens the settings on that yard, for a new code. A
yard that has no device at all answers 404 under
`/user/`, to a token as to none, and so does an address that is no yard's
listener: the window says that, offers the same, and goes on trying every
3 seconds, since a proxy that is being set right answers so too. When
nothing answers at the address it says so, names the address, and tries
again.

**"A gateway turned the request away (401), which does not mean this
device is out."** Only the yard's own answer says the device is out. A 401
that carries neither its `WWW-Authenticate: Bearer` nor the edge's
`X-Edge-Refusal: session` was answered by something else, and says nothing
of the device: it stays paired. A read that meets one is said with those
words, and the window, which is told the yard cannot be read, tries again
every 3 seconds as it does a yard that is slow, instead of stopping.

What a device reads and writes, and how it is taken back, is the yard's:
`docs/users.md` there. In short, it reads what any user of the yard can
and writes as its user, marked `via <device>`; a token does not end until
the device is removed.

## A hosted yard

A yard behind the hosted edge, at the edge's one origin, is paired as a
yard at an address is, with a sign-in first. "open a hosted yard" on the
[first screen](#what-it-talks-to), or "add a hosted yard" in the settings,
asks for the origin, which of the account's yards there, and the Kratos
account's email and password, in the window: no terminal, and no token
pasted anywhere. Empty is the account's own yard. Signing in keeps the
session in a file of the yard's own beside the settings, mode 0600 as the
device's token is, with the edge's address under it; the settings keep the
file and the signed-in account, never the session. The road goes on to
pairing the device with the code from the ready page, and then the yard's
conversations, with the signed-in account under the yard's name: in the
head's line, in the whole telling the settings say, and on the yard's row.

Every call to a hosted yard carries both: the device token in
`Authorization: Bearer`, as ever, and the session token in `X-Edge-Token`
beside it, which the edge checks before the call reaches the yard. Where
the yard names which of the account's yards, every call names it in
`X-Edge-Yard` beside them. Pairing sends the session beside the code, and
the yard where one is named: where the pairing's QR code names one,
`<address>#<code>#<yard>`, pasting it for the code pairs that yard and
names it from here on. A refused session and a refused device
are told apart on screen, and neither is tried again: the edge refuses a
session it does not know with 401 carrying `X-Edge-Refusal: session`, and
the window says to sign in again; the yard refuses a token that is not a
device's with its own 401, which the edge passes through byte for byte
(it carries `WWW-Authenticate: Bearer`), and the window says to pair again.
A 401 with neither header is neither refusal and is tried again, with
the words above. "sign in again" stands under the words
and on the yard's row in the settings; where the session is what is out,
pairing again is not offered, since it helps nothing. Removing the yard
deletes its session's file with its token's, where footplate wrote them;
the session itself stays signed in at the edge until it expires or is
revoked, and the device stays at the yard until its operator removes it.

A yard of another account's, one the account is a member of, is named in
the yard field as the edge names it, `owner/yard`, and stands marked
"member of owner's yard" wherever it is listed. Its operator never holds
the member's code, so there is no code on a ready page to type: "pair this
device", beside the yard in the settings and in its code form, asks the
edge to open a pairing with the member's own session, `POST /user/pairing`
with `X-Edge-Yard: owner/yard` and a name for this device (this machine's
name, made to the yard's rule for a device's name). The edge opens it for
the yard user the yard maps the member to and answers the code. The code
stands in the code field, and the same session redeems it as a typed code
is, so that one that was not used up is tried again by pressing pair. The
device calls as the member's yard user from then on. The edge's refusals
are shown in its words: a pairing already open at the yard, with its yard
user where the yard says it, and a code presented in another account's
session, which is refused and spent. An edge that does not know the call
answers it with the yard's 401 or a 404, and the window says the edge does
not open a member's pairing from the app yet.

A request that fails, at a hosted yard or any other, leaves one line on
stderr and in footplate's own log (below), so that it can be found in the log
the edge keeps of its own requests: the edge echoes its id for a request in
`X-Edge-Request`, and the line names it, `none` where the answer carried none
or never came:

    footplate: call failed: request=r-3f9a2c4d81b07e5a6c1d9e02 method=POST path=/call/Flows outcome=timeout status=none timeout=30s

`outcome` is `timeout` (a wait ran out), `down` (nothing answered, or the
answer was cut off), `refused` (the yard or the edge said no) or `unreadable`
(what answered is no reply). `status` is that of the answer's head, where
one came. `timeout` is the wait set on the step that failed: the connect's
10 seconds, else the request's own, which is 30 for a call. Every try that
fails leaves a line, so a GET asked again after a timeout leaves one for
each, and so does a call that a server replacing itself refuses and that is
made again. A request that answers leaves none, nor does a reply that came
whole and is not the shape asked for. No token and no body is in a line.

An app started by the Finder or by launchd has its stderr on `/dev/null`,
and a Windows release has none, so the line is kept in a log of footplate's
own as well, led by the time it was said:

    2026-10-26T21:47:03.512+01:00 call failed: request=r-3f9a2c4d81b07e5a6c1d9e02 method=POST path=/call/Flows outcome=timeout status=none timeout=30s

The time is the person's own clock, with its offset, to the millisecond. It
is when the call gave up, so a timeout began its `timeout` earlier; where the
edge gave no id (a connect that failed, a head that never came) it is what
the edge's own log is found by. The log is
`$XDG_STATE_HOME/footplate/footplate.log` when `XDG_STATE_HOME` is set,
`~/Library/Logs/footplate/footplate.log` on macOS otherwise, and
`~/.local/state/footplate/footplate.log` on Linux otherwise; on Windows
`%LOCALAPPDATA%\footplate\footplate.log`, and nothing else. A relative
`XDG_STATE_HOME` is ignored. A line that would take the file past 1 MiB moves
it to `footplate.log.1` first, in the place of the one before, so between 1
and 2 MiB of lines are kept once the log has grown so far. A line that cannot
be kept is said so on stderr, and the call that failed is no more failed for
it. The log stays on the machine: footplate sends it nowhere.

The sign-in is Kratos's API login flow at the origin's `/kratos` path, as
a native client uses it: one flow is opened
(`GET /kratos/self-service/login/api`), and the password method answers it
(`POST /kratos/self-service/login?flow=<id>` with the email and the
password). The password goes to the typed origin alone: the flow's own
action is not followed anywhere else, and no redirect is. An `http://`
origin off this machine is refused before anything is sent, as a yard's
is, so a password crosses the network only over `https://` or to this
machine. A flow the edge forgot meanwhile is opened once more; one that is
forgotten twice is said. The password is in the request's body and in
nothing else of footplate: no field once it is sent, no file, no setting,
no error and no log.

## yardr, a dependency

yardr is the core and footplate a client of it, so footplate treats it as it
treats herdr or any other program it needs: it uses the yardr on this
machine, and where there is none it offers to run yardr's own installer. It
keeps no copy, and neither downloads nor checks yardr itself.
`src/fetch.rs` is the mechanism.

It looks in this order, by whole path, and takes the first that runs:

1. `<home>/binary` of the local yard (the directory of its socket), which the
   yard writes for the yardr that runs it. A file that is missing, or holds
   anything but one whole path to a program, is no answer.
2. `~/.local/bin/yardr`, where yardr's installer puts it.
3. `yardr` on the PATH footplate was started with.
4. Your login shell (`$SHELL -l -c 'command -v yardr'`), asked once and given
   five seconds: an app opened from the Dock has none of your shell's PATH. A
   shell that fails, hangs or answers with anything but a whole path has not
   found it, and a shell that hangs is stopped with what it started.

These four read and nothing else: they fetch nothing and write nothing. A
yardr that is found is used as it is. Nothing checks it, compares versions or
asks what the latest is. The first run looks when it starts, and [the switch
in the head](#starting-and-stopping-the-yard) when the window opens a yard of
this machine, until it has its yardr for this run of footplate.

**None found.** The first run and the switch say so and offer "install yardr".
On your word alone, footplate reads `https://yardr.dev/install.sh`, from that
one host and over TLS, not following it anywhere else, and runs it as a maker's
installer: from a directory of footplate's own that nobody else can write to,
with no shell around it and nothing to read, showing what it prints, and gone
afterwards. The script checks what it installs (`install.sh` in the yardr
repository); footplate has no checksum for the script itself, so that host is
what is trusted, as it is by anyone who pipes it to a shell. Then step 2 is
looked at again. A failure shows what the installer said and offers to try
again.

**The yardr an older footplate kept** stood in a `bin` beside the
[settings](window.md#settings), with a `yardr.version`. Where it is there and the four
steps find nothing else (a `<home>/binary` that names it counts as nothing
else), it is moved, once, with no click: copied to `~/.local/bin`, `daemon
install` run again from there on each yard of this machine with a login
service, since its unit names the old path, and only then the old `yardr` and
`yardr.version` removed. A failure leaves the old one running and says so,
beside the switch and in the first run. If no service was moved, the copy is
removed and the move is tried again at the next start. If some services were
moved before a failure, the copy and the old one both stay, as each is what a
service starts.

Updating yardr is not here: that is yardr's own. Windows has no yard of its
own, so it finds and installs none.

## Setting up a yard here

"Set up a yard here" on the [first screen](#what-it-talks-to) takes a person
from no yard to talking with the yardmaster, and asks them to type no
command. Each step runs the yardr on this machine, with `--json`, and shows
what it answers; where yardr refuses, its own words are shown. The list of
commands is yardr's: `docs/setup.md` of its repository, "For a program that
drives the first run". The yard is this run's own: the home its socket is
in, which every command is told (`--home`).

| Step | What is shown | What is run |
|---|---|---|
| yardr itself | that it is found; where it is not, "install yardr", what the installer prints, and "try again" where it failed | the lookup and the installer [above](#yardr-a-dependency) |
| What this needs | a row for git, herdr and each harness: installed or not, what it is for, and how it comes. "check again" asks once more. A herdr whose server is down is said to wait, and keeps nothing back. A program that answers a harness's version flag as another (a `claude` that is not Claude Code) is said not to be it, with what it printed, and does not count; a harness whose version flag failed or did not answer is installed, with a warning, and keeps nothing back. "go on" needs git, herdr and one harness | `yardr doctor --json` |
| Your terminal | shown only on a Mac, and only when a program is in `~/.local/bin` and a terminal does not look there: which programs, the line that makes it look, and the shell's file it goes to. "add it to ~/.zshrc" adds that one line, and "not now" writes nothing. [Below](#your-terminal) | `$SHELL -l -c 'printenv PATH'`; on the yes, one line appended to the shell's startup file |
| Your yard | your name (the login name to begin with), the harness (the installed ones; one yardr cannot log in for you is marked "needs a terminal once to log in"), the yardmaster's name, a model, how agents work with you (the four styles, "Guide me" to begin with), a line about you | `yardr init --harness … --operator … --yardmaster … --style … [--model …] [--about …]`: it creates the yard and starts nothing |
| Logging in | "log in", or "it is logged in already". Then what yardr asks for: the ways to log in as buttons, the page as a button, the code in a field, and the question before the one call that proves the login, which costs a model call | `yardr harness login <kind> --json`, answered on its input |
| Start the yard | "start the yard"; what could not be started, each with the one thing to do, in yardr's words | `yardr up` |
| A folder to work in | a folder, typed or chosen, its name and its base branch, both offered from the folder. A folder that is no git repository has no branch, and is worked in as it is | `git rev-parse --is-inside-work-tree` and `git branch --show-current` in the folder; `yardr depot add`; `yardr depot init --yes`, again with a flag for each thing its `missing` asks for: the yard's harness for a group of agents, you for the decisions, anyone for the other groups of people |
| A first piece of work | a title, when it is done, and whether you are told when work lands | `yardr bead create`, `yardr bead advance --to new`, `yardr subscribe … --done` |

Then the first run is gone and the yardmaster's conversation is open, with
the keys in its line. The last two steps have "not now" beside their
button, which goes to the yardmaster at once: a yard is whole without a
depot. "leave" and Esc leave the first run where it stands; what was made
stays, and taking the way again goes through the steps anew, each of which
is safe to repeat. A home that has a yard already, which does not run, is
not created over: after "What this needs" comes "Start the yard", and then
the conversation of the yardmaster that yard has.

Once you said your name, every command acts as you (`--actor <your name>`),
whatever the login is called: the yard's operator is who its settings
belong to.

**How a missing program comes.** yardr says, per program, in doctor's JSON:

- On a Mac, `fetch`: one file at its maker's own address, of one version,
  with the size and the SHA-256 it had when yardr's makers verified the way
  (`docs/doctor.md` of the yardr repository, "A program fetched on a Mac").
  The row has "install", which follows that page's six steps: a program
  that is at its place already is not fetched; the file is read over https,
  to its named size at the most; it is kept or run only when its size and
  checksum are the named ones; of an archive the one named member is taken,
  and an archive holding anything else is refused; a maker's installer
  (Claude Code's) is written to a directory of footplate's own that no one
  else can write to, run with yardr's arguments and nothing to read, and
  removed; every other program is moved to its place, mode 0755. The place
  is under your home, `~/.local/bin` for all three, and is refused where it
  is not, as written or by a link on the way.
- git on a Mac has "install" too: `xcode-select --install`, Apple's own
  window, which footplate does not wait for.
- Everywhere else, and for a program yardr names no way for: the row says
  how it is installed by hand, in yardr's words, with a button for its
  maker's page where the words name one. On Linux that is every program:
  there is no one way to install without a terminal there.
- A program on PATH under a harness's name that answers its version flag as
  another program (`docs/doctor.md`, "A program that answers as another"):
  doctor reports the harness not found, with `path` the program that
  answered, `version` holding `not_harness` and what it printed, and the
  way to get the real one. The row says "the claude on PATH is not Claude
  Code", which program it is and what it printed, and offers the real one as
  it does for a harness that is missing. herdr starts the first program of a
  name on PATH, so a Claude Code fetched to `~/.local/bin` behind it never
  runs: where the file at the fetch's place stands already (a file that
  runs, or a link to one), the row offers no second fetch, which would find
  it there and do nothing, and says that the other program is first on PATH.
  That file is looked at with each check, on a Mac. A harness whose version
  flag failed or did not answer (`fails`, `hangs`) stops nothing: it is
  installed, with a warning, and a session started on it shows what is
  wrong. A state footplate does not know is read as a harness that answers.

**What this adds to what footplate reaches.** yardr's own site apart,
"install" reads from the address yardr names and from wherever that
address sends it on to, five times at the most and never off https: a
maker's release address answers with its storage's. Nothing is trusted
for where it came from. What is trusted is the checksum, which came inside
the yardr whose signature was verified. herdr publishes no signature of its
own, so for herdr that checksum is all there is; and Claude Code's installer
downloads its version again itself, so the checksum is of the installer
that is run. footplate opens a page on the web in your browser, an
`https://` address only: a maker's page from doctor's words, and the page a
login names.

**The PATH of what is started** is made from data, not by a shell: the PATH
footplate was started with, and after it the directory of each place doctor
names, where your login shell finds a program that the PATH lacks
(`login_shell_path`) and where a program is fetched to. Doctor is asked
again with it, and then finds them. footplate writes no profile unasked: the
one line [your terminal](#your-terminal) offers goes into a shell's file on
your yes alone. The login shells in all of this are doctor's
own, which it runs once for a program missing from the PATH, and the two
questions that [look for yardr itself](#yardr-a-dependency) and for the PATH
a terminal starts with. A terminal opened later finds a fetched program only
once its own PATH has `~/.local/bin`, which that step adds.

**What footplate starts**, all of it, each with its arguments and never
through a shell:

| It starts | For |
|---|---|
| the `yardr` on this machine, by its whole path: `doctor`, `init`, `harness login`, `up`, `depot add`, `depot init --yes`, `bead create`, `bead advance`, `subscribe`, each with `--json` | the steps above |
| the `yardr` on this machine, by its whole path: `doctor` (once per run of footplate, for the PATH above; its one login shell with it) and `status` when a yard of this machine is opened, with no click; `up`, `down`, `up --login`, `daemon uninstall` on a click; each with `--json` | [the switch in the head](#starting-and-stopping-the-yard) |
| a maker's installer that yardr names, on a Mac (Claude Code's `install <version>`) | "install" on a harness's row |
| yardr's installer, `https://yardr.dev/install.sh` | "install yardr" where there is none |
| your login shell, `$SHELL -l -c 'command -v yardr'`, under a timeout | [looking for yardr](#yardr-a-dependency), where the first three steps found none |
| your login shell, `$SHELL -l -c 'printenv PATH'`, under a timeout | [your terminal](#your-terminal), on a Mac, once a program is in `~/.local/bin` and the shell's file does not name it |
| `/usr/bin/xcode-select --install`, on a Mac | "install" on git's row |
| `git`, to read: `rev-parse --is-inside-work-tree`, `branch --show-current` | the branch offered for a folder |
| `notify-send`, on Linux | a [notification](window.md#notifications) |

It never runs anything as another user, types into a pane, starts herdr's
server itself (that is `yardr up`'s), or drives a harness login: that login
command is yardr's to hold, and footplate shows what it asks for. A code
from the harness's login page is passed to yardr's input as it is typed and
is in the field no longer; it is in no file, no setting and no error. The
credential that login makes is the harness's own, kept where the harness
keeps it. A hosted-yard sign-in goes to the edge's Kratos API as described
in [A hosted yard](#a-hosted-yard); it starts no process.

### Your terminal

A program fetched to `~/.local/bin` is found by footplate and not by a
terminal, which on a Mac as it comes does not look there. After "go on", a
step is shown when a program of setup's (the yardr that is run, or one that
doctor found there) is in that directory, the PATH your login shell reports
(`$SHELL -l -c 'printenv PATH'`, under the timeout of the question that looks
for yardr) does not name it, and the shell's file does not either. It shows
the line and the file, and writes only on "add it to …": one line at the end
of the file, on a line of its own, and never twice. "not now" writes nothing,
creates nothing and marks nothing, so the step comes again in the next run.
It is on a Mac alone.

| Shell, by `$SHELL` | The file | The line |
|---|---|---|
| zsh | `$ZDOTDIR/.zshrc` where footplate was started with `ZDOTDIR` set, else `~/.zshrc` | `export PATH="$HOME/.local/bin:$PATH"` |
| bash | the first of `~/.bash_profile`, `~/.bash_login` and `~/.profile` that exists, as a login bash reads that one alone; `~/.bash_profile` where none does | `export PATH="$HOME/.local/bin:$PATH"` |
| fish | `~/.config/fish/config.fish` | `set -gx PATH $HOME/.local/bin $PATH` |
| any other, or none named | none: the line is shown with the words that footplate does not know where that shell keeps its file, and nothing is written | `export PATH="$HOME/.local/bin:$PATH"` |

A file that has a line naming both `PATH` and `.local/bin` gets no step and
no write. A comment names nothing: oh-my-zsh's stock `.zshrc` carries
`# export PATH=$HOME/bin:$HOME/.local/bin:…`. The file is asked before the
shell, and has the last word: a zsh started to answer `-l -c` does not read
`.zshrc`, so it is the file, not the shell, that knows of a line added in an
earlier run. A file that is a link is written through, and stays one; one
that does not end in a newline gets one first, so the line is never joined to
the last. A file that cannot be read or written is said so on the step, in
the system's words. The step stays: setup does not fail for it, and "not now"
goes on.

### Trying the setup without touching your yard

Set `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `YARDR_HOME` to a
temporary directory before opening Footplate. This keeps its settings, the
installed yardr, the new yard, the desktop entry and any fetched prerequisites
under that directory. The restricted PATH also keeps the test from using
programs already installed in your usual home:

```sh
scratch="$(mktemp -d)"
mkdir -p "$scratch/home" "$scratch/config" "$scratch/data"
HOME="$scratch/home" XDG_CONFIG_HOME="$scratch/config" XDG_DATA_HOME="$scratch/data" YARDR_HOME="$scratch/yard" \
  PATH="$scratch/home/.local/bin:/usr/bin:/bin" ./target/debug/footplate
```

After closing Footplate, stop the test yard and its Herdr server if you
completed setup, then remove the temporary directory:

```sh
if [ -x "$scratch/config/footplate/bin/yardr" ]; then
  HOME="$scratch/home" YARDR_HOME="$scratch/yard" \
    "$scratch/config/footplate/bin/yardr" down --yes
fi
if [ -x "$scratch/home/.local/bin/herdr" ]; then
  HOME="$scratch/home" PATH="$scratch/home/.local/bin:/usr/bin:/bin" \
    "$scratch/home/.local/bin/herdr" server stop
fi
rm -rf "$scratch"
```

## Starting and stopping the yard

Under the yard's line in the head, one status row (on macOS, in the band
at the window's top, beside the traffic lights): a coloured dot, the
yard's state, read from `yardr status --json`, and Start or Stop at its
right end, which run `yardr up` and `yardr down`. The yard answers:
Running. Nothing runs: Stopped. Between them, Starting while it is
started, Stopping while it is stopped. Where a start could not start
something, the head says that in yardr's words, with Start to try again.
Start and Stop are safe to repeat, as `up` and `down` are. Where the home
holds no yard, the head says No yard, and the first screen offers setting
one up. Reading the switch starts nothing.

Stop first asks `down` without its yes-flag, which stops nothing on a yard
with agents and answers what it would stop. The window asks then: the
agents in the middle of something by name, the idle ones as a count, and
that what they have not committed stays in their worktrees while the turn
they are in is lost. "Stop them" runs `down` again with the flag; "Leave
running", or Esc, stops nothing. With no agent, Stop stops at once and asks
nothing.

On the open yard's card in the settings, where it is a yard of this
machine, "start at login": on, the yard starts at login. Switching on
installs the login services with `up --login`: the daemon's and herdr's
server's. Where the yard is stopped, switching on starts it now as well
as at every login, so the help says that before the click. herdr's is one
service for every yard on the machine, the user's and not the yard's.
Switching off removes the daemon's service; herdr's shared service is
left, and its server still starts at login. Where removing the daemon's
service stops the daemon it ran, the switch starts it again while the yard
runs, so the yard runs on. A yard at an address has no switch: its
processes are elsewhere, and no other card carries one either.

Every one of the switch's calls names the open yard's home: its socket
minus `yard.sock` as `--home`. This run's yard, whose socket is empty,
takes no flag, so yardr resolves it as footplate did. A yard at an address
has no switch: its processes are elsewhere. A socket that names no yard
home says so, and starts nothing.

The home is the one the settings entry names, and yardr runs on it as you:
Start starts that home's yard with that home's settings, its harnesses and
its agents. List the socket of a home that is yours alone; yardr does not
ask whose a home is.
