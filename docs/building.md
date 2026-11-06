# Building footplate

What a build needs, how a release is made and checked, how the screenshots are
taken, the icon, and what each file of the code holds. Part of footplate's
manual; the [README](../README.md) is the front door.

## Building

Rust, stable (`rustup default stable`), and what gpui needs of the system:

- macOS: the Xcode command line tools. The Metal toolchain, a separate
  download since Xcode 26, is not needed: gpui is built with its
  `runtime_shaders` feature, which compiles the shaders when the window opens.
- Linux: the development packages gpui builds against, which are those Zed
  lists for building from source: X11 and Wayland, xkbcommon, fontconfig and
  a Vulkan loader, and ALSA's, for a voice session's microphone and speaker
  (`libasound2-dev`, found by `pkg-config`); to run, a Vulkan driver (Mesa's
  software one will do) and ALSA's library, `libasound.so.2` (Debian's
  `libasound2t64`), which the program is linked to: without it footplate does
  not start at all, voice on or off, and the archive does not carry it.
  The yard runs on a Mac and so does the gate: the Linux build is tried by
  hand, in a container (Debian 13, arm64) under Xvfb and under a headless
  sway.
- Windows: the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
  and the Visual Studio Build Tools with the C++ workload, which bring the
  linker and the Windows SDK (gpui embeds a manifest with its resource
  compiler). A debug build compiles gpui's shaders when the window opens; a
  release build compiles them with the SDK's `fxc.exe`, which
  `GPUI_FXC_PATH` names if the build does not find it. The jobs in
  `.github/workflows/windows.yml` run `cargo build` and `cargo test` on a
  `windows-latest` runner when a release asks, or by hand, and build the
  release's zip there (below); the gate runs on a Mac and tries no Windows.

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

A build of your own has no installer. The binary is the application, so copy
it where you like and run it. A release is installed as
[Installing](../README.md#installing) says.

On macOS a script wraps it as an application for Finder, with the icon as
an icns:

    cargo build --release && scripts/bundle-mac.sh

That is the whole of it. `target/Footplate.app` runs on a double-click and
can be moved to `/Applications`. It is signed ad hoc, with no identity, so
that its signature carries the bundle's id, which macOS files an
application's [notifications](window.md#notifications) under; it is not notarized,
so it is for the Mac that built it. The script needs `sips` and `iconutil`, which
macOS has, and says so on another system.

With `FOOTPLATE_RELEASE_SIGN_IDENTITY` set to the name of a "Developer ID
Application" identity of the keychain, the script signs the bundle with it
instead, with the hardened runtime and a secure timestamp, and checks the
signature (`codesign --verify --strict`). Unset, the signature is ad hoc as
above, which is what a builder's or a contributor's build gets. The bundle
carries one entitlement, `com.apple.security.device.audio-input`, for the
microphone alone, and both signatures hold it: the hardened runtime holds
back code made at run time, libraries of another maker's, and the camera,
the microphone and the like, and of those footplate uses only the
microphone, for a [voice session](window.md#settings). The Info.plist says why
(`NSMicrophoneUsageDescription`), which is what macOS shows when it asks,
once, the first time a session opens the microphone. The signature alone
leaves the app unnotarized, which the release script does next.

A release is everything the site carries, built by one script from a clean
tree whose version is the tag:

    scripts/release.sh v0.4.0 [out-dir]

It builds both macOS targets, bundles each as above, zips the bundle and
makes a disk image of it, then builds both Linux targets in the shot's
Debian container (arm64 natively, amd64 emulated, which takes a while the
first time) and packs each binary with `footplate.desktop`, its entry for the
menu of applications (`assets/linux`), `footplate.png`, its icon
(`assets/icon-512.png`), README.md, LICENSE and NOTICE, and takes the
Windows zip from GitHub (below), with one `checksums.txt` over the seven
archives (six without the Windows zip: `FOOTPLATE_RELEASE_WINDOWS=off`,
below). The archives go to out-dir, else to `dist/`. The disk image is a
compressed read-only HFS+ volume named Footplate, made with `hdiutil create`.
It holds the bundle beside a link to `/Applications`, so installing is a
drag. The zip stays as it was. The Linux binaries link Debian 13's glibc and
loaders, so they run on distributions as new as that.

Each archive is listed as it will ship, the zips and the tarballs with
`tar -tf` and the disk image mounted read-only with `hdiutil attach`. The
release stops if a member's path has yardr in it, in any case, since
footplate uses the yardr on the machine
([yardr, a dependency](yards.md#yardr-a-dependency)) and carries none. An image that will not
mount cannot be listed, and stops the release too. Nothing reaches out-dir
until all seven pass (six with the switch off). yardr.dev's `scripts/sync-release` checks the members of
a Linux archive and of the Windows zip against lists of its own, so a member
added here is added there before the release is cut.

The Windows zip is not built on the Mac. A release build of gpui compiles its
shaders with `fxc.exe` of the Windows SDK, which a Mac does not have. The
`release` job of `.github/workflows/windows.yml` builds the zip on a
`windows-latest` runner instead, when a release asks, or by hand: the workflow
runs on no push. A builder on a Windows bead starts a run on its branch with
`gh workflow run windows.yml --ref yard/<bead>`. It runs `cargo build
--release`, zips `footplate.exe`, README.md, LICENSE and NOTICE as
`footplate_<version>_windows_amd64.zip`, unpacks that zip and checks that its
`footplate.exe --version` says Cargo.toml's version, and keeps the zip as an
artifact of a successful run for 30 days. Each successful run leaves one, and
they count against the account's storage.

`scripts/release.sh` takes the zip of the run for the commit it releases
(`HEAD`) with `gh`, which must be logged in, starting the run itself when the
commit has none, on its branch (or on `main` from a detached checkout). Before
it builds anything it stops if the run ended in anything but success, a
cancelled run included: run its failed
jobs again with `gh run rerun --failed`, or fix Windows and release again,
which starts a fresh run. After the builds it waits for a run that is still
going, 75 minutes at most, since GitHub stops a run at 60. The zip must hold
`footplate.exe`, README.md, LICENSE and NOTICE at its root and nothing else,
and its `footplate.exe` must be a Windows program for x86-64. As in
`scripts/release-stage`, `YARDR_RELEASE_REPO` names the repository the run is
in (default `yardr-dev/footplate`). The zip is not signed, so Windows asks once
before it opens ([Installing](../README.md#windows-pc)).

With `FOOTPLATE_RELEASE_WINDOWS=off` in the environment the release ships
without the Windows zip: no run is started or asked for, nothing is
fetched, and the zip is left out of the archives and of `checksums.txt`.
Unset or any other value, the release is as above. The release group reads
its environment from its `env_file` (below), which is where the operator
sets it.

The macOS apps are signed ad hoc and not notarized, and the disk images not
signed, unless the release is asked for more, and says so. With both
`FOOTPLATE_RELEASE_SIGN_IDENTITY` (the identity's name, as `security
find-identity -v -p codesigning` lists it) and
`FOOTPLATE_RELEASE_NOTARY_PROFILE` (a `notarytool` keychain profile) set,
each app is signed with that identity as above, sent to Apple's notary
service in a zip (`xcrun notarytool submit --wait`: minutes, as a rule) and,
once Apple has accepted it, stapled with the ticket. The zip that ships is
made from the stapled app, so the app opens on any Mac with no "unidentified
developer" block (a downloaded app still asks once whether to open it, and
says that Apple checked it). The release makes the disk image from that
stapled app, signs it with the same identity, sends it to the notary service
in its turn and staples it, so the image holds a ticket of its own.
`checksums.txt` covers the zip and the image as they ship:

    FOOTPLATE_RELEASE_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID1234)" \
    FOOTPLATE_RELEASE_NOTARY_PROFILE=yardr-notary \
      scripts/release.sh v0.4.0

A submission Apple does not accept, of an app or of an image, fails the
release, which prints the notary service's log of it. One variable without
the other is an error before anything is built, and so are both on a system
that is not macOS (a release is built on macOS in any case). The two are
named after yardr's own (`YARDR_RELEASE_SIGN_IDENTITY`,
`YARDR_RELEASE_NOTARY_PROFILE`) and take the same values, so one setup
serves both repositories: the certificate and its key in the login
keychain, an App Store Connect API key, and `xcrun notarytool
store-credentials`, once per Mac, are in yardr's `docs/install.md` under
"Signed darwin binaries", and are not repeated here. The identity's key is
read from the login keychain: the Mac must be logged in with that keychain
unlocked, and the key must let `codesign` use it without asking.

With the yard a release bead cuts the release instead: `build` makes the
archives and the GitHub release, `sign` asks mckean to sign the checksums,
`verify` checks the signature, and `publish` files the yardr.dev sync bead
(the release flow, `.yardr/flows/type/release.flow`; the script its stages
run is `scripts/release-stage`).

    yardr bead create "footplate v0.5.0" -t release -d footplate -b - < notes.md
    yardr bead advance <id> --to build

The title's last word is the version, which must equal `Cargo.toml`'s; the
body is the release notes as they will stand on the GitHub release. The
yardmaster shapes the bead in `backlog`, then advances it to `build`, and
from there:

- `build` refuses a bad title, a version that is not `Cargo.toml`'s, an
  existing release or tag, a dirty tree, or a checkout that is not the
  remote's `main`; builds the seven archives and `checksums.txt` with
  `scripts/release.sh` (six without the Windows zip when
  `FOOTPLATE_RELEASE_WINDOWS=off` is in the group's environment); tags the
  base commit, pushes the tag, and makes the GitHub release; notes on the
  bead whether the macOS apps and disk images came out signed and notarized
  or ad hoc and unsigned, and the five signing commands.
- `sign` asks mckean whether `checksums.txt.minisig` is uploaded. Run the
  five commands from the build's note, in a directory of their own, then
  Done. Each names the version, so a stale file or a wrong release shows
  itself.
- `verify` downloads the release, checks the signature against the release
  key and the archives against the checksums, then checks each macOS
  archive: each zip is unpacked and its app must be notarized (`xcrun
  stapler validate`, and `spctl -a -vv -t exec` naming `Notarized
  Developer ID`), and each disk image must be stapled and notarized
  itself, mount read-only holding the app and the `Applications` link,
  and hold a notarized app. A bad signature asks `sign` again; an app or
  image that is not notarized, as an ad hoc release's is not, waits in
  `decide` for the release to be remade. Off a Mac the macOS check is
  skipped, and the note says so.
- `publish` files the yardr.dev sync bead
  (`scripts/sync-release footplate <version>`) and advances it to `new`;
  the release bead is `published`.

Anything failed or questioned waits in `decide` for a person.

Once, the yardmaster adds one exec group and four routes; from then on a
release bead cuts the release:

    yardr group add footplate-release --runner exec --limit 1 \
      --set cmd='scripts/release-stage $YARDR_STAGE' \
      --set log="$HOME/.yardr/logs/release-footplate.log"
    yardr route add --stage build --group footplate-release --depot footplate
    yardr route add --stage verify --group footplate-release --depot footplate
    yardr route add --stage publish --group footplate-release --depot footplate
    yardr route add --stage sign --group decisions --depot footplate

The group runs the depot's own script, one release at a time; the stage in
`cmd` says which of `build`, `verify` or `publish` it is. The build refuses
a checkout that is not clean at the remote's `main`, so the script it runs
is `main`'s. `sign` goes to the manual group mckean is in, whose Done says
the signature is uploaded; `backlog` and `decide` ride the depot's existing
routes. The group runs on a Mac, where `scripts/release.sh` builds, and
where `jq`, `git`, `gh` (logged in) and `minisign` are.
`scripts/release-stage` is vendored from yardr's `contrib/release/stage`
(yardr@a3870526) with footplate's release as its default: one checkout
holds the whole release, and no release depends on what a sibling yardr
clone has checked out. A fix upstream is mirrored here, and a fix here
belongs upstream too.

The group runs `scripts/release.sh` as the operator, so for the macOS apps
and disk images to come out signed and notarized it needs the two variables
above in its environment, and the Mac logged in as that operator. An exec group takes
them from an `env_file`, a `0600` file of `KEY=VALUE` lines the store never
holds (yardr's `docs/groups.md`, "Outbound credentials"):

    yardr group set footplate-release env_file=~/.yardr/groups/footplate-release.env

Without them the release is made, with the apps signed ad hoc and the images
unsigned. The build's note says which it was, so it shows before the
checksums are signed.

A builder with a bead to prove takes a picture of the window with
`scripts/shot-linux.sh`, from the worktree, after the last commit:

    shot=$(mktemp -d)/footplate-shot.png
    scripts/shot-linux.sh "$shot"

The picture goes into a directory of its own, made for this shot: every
builder on this Mac shares /tmp, and under one fixed name there a builder
would attach, or remove, another builder's picture.

It draws footplate in a Debian container on this Mac: headless sway,
Mesa's software Vulkan driver, grim, against a throwaway home. Each shot
provisions a fresh container and removes it afterwards, so a shot leaves
nothing behind but the picture; cargo's home lives in a volume every shot
shares, and each checkout keeps its own build tree in another, so a
checkout's first shot builds for minutes and its later ones only rebuild
what changed. One shared build tree would shoot another checkout's
binary: cargo takes a package as fresh from its sources' mtimes.

A look the Linux container cannot draw — on macOS the content reaches
into the title bar and the traffic lights stand over the rail — is shot
on the Mac itself instead, with `scripts/shot-mac.sh`, from the
worktree, after the last commit:

    scripts/shot-mac.sh <state> <light|dark> "$shot"

It opens the real window on this Mac against a fake yard serving the
state, isolated from the session it is run from (its own home and temp,
the canned yardr), takes the picture with `screencapture` and quits it;
the states are the same ones below. `hold` in place of the file leaves
the window up to try by hand (quit it with cmd+q). The script wakes the
display if it sleeps and holds it awake while it runs (`caffeinate -d
-u`): on a Mac that asks for no password, a sleeping display is what
reads as a locked screen, and waking it unlocks. It brings the window to
the front before the picture, so that it is the key window and its
traffic lights are coloured (a window started from a shell is not):
shoot one at a time, since a second shot takes the key window from the
first. Two pitfalls: a session still locked once the display is awake
(one that asks for a password) reads every window as occluded and lets
nothing come forward, so the script stops and says so; and a builder
session cannot post input (no Accessibility permission), so no click or
drag tries — hold mode is for a hand at the Mac.

A state opens the window against a fake yard serving it, instead of the
first screen, or with no yard where the state says none:

    scripts/shot-linux.sh <state> "$shot"

The states are `yardmaster` (the conversation with the yardmaster, with a
few mails), `crew` (chat with the sidebar's groups full: crew working
and stopped, a linked yard's people, this yard's people, and unread
notices), `thread` (a long wrapped thread from alternating senders,
the newest mail with an image), `column` (a long conversation over
three days, grouped by sender, with short links, a correction and a
code block of one line),
`column-hover` (that conversation with the pointer on the code block, a
letter under its head: its time in the gutter, its bar floating),
`column-copy` (that with the pointer on the block itself: its copy
control shows at the block's top right, clear of the bar),
`column-top` (that conversation scrolled to stand a letter's head at
the top of the list, with the pointer on it: its bar of reactions stops
at the list's top, clear of the Screen control in the header),
`older-unread` (the yardmaster's conversation of 120 mails, ninety of
them unread and forty of those older than its newest fifty, opened: the
window reads back to the oldest unread mail, so the yardmaster's row
shows no count while the assembly's, not open, shows its three),
`plain` (an open decision card of a plain ask), `plain-select`
(that card with a selection posed over its context), `lines` (that
card with one of jo's beads changed, so Lines carries its badge), `page` (an
open card whose note carries a page, an image and a file), `mail` (a mail with
a page and an image), `image` (that mail's image open full size over
the window), `run` (an
open card of a run ask: the forged one of the run card's test, whose
question forges the yard's lines), `teammates` (an open card of a
teammate invite), `teammates-list` (the teammates view open on its row,
with a paired and a waiting teammate), `weighty` (a weighty run ask's
card beside a routine one's), `weighty-mapping` (a weighty mapping ask's
card beside a routine one's), `stopped` (the head with a stopped
yard), `far` (a yard at an address, away), `settings` (the settings
on each of their sections: `settings` is this window, and
`settings-appearance`, `settings-yards`, `settings-agents`,
`settings-notifications`, `settings-connection`, `settings-voice` and
`settings-telemetry` the rest, with `settings-telemetry-off` the
telemetry section off, and `settings-voice-list` the voice section on,
with a voice chosen and the list of voices open under its field, and
`settings-voice-microphone` the same with a microphone chosen among a
stand-in's and its list open),
`voice` (the yardmaster's conversation with a voice session standing in
it, against the fake yard and a stand-in for OpenAI's socket, with no
microphone or speaker opened: the composer's button saying Stop with
`listening` beside it, the session's minutes and what they cost at the
head's right, and the caption of what the person has said under the
head, two asks with a pause between them, which it holds together until
a request takes them), `voice-ready` and `voice-draft` (that conversation with voice on
and no session standing: the composer's button saying Voice, and saying
Send with a line typed in the composer),
`settings-yards-rename` (the yards with the far yard's rename form open
in its card), `reconnecting` (a running yard whose
server is then stopped, with a draft in the composer, its Send held),
`refusal` (the first screen's way to a yard at an address, with a
pairing's http address refused under its fields), `undelivered` (the conversation with one of
a linked yard whose mail was refused over the hour), `answered` (the
first screen with a way's fields open after the yard answered, with the
line that esc opens it), `setup` (step 2 of setting up a yard here,
posed: a canned doctor of two providers and their model offers, and the
working styles as a numbered scale), `setup-terminal` (the step that asks
whether the line that puts `~/.local/bin` on the PATH goes in the shell's
startup file, posed on zsh with yardr, herdr and claude in that
directory), `setup-other` (the step that lists what the machine has, posed
with a claude on PATH that is not Claude Code, what it printed, and the
claude fetched to `~/.local/bin` standing behind it), `hosted` (the first screen with
the hosted way's fields open and filled), `signedout` (the away
screen of a hosted yard that is signed out), `away` (the away
screen of a yard at an address that no longer lets the device in,
with a way's fields open), `arrivals` (the
arrivals panel open on the arrivals row, with its lamps, headcodes,
a line with two beads being worked and two waiting for a slot, and quiet
lines), `departures` (the departures board open under the toggle, with its
banner), `arrival` (one
arrival's bead open from the panel), `flow` (yardr's line open on
its flow), `old-lines` (the Lines pane against a yard too old to
serve it, saying so in plain words), `grants` (the grants view open on
its row, with a read, an update and an `--as` grant, and a source's named
read call and update call), `pairing` (the Pair a phone pane open on
its row, with a pairing's QR, code and minutes left), `secret` (an open card of a
secret ask, posed with a value typed and hidden), `secret-shown`
(that card where the yard marks the value no secret: typed and
shown), `secret-refused` (that card with the yard's refusal of
its answer), `switcher` (the switcher's list open over four
yards, one of them away), `switcher-shut` (that list, shut a second after
it drew as a click outside it shuts it: the scripts drive no pointer, so
the state asks the window for what the click asks), `switcher-member`
(that list over an own and a member hosted yard, the member marked with
its operator's handle), `member-pair` (the settings' yards, with a member
yard's pair this device pressed while the operator holds a pairing open:
the edge names its yard user), `member-paired` (the same with none open:
the device paired as the member's yard user), `refs` (the conversation with the
yardmaster, naming an ask and two bead ids: the ask's chip with its
preview under the mail), `decisions` (the decisions list with one
open ask and the 156-item history open: day groups and a folded
look-alike row), `decisions-filtered` (that history with `docs`
in its filter), `keys` (the cheatsheet open over an open card),
`about` (the About card open over the yardmaster's conversation,
naming footplate's version and the yardr's, with its path), `about-missing`
(that card with no yardr on the machine), `feedback` (the Feedback form open over
the yardmaster's conversation, with a message typed, against a capture of
this machine's that stands in for PostHog), `reactions` (the conversation with
the parties' reactions under three mails), `reactions-hover` (that
conversation with the bar of reactions to tap floating over one
mail), `reactions-zoom`
(that bar with the pointer on one emoji), `notice` (the
conversation with the notice a flows view that did not answer
leaves under the mail), `drawer` (the conversation with the
bead its chip names open in the drawer beside it), `delivery`
(the yard row open on a hook delivery whose photo the yard holds,
with a document it did not take under the photo), `screen`
(the yardmaster's screen open over the window, as its Screen
control opens it), `screen-hosted` (that card on a hosted
yard, saying the screen is read on the yard's machine, with no
Screen offered), `screen-refused` (that card on a yard that
refuses the read in many words, which wrap inside it) and `screen-wide`
(that card on a pane of 200 columns, wider than the window, scrolled to
the end of its lines).
With no state the first
screen is shot, as
above. A state is a debug build's alone: the
binary reads it from `FOOTPLATE_SHOT`, and a release build does not read
that variable.

To add one, serve it in `src/shot.rs` (`STATES` and `populate`, or a
canned answer where no yard listens), cover it
in that file's test, add its name to each shot script's list beside
this one (`shot-linux.sh` and `shot-mac.sh` keep the same list),
and shoot it: the picture is the proof.

Attach it to a note that names the short SHA of the commit it was shot from,
passing the PNG path after `--file`, in the same shell command as the
shot, since `$shot` does not outlive it:

    yardr bead note "$YARDR_BEAD" \
      "Screenshot from $(git rev-parse --short HEAD)" \
      --file "$shot" &&
      rm -r "$(dirname "$shot")"

The temporary PNG is removed after attachment; evidence is never left in
/tmp. The yard's /private/tmp was emptied on 13 Oct, cause unknown — the
note's file is the record.

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

  The dock's icon also carries a number, when something waits for you: the
  mail you have not read plus the cards to decide, over every yard footplate
  follows. They are the numbers the switcher says per yard, summed; the
  notices of a yard are not in it. It changes as they do, with the window
  behind another or hidden, and it is gone when nothing waits. A yard that
  is not reached counts what it last had (`src/dock.rs`). There is no
  setting for it, and no count on a Linux taskbar.
- Linux: a desktop shows the icon and the name of the desktop entry whose id
  is the window's (`footplate`). There is no installer to put one there, so
  footplate writes its own, for you alone: `footplate.desktop` under
  `$XDG_DATA_HOME/applications` (`~/.local/share` without the variable) and
  the icon under `icons/hicolor/512x512/apps` beside it. They are written
  when they are missing or differ, so the entry names the binary that ran
  last. It is for the open window and not in the menu of applications
  (`NoDisplay`). The Linux archive carries an entry that does show in the
  menu, which [Installing](../README.md#installing) has you copy to the same place.
  footplate leaves an entry like that as it is, whichever binary ran, and
  makes over only one that hides itself or is no entry at all. A desktop that
  has not read the entry by the time the window opens, on the very first
  start, shows the icon from the next.
  Under X11 the icon is also set on the window itself (`_NET_WM_ICON`), for
  a panel that reads no desktop entries. To be rid of both, delete the two
  files.

## The code

| File | Holds |
|---|---|
| `src/api.rs` | the client: the calls, the stream, their JSON, by the socket or at an address over TCP and TLS; a pairing code redeemed, a member's pairing opened at a hosted edge, a Kratos account signed in at a hosted edge; a file read from a site, and a JSON post to one, that is no yard |
| `src/feedback.rs` | what the Feedback form sends, and where: one survey answer to yardr's PostHog (EU), made as plain functions (the payload, a fresh id for each send, the words of a refusal) and sent by `api::post_json`; the write key and the survey's id are two constants, and with either empty there is no project to offer |
| `src/fetch.rs` | the yardr on this machine, as plain functions over a `Machine`: found in order by whole path, the one an older footplate kept moved, the installer fetched from yardr.dev and run; what `yardr doctor` names for a Mac, fetched and checked by checksum; and the yardr asked its version for the About card |
| `src/setup.rs` | the first run as yardr's commands, with no window: each run with its arguments and its JSON read, the PATH made from doctor's data, the import's questions answered, a login led by what yardr prints |
| `src/yard.rs` | which yard is opened, as plain functions: the environment's one yard or the open one of the settings, each yard's token file and its mode, a hosted yard's session file beside it |
| `src/session.rs` | following a yard on a thread, one for each yard of the settings: who the window is for, the state it starts from, trying again, or not when the device is not let in |
| `src/model.rs` | what the window shows, as plain functions: the rows, the open conversation, unread and what is acknowledged |
| `src/draft.rs` | what is not sent yet, as plain functions: for each yard each conversation's attachments and text, what is refused and why |
| `src/window.rs` | the window: draws the model and the draft, and makes the calls they ask for |
| `src/window/firstrun.rs` | "set up a yard here": the steps of the first run, drawn where the yard will be |
| `src/window/feedback.rs` | the Feedback form over the window: its entry at the rail's foot, its fields, and the send, off the window's thread |
| `src/input.rs` | the line that is typed in |
| `src/select.rs` | a message's text, drawn as its blocks, and the selection the mouse makes in it |
| `src/markdown.rs` | the light Markdown a text is drawn with, as plain functions: fenced and inline code, lists one level deep, headings to the third, bold and italic; the rest stays as written |
| `src/taskbar.rs` | Linux: the desktop entry and the window's icon, for a taskbar |
| `src/shown.rs` | Linux: whether the window shows, for a voice session that does not go on under one that shows nothing: the rule as plain functions, then the X server's word on a connection of its own |
| `src/dock.rs` | macOS: the number on the dock's icon, of what waits |
| `src/titlebar.rs` | the window's title bar: its options for each platform, its measures (the band's height, where the traffic lights end, where a head may begin), and on macOS, where the content reaches up into it, the drag and the double-click on the window's top strip, as plain functions; then AppKit's move of the window, which gpui has no call for |
| `src/tray.rs` | the item in the Mac's menu bar and in a Linux desktop's tray: what it says and what its icon shows as plain functions, and its menu of the yards |
| `src/notify.rs` | the system's notifications: what is told of and that a burst is one, as plain functions; then macOS's notification center and Linux's `notify-send` |
| `src/fake.rs` | a yard for the tests: a unix socket in a temp dir (what a test starts with, except on Windows, which has no socket), or a TCP port of this machine with tokens and a pairing (what a test starts with on Windows), that answers the documented JSON; a stand-in edge in front of it with the login flow, which can turn a path away as the yard's web gateway does, and with member sessions: it opens a member's pairing as the router would and refuses the code in any other session; and a stand-in for PostHog's capture, which keeps each request whole |

The tests run without a yard and without a screen. The yard a test starts
with is the fake's socket, and on Windows, which has none, its TCP port, so
a test that needs no socket runs on both. On Windows the ones that are about
the socket, make a symlink, read a file mode, run the shell stand-in for
yardr below or check what the window offers on a Mac or Linux alone are left
out (`#[cfg(unix)]`), and the rest run in the Windows job. The client is
tried against the
fake yard for each call, and through a stand-in edge for Kratos sign-in,
including wrong passwords and expired flows; an image and the stream, by
its socket and at an address, with a pairing, a wrong code and a removed
device, and a stream stopped while it waits, against a yard that keeps it
open when its client hangs up (the fake ends it by default, as a real yard
does, and Windows needs a stop of the client's own), the model as it is, and
the window itself on gpui's test platform, where an image on its clipboard
is pasted and the draft sent with Enter. gpui gives a test no way to make
the paths of a file drop, so the window's test hands the files to what a
drop calls and drops nothing but an empty one. The fake yard speaks no TLS:
at an address the tests run over plain HTTP on this machine, and one test
that is not run by the gate (`cargo test tls -- --ignored`) makes the
handshake with a site of the internet.

The lookup of yardr is tested on a machine under a temporary directory, with
stand-ins for yardr and the login shell: each step in order with the ones
before it taken away, no network and no write up to the installer, a broken
`<home>/binary`, a shell that fails or hangs. The installer and what doctor
names are fetched from a stand-in for the site, a port of this machine, and
yardr.dev is asked nothing: sent on from one address to another, checked, kept
or run, and refused for each reason. The move of a kept yardr is tested with
and without a login service, and failing at each step. The yardr's version is
asked of a script that stands in for it, which says what each test writes it
to say, or ends badly, or does not end.

The Feedback form is tested against a stand-in for PostHog's capture, a
port of this machine that keeps each request whole and answers with the
status a test gives it: the payload is held to the survey's answer and its
four properties, no token rides on the request, each send has an id of its
own, and a send that fails leaves the form as it was. No test uses the real
key or host.

The first run is tested against a stand-in for yardr, a script that answers
each command with the JSON a test gives it and writes down what it was
called with: the window goes from no yard to the yardmaster's conversation
against it and the fake yard, and no test fetches or runs a real yardr. One
test that the gate does not run puts the same commands to a real yardr, in
a home of its own (`a_real_yardr_takes_the_first_run_s_commands` in
`src/setup.rs` says how).
