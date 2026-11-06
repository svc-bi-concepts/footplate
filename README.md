# footplate

footplate is the yard's desktop app, for macOS and Linux, and for Windows
against a yard at an address. It sets a yard up on a machine with no
terminal, starts and stops it, puts the yard's decisions in front of you to
answer, and lets you talk with the crew of this yard and of the yards it is
linked with, on this machine or paired with one elsewhere by a code.

It is an interface over yardr's socket API; the yard runs as it does
(`yardr status`). Where this machine has no yard, the window sets one up: it
fetches yardr and runs its commands, with no terminal. Its head starts and
stops the open yard the same way. Beside those two it starts nothing of
yardr's. The conversations are as `yardr chat` shows them in a terminal.

Optional voice is configured in [Voice settings](docs/window.md#settings),
including provider choice, keys and data flow. Before using ElevenLabs Agent,
read its [setup, billing and privacy guidance](docs/window.md#elevenlabs-agent-template).

## Installing

A release is one download for each of macOS, Linux and Windows, and none of
it needs a terminal. The files are on yardr.dev.
`https://yardr.dev/releases/footplate/latest.json` names the newest version
and each of its files, and a file is at
`https://yardr.dev/releases/footplate/v<version>/<file>`.

No footplate release holds yardr. On macOS and Linux footplate uses the yardr
on the machine, and offers yardr's installer when none is found
([yardr, a dependency](docs/yards.md#yardr-a-dependency)). Windows sets no yard up and
installs no yardr. The release script checks every archive it makes and stops
if a member's name has yardr in it ([Building](docs/building.md#building)).

### macOS

Take `Footplate_<version>_darwin_arm64.dmg` for a Mac with Apple silicon, or
`Footplate_<version>_darwin_amd64.dmg` for an Intel Mac. About This Mac, in
the Apple menu, says which you have.

1. Open the disk image. Its window shows Footplate and a shortcut to
   Applications.
2. Drag Footplate onto Applications.
3. Eject the disk image, then open Footplate from Applications.

The releases on yardr.dev are signed with a Developer ID and notarized by
Apple, so macOS asks once whether to open an app that came from the internet,
and says that Apple checked it. The zip beside each disk image holds the same
app, for anyone who would rather unpack it and drag the app to Applications.
To remove footplate, drag it from Applications to the Trash.

### Linux

Take `footplate_<version>_linux_amd64.tar.gz` for a PC with an Intel or AMD
processor, or `footplate_<version>_linux_arm64.tar.gz` for an ARM machine. It
is built on Debian 13, so it runs on that and on anything newer. It draws
with Vulkan, so the machine needs a Vulkan driver, and a voice session
listens and speaks through ALSA, so it needs ALSA's library, `libasound.so.2`
(Debian's `libasound2t64`); most desktop installs include both. The program
is linked to the library, so without it footplate does not start at all,
voice on or off, and the archive does not carry it.

Every step is in the file manager.

1. Extract the archive. A right click offers Extract Here, and most file
   managers put what comes out in a folder of its own. It holds `footplate`,
   the program, `footplate.desktop`, its entry for the menu of applications,
   and `footplate.png`, its icon, beside README.md, LICENSE and NOTICE.
2. Show hidden files, which is ctrl+H in most file managers, and open the
   folder `.local` in your home folder.
3. Copy `footplate` into `.local/bin`. Make the folder `bin` if it is not
   there.
4. Copy `footplate.desktop` into `.local/share/applications`. Make the
   `share` and `applications` folders if either is missing. Replace the file
   of that name if there is one. footplate writes one when it runs and finds
   none.
5. Open footplate from the menu of applications. Most desktops list it at
   once. One that does not will after you log out and in.

The entry runs `$HOME/.local/bin/footplate`, through `sh`. An entry cannot
name a home folder, and `.local/bin` is on no desktop's PATH until its owner
puts it there, which is why steps 3 and 4 go together.

The first time it runs, footplate puts its icon in
`.local/share/icons/hicolor/512x512/apps`, so the menu shows the icon from the
second start. To have it from the first, copy `footplate.png` there yourself,
making any folder that is missing. footplate leaves the `footplate.desktop`
you copied as it is.

To update, copy the new `footplate` over the old one. To remove it, delete
`footplate` from `.local/bin`, `footplate.desktop` from
`.local/share/applications` and `footplate.png` from
`.local/share/icons/hicolor/512x512/apps`.

### Windows PC

Take `footplate_<version>_windows_amd64.zip` for a PC with an Intel or AMD
processor. Windows on ARM is not built. footplate on Windows has no yard of
its own and sets none up. It opens a yard on another machine, at its address
over TLS, with the code that yard's operator gives you ([Windows](docs/windows.md#windows)
has the rest, a yard in WSL2 included).

1. Extract the zip. A right click offers Extract All. It holds `footplate.exe`,
   the program, beside README.md, LICENSE and NOTICE.
2. Open `footplate.exe`. It needs no installing, so the folder can go where
   you like.
3. The first time, Windows shows "Windows protected your PC". footplate's
   Windows release is not signed, and SmartScreen warns about a program it
   has no signature to check. Choose More info, then Run anyway. Windows asks
   once for that file.

To update, replace `footplate.exe` with the new one. To remove it, delete the
folder you extracted, `%APPDATA%\footplate`, where footplate keeps its
settings and the token of each yard it is paired with, and
`%LOCALAPPDATA%\footplate`, where it keeps its log.

## Which version you run

The About card shows the version you run. On a Mac, open it with "About
footplate" in the application menu. Linux and Windows have no application
menu, so open the settings and, under "This window", press "show" on the
"about footplate" row. [About](docs/window.md#about) has the rest.

From a terminal, `--version` prints `footplate <version>` and exits without
opening the window:

- macOS: `/Applications/Footplate.app/Contents/MacOS/footplate --version`
- Linux: `~/.local/bin/footplate --version`, where the Linux steps put it
- Windows: `.\footplate.exe --version | more`, in the folder that holds it.
  Without `| more` a prompt shows nothing, since `footplate.exe` has no
  console.

## Running and building

Rust, drawn with [gpui](https://gpui.rs/), the UI framework Zed is built on.
[Building](docs/building.md#building) lists what a build needs of the system.

    cargo run                # the window, against the yard on this machine
    FOOTPLATE_YARD=https://mac.example.net cargo run    # against a yard at an address
    cargo build --release    # one binary: target/release/footplate
    cargo test

## How it is developed

Tracked in Linear as YRD-22. Developed in a yardr yard; `.yardr/` holds its
flow, roles and merge gate (`cargo fmt`, `clippy`, `test`, `build`). Every
change is reviewed twice before it lands: against its bead, then for security
alone.

## Read more

The manual is in the `docs/` folder of footplate's source, and not in a
release archive:

- [Yards](docs/yards.md): what footplate talks to (the yard's socket and its
  API, a yard at an address, a hosted yard), the yardr it depends on, setting
  a yard up here, and starting and stopping it.
- [Using the window](docs/window.md): what the window shows, the keys it
  answers to, its settings, About and Feedback, closing it, and
  notifications.
- [footplate on Windows](docs/windows.md): what differs there: only a yard at
  an address (or in WSL2), where it keeps its files, and what is left out for
  now.
- [Building footplate](docs/building.md): what a build needs, how a release
  is made and checked, the screenshots, the icon, and what each file of the
  code holds.
- [Design notes](docs/design/): dated records of why parts of footplate are as
  they are.

## Licence

Source-available under the Functional Source License, FSL-1.1-ALv2
([LICENSE](LICENSE)): use, change and run it for anything except a competing
product or service; each release becomes Apache 2.0 two years after it was
made available.

The licence covers the code. It does not cover the names yardr and footplate
or the project's marks (the FSL's Trademarks clause): a fork takes a name and
a mark of its own.
