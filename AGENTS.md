# footplate

A native chat window onto a yardr yard (Rust, gpui). Product behaviour:
`README.md` is the front door (what it is, installing) and `docs/` the
manual: `docs/yards.md` (what it talks to, setting a yard up, starting it),
`docs/window.md` (the window, the settings, notifications),
`docs/windows.md` (Windows) and `docs/building.md` (building, releasing, the
icon, the code). Design notes: `docs/design/`. The yard's side of the
protocol (`docs/api.md`, `docs/setup.md`, `docs/doctor.md`) lives in the
yardr repository, not here. `.yardr/depot.md` holds the yard's rules for
builders.

## Layout

Top level: `src/` the program, `build.rs` (the Windows stack size, nothing
else), `scripts/` release, screenshot and shell-test helpers, `assets/`
icons, the Linux polkit action and the Linux desktop entry, `docs/` the manual
and `docs/design/` decision notes, `.yardr/` the yard's gate, `.github/` the
Windows jobs (build and test, which the gate does not try, and the release's
Windows zip), `spikes/` throwaway programs that try a provider or a dependency
before the product takes it, each its own cargo package and its own workspace,
so that neither the gate nor a release sees it: `spikes/voice-harness/` is the
GPT-Live voice session of `docs/design/2026-10-31-footplate-vqze-voice.md`, run
by a person with a key (its `RUN.md` has the command); it is deleted when
`src/voice.rs` and `src/audio.rs` take what it proved.

The window, its state and the yard:

- `src/main.rs`: opens the one window, owns the app lifecycle; draws nothing.
  `--version` is answered before any of it (the Windows build's smoke test
  runs it).
- `src/window.rs`: draws the window from the model; no protocol. It holds the
  `Footplate` view, its actions and key bindings, and its `Render`, which
  picks what the window shows (the list with the open conversation, the
  settings, the first run, the away screen) and stacks the layers over it.
  The rest is in `src/window/`:
  - `src/window/arrivals.rs`: the timetable's views: the arrivals pane with
    its panel of tracks and departures board, the lines' flows, and the
    beads that travel with no line.
  - `src/window/ask.rs`: the words and keys of an ask's card: its heads and
    counts, its buttons, and the texts a run or a setup holds.
  - `src/window/asks.rs`: the decisions' views: the decisions list with its
    decided history, each open ask's card with its secret, run and setup
    blocks, and what the cards do (answer, approve, give, decline, put
    aside).
  - `src/window/bead.rs`: one bead's views: the bead whole, the conversation
    thread with its mail, and the pictures and attachments of both.
  - `src/window/chrome.rs`: the shell the views stand in: the rail, the list
    and the open conversation, the settings and the keys, the tray and the
    badge. `told` takes in what the yard says, and `Footplate::run_in`
    performs the `Effect`s the model asks for, each a call by the yard's
    client. `change`, `change_in`, `keep_place`, `reach_top` and `jump` are
    the plumbing that applies a change of the model to the view.
  - `src/window/feedback.rs`: the Feedback form over the window; what it
    sends is `src/feedback.rs`'s.
  - `src/window/firstrun.rs`: the first run, "Set up a yard here", where the
    yard will be; each step is a command of the yardr on this machine, made
    through `src/setup.rs`.
  - `src/window/keys.rs`: the contexts the window's key bindings answer in,
    and the tables the cheatsheet names for each view.
  - `src/window/metrics.rs`: the window's measures: fixed sizes in pixels,
    sizes the words set in rems, the char budgets of the rows that end a long
    string in an ellipsis (`ui::shorten`), and the small functions that fit a
    pane or a drawer to them.
  - `src/window/panel.rs`: the reading pane's column, day breaks and rows,
    and the panel's heads, tracks and cards.
  - `src/window/rail.rs`: the images a mail names, the tooltips under the
    pointer, the web view's probes, and what leaves the window: the calls
    that post a draft or a note (`post`, `post_note`).
  - `src/window/screen.rs`: one session's screen over the window, drawn like
    the terminal (monospace, dark, the styling as printed) from the
    `SessionScreen` answer, which `src/ansi.rs` parses. Each answer, the open
    and every refresh, stands the lines at the pane's bottom, where the
    agent's input line is (`screen_scroll`); the open stands them at the left
    edge too. A pane wider than the card scrolls sideways: the lines are a
    `flex_none` item of a flex scroll, so gpui measures their width, which
    the card's estimate (`CHAR_ADVANCE`) is not good enough for.
  - `src/window/state.rs`: what the window keeps beside the model: where a
    yard stood when it was left, what waits in the yards it follows, and
    what the switch and the settings are asking or doing.
  - `src/window/ui.rs`: the small pieces the window draws with: glyphs, the
    band of its top (`ui::band`), buttons, segments, checkboxes and selects
    (the field, list and option the Voice settings' two pickers are made of),
    and `ui::shorten`, which ends a string in an ellipsis after a char budget:
    gpui 0.2.2's `truncate()` clips mid-glyph, with no ellipsis, in a flex item
    that shrinks, so a row whose text may outrun it shortens the string too.
  - `src/window/words.rs`: the window's words for its rows (what its heads
    and counts say) and the colours that say what waits and for whom.
  - `src/window/yards.rs`: the yards' views: the switcher with its rows, each
    yard's card with its login switch and agents settings, pairing and
    signing in, and the away screen with the reconnecting strip.
- `src/ansi.rs`: a session pane's ANSI parsed to styled lines, which
  `src/window/screen.rs` draws. Pane bytes are untrusted, so vte parses
  them and keeps only the text and its SGR styling.
- `src/model.rs`: what the window shows, without the window (rows, the open
  conversation, unread); never calls the yard, it answers an `Effect` the
  window performs.
- `src/api.rs`: the yard's socket API (calls, event stream, their JSON), by
  socket (unix alone) or at an address over TCP+TLS; HTTP hand-written, no
  client library.
- `src/session.rs`: follows one yard on a thread: login, initial state,
  stream, retry.
- `src/voice.rs`: the voice session's rules, as plain functions and one small
  book, tested on any system: the request the spoken sentences stand for (every
  request since the last delegation, in order, noise tags and greetings cut),
  which answer the voice is given to speak and under which delegation's id,
  the queue its speech waits in and
  drops (the server's padding let go, a holdoff that counts what it throws
  away), the instructions and the state words, the voices GPT-Live offers
  (`VOICES`, which the Voice settings' picker lists and a session is started
  in, from OpenAI's reference), and the events of OpenAI's GPT-Live, which
  this is the one module to speak. `Provider` selects GPT-Live or the optional
  ElevenLabs Agent at explicit start; Agent configuration is saved separately,
  and its protocol lives in `src/voice/elevenlabs.rs`: signed-URL authentication,
  configuration checks, negotiated PCM conversion, interruptions and the
  `send_to_yard` client tool. Tool results are pending or uncertain, never
  claimed replies; selected-conversation mail is labelled context. No provider
  wire events enter the window's mail path. Below the rules, the live socket itself: a
  hand-written WebSocket over `src/api.rs`'s TLS (its upgrade's accept key is
  the one SHA-1 in the tree, `sha1_smol`), and the session's one thread in the
  `FollowControl` manner, which owns the microphone's frames in and the
  speaker's queue out; the devices that plug into those ends are
  `src/audio.rs`'s, and the window's one composer button (voice, send or
  stop, by what the box holds and whether a session stands), which starts
  and ends the session, and the speaking of the open conversation's
  answers to it, are in `src/window/bead.rs`. The caption, the meter and
  the idle rule are plain functions here too, and `stand_in` serves
  `src/fake.rs`'s `Live` on a loopback WebSocket, for the tests and the
  voice shot.
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
- `src/shown.rs`: whether the window shows, on Linux, where gpui tells of
  neither a minimized window nor a hidden one (macOS asks AppKit, in
  `src/main.rs`): the rule is a plain function of the compositor, the X
  server's word and the age of the window's last frame, tested on any system.
  Under X11 the word is `_NET_WM_STATE_HIDDEN`, read on a connection of its
  own that is kept; under Wayland the compositor says nothing, so it is the
  frames a standing voice session has the window stamp (`Means::frame`),
  and a window that others cover wholly sends none, so it counts as hidden.
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
- `src/audio.rs`: the voice session's microphone and speaker, mono PCM16 at
  24 kHz in and out whatever the devices speak, and nothing of any provider.
  The rules are plain functions tested with no device, calling the callbacks
  as a device does (the resampler, `Capture` for the microphone's, `Render`
  for the speaker's, the names a person tells the devices by, and `pick`,
  which of the microphones the host lists a kept id opens: the default where
  it names none); the devices are cpal's behind `cfg(any(macos, linux))`,
  opened on their own default format, and elsewhere `open` says there is no
  audio. The microphone `open` takes is the id the settings keep
  (`Input::id`, the host's own: the UID on a Mac), and `inputs` is the one
  listing of them, which the Voice settings' picker reads through
  `Means::inputs` and which opens no device, since ALSA opens one to read its
  format. CoreAudio is bound through objc2 and needs no C library; Linux's
  ALSA backend is the one C library this adds: `libasound2-dev` (with
  `pkg-config`) to build, and `libasound.so.2` at run time. A Mac's consent
  to the microphone is read apart from the devices, since an app without it
  is given silence and no error: `Consent` is AVFoundation's authorization
  status for audio (through objc2-av-foundation, behind `cfg(macos)`),
  `allowed` turns a refusal into words, and `open` asks the system and waits
  for its answer when consent has not been determined, before opening the
  microphone, then reads it again after (`consenting`).

Setup, release and tests:

- `src/feedback.rs`: what the Feedback form sends, and where: one survey
  answer to yardr's PostHog (EU) with the project's write key, carrying the
  message, the version, the OS and the email where given, from an id made for
  that send; the key and the survey's id are two plain constants, and with
  either empty there is no project, so no button. It sends by
  `api::post_json`, the post to a site that is no yard.
- `src/fetch.rs`: finds the yardr on this machine (`Machine`: the yard's
  `<home>/binary`, `~/.local/bin`, the PATH, the login shell, which it also
  asks for the PATH a terminal starts with), moves the copy
  an older footplate kept, and runs yardr's installer from yardr.dev; fetches
  `yardr doctor`-named programs by checksum; and asks the yardr its version,
  for the About card.
- `src/setup.rs`: the first run as yardr's commands, with no window (args in,
  JSON out, PATH from doctor's data, never through a shell); and the one line
  that puts `~/.local/bin` on a terminal's PATH, which it appends to the
  shell's startup file on the person's yes alone (`Reach`, `startup_file`:
  plain functions, tested with no screen).
- `src/shot.rs` (debug builds only): scripted states against a fake yard for
  screenshots, named by `FOOTPLATE_SHOT`; the voice state stands a session
  against a fake `Live` as well, with no microphone or speaker opened.
- `src/fake.rs`: a yard for tests (socket or TCP, scripted JSON, tokens and
  pairing; no TLS), a stand-in for PostHog's capture (`Posthog`), and, for
  tests and the voice shot, one for OpenAI's Live socket (`Live`: the
  server's side of a voice session as events in and out, held to the
  reference, not to the module that uses it). `Yard::start()` is the
  socket's on unix and the TCP one's on Windows.
- `src/fixtures/`: `shot-page.png` and the terminal screens.

## Where to start

- A new socket call: the call in `src/api.rs` with its row in
  `docs/yards.md`'s "It reads" or "It writes" (a test in `src/api.rs` asks for
  it, and for a call that only reads to be in `READS`, which is what a
  timeout on it says: a call not there may have run), its answers in
  `src/fake.rs`, the state it needs in `src/model.rs` (an `Effect` for what
  the window must do), its call site in `src/window/chrome.rs` (an arm of
  `Footplate::run_in`), the drawing in `src/window.rs` or a file of
  `src/window/`.
- A new view or pane: state in `src/model.rs`, drawing in `src/window.rs`
  or a file of `src/window/`; window chrome in `src/window/chrome.rs`. A
  pane's top row is under the macOS
  title bar's strip (`src/titlebar.rs`): a press on it drags the window, so
  a control there still clicks, and a text field or a drag of its own does
  not belong in the strip. What stands at the rail's edge begins with
  the strip's band (`ui::band`), so that nothing is under the traffic
  lights: the list's head is the band, and a pane that starts there (the
  settings' sheet) stands under an empty one.
- A first-run step: the command in `src/setup.rs`, the screen in
  `src/window/firstrun.rs`.
- A setting: `src/settings.rs`, then where it takes effect.
- A voice rule: a plain function or a `Book` method in `src/voice.rs` with
  its test, and what the stand-in must say for it in `Live` (`src/fake.rs`).
  GPT-Live's events stay in that module; what is outside speaks of speech
  started, a request, an answer to speak and stop.
- Platform surface (badge, tray, notifications, Linux icon, whether the
  window shows, the Mac's unlock, title bar, log, voice's microphone and
  speaker): `src/dock.rs`, `src/tray.rs`, `src/notify.rs`,
  `src/taskbar.rs`, `src/shown.rs`, `src/unlock.rs`,
  `src/titlebar.rs`, `src/log.rs`, `src/audio.rs`; rules
  stay plain functions, system calls stay behind `cfg`. A test never writes
  the person's own log: `api::say` keeps in the file its test's thread was
  given (`tests::keep_in`), or none.
- Screenshots: states in `src/shot.rs`, taken by `scripts/shot-linux.sh`;
  the states are named in `docs/building.md`.
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
  stands up (`view_up`). One that is down it stands up as well
  (`view_down`): a listener that lets every connection go with no answer,
  which holds its port until `view_serve` has it answer there, since a
  port let go is free to any other test that asks for one.
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
- `assets/*.png`: rendered masters (see `docs/building.md` "The icon",
  `scripts/bundle-mac.sh`); new marks come from yardr-design.
- The `eyes` rule in `.yardr/depot.md` still applies: a change that
  shows reads the yardr design system first.
