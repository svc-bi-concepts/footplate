# footplate

A native chat window onto a yardr yard (Rust, gpui). Product behaviour:
`README.md`. Design notes: `docs/design/`. The yard's side of the
protocol (`docs/api.md`, `docs/setup.md`, `docs/doctor.md`) lives in the
yardr repository, not here. `.yardr/depot.md` holds the yard's rules for
builders.

## Layout

Top level: `src/` the program, `scripts/` release/screenshot/shell-test
helpers, `assets/` icons, `docs/design/` decision notes, `.yardr/` the
yard's gate.

`src/main.rs` opens the one window and owns the app lifecycle; it draws
nothing itself. `src/window.rs` draws the window and makes the calls the
model asks for; it holds no protocol and no state of its own.
`src/model.rs` is what the window shows, without the window: rows, the
open conversation, unread; it never calls the yard, answering an `Effect`
the window performs. `src/api.rs` is the yard's socket API: calls, the
event stream, their JSON, by socket or at an address over TCP+TLS; HTTP
hand-written, no client library. `src/session.rs` follows one yard on a
thread: login, initial state, stream, retry. `src/yard.rs` picks which
yard opens: socket or address, token files, hosted sessions; tokens live
beside the settings, never in them. `src/settings.rs` is what the window
keeps between runs, one JSON file in the platform config dir. `src/draft.rs`
is what is written and not sent yet: attachments and text per
conversation; it neither calls nor draws. `src/input.rs` is the line
typed in (gpui has no text field). `src/select.rs` is a message's text
with mouse selection; selection lives beside the texts, one at a time.
`src/markdown.rs` is the light Markdown a message draws with: code,
one-level lists, headings to h3, bold/italic; the rest stays as written.
`src/theme.rs` is the light/dark palettes. `src/notify.rs` is OS
notifications: rules as plain functions, then macOS's notification
center and Linux's `notify-send`. `src/tray.rs` is the Mac menu-bar item
and the Linux tray item, with the yards menu. `src/dock.rs` is the
number on the macOS dock icon. `src/taskbar.rs` is the Linux desktop
entry and window icon. `src/fetch.rs` fetches yardr's release from
yardr.dev: minisign signature over checksums against the built-in key,
archive against checksums, binary kept in footplate's directory; plus
`yardr doctor`-named programs by checksum alone. `src/setup.rs` runs the
first run as yardr's commands, with no window: args in, JSON out, PATH
from doctor's data, never through a shell. `src/window/firstrun.rs`
draws that first run where the yard will be. `src/shot.rs` (debug
builds only) opens scripted states against a fake yard for screenshots,
named by `FOOTPLATE_SHOT`. `src/fake.rs` is a yard for tests: socket or
TCP, scripted JSON, tokens and pairing; no TLS. `src/fixtures/` holds
test fixtures: `shot-page.png`, and the published yardr v0.10.0
checksums plus signature the built-in key is held to.

## Where to start

- A new socket call: the call in `src/api.rs`, its answers in
  `src/fake.rs`, the state it needs in `src/model.rs` (an `Effect` for
  what the window must do), the drawing and the call site in
  `src/window.rs`.
- A new view or pane: state in `src/model.rs`, drawing in `src/window.rs`;
  window chrome in `src/main.rs`.
- A first-run step: the command in `src/setup.rs`, the screen in
  `src/window/firstrun.rs`.
- A setting: `src/settings.rs`, then where it takes effect.
- Platform surface (badge, tray, notifications, Linux icon):
  `src/dock.rs`, `src/tray.rs`, `src/notify.rs`, `src/taskbar.rs`; rules
  stay plain functions, system calls stay behind `cfg`.
- Screenshots: states in `src/shot.rs`, taken by `scripts/shot-linux.sh`;
  the states are named in `README.md`.
- Releases: `scripts/release.sh` builds, `scripts/bundle-mac.sh`
  bundles, `scripts/release-stage` drives the yard's release flow.

## Conventions

- Rust modules open with `//!` comments describing their role. `///`
  comments are common on public types, fields and functions, and on private
  behavior that needs explanation; small helpers, trait methods and tests
  are not all documented. Comments explain why, in plain words; the code
  says what.
- Logic lives in plain functions tested on any system; what needs the
  OS sits behind `cfg(target_os)` in the same file. Keep that split:
  a new system call arrives with its rule beside it, tested.
- Errors are enums with person-facing `Display` (`api::Error`,
  `fetch::Refusal`); a refusal's message is shown, never parsed.
  `io::Error` passes through where the caller reports it.
- Names are plain nouns and verbs (`State`, `Drafts`, `wanted`,
  `keep`); files are one module each, `snake_case.rs`.
- Tests live in-file (`mod tests`), run with no yard, no network and
  no screen: the client against `src/fake.rs`, the window on gpui's
  test platform. Fixtures live in `src/fixtures/`. Shell tests are
  `scripts/test-*.sh`, run directly. Ignored tests needing the net or a
  real yardr say so where they are defined.
- Gate: `.yardr/test` (`cargo fmt --check`, `clippy -D warnings`,
  `cargo test`, `cargo build`). Landing runs the base copy of that
  script, not yours.

## Do not touch

- Build output ignored by `.gitignore`: generated files, not source to edit.
- `Cargo.lock`: committed; cargo owns it, never hand-edit.
- `src/fixtures/yardr-v0.10.0/`: published checksums the built-in key
  must keep verifying; do not regenerate.
- `assets/*.png`: rendered masters (see `README.md` "The icon",
  `scripts/bundle-mac.sh`); new marks come from yardr-design.
- The `eyes` rule in `.yardr/depot.md` still applies: a change that
  shows reads the yardr design system first.
