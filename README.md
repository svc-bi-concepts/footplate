# footplate

A native chat window onto a yardr yard, for macOS and Linux: the deck where
the crew stand. It is an interface over yardr's socket API and starts nothing
of yardr's; the yard runs as it does (`yardr status`). The yard is this
machine's, or one at an address, which the window is paired with by a code.
The first slice is the
conversations a person has with this yard's crew and with the crew a linked
yard exposes, as `yardr chat` shows them in a terminal.

Rust, drawn with [gpui](https://gpui.rs/), the UI framework Zed is built on.
Tracked in Linear as YRD-22. Developed in a yardr yard; `.yardr/` holds its
flow, roles and merge gate (`cargo fmt`, `clippy`, `test`, `build`).

    cargo run                # the window, against the yard on this machine
    FOOTPLATE_YARD=https://mac.example.net cargo run    # against a yard at an address
    cargo build --release    # one binary: target/release/footplate
    cargo test

## Building

Rust, stable (`rustup default stable`), and what gpui needs of the system:

- macOS: the Xcode command line tools. The Metal toolchain, a separate
  download since Xcode 26, is not needed: gpui is built with its
  `runtime_shaders` feature, which compiles the shaders when the window opens.
- Linux: the development packages gpui builds against, which are those Zed
  lists for building from source: X11 and Wayland, xkbcommon, fontconfig and
  a Vulkan loader; to run, a Vulkan driver (Mesa's software one will do).
  The yard runs on a Mac and so does the gate: the Linux build is tried by
  hand, in a container (Debian 13, arm64) under Xvfb and under a headless
  sway.

gpui is the release on crates.io, pinned exactly in `Cargo.toml`, and
`Cargo.lock` is committed: the gate builds with `--locked`. `Cargo.toml`
holds libc below 0.2.190, which footplate does not use itself: with that
release a crate in gpui's tree no longer builds on Linux, and the gate, on
a Mac, would not say so.

Compiling gpui's tree takes a while, and every fresh worktree would pay it
again. The gate keeps the built dependencies of a worktree that passed it,
in `~/.cache/footplate-warm-target`, and a fresh worktree starts from that
copy, compiling only footplate itself. Deleting that directory loses
nothing but time: the next gate run builds from nothing and leaves a fresh
copy behind it.

There is no installer. The binary is the application: copy it where you
like and run it.

On macOS a script wraps it as an application for Finder, with the icon as
an icns:

    cargo build --release && scripts/bundle-mac.sh

That is the whole of it. `target/Footplate.app` runs on a double-click and
can be moved to `/Applications`. It is signed ad hoc, with no identity, so
that its signature carries the bundle's id, which macOS files an
application's [notifications](#notifications) under; it is not notarized,
so it is for the Mac that built it. The script needs `sips` and `iconutil`, which
macOS has, and says so on another system.

## What it talks to

The yard's server, over its unix socket: `$YARDR_HOME/yard.sock` when
`YARDR_HOME` is set, `~/.yardr/yard.sock` otherwise. It speaks what
`docs/api.md` of the yardr repository promises a program that is not yardr,
and nothing else: HTTP/1.1 on the socket, a call a `POST /call/<Method>` with
its arguments as a JSON object, the events a `GET /events?after=<seq>` of one
JSON object a line. `src/api.rs` is the whole client.

Or a yard that is not on this machine, at the address of its listener: see
[A yard at an address](#a-yard-at-an-address). The calls are the same ones.

| It reads | For |
|---|---|
| `Head`, then `GET /events` after it | what happens from then on |
| `Crew`, `Peers`, `Users` | whom there is to talk with: the crew, `<name>@<peer>` for each one a linked yard exposes, and the users of this yard, yourself among them. A peer's `trust` says which of its names are a user of this yard speaking |
| `GetUser`, `Settings` | who the window is for; `Settings` also for the yard's cap on an image, before one is sent |
| `UserInbox` | the unread mail, counted on each row |
| `UserNotifications` | what the yard reports, counted on the `yard` row |
| `UserAsks` | the open questions above the conversations, grouped by topic, with the decided folded under |
| `Bead` | the open bead whole: each ask's options, recommendation and words, the body, notes and about-beads |
| `Conversation` | the mail both ways with the one whose row is open, and with each name trusted as them; on the `yard` row with the yard and with each sender who has no row |
| `GET /blobs/<hash>` | an image a mail of the open conversation carries, once for each hash |

| It writes | When |
|---|---|
| `Mail` | Enter, in a conversation with a crew member or a user of this yard, your own too |
| `PutBlob` for each image, then `Mail` with their hashes | Enter, when images are attached to the draft |
| `MailPeer` | Enter, in a conversation with one of a linked yard, or in a user's row when a name trusted as them wrote last; after `PutBlob` for each image, with their hashes |
| `AnswerAsk` | Enter, on a card: the option chosen, with the line's words in the one answer; an ask with no options is answered with words |
| `AddNote` | Enter, with words and no option chosen and none recommended: a plain note, the ask stays open |
| `DeferAsk` | `x`, on an open card: the ask put aside, to come back as the yard says |
| `DeclineAsk` | `d`, on an open card: the ask declined with words, not a pick and not "you decide"; a secret ask with none |
| `GiveSecret` | Enter, in the field of a secret ask's card: each name the ask asks for with the value typed |
| `AckUserInbox` | a conversation or the `yard` row showed mail that was unread |
| `AckUserNotifications` | `x`, on the open `yard` row: one notice read |

It follows the stream and asks nothing of the yard in a loop: `mail`,
`mail_read`, `notifications_read` and `crew_status` are applied as they
come, the crew is read again on the other `crew_*` events, the linked
yards on `peer_*`, the users on mail from a name of this yard that has no
row, the notices on an event that notified the user, the
queue on every `noted` event and once more at each put-aside ask's
return, and a bead shown on every event whose bead it is. Mail and
notices arriving never cover or move the open card.

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

**Images.** Mail can carry images, and the window shows those of a mail
under its text, each in a space 240 points high: scaled down to fit it,
never up.
An image is read from the yard once and kept in memory by its hash, for as
long as the window runs. One the yard no longer has (`yardr store prune`
removes an image no mail names) is a line that says so.

**Sending images.** Paste an image (`cmd+v`, `ctrl+v` on Linux) or drop
image files on the window, anywhere: each is attached to the draft of the
open conversation and shown small above the line. Nothing is sent yet. Write
something or not, and Enter sends one mail with the text and every image
attached, in the order they came; an image alone is a mail. A draft belongs
to its conversation: open another, and the text and the images wait where
they were. The `×` on an attachment removes it, and so does `backspace` in
an empty line, the last one first.

An attachment that cannot go is marked, pale in a frame of the warning
colour, with the reason above the line:

- one that is neither a PNG nor a JPEG, which are what the yard takes (a
  screenshot is a PNG; an image copied out of another program may be a TIFF,
  and a dropped file may be anything). It is told by its first bytes, not by
  its name, and a dropped file that is none is not read further;
- one over the yard's cap (`blob_max_bytes` of `yardr settings`, 8 MiB when
  unset);
- one the yard refused when it was sent, with the yard's words.

In a conversation with `<name>@<peer>` an image is attached and sent the
same way, and whether it crosses is the yard's to say: when the link does
not carry files (`yardr peer allow <peer> --send file`), the linked yard's
yardr is too old for them, or the image is over `peer_file_max_bytes`, the
yard refuses the mail, and the line says its reason. Nothing is marked then,
since no one image is at fault: the draft is back as it was. An image of
mail from a linked yard is shown like any other; one this yard did not take
is a line with the yard's reason.

While an attachment is marked, Enter sends nothing and says so: the mail
that goes is the draft as it stands, never the draft less an image. Remove
the marked one and Enter sends the rest. A send that fails puts the text and
the images back in the draft. An image the yard took before the send failed
stays in its store, named by no mail, until `yardr store prune`.

**No yard** (the socket is missing, or nothing answers): the window says so,
names the socket, and tries again every 3 seconds. footplate does not start
the yard's server; any yardr command that reads the yard does
(`yardr status`). When the server comes back, the conversation that was open
is open again.

## A yard at an address

The window opens one yard: this machine's, by its socket, as above, or one
on another machine, at the address of its listener (the yard's setting
`hook_addr`, or what stands in front of it). There it is a *device* of one
user of that yard, as `docs/api.md` ("Over the network") and
`docs/users.md` ("Devices") of the yardr repository have it: the same
calls, stream and images under `<address>/user/`, each request with
`Authorization: Bearer <the device's token>`, and `GET /user/me` for who
the token is. Under your name the window says which yard it shows: `this
machine's yard`, or `the yard south at https://mac.example.net, as its
device laptop`.

**Choosing it in the window.** The settings have "yard": "this machine's"
or "a yard at an address". For the second the window asks for the address
and a code. The yard's operator opens a pairing for the device and reads
you both:

    yardr user device pair <user> <device> --url https://mac.example.net

Type them and press Enter, or "pair". What the pairing's QR code holds,
`<address>#<code>`, can be pasted into the address as it is. footplate
sends the code once (`POST <address>/user/pair`), is answered with the
device's token, keeps it, and shows that yard. What the yard says of a code
is shown under the fields: that it is not the code (the yard counts a wrong
one, and its fifth ends the pairing), that no pairing is open, that the
code was used up and the device could not be added. Nothing changes then:
the window shows the yard it showed.

Choosing "this machine's" opens the socket again. The address and the token
stay where they are kept, so "a yard at an address" with no code goes back
to the yard the device is paired with.

**Where they are kept.** The address is in the settings file (`remote` and
`address`). The token never is: it has a file of its own beside it,
`yard.token`, written with mode 0600 and replaced by the next pairing.
There is one such file, so the window is paired with one yard at a time.
It is a file on macOS too, not the Keychain.

**By the environment.** Two variables, which win over the settings when
set:

    FOOTPLATE_YARD=https://mac.example.net FOOTPLATE_TOKEN_FILE=~/.config/footplate/mac.token footplate

`FOOTPLATE_YARD` is the address; the window's choice then says that it is
set and changes nothing. `FOOTPLATE_TOKEN_FILE` is the file the token is
read from, a token as `yardr user device add` printed it or as a pairing
gave it, on one line; a pairing in the window writes to it. It names no
yard by itself: without `FOOTPLATE_YARD` and with this machine's yard
chosen, the socket is opened as ever.

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
  again. A file that is missing, or holds no token, is said too.

A yard that is refused is not tried again: the window says why, and the
settings are there to choose another.

**"This device is no longer let in."** The yard answers 401 when the token
is not a device's: the operator removed the device (`yardr user device rm`),
or the token is another yard's. That does not pass by itself, so footplate
says so and stops asking: no call, no stream, until the device has a token
again. "pair again" under the words opens the settings with the address
there, for a new code. A yard that has no device at all answers 404 under
`/user/`, to a token as to none, and so does an address that is no yard's
listener: the window says that, offers the same, and goes on trying every
3 seconds, since a proxy that is being set right answers so too. When
nothing answers at the address it says so, names the address, and tries
again.

What a device reads and writes, and how it is taken back, is the yard's:
`docs/users.md` there. In short, it reads what any user of the yard can
and writes as its user, marked `via <device>`; a token does not end until
the device is removed.

## The window

Left, the rows: above the conversations the open questions addressed to
you, oldest first and grouped by topic (the bead's first `topic:` label,
without the prefix; asks with none under "Everything else"), each row the
question in plain words with whose it is and how long it waits, and the
bead's id small at its end; a bead with several open asks is one row. Under
them, folded, "Decided" with the answered, declined and put-aside asks and
their picks. Then the crew with what each one's session does (`working`,
`idle`, `blocked`, `running`, `not running`, `failed to start`), then those
the linked yards expose, each with what they are there (a crew member or a
person) and their role line, then the users of this yard, your own row
among them (`you, notes to self`), and on every row the number of mails
from it you have not seen. A name of a linked yard that the operator trusts
as a user of this yard (`yardr peer allow <peer> --trust <name>=<user>`)
has no row of its own: its mail is in that user's row, and what you write
there goes to whoever wrote last, as in `yardr chat`: one mail to one
place, never a copy to the other. The empty line says where before you type
(`Write to mckean@north`), or `Reading mckean` until all of the row is read,
and Enter sends nothing until then. When the yard refuses it, the window
says so with that address (`not sent to mckean@north: …`), keeps what you
typed, and writes nothing to the user in this yard instead. Below them the `yard`
row with the number of the yard's notices you have not read and of the mail
it shows: the yard's own, and that of a sender who has no row. Right, what is open: a conversation, oldest at
the top and the newest in view, an arrow at its bottom right back to the
newest when scrolled up, with a line under your own mail that says
where it stands (`read`, `sent; yardmaster is idle`, `queued for north:
…`) and the line to write in; the notices, oldest first; or a bead's open
asks, each as its card.

A conversation opens on its newest fifty mails. Towards its top the fifty
before are read and put in above, and the mail in view stays where it is;
the top says `Loading older mail…` until the conversation starts there.
Unread mail older than what was read stays unread, and counts on the row,
until its page is shown. A yard built before `Conversation` had pages
answers the whole conversation at once.

A card shows, in this order: the question; the asker and the recommendation;
the context; the options numbered, each with its consequence, `you decide`
last; the recommended one with its why; a line for words; the keys. A number
key or the arrows choose an option; Enter sends the pick with the line's
words in the one answer, or the recommended option when none is chosen, as
the keys say. Words with no option chosen and none recommended go as a plain
note, and the ask stays open; an ask with no options says "words wanted",
and Enter sends the words as the answer. `x` puts the ask aside, to come
back as the yard says. `d` declines it with words, which is not a pick and
not "you decide". `e` expands the card in place to the bead's body, notes
and about-beads; a design bead's document is named by path and branch, not
fetched. After a pick the ask moves to "Decided" at once, and a later pick
on it from there replaces the earlier one. The window holds the card's keys,
`i` types the words in, `esc` holds the keys again. What is typed stays with
its ask, as a draft stays with its conversation.

**A secret ask** (`yardr bead ask --secret`) asks for a credential of a
source, and its card has no options and no line for words. In their place
it shows the yard's own line of what is asked for, made from the source's
declaration and never from the asker's words, and a field that draws a mark
for each character. `i` or a click types in it. What is pasted goes in;
copy and cut take nothing out. Enter sends the value to the yard in
`GiveSecret` and empties the field, whatever the yard answers; an ask for
several names takes them one after the other and sends them together. `esc`
leaves the field and nothing is sent. `d` declines with no words. The value
is typed in that field and held nowhere else in the window: not in a
draft, a setting, a notice or the clipboard, and it is gone when the card
is left. Afterwards the card says the yard's words, which hold the names
and the file and never the value. When the call ends with no answer it
says "the yard did not answer. The value may be written. Give it again.",
never that it was sent. A yard that does not serve the call to this window
is said so, with the command that answers in a terminal on the yard's
machine: `yardr bead answer <bead> <ask> --secret`. One that was given can
be given again from its card under "Decided", which replaces it.

| Key | Does |
|---|---|
| `enter` | sends the draft, the line and the images attached, as one mail to the one whose conversation is open; on a card, the pick with the line's words |
| `shift+enter` | breaks the line where the cursor is; a text wider than the box goes on in the next row, and the box grows with its rows up to a third of the window, then scrolls |
| `up`, `down` in a text of several rows | the row above, below, in the same column; `home` and `end` are the row's |
| `cmd+v` (`ctrl+v` on Linux) | pastes text into the line, with its line breaks; an image is attached to the draft |
| a drag over a message's text, a double click on a word | selects it; one selection at a time, within one message |
| `cmd+c` (`ctrl+c` on Linux) | copies what is selected, as it was written: of a message, with the Markdown marks that are not drawn, or of the line while you type in it |
| a file dropped on the window | is added to the draft; a non-PNG or non-JPEG is marked and refused |
| `backspace` in an empty line | removes the last attachment |
| `tab`, `shift+tab` | the next, the previous row |
| a click on a row | opens it |
| a click on the arrow at a scrolled-up conversation's bottom right, `cmd+end` (`ctrl+end` on Linux) | jumps to the newest message; the arrow carries how many mails arrived below |
| `pageup`, `pagedown` | scroll the open conversation, card or yard row by a page, with the focus anywhere |
| `cmd+home` (`ctrl+home` on Linux) | the oldest loaded message; the newest is `cmd+end`, on a card or the yard row too |
| a click on "Decided" | folds it open, shut |
| `1`–`9`, `up`, `down`, on a card | chooses an option; the arrows choose even while words are typed, from the first row of the words up and from the last down |
| `i`, on a card | types the words in, or a secret in its field; the window holds the card's keys |
| `x`, on the open `yard` row | marks one notice read; a notice never needs more than one key |
| `x`, on an open card | puts the ask aside |
| `d`, on an open card | declines the ask with words; with an empty line the next Enter sends them. A secret ask is declined at once, with none |
| `e`, on a card | expands it to the bead whole, or folds it again |
| `esc`, on a card | leaves a decline asked for words, the words for the card's keys, the pick, then the words; in the field of a secret, the field, and what was typed is gone |
| a click on an option, on "decide this one" | chooses it; makes the ask current |
| `cmd+,` (`ctrl+,` on Linux), a click on "settings" | shows the settings where the conversation is, and what was open again; so does `esc` there |
| `cmd+=`, `cmd+-`, `cmd+0` (`ctrl` on Linux) | the text a pixel larger, smaller, the usual size |
| `cmd+shift+l` (`ctrl+shift+l` on Linux) | the next of auto, light, dark |
| `cmd+q` (`ctrl+q` on Linux) | quits; so does closing the window |

## Settings

Two things can be set, in the settings or by their own keys, and both are
drawn at once and kept for the next start; the third is the yard, which
[A yard at an address](#a-yard-at-an-address) has, and the fourth what the
system tells of, which [Notifications](#notifications) has:

- the text size, 11 to 20 pixels, a pixel a step; 14 is the usual size.
  Everything that goes with the text goes with it: the left column, the
  rows, the logo.
- light, dark or auto. Auto is the desktop's look, and follows it when it
  changes while the window is open.

They are kept, with the yard and the notifications that were chosen, in one
file footplate writes on every change:
`~/Library/Application Support/footplate/settings.json` on macOS,
`$XDG_CONFIG_HOME/footplate/settings.json` on Linux (`~/.config` when that
is not set). Without the file, or with one footplate cannot read, the window
is as it was before there were settings: 14 pixels, auto.

## Notifications

footplate can have the operating system show a notification for what
arrives while you do not look at it. It is off until you switch it on: in
the settings (`cmd+,`), under "notifications", choose "on", and then what it
tells of: mail to you, decisions (a new ask, or one you put aside that
comes back), the yard's notices. Mail and decisions are chosen to begin
with; the notices tell of every bead that moves, so they wait to be chosen.
The choice is kept with the other settings.

What you get:

- A notification names who it is from and the first line of what they
  wrote. Several things that arrive within three seconds are one
  notification ("3 from yardmaster@mac"), so one is up to three seconds
  late.
- Nothing while the window is in front and shows the row it belongs to:
  you read it there. In front on another conversation, behind another
  window, or with the settings open, you are told.
- Nothing for what you wrote yourself, and nothing for what was already
  unread or open when the window started. What arrived while the yard was
  away is told of when it is back.
- What is shown is text, whoever wrote it and in whichever yard: nothing of
  it is run or read as markup.

What the system asks:

- **macOS** asks you once, the first time you choose "on": "Footplate
  would like to send you notifications". Allow it. If you do not, or
  switch it off later, the settings say so in a line, and the way back is
  System Settings, Notifications, Footplate. macOS notifies for an
  application and not for a bare program, so this needs `Footplate.app`
  ([Building](#building): `scripts/bundle-mac.sh`); from `cargo run` the
  settings say that instead and nothing is shown. Clicking a notification
  brings the window forward on that conversation, card or the yard row.
- **Linux** asks nothing. footplate hands the notification to the
  desktop's notification service through `notify-send` (libnotify's), and
  the settings say so when that is not installed. Clicking one opens
  nothing: `notify-send` has no way back into a running program.
- On another system nothing is shown, and the settings say so.

**This is not the yard's own telling.** A yard can tell a user on a phone
through a notify source (`yardr user set <user> notify=pushover`, yardr's
`docs/users.md`, "Told outside the yard"). That is sent by the yard's
server, for all that lands in your inbox, your own mail too, whether or not
footplate runs and whether or not you have read it here. footplate's
notifications are this window's alone: they come from the machine it runs
on, only while it runs, and only for what the window does not show. The two
know nothing of each other, so with both on a mail that arrives while the
window is behind another is told twice, once by each. Switch off the one
you do not want: footplate's in its settings, the yard's with
`yardr user set <user> notify=`.

Not in this slice, and in `yardr chat`: a crew member's pane, and a text
of several lines or longer than one mail (4096 bytes): the line is one
line, and a longer text is refused with its size, not split. Nor here:
saving an image to disk, resizing one that is over the cap, or sending
files that are no image.

## The icon

The icon is a painting by mckean: a navy tank engine with brass fittings,
head-on, a speech bubble on its buffer beam, a yard at dusk behind.
`assets/icon.png` is its master, 1024 pixels square: the rounded square is
824 wide in the middle of it, with the 100 pixels of margin Apple's template
has, and the corners are transparent. The window and macOS dock load this
PNG directly. Linux uses a 512px copy for its conventional hicolor icon path:

    sips -z 512 512 assets/icon.png --out assets/icon-512.png

It stands in the window, and in the dock or the taskbar. gpui's window
options have no icon, so the program sees to it when it starts
(`src/taskbar.rs` for Linux):

- macOS: it is set on the application, which is what a bare binary
  (`cargo run`) has to show in the dock. Finder, Spotlight and the dock
  while footplate is not running show the icon of the bundle, when there is
  one: see "Building".
- Linux: a desktop shows the icon and the name of the desktop entry whose id
  is the window's (`footplate`). There is no installer to put one there, so
  footplate writes its own, for you alone: `footplate.desktop` under
  `$XDG_DATA_HOME/applications` (`~/.local/share` without the variable) and
  the icon under `icons/hicolor/512x512/apps` beside it. They are written
  when they are missing or differ, so the entry names the binary that ran
  last. It is for the open window and not in the menu of applications
  (`NoDisplay`). A desktop that has not read the entry by the time the
  window opens, on the very first start, shows the icon from the next.
  Under X11 the icon is also set on the window itself (`_NET_WM_ICON`), for
  a panel that reads no desktop entries. To be rid of both, delete the two
  files.

## The code

| File | Holds |
|---|---|
| `src/api.rs` | the client: the calls, the stream, their JSON, by the socket or at an address over TCP and TLS; a pairing code redeemed |
| `src/yard.rs` | which yard is opened, as plain functions: the environment over the settings, the token's file and its mode |
| `src/session.rs` | following a yard on a thread: who the window is for, the state it starts from, trying again, or not when the device is not let in |
| `src/model.rs` | what the window shows, as plain functions: the rows, the open conversation, unread and what is acknowledged |
| `src/draft.rs` | what is not sent yet, as plain functions: each conversation's attachments and text, what is refused and why |
| `src/window.rs` | the window: draws the model and the draft, and makes the calls they ask for |
| `src/input.rs` | the line that is typed in |
| `src/select.rs` | a message's text, drawn as its blocks, and the selection the mouse makes in it |
| `src/markdown.rs` | the light Markdown a text is drawn with, as plain functions: fenced and inline code, lists one level deep, headings to the third, bold and italic; the rest stays as written |
| `src/taskbar.rs` | Linux: the desktop entry and the window's icon, for a taskbar |
| `src/notify.rs` | the system's notifications: what is told of and that a burst is one, as plain functions; then macOS's notification center and Linux's `notify-send` |
| `src/fake.rs` | a yard for the tests: a unix socket in a temp dir, or a TCP port of this machine with tokens and a pairing, that answers the documented JSON |

The tests run without a yard and without a screen: the client against the
fake yard for each call, an image and the stream, by its socket and at an
address, with a pairing, a wrong code and a removed device, the model as it
is, and
the window itself on gpui's test platform, where an image on its clipboard
is pasted and the draft sent with Enter. gpui gives a test no way to make
the paths of a file drop, so the window's test hands the files to what a
drop calls and drops nothing but an empty one. The fake yard speaks no TLS:
at an address the tests run over plain HTTP on this machine, and one test
that is not run by the gate (`cargo test tls -- --ignored`) makes the
handshake with a site of the internet.

## Licence

Apache License 2.0, see [LICENSE](LICENSE) and [NOTICE](NOTICE).

The licence covers the code. It does not cover the names yardr and footplate
or the project's marks (Apache 2.0, section 6): a fork takes a name and a
mark of its own.
