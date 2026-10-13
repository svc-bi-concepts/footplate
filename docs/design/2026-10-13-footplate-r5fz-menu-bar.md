# Footplate in the menu bar: what waits, without the window

Design footplate-r5fz, proposed 2026-10-13. A proposal as it was decided on, not a description of what the code does today; the decision is on the bead.

Design for bead footplate-r5fz (flow `design`, stage `draft`). A yard is
one installation of yardr: its store, its crew and its depots. A user is a
person of a yard, a name and an inbox; the operator is the user who runs
the yard. A device is one program of a user that calls the yard from
outside it, let in by a token the yard gave it. An ask is one question as
data on a bead; footplate shows asks as cards, and answering one from the
window counts as the user's. footplate is the native window in which a
person reads mail and answers asks. See docs/glossary.md in the yardr
repository for all of yardr's words.

## 1. The question

mckean, 13 Oct 2026: "would a tray icon make sense there?" and, after the
yardmaster's answer, "the tray can be sent into the works". yardr itself
gets none (his word: "no tray icon for yardr"). The wish: a small item in
the Mac's menu bar that says whether something waits (cards to decide,
unread mail, over every yard footplate follows), and on a click lists it
and opens the card or the conversation in the window.

## 2. What exists today

**Closing the window quits.** `cx.on_window_closed` in src/main.rs:56-61
quits the application once no window is left. The README's key table says
it too: `cmd+q` quits, "so does closing the window". There is exactly one
window; design footplate-5wz5 fixed "one footplate, one window".

**gpui can veto a close and can hide.** gpui 0.2.2, the pinned release in
Cargo.toml, offers `Window::on_window_should_close` (src/window.rs:4329):
a callback whose false leaves the window open, wired on macOS to
`windowShouldClose:`. It offers `App::hide` (src/app.rs:984), the
platform's hide: on macOS the application hides with its window and
keeps running. Coming back is three roads: a dock click and `cmd+tab`
unhide a hidden application by the system's own hand, while starting the
application again while it is hidden reaches only a registered callback,
`App::on_reopen` (src/app.rs:198), which the Mac delegate calls when no
window is visible (src/platform/mac/platform.rs, `should_handle_reopen`);
footplate registers none today. gpui offers no unhide of its own (only
`unhide_other_apps`), and `App::activate` (src/app.rs:979) is
`activateIgnoringOtherApps:`
(src/platform/mac/platform.rs:564-569), which the builder must prove
restores a hidden window or pair with `NSApplication::unhide`, whose
binding the tree already has (objc2-app-kit 0.3.2, generated
NSApplication.rs:507). It offers no tray, status-item or dock-badge
call: a search of its sources for badge, dock tile, tray and status finds
none.

**gpui's own status item is a dead file.** gpui 0.2.2 carries
src/platform/mac/status_item.rs, 388 lines, but no module declares it: no
`mod status_item` anywhere, so nothing compiles or exposes it. Read
further, it could not compile as it stands: it uses the `cocoa` and `objc`
crates, an older generation of bindings than the `objc2` family the rest
of gpui 0.2.2 builds on, and it refers to `super::screen`, of which no
file exists (the module is `display.rs`). Waiting for gpui means waiting
for Zed to rewrite it; nothing on the crates.io release path suggests a
date.

**The bindings the item needs are already depended on.** footplate uses
`objc2-app-kit` 0.3 on macOS today (Cargo.toml; `dock_icon` in
src/main.rs:97-113 sets the dock icon through it). Version 0.3.2, as
vendored, carries generated bindings for `NSStatusBar`, `NSStatusItem`,
`NSMenu` and `NSMenuItem`, and `NSApplication` answers
`setActivationPolicy` and `dockTile`. A menu-bar item from our own Cocoa
calls needs only new features on that dependency, no new crate.

**The crate alternative is named: `tray-icon`.** `tray-icon` 0.26.1
(tauri-apps, Apache-2.0/MIT, per docs.rs) makes tray icons with a menu
from its sibling crate `muda`, and reports clicks over a channel. It
pulls in `muda`, `crossbeam-channel`, `once_cell`, `thiserror` and `png`,
and on macOS the same `objc2` family footplate already builds on. On
Linux it offers two backends, both optional: `ksni`, a pure-Rust
StatusNotifier speaker over D-Bus, or `libappindicator`, which needs the
system's gtk and libappindicator libraries installed to build against.

**Every yard is already followed while the window is up.** Since
footplate-qhzm the window holds one `Client`, one follower thread and one
`Model` per yard of the settings, the open yard's in the `Footplate`
view's own fields and the rest in its `held` map (src/window.rs:656-696).
`session::follow_with_control` (src/session.rs:252-315) reads the state
once and then the event stream after it, retrying while the yard is away.
So the cost of staying up with no window is exactly what the open window
pays now, minus drawing: per yard, one thread, one held event-stream
connection, and one model in memory. Nothing extra is held for the tray;
the tray only keeps from being torn down what closing tears down today.

**The waiting is already counted, in words, per yard.**
`Counts::of(model)` (src/window.rs:591-597) sums a yard's unread mail, its
open decisions (`Model::decisions_count`) and its unread notices
(`Model::yard_unread`), and `Counts::line` (599-619) says it in words
("2 mail, 1 decision", "nothing waiting"). The switcher draws one such
line per yard. The dock-badge bead footplate-k4jr, filed beside this one
and already being built, puts the total over every yard, mail plus
decisions, on the dock icon through `NSApp.dockTile.badgeLabel`, with
notices left out and no setting. Its total function is the tray's source:
the tray and the badge say the same number.

**A notification's activation already opens its row in its yard.**
`Footplate::heed` (src/window.rs:991-1003) takes activations off the
system's channel and `come_forward` (1080-1098) brings the window forward
on the row, opening that yard first where another is open. A menu row's
click is the same shape only in its first half: an event from outside
gpui that must activate the application and its window. Its second half
differs: a notification names a row, while a menu row names a yard, and
the yard opens on its own row, below. `come_forward` is not the menu's
road, because it takes a `Selected` the menu does not have.
`notify::system` (src/notify.rs) is the seam: macOS
through `UNUserNotificationCenter`, which knows an application by its
bundle, so notifications need `Footplate.app` (scripts/bundle-mac.sh) and
say so from a bare binary; Linux through `notify-send`, whose activation
opens nothing. A status item needs no bundle: `NSStatusItem` works from a
bare binary, so the tray is also there from `cargo run`.

**A shown yard opens on its own row.** `show` (src/window.rs:1160-1171)
puts the held yard back and `arrive` (1319-1345) opens it through
`Model::back_to` (src/model.rs:571-577) on the row it was left on, kept
in `places` (src/window.rs:1249-1256), falling back per `Model::row`
(src/model.rs:535-555): an open decision, else the yardmaster's
conversation, else the first row from the top. So switching yards, as
the switcher does through `switch_to` (src/window.rs:3537-3549),
always lands on a defined row without naming one: the existing
selection, or the yard's own fallback where that is gone.

**The window knows when it is in front.** `front` (src/window.rs:819) is
kept by `observe_window_activation` (916-918); `notify::wanted`
(src/notify.rs:45-47) tells of news the settings choose except what
arrives on the open row of a window in front. Hidden, the window is not
in front, so what arrives while it is hidden notifies by the settings'
kinds, as it does today when the window is behind another. The builder
proves that hiding reports inactive; if it does not, hiding sets it.

**The settings' switches default to not surprising.** `Notify`
(src/settings.rs:211-261) is off until switched on, keeping its kinds for
the next time. The settings file is written on every change
(`~/Library/Application Support/footplate/settings.json` on macOS). The
badge bead deliberately has no setting; whether the tray has one is
settled below.

**The one-file-per-platform pattern exists.** src/taskbar.rs is compiled
on Linux and in tests only (`mod taskbar` in src/main.rs:18-19): plain
testable functions of what can be said without a desktop, and the system's
calls behind them. A macOS-only tray file mirrors it, and the gate, which
builds on Linux too, sees the plain functions and no Cocoa.

## 3. The proposal

The smallest version that gives him the sign in the menu bar: on macOS,
closing the window hides the application instead of quitting it, and a
menu-bar item shows the total that waits and lists the yards. Three
pieces, each shippable alone, in this order: the close behaviour, the
item and its menu, the setting that turns the item off (which lands with
the first piece, since it governs what closing does).

### 3.1. Close hides, quit quits, the dock icon stays

On macOS, with the item on: the red button hides the application. The
mechanism is gpui's own: `on_window_should_close` returns false and calls
`cx.hide()`. The `Footplate` view is never dropped, so every follower,
every model, every draft and the notification burst survive; closing
stops costing a reload and starts costing the followers' keep-up, which
section 2 priced. `cmd+q`, the menu's "Quit footplate", and the tray
menu's Quit all quit: quitting is the only way out, as in any Mac
application that stays up with no window. The dock icon stays a regular
dock icon: its click and `cmd+tab` bring the window back, a start while
hidden reaches the reopen handler, and the badge bead's number sits on
it. Coming back is specified, not hoped for: `main` keeps the window's
handle and registers `cx.on_reopen` to activate the application and that
window again, calling `NSApplication::unhide` first where activation
alone leaves it hidden. The builder proves all three roads on the
machine: the dock click, `cmd+tab`, and starting the application again
while it is hidden, each showing the window. The builder also checks
that quitting never consults the close veto, so Quit always quits.

With the item off, or off macOS, closing quits, exactly as today. The
rule is absolute, because the alternative is a trap: never hidden with
no way back. A hidden application with no menu-bar item is found only by
those who know the dock click; a person who switched the item off asked
for no such state, so they never enter it.

### 3.2. The item: the total, and the yards under it

The item shows one number: how many things wait over every yard
footplate follows, the same total the dock badge shows (mail plus
decisions; what the total counts is the badge bead's to settle, and the
tray follows it, so the two never disagree). Nothing waits: the title is
empty, and a small glyph that is always there, the application's own mark
made from assets/icon.png as `dock_icon` makes the dock icon, keeps the
item a clickable width; whether it is a template-masked silhouette or the
plain mark is the builder's to choose. The item never depends on its
title for its target, so "Show window" and "Quit footplate" stay
reachable while the window is hidden. On a click the item opens a menu,
one row per yard in the settings' order, each row the yard's name with
its waiting line ("plantime · 2 mail, 1 decision", "shop · not reached:
nothing answers"), reusing `Counts::line` and the switcher's whys, then
a separator, then "Show window" and "Quit footplate". Choosing a yard
row opens the window on that yard, as the switcher does through
`switch_to`: the yard shows the row it was left on, or its own fallback
where that is gone (an open decision, the yardmaster's conversation, the
first row, per `Model::row`). The menu promises no narrower target: it
opens yards, never a named card or conversation. Choosing "Show window"
brings the window forward on whatever is open.

What the person sees, with two yards followed and three things waiting:

```text
menu bar:  [ …  3  … ]

click:
  mac · 1 mail
  plantime · 1 mail, 1 decision
  ─────────────
  Show window
  Quit footplate
```

The item is Cocoa behind one small file, src/tray.rs, compiled on macOS
and in tests only, mirroring src/taskbar.rs: plain functions (the title
for a total, the rows for yards and counts) tested anywhere, and the
`NSStatusBar`/`NSStatusItem`/`NSMenu` calls behind them, through new
features on the `objc2-app-kit` footplate already depends on. Clicks
reach gpui over a channel consumed by a spawned task, as `heed` consumes
the notification system's activations. Linux gets nothing: the file's
plain functions compile there, the item does not exist, and close quits
as today; the gate builds either way.

### 3.3. One source, three jobs

The notifications (footplate-prw2), the dock badge (footplate-k4jr) and
the tray share one source, the yards' counts and news, and each has one
job, so nothing is said three times. The notification announces an
arrival once, in the moment, with its words; it needs the system's
permission and a bundle, and stays off until switched on. The badge says
at a glance, on the dock, how many wait; it needs neither permission nor
bundle. The tray says the same number in the menu bar, where the dock is
not looked at, and is the way back into a hidden window: it lists the
yards and opens them. While the window is hidden, arrivals still notify
by the settings' kinds (section 2); the tray never carries words, only
counts, so a notified mail is announced once and then counted, not told
twice.

### 3.4. The setting

One boolean in the settings, beside "notifications", switching the
menu-bar item on and off. When it is off, closing quits, per section 3.1.
Whether it starts on or off is open question 1.

## 4. Left out

**Answering from the menu.** No picking, no approving, no replying. An
approval is only safe on the card: the run ask's block shows the command
whole with the yard's sentence for what it changes, and design
footplate-5wz5 put the yard on every card and on the approve control so
that no approval is pressed without its yard on it. A menu row cannot
carry any of that, and two yards' rows one key apart would undo "nothing
goes from one yard to another". The menu opens; the window acts. One
road for acts is also easier to learn than two.

**Hiding the dock icon.** An accessory application (no dock icon, menu
bar only) would need the activation policy changed at runtime and would
take the badge with it: no dock icon, no badge. The dock icon stays,
section 3.1; the tray is for those looking at the menu bar, not a second
home for the application.

**Notices in the total, or their own sign.** The tray shows the badge's
total, and the badge bead left notices out (they tell of every bead that
moves). If the decided direction wants them, both change together; the
tray never counts differently from the badge.

**Rows for named cards or conversations.** The menu lists yards, one row
each, and opens the yard's own row. A row per waiting card would carry
the menu past a glance, and across yards would mix rows of two yards
together, which design footplate-5wz5 refused. The menu opens; the
window's list chooses.

**Hiding the item while empty.** The glyph stays whatever the count,
section 3.2, so the way back into a hidden window is always where it
was; an item that comes and goes would hide the application's only
visible sign with it.

**Linux tray, start at login.** Each is an open question with its own
later bead; neither blocks the sign on the Mac.

**A merged list of every yard's rows.** As in design footplate-5wz5: no.
The menu lists yards with counts, as the switcher does, never rows of
two yards together.

## 5. Open questions

1. **Whether the item starts on or off.** Option A, on: closing hides
   from the first start, and the sign is there without asking; it costs
   one surprise (close no longer quits) and the switch for those who want
   today back. Option B, off: nothing changes until the person switches
   it on, in the settings' tradition that nothing surprises one who did
   not ask; it costs the discovery (a person who never opens the settings
   never learns the tray exists) and, while off, the close behaviour this
   design is for. I would take A: he asked for the tray to be sent into
   the works, hiding on close is standard Mac behaviour, and the off
   switch keeps today's quit for whoever wants it.
2. **Whether the item shows the number or only a dot.** Option A, the
   number, as section 3.2 proposes: it says how much waits at a glance
   and matches the badge; it costs nothing beyond the title. Option B, a
   dot when anything waits: quieter, and the count is one click away in
   the menu; it costs the glanceable number and disagrees with the badge
   in spirit. I would take A: the badge already says the number on the
   dock, and the tray saying less in the menu bar would be the poorer
   twin for no saving.
3. **How the item is built.** Option A, our own Cocoa calls in one
   mac-only file, as section 3.2 proposes: no new dependency, only new
   features on `objc2-app-kit`; the menu is a plain `NSMenu`, no custom
   drawing; it costs the file itself (a few hundred lines, most of it
   the menu and the channel into gpui) and macOS only. Option B, the
   `tray-icon` crate (0.26.1, Apache-2.0/MIT): the item and its menu from
   one maintained crate, and Linux comes along via `ksni` or
   `libappindicator`; it costs a new dependency tree (`muda`,
   `crossbeam-channel`, `png` and the rest), a second menu concept
   beside gpui's, and on Linux either D-Bus plumbing or system libraries
   to build against. Option C, waiting for gpui: no code at all, but the
   file is dead, of the older bindings generation, and not compiling, so
   the wait has no date. I would take A: the bindings are already in the
   tree, the need is one number and a plain menu, and B's Linux prize is
   open question 4's to want or refuse.
4. **What Linux gets.** Option A, nothing, as section 3.2 proposes: the
   gate builds, close quits, and the tray is a Mac affair; it costs the
   Linux users the sign. Option B, a StatusNotifier tray in a later bead,
   through `ksni` (pure Rust, D-Bus) or `tray-icon` with its `ksni`
   backend: it costs the bead, the D-Bus menu plumbing, and desktops
   without a StatusNotifier host showing nothing anyway. I would take A
   now with B as its own later bead: the laptop is a Mac, and a Linux
   tray designed blind, without its desktop to try it on, is the kind of
   code that rots.
5. **Whether footplate starts at login.** Option A, no: the window, and
   the tray with it, start when opened; it costs mckean one opening after
   a restart. Option B, a later bead in which footplate offers it: a
   switch in the settings, off to begin with, that writes and removes a
   `LaunchAgent` property list under `~/Library/LaunchAgents`, the way
   src/taskbar.rs already installs the Linux desktop entry from the
   program itself, needing no installer and no new dependency; it costs
   the bead and the property-list plumbing. Option C, the same through
   `SMAppService`, the system's login-items call: it costs bindings
   footplate does not have and a bundle-only path (a bare binary cannot
   register). I would take B as a later bead: it mirrors a mechanism
   footplate already owns, and A stays true until anyone misses it.

## 6. The beads it would become

All client-side, in the footplate depot; each names what it depends on.
The first two are the design; the last two are optional and wait on open
questions 4 and 5.

**Footplate 1. On macOS, closing the window hides; quitting quits.**
Goal: with the menu-bar item on, the red button hides the application
and everything goes on (followers, models, drafts, notifications); with
it off, or off macOS, closing quits as today. Where to look: the
`on_window_closed` handler in src/main.rs:56-61; gpui 0.2.2's
`Window::on_window_should_close`, `App::hide` and `App::on_reopen`
(src/app.rs:198), the last reactivating the kept window handle, with
`NSApplication::unhide` where activation alone leaves it hidden; the
`Notify` switch in src/settings.rs:211-261 for the new boolean beside
it; the `front` flag in src/window.rs:819 and its
`observe_window_activation` at 916-918, which must read inactive while
hidden. Depends on nothing. Done when: on macOS, closing with the item
on leaves the process running, arrivals still notify by the settings'
kinds, and the dock click, `cmd+tab`, and starting the application
again while hidden each show the window, seen on the machine and said
so in the hand-on note; closing with it off quits; `cmd+q` and the
menu's Quit quit from either state (the builder proves the close veto
is never consulted by Quit); on Linux nothing changes; the README's
key table no longer says closing quits unconditionally; and
`.yardr/test` passes.

**Footplate 2. The menu-bar item and its menu.** Goal: the sign in the
menu bar from section 3.2: the total while anything waits beside a glyph
that is always there, a menu of the yards with their waiting lines,
"Show window" and "Quit footplate", a yard row opening the window on
that yard's row, never a named card. Where to look: Footplate 1 for
the setting and the hide behaviour; the badge bead footplate-k4jr for
the total function, which this bead reuses and never recomputes;
`Counts::line` (src/window.rs:599-619) for the rows; `heed` (991-1003)
for the road an outside event takes into the window, and `switch_to`
(3537-3549) with `arrive` (1323-1345) for what a yard row opens (the
row it was left on, per `Model::back_to`, or its `Model::row`
fallback); src/taskbar.rs for the shape of the new mac-only src/tray.rs
(plain functions tested anywhere, Cocoa behind them); assets/icon.png
with `dock_icon` (src/main.rs:97-113) for the glyph; the `NSStatusBar`,
`NSStatusItem` and `NSMenu` features of the `objc2-app-kit` in Cargo.toml.
Depends on Footplate 1 and on footplate-k4jr. Done when: the title and
the rows are plain functions with tests (no yard, one, several, one not
reached, nothing waiting); the item updates where the badge updates;
the empty item keeps its glyph and opens the menu; a yard row's click
shows that yard on the row it was left on, or its fallback where that
is gone; Linux builds with no item and close quitting; the README says
what the item shows and that, unlike notifications, it needs no bundle;
the hand-on note says the item was seen or that it was not; and
`.yardr/test` passes.

**Footplate 3 (only if open question 4 goes B). A tray on Linux.**
Goal: the same total and the same per-yard menu through the desktop's
StatusNotifier host, or nothing where there is none. Where to look:
Footplate 2's plain functions, reused whole; `ksni`, or `tray-icon` with
its `ksni` backend, for the host side. Depends on Footplate 2. Done
when: on a desktop with a StatusNotifier host the tray shows and opens
as on macOS; without one nothing breaks and close quits; and
`.yardr/test` passes.

**Footplate 4 (only if open question 5 goes B). Start at login.**
Goal: a settings switch, off to begin with, that starts footplate at
login through a `LaunchAgent` property list the program writes and
removes itself. Where to look: Footplate 1's setting for the pattern;
src/taskbar.rs `install` for writing a desktop file from the program
itself; `~/Library/LaunchAgents` and `launchctl` on the Mac. Depends on
Footplate 1 and Footplate 2, the second for the tray it starts with.
Done when: switching on writes the list and a fresh login starts
footplate with the tray; switching off removes it; Linux is untouched;
the README says it; and `.yardr/test` passes.
