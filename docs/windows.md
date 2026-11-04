# footplate on Windows

Where footplate runs on Windows, and where its yard is. Part of footplate's
manual; the [README](../README.md) is the front door.

## Windows

footplate runs on Windows (x86_64) against a yard at an address, and against
no other: a yard of this machine is reached by a unix socket, which Windows
has not, so the yard is on another machine, or in WSL2. The window offers
no "yard of this machine", sets none up ("set up a yard here" is not
there), fetches no yardr, and has no Start or Stop. The entry
`this machine`, which the settings always list, cannot be opened: opening it
says so and offers the other ways, "pair with a yard at an address" and "open a
hosted yard", which [A yard at an address](yards.md#a-yard-at-an-address) and [A
hosted yard](yards.md#a-hosted-yard) describe. The calls, the stream and the
pairing are the same over TCP, with TLS trusted by the roots Windows
trusts.

**Where it keeps things.** The settings are
`%APPDATA%\footplate\settings.json`, and the token of each paired yard is a
file beside them, `yard-<name>.token`, or the whole path a yard's
`token_file` names. footplate sets no mode on them and reads none: a user's
profile folder is private to that user by default (other users cannot read
it; administrators can), which is what mode 0600 stands for on a Mac or on
Linux, where a token file others can read is refused. Keep a token file in
the profile, and not in a folder others share. The line a failed call leaves
([A hosted yard](yards.md#a-hosted-yard)) is kept in `%LOCALAPPDATA%\footplate\footplate.log`,
this machine's own and not the roaming profile the settings are in.

**A yard on another machine.** As [A yard at an address](yards.md#a-yard-at-an-address)
has it: "add a yard at an address", the `https://` address and the code its
operator opens with `yardr user device pair <user> <device> --url
https://...`.

**A yard in WSL2 on the same PC.** yardr runs inside WSL2, and footplate,
as a Windows program, reaches its listener at `http://localhost:<hook
port>`: plain http is accepted for `localhost`, and WSL2 forwards the
localhost ports of its virtual machine to Windows (`localhostForwarding` in
`.wslconfig`, on by default). footplate tries `localhost` at `127.0.0.1`
before `::1`, as Windows lists `::1` first and is slow to refuse a
connection there. The listener is the yard's setting `hook_addr`, port 8790
unless that says otherwise. In WSL2:

    yardr user device pair <user> <device> --url http://localhost:8790

then, in footplate, "add a yard at an address" with that address and the
code it printed.

**Left out of Windows for now.** The system's unlock before a weighty
approval: the card asks for the ask's number typed back, as it does where a
system offers none. System notifications, which the settings say there are
none of; the item in the tray, whose settings are not shown; the number on
the taskbar. The title bar is the system's. An installer, and signing, are
for later. The release's zip is not signed, and [Installing](../README.md#windows-pc) says
what Windows shows for that. A release build is a console-less
`footplate.exe`.
