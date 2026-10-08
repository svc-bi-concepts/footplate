# footplate

A native chat window onto a yardr yard, for macOS and Linux: the deck where
the crew stand. It is an interface over yardr's socket API and starts nothing
of yardr's; the yard runs as it does (`yardr status`). The first slice is the
conversations a person has with this yard's crew and with the crew a linked
yard exposes, as `yardr chat` shows them in a terminal.

Rust, drawn with [gpui](https://gpui.rs/), the UI framework Zed is built on.
Tracked in Linear as YRD-22. Developed in a yardr yard; `.yardr/` holds its
flow, roles and merge gate (`cargo fmt`, `clippy`, `test`, `build`).

    cargo run                # the window, against the yard on this machine
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

There is no installer. The binary is the application: copy it where you
like and run it.

## What it talks to

The yard's server, over its unix socket: `$YARDR_HOME/yard.sock` when
`YARDR_HOME` is set, `~/.yardr/yard.sock` otherwise. It speaks what
`docs/api.md` of the yardr repository promises a program that is not yardr,
and nothing else: HTTP/1.1 on the socket, a call a `POST /call/<Method>` with
its arguments as a JSON object, the events a `GET /events?after=<seq>` of one
JSON object a line. `src/api.rs` is the whole client.

| It reads | For |
|---|---|
| `Head`, then `GET /events` after it | what happens from then on |
| `Crew`, `Peers` | whom there is to talk with: the crew, and `<name>@<peer>` for each one a linked yard exposes |
| `GetUser`, `Settings` | who the window is for |
| `UserInbox` | the unread mail, counted on each row |
| `Conversation` | the mail both ways with the one whose row is open |

| It writes | When |
|---|---|
| `Mail` | Enter, in a conversation with a crew member |
| `MailPeer` | Enter, in a conversation with one of a linked yard |
| `AckUserInbox` | a conversation showed mail that was unread |

It follows the stream and asks nothing of the yard in a loop: `mail`,
`mail_read` and `crew_status` are applied as they come, the crew is read
again on the other `crew_*` events, the linked yards on `peer_*`.

**Who you are** is not typed. The window acts as `yardr chat` does: as the
user named in `$YARDR_ACTOR`, else as your login name, when the yard has a
user of that name; else as the yard's operator. A yard that has neither says
how one is added (`yardr user add <name>`).

**Unread** is the yard's, so `yardr chat`, `yardr inbox` and the window
agree. Opening a conversation marks its mail read, and so does mail that
arrives in the open one. A user's mail has one position for all senders, so
mail of a conversation you have not opened holds back what came after it:
nothing is marked read that the window did not show.

**No yard** (the socket is missing, or nothing answers): the window says so,
names the socket, and tries again every 3 seconds. footplate does not start
the yard's server; any yardr command that reads the yard does
(`yardr status`). When the server comes back, the conversation that was open
is open again.

## The window

Left, the rows: the crew with what each one's session does (`working`,
`idle`, `blocked`, `running`, `not running`, `failed to start`), then those
the linked yards expose, each with what they are there (a crew member or a
person) and their role line, and on every row the number of mails from it
you have not seen. Right, the open conversation, oldest at the top and the
newest in view, with a line under your own mail that says where it stands
(`read`, `sent; yardmaster is idle`, `queued for north: …`), and the line to
write in.

| Key | Does |
|---|---|
| `enter` | sends the line as mail to the one whose conversation is open |
| `tab`, `shift+tab` | the next, the previous row |
| a click on a row | opens it |
| `cmd+q` (`ctrl+q` on Linux) | quits; so does closing the window |

Not in this slice, and in `yardr chat`: the `yard` row with your
notifications, a crew member's pane, and a text of several lines or longer
than one mail (4096 bytes): the line is one line, and a longer text is
refused with its size, not split.

## The icon

`assets/icon.svg` is the drawing and `assets/icon.png` its rendering, which
the binary carries:

    rsvg-convert -w 512 -h 512 assets/icon.svg -o assets/icon.png

It stands in the window, and in the dock or the taskbar. gpui's window
options have no icon, so the program sees to it when it starts
(`src/taskbar.rs` for Linux):

- macOS: it is set on the application.
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
| `src/api.rs` | the socket client: the calls, the stream, their JSON |
| `src/session.rs` | following a yard on a thread: who the window is for, the state it starts from, trying again |
| `src/model.rs` | what the window shows, as plain functions: the rows, the open conversation, unread and what is acknowledged |
| `src/window.rs` | the window: draws the model and makes the calls it asks for |
| `src/input.rs` | the line that is typed in |
| `src/taskbar.rs` | Linux: the desktop entry and the window's icon, for a taskbar |
| `src/fake.rs` | a yard for the tests: a unix socket in a temp dir that answers the documented JSON |

The tests run without a yard and without a screen: the client against the
fake yard for each call and the stream, the model as it is, and the window
itself on gpui's test platform.
