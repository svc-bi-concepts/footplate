# footplate

A native chat window onto a yardr yard (Rust, gpui). Product behaviour:
`README.md`. Design notes: `docs/design/`. The yard's side of the
protocol (`docs/api.md`, `docs/setup.md`, `docs/doctor.md`) lives in the
yardr repository, not here. `.yardr/depot.md` holds the yard's rules for
builders.

## Layout

Top level: `src/` the program, `build.rs` (the Windows stack size, nothing
else), `scripts/` release, screenshot and shell-test helpers, `assets/`
icons, the Linux polkit action and the Linux desktop entry, `docs/design/`
decision notes, `.yardr/` the yard's gate, `.github/` the Windows jobs
(build and test, which the gate does not try, and the release's Windows zip).

The window, its state and the yard:

- `src/main.rs`: opens the one window, owns the app lifecycle; draws nothing.
  `--version` is answered before any of it (the Windows build's smoke test
  runs it).
- `src/window.rs`: draws the window and makes the calls the model asks for;
  no protocol, no state of its own. `src/window/firstrun.rs` draws the
  first run where the yard will be.
- `src/model.rs`: what the window shows, without the window (rows, the open
  conversation, unread); never calls the yard, it answers an `Effect` the
  window performs.
- `src/api.rs`: the yard's socket API (calls, event stream, their JSON), by
  socket (unix alone) or at an address over TCP+TLS; HTTP hand-written, no
  client library.
- `src/session.rs`: follows one yard on a thread: login, initial state,
  stream, retry.
- `src/yard.rs`: picks which yard opens (socket or address, token files,
  hosted sessions); tokens live beside the settings, never in them.
- `src/settings.rs`: what the window keeps between runs, one JSON file in the
  platform config dir (`%APPDATA%\footplate` on Windows).

Text and drawing helpers:

- `src/draft.rs`: what is written and not sent yet, per conversation; neither
  calls nor draws.
- `src/input.rs`: the line typed in (gpui has no text field).
- `src/select.rs`: a message's text with mouse selection, one at a time.
- `src/markdown.rs`: the light Markdown messages draw with (code, one-level
  lists, headings to h3, bold/italic); the rest stays as written.
- `src/theme.rs`: the light and dark palettes.

Platform surface:

- `src/notify.rs`: OS notifications; rules as plain functions, then macOS's
  notification center and Linux's `notify-send`.
- `src/tray.rs`: the Mac menu-bar item and the Linux tray item, with the
  yards menu.
- `src/dock.rs`: the number on the macOS Dock icon.
- `src/taskbar.rs`: the Linux desktop entry and window icon; an entry that
  shows in the menu (the archive's `assets/linux/footplate.desktop`, copied
  there) is the person's, and stays.
- `src/unlock.rs`: the system's unlock in front of a weighty approval
  (the Mac's LocalAuthentication, Linux's polkit with the action in
  `assets/linux/`), with the reasons as plain functions; where no
  unlock is offered, the card takes the ask's number typed back instead
  (`Approval::TypeBack`).
- `src/titlebar.rs`: the window's title bar options per platform, and on
  macOS, where the content reaches up into it, the drag and double-click on
  the window's top strip, which gpui has no call for there.
- `src/log.rs`: what footplate says on stderr, kept as well in a file in the
  platform's log directory, with the time, where an app started by the
  Finder or launchd has no stderr to find it on; the path rules as plain
  functions, then the file and its cap.

Setup, release and tests:

- `src/fetch.rs`: fetches yardr's release from yardr.dev (minisign over
  checksums against the built-in key, archive against checksums), plus
  `yardr doctor`-named programs by checksum alone; and asks the kept yardr
  its version, for the About card.
- `src/setup.rs`: the first run as yardr's commands, with no window (args in,
  JSON out, PATH from doctor's data, never through a shell).
- `src/shot.rs` (debug builds only): scripted states against a fake yard for
  screenshots, named by `FOOTPLATE_SHOT`.
- `src/fake.rs`: a yard for tests (socket or TCP, scripted JSON, tokens and
  pairing; no TLS). `Yard::start()` is the socket's on unix and the TCP
  one's on Windows.
- `src/fixtures/`: `shot-page.png`, and the published yardr v0.10.0 checksums
  plus signature the built-in key is held to.

## Where to start

- A new socket call: the call in `src/api.rs` with its row in
  `README.md`'s "It reads" or "It writes" (a test in `src/api.rs` asks for
  it, and for a call that only reads to be in `READS`, which is what a
  timeout on it says: a call not there may have run), its answers in
  `src/fake.rs`, the state it needs in `src/model.rs` (an `Effect` for what
  the window must do), the drawing and the call site in `src/window.rs`.
- A new view or pane: state in `src/model.rs`, drawing in `src/window.rs`;
  window chrome in `src/main.rs`. A pane's top row is under the macOS
  title bar's strip (`src/titlebar.rs`): a press on it drags the window, so
  a control there still clicks, and a text field or a drag of its own does
  not belong in the strip. What stands at the rail's edge begins with
  the strip's band (`ui::band`), so that nothing is under the traffic
  lights: the list's head is the band, and a pane that starts there (the
  settings' sheet) stands under an empty one.
- A first-run step: the command in `src/setup.rs`, the screen in
  `src/window/firstrun.rs`.
- A setting: `src/settings.rs`, then where it takes effect.
- Platform surface (badge, tray, notifications, Linux icon, the Mac's
  unlock, title bar, log): `src/dock.rs`, `src/tray.rs`, `src/notify.rs`,
  `src/taskbar.rs`, `src/unlock.rs`, `src/titlebar.rs`, `src/log.rs`; rules
  stay plain functions, system calls stay behind `cfg`. A test never writes
  the person's own log: `api::say` keeps in the file its test's thread was
  given (`tests::keep_in`), or none.
- Screenshots: states in `src/shot.rs`, taken by `scripts/shot-linux.sh`;
  the states are named in `README.md`.
- Releases: `scripts/release.sh` builds and lists each archive, stopping at
  a yardr in one, `scripts/bundle-mac.sh` bundles, `scripts/release-stage`
  drives the yard's release flow. The Windows zip is not built there: the
  `release` job of `.github/workflows/windows.yml` builds it, and
  `scripts/release.sh` takes the zip of the run for `HEAD` with `gh`. A new
  file in the Linux archive is packed in `scripts/release.sh`, listed in
  `scripts/test-release.sh`, and added to yardr.dev's `scripts/sync-release`
  (another repository), which refuses a member it does not know; one in the
  Windows zip is zipped in the workflow, checked in `scripts/release.sh` and
  `scripts/test-release.sh`, and added to `sync-release` the same way.

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
  real yardr say so where they are defined. A test probes no port of its
  machine that it did not open itself: the usual web view and sandbox
  origin are not asked in a test (`rail::reaches`), since the person's own
  may answer there, and Windows refuses a connection to a closed port only
  after two seconds, on the test's own thread. A view a test needs it
  stands up (`view_up`); `view_down` costs those two seconds for each
  probe, so use it only where down is the point.
- Windows is remote only: it has no yard of its own, so the unix socket,
  file modes, symlinks, the shell stand-in for yardr and the Linux
  desktop's taskbar sit behind `cfg(unix)` (or the OS), and so do the tests
  that need them. A call that exists on unix alone arrives with its `cfg`
  and, where Windows would reach it, an arm that says so. A test that
  starts `Yard::start()` needs no gate: on Windows that yard is the TCP one
  and `client()` reaches either. A window test opens that yard as "this
  machine's" with `env_on` (the tests of `src/window.rs`), which keeps the
  socket where the yard has one and, where it has none, lists the yard at
  its address under the same name; `listed` does the same for the other
  yards of a list, and `Yard::at_listener()` is what a test asks to expect
  the one kind or the other. One that names the socket (`socket()` written
  out, `start_at`, a yard stopped and started again at its path), asserts
  what only the socket does (its request text, `is_remote()`), or checks
  what the window offers on one system alone, is `#[cfg(unix)]`, with the
  reason in a comment above the gate.
- Gate: `.yardr/test` (`cargo fmt --check`, `clippy -D warnings`,
  `cargo test`, `cargo build`). Landing runs the base copy of that
  script, not yours. It runs on a Mac; `.github/workflows/windows.yml`
  runs `cargo build` and `cargo test` on Windows, and builds the release's
  Windows zip there.

## Do not touch

- Build output ignored by `.gitignore`: generated files, not source to edit.
- `Cargo.lock`: committed; cargo owns it, never hand-edit.
- `src/fixtures/yardr-v0.10.0/`: published checksums the built-in key
  must keep verifying; do not regenerate.
- `assets/*.png`: rendered masters (see `README.md` "The icon",
  `scripts/bundle-mac.sh`); new marks come from yardr-design.
- The `eyes` rule in `.yardr/depot.md` still applies: a change that
  shows reads the yardr design system first.
