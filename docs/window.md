# Using the window

What the window shows and the keys it answers to, its settings, About and
Feedback, closing it, and notifications. Part of footplate's manual; the
[README](../README.md) is the front door.

## The window

Left, the rows: above the conversations the open questions addressed to
you, oldest first and grouped by topic where a topic has two asks or more
(the bead's first `topic:` label, without the prefix; asks with none under
"Everything else"), each row the question wrapped to two lines, then whose
it is, what it wants and how long it waits, and the bead's id small at its
end; a bead with several open asks is one row, with each ask beneath its
title in the same shape. Under them, folded, "Decided" with the answered,
declined and put-aside asks and the ones taken back, newest first. Open,
the history stands grouped by the day it was asked, with the outcome's
icon first on every row, and look-alikes folded into one row with their
count; `/` types in its filter, over question, asker and outcome, with the
chips All, Approved, Words, Taken back and Put aside above the days. Last
of them, where the window runs as the yard's operator, the Grants row:
every source × holder × call, by source, with each call's own time and
approver, the weighty words for update and `--as` rows in rust as on a
card, and each row's `yardr source revoke` line to copy. A named call is
weighty by the kind the yard serves for it: a read has no weighty words
and an update has them; a call with no kind served, as every call of a yard
from before it served kinds, counts as an update. It reads through
`SourceGrants` and changes nothing itself; a yard too old for the call
says so in its pane. A hosted yard is not asked: it serves the list to
no request that comes down the link from its edge, so the row and the
pane say the grants are read on the yard's machine
(`yardr source grants`), with no notice. Beside it the Teammates row:
every teammate of the yard's mapping, each with their yard name, email,
role, who invited them and when, and paired with each device's name and
last use, or waiting where none is paired. Each row but your own has its
Remove, which files the removal as a run ask on a new bead and says the
bead and the ask; the card in the queue is where approval happens. It
reads through `Teammates`, and a yard too old for the call says so in
its pane; a hosted yard answers down the link, as for an approval. Last
of them, where the window runs as the yard's operator on the yard's own
machine, the Pair a phone row: the device's name with Open, and while a
pairing lives its QR, its code as text, and the minutes the code is good
for, with End. It is the terminal's `yardr user device pair` in the
window, for the window's own user. On a yard at an address the row is
absent, and no pairing call is made; a yard too old for the calls says so
in its pane. Then
the crew with what each one's session does (`working`,
`idle`, `blocked`, `not logged in`, `running`, `not running`, `failed to start`), then those
the linked yards expose, each with what they are there (a crew member or a
person) and their role line, then the users of this yard, your own row
among them (`you, notes to self`), and on every row the number of mails
from it you have not seen. A name of a linked yard that the operator trusts
as a user of this yard (`yardr peer allow <peer> --trust <name>=<user>`)
has no row of its own: its mail is in that user's row, and what you write
there goes to whoever wrote last, as in `yardr chat`: one mail to one
place, never a copy to the other. What you wrote from a linked yard under
such a name stands in the row of whoever you wrote to, as your own mail under
the name you wrote it by, and answers nobody: what you type in that row goes
to the one it is with. The empty line says where before you type
(`Write to mckean@north`), or `Reading mckean` until all of the row is read,
and Enter sends nothing until then. When the yard refuses it, the window
says so with that address (`not sent to mckean@north: …`), keeps what you
typed, and writes nothing to the user in this yard instead. Below them the `yard`
row with the number of the yard's notices you have not read and of the mail
it shows: the yard's own, and that of a sender who has no row. Right, what is open: a conversation, oldest at
the top and the newest in view, an arrow at its bottom right back to the
newest when scrolled up, with a line under your own mail that says
where it stands (`read`, `sent; yardmaster is idle`, `queued for north:
…`, or the yard's words for a crew member at its harness's login screen: it
cannot read mail, and where the yard leads the login it waits for the
operator at the asks) and the line to write in; the notices, oldest first, with a hook
delivery's files under its notice as a mail's are (its images, a page or file as a row,
a line for one the yard did not take): they come after the notice, and show when the yard
says it has them, with the row left open; or a bead's open
asks, each as its card.

The line's frame ends in one button at its bottom right, beside the
`? all keys` hint: Voice, Send or Stop, by what the line holds and whether
a voice session stands ([Settings](#settings)). With nothing in the line
(spaces are nothing) and no file attached it says Voice, where voice is on
and the conversation is with someone other than yourself: a tap starts a
session. With words or a file it says Send, and a tap sends the draft
exactly as Enter does, with the same refusals and the same hold while the
yard is away, when Send stands faint with its reason under the line. While
a session stands, or is starting, it says Stop, whatever the line holds, so
that a tap never sends when you meant to end: it ends the session, sends
nothing and leaves what you typed in the line, and Enter still sends it.
Where voice is not on offer (voice off in the settings, your own row, or
no target for the open conversation) it is Send alone, faint and without a
tap while the line is empty. A card has no voice to offer either: the button
that answers there is the card's own, faint until there is something to send.

Where the window runs as the yard's operator, a crew member's conversation
carries a terminal icon at the right of its header, and a bead a session
works carries Screen where the window says so (its row of the arrivals, its
details). Either opens what that session's pane shows over the window
(`SessionScreen`), as the terminal draws it, with Refresh, and `esc` to
leave it. A pane taller than the card opens at its bottom, where the agent's
input line and its latest output are, and Refresh reads it again and returns
there. A pane wider than the card (the card is the window's width at most)
opens at its left edge and scrolls sideways, with the wheel or a swipe, to
the end of its longest line; Refresh leaves you where you are sideways. A
screen can show what an agent printed, a secret among it, so the
yard reads it to no request that comes down the link from its edge: a hosted
yard is not asked and offers no Screen, since the screen is read on the
yard's machine. A yard reached any other way that refuses the read as such
has the card say the same, in no red, and is asked no more; its other
refusals are shown as they come, in red, wrapped to the card's width and
scrolled where the window is too short for them.

A conversation opens on its newest fifty mails. Towards its top the fifty
before are read and put in above, and the mail in view stays where it is;
the top says `Loading older mail…` until the conversation starts there.
Where unread mail is older than those fifty, opening reads the pages back
to the oldest of it and no further, so that the count on the row does not
stay for mail the reader cannot see from the newest. Nothing is marked read
until it is shown, and a conversation that is not open keeps its count. A
yard built before `Conversation` had pages answers the whole conversation
at once.

Above the cards the head names the yard, the bead and the topic once. A
card shows, in this order: the question, the largest text, with how long it
waits beside it; the asker; the context, with the bead whole one disclosure
away and, on the disclosure's row, a faint line with the bead's id and the
ask's number (`bead-a · ask 3`), the two that `yardr bead answer` takes, so a
decision named by its number in mail, chat or a terminal is found on its
card; the line is text to select and copy, and drops under the disclosure on
a card too narrow for both; the options numbered, each with its consequence,
`you decide` last;
a line for words; the Answer button with ask-about-this, Later and Decline
beside it. The recommended option stands preselected, marked with its why
under its own row, and the Answer button names it (`Answer: 1 · Move them
now ↵`); Enter answers what the button says. A number key or the arrows
choose another option instead. Words with no option chosen and none
recommended go as a plain note, and the ask stays open; a file dropped on
the card is attached to the note, and goes with the words. An ask with no
options says "words wanted", and Enter sends the words as the answer. `x`
puts the ask aside, to come back as the yard says. `d` declines it with
words, which is not a pick and not "you decide". `e` opens the disclosure
to the bead's body, notes and about-beads; a design bead's document is named
by path and branch, not fetched. After a pick the ask moves to "Decided" at
once, and a later pick on it from there replaces the earlier one. The window
holds the card's keys, `i` types the words in, `esc` holds the keys again.
What is typed stays with its ask, as a draft stays with its conversation.

A mapping ask (who an unmapped sender is) is weighty: its card shows the
yard's line first, in rust, and each pick passes the Mac's unlock first —
Touch ID, a paired watch or the login password — with `weighed` sent where
the person passes it; its cancel sends nothing and keeps the words for
another try. Where no system unlock is offered, the card asks for the ask's
number in a separate field and picks only when it matches. Words alone,
later and decline stay ungated.

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
and the file and never the value. Where the ask was for a person's own
address in a source that tells them (`--for-user`), they end with the
yard's line of what is left to do: the command that tells that person
through the source. When the call ends with no answer it
says "the yard did not answer. The value may be written. Give it again.",
never that it was sent. A yard that does not serve the call to this window
is said so, with the command that answers in a terminal on the yard's
machine: `yardr bead answer <bead> <ask> --secret`. Every secret ask is
weighty: its card shows the yard's lines for what giving does first, in
rust. Giving passes the system unlock first: on a Mac, Touch ID, a paired
watch or the login password; on Linux, polkit as described under run asks.
Enter opens it, and its cancel sends nothing and keeps what was typed for
another try. Where no system unlock is offered, the
card asks for its number in a separate field before the value: `i` types
the number first, then the value. The value goes only when the number
matches; otherwise it stays in its field and nothing is sent. One that
was given can be given again from its card under "Decided", which
replaces it.

**A run ask** (`yardr bead ask --run`) proposes one yardr command that is
the operator's to give, and approving it has the yard run that command. Its
card has no options and no line for words. The asker's context lines each
begin with who says them (`yardmaster says: …`) and are drawn as the text
they are, with no block in them. Under them is the one block of the card,
and all of it is the yard's: the command whole, in a code block that wraps
where the window is narrower and is never cut; what the command does, in
one line from its help; the yard's sentences for what the command changes
and what is there now, each on its own line — for a source or a pack every
call, command and object, never a summary; and the command that takes the
change back, with who may run it. An ask that adds a source or applies a
pack holds its texts behind one tap in the block: each declaration,
command file, file change and removed entry, and each depot file's change
and new text, whole as carried, each in a block that copies it to the
letter. The tap is for checking the yard's reading against them; the
approval is live whether or not it was opened. Every card's header leads
with the name of the open yard, before the bead and how long it waits, and
a run ask's block leads with it too, with the user the command runs as: "in
plantime, when you approve, the yard runs this as mckean, and nothing else:".
A teammate invite or removal carries its own head above that lead: the
address, the yard user and the role the command was given — an operator
named as one that can approve and administer the yard — with the note that
approval runs the command as shown, so a correction goes by declining with
words that name the corrected invite.
Directly under the block is the Approve button, which names the yard and
its key, "Approve in plantime (⌘↵)": `cmd+enter` (`ctrl+enter` on Linux),
which no other card answers to. Enter alone approves nothing. Approving
calls `ApproveAsk`, which names the ask and nothing of the command: the
yard runs what it stored. Afterwards the window says `approved, ran:` with
the command and what it printed, `approved, being carried out:` where the
yard carries the approval out after the call, or the yard's refusal whole,
as when the yard has changed since the ask was written. When the call ends
with no answer it says "the yard did not answer; read the card again",
never that it was approved. A yard that does not have the call, or does not
serve it to this window, has the card say so in place of the button, with
the command that approves in a terminal on the yard's machine in a block
that copies it: `yardr bead answer <bead> <ask> --approve`. A run ask the
yard weighed weighty shows each class's line first on its card, in rust:
`WEIGHTY · trust: …`. On a Mac its approval passes Touch ID, a paired
watch or the login password first, and the control says so:
"Approve in plantime after unlocking". ⌘↵ alone approves nothing: it opens
the unlock, and the approval goes where the person passes it; its cancel
sends nothing. On Linux the unlock is polkit's: the desktop's own
prompt, for the person's own password or fingerprint, each time. It is
offered where polkit runs with an agent, as a desktop session has one, and
footplate's action is installed, which a root copy does:
`sudo cp assets/linux/dev.yardr.footplate.policy /usr/share/polkit-1/actions/`.
The prompt says the action's own words, not the command: polkit takes
none from a program that is not root's, so the card is what to read.
Where no system unlock is offered, the card asks for the
ask's number in a separate field and approves only when it matches;
anything else leaves the card and sends nothing. The number is used up
after each approval. `x` puts a run
ask aside and `d` declines it with words, as on any card; one put aside can
be approved from its card under "Decided". An approved one reads `approved,
ran:` there, with the command and, under it, what it printed; one the yard
has not carried out yet reads `approved, being carried out:`, and one whose
carrying out failed reads `approved, failed:` with the failure under it.

A setup ask proposes a source pack to set up, with the account and the
grants that go with it: its card has the yard's block with what approving
does, in one line, and the yard's sentences for what it changes, each on
its own line — the pack, each source with where its credential goes and
each call with its method and path, the mint with its digest, the
account, and the grants with their grantee. Under the block is the
account choice: each candidate the yard lists with the account its file
holds, to take with a tap, or the secret field to type another in. The
presets stand to pick under that, each with what it grants, and the
take-backs close the block; the ask's own texts stand behind one tap, for
checking the yard's reading against them. ⌘↵ (`ctrl+enter` on Linux) or
the Approve button approves the preset pick with the account together;
Enter alone approves nothing. Afterwards the window says `approved, being
carried out:` with the pack, the preset and who the grants are for, since
the server carries the setup out after the call; among the decided it
reads `approved, ran:` once the yard has carried it out, or `approved,
failed:` where that failed, with what it printed under it. An approval
the yard refuses is shown whole on the card, as when the yard has changed
since the ask was written, and a call that ends with no answer is never
said to be approved or not: the queue is read again for what the yard
says. Where the yard takes no approval from this window, the card says so
in place of the button, with the command that approves in a terminal in a
block that copies it. Every setup ask is weighty: its card shows the
yard's lines for what approving does first, in rust, and its approval
passes the system unlock first, as a run ask's does. Where no system
unlock is offered, the card asks for its number before the account in a
separate field, and approves only when the number matches. `x` puts a setup ask
aside and `d` declines it with words; one put aside can be approved from
its card under "Decided".

| Key | Does |
|---|---|
| `enter` | sends the draft, the line and the files attached, as one mail to the one whose conversation is open (a long text as several: "A long text"), as the Send button does; on a card, the pick with the line's words |
| `shift+enter` | breaks the line where the cursor is; a text wider than the box goes on in the next row, and the box grows with its rows up to a third of the window, then scrolls |
| `up`, `down` in a text of several rows | the row above, below, in the same column; `home` and `end` are the row's |
| `cmd+v` (`ctrl+v` on Linux) | pastes text into the line, with its line breaks; a file is attached to the draft |
| a drag over a message's text, a double click on a word | selects it; one selection at a time, within one message |
| `cmd+c` (`ctrl+c` on Linux) | copies what is selected, as it was written: of a message, with the Markdown marks that are not drawn, or of the line while you type in it |
| a file dropped on the window | is added to the draft; a zip drops to its files, a folder to the files one level deep |
| a drag that starts in the window's top 32 pixels, a double-click there (macOS) | moves the window; the double-click zooms it, minimises it or does nothing, as the system's setting says of a title bar. On macOS the content reaches the top of the window and no title bar is drawn above it: the top 32 pixels are one band across the rail and the list's head, the traffic lights over it, with a hairline along its foot from the window's left edge, the rail's own line starting under it, and the list's status row centred on the lights and clear of the last; the settings' sheet stands under an empty band; on Linux the desktop draws the title bar, and these are its own |
| `backspace` in an empty line | removes the last attachment |
| `tab`, `shift+tab` | the next, the previous row |
| a click on a row | opens it |
| a click on the arrow at a scrolled-up conversation's bottom right, `cmd+end` (`ctrl+end` on Linux) | jumps to the newest message; the arrow carries how many mails arrived below |
| `pageup`, `pagedown` | scroll the open conversation, card or yard row by a page, with the focus anywhere |
| `cmd+home` (`ctrl+home` on Linux) | the oldest loaded message; the newest is `cmd+end`, on a card or the yard row too |
| a click on "Decided" | folds it open, shut |
| a click on a folded history row | opens it to show its rows, shut |
| `/`, in decisions | opens the history with its filter typed in; `esc` leaves the filter, and what is typed stands |
| `1`–`9`, `up`, `down`, on a card | chooses an option; the arrows choose even while words are typed, from the first row of the words up and from the last down |
| `i`, on a card | on a weighty card without a system unlock, types the ask's number first, then a secret, a setup account or words; otherwise types words or the secret or account in its field. The window holds the card's keys |
| `cmd+enter` (`ctrl+enter` on Linux), on a run or setup ask's card | approves it: the yard runs the command the card shows, or applies the setup with the preset pick and the account. On a weighty card it opens the offered system unlock first; where none is offered, the typed number must match the ask. No other card answers to it, and Enter alone approves nothing |
| `x`, on the open `yard` row | marks one notice read; a notice never needs more than one key |
| shift-`x`, on the open `yard` row | marks every notice read, as the button in the row's head does, shown with their count while one is unread |
| `x`, on an open card | puts the ask aside |
| `d`, on an open card | declines the ask with words; with an empty line the next Enter sends them. A secret ask is declined at once, with none |
| `e`, on a card | expands it to the bead whole, or folds it again |
| `esc`, on a card | leaves a decline asked for words, shuts the bead drawer where one is open, the words for the card's keys, the pick, then the words; in the field of a secret, the field, and what was typed is gone; in the number field, leaves it and keeps the number |
| a click on a bead's chip in a mail | opens the bead in the drawer beside what is open, which stays as it was; the drawer's Open shows the bead whole, and `esc` shuts the drawer |
| `?`, on a card, the yard row, Lines or the switcher; a click on "? all keys" | opens the cheatsheet of the open view's keys; `esc` closes it |
| a click on an option, on "decide this one" | chooses it; makes the ask current |
| `cmd+[` · `cmd+]` (`ctrl` on Linux) | the modes back, forth |
| `cmd+y` (`ctrl+y` on Linux), a click on the line under your name | lists the yards under the head: the arrows move, Enter opens the yard they are on, `esc` or a click outside the list leaves |
| `cmd+1`–`cmd+9` (`ctrl` on Linux) | opens the yard at that place in the list, from anywhere |
| `cmd+,` (`ctrl+,` on Linux), a click on "settings" | shows the settings where the conversation is, and what was open again; so does `esc` there |
| a click on "Feedback", at the rail's foot above "settings", where it is offered | opens [the Feedback form](#feedback) over the window; Enter sends it, `esc` or a click outside shuts it |
| `cmd+=`, `cmd+-`, `cmd+0` (`ctrl` on Linux) | the text a pixel larger, smaller, the usual size |
| `cmd+shift+l` (`ctrl+shift+l` on Linux) | the next of auto, light, dark |
| `cmd+q` (`ctrl+q` on Linux) | quits. On Linux so does closing the window, with the tray's item on or off; on macOS closing hides footplate while the menu bar item is on, and quits while it is off ([Closing the window](#closing-the-window)) |

## Settings

`cmd+,` opens the settings as their own sheet, with the sections down the
left side and the file they are kept in as a footer, with what opens its
folder. A change is drawn at once and kept for the next start:

- This window: the text size, 11 to 20 pixels, a pixel a step; 14 is the
  usual size. Everything that goes with the text goes with it: the left
  column, the rows, the logo. And the menu bar item, on Linux the tray's,
  which [Closing the window](#closing-the-window) has, with its mark.
  And "about footplate", which opens [the About card](#about).
- Appearance: light, dark or auto. Auto is the desktop's look, and follows
  it when it changes while the window is open.
- Notifications: what the system tells of, which
  [Notifications](#notifications) has.
- Yards: the yards the window keeps, each as a card with its name, its
  kind, how it is reached, whether it is open and what waits in it, with
  Open and a … menu holding pair again, rename and remove; which
  [A yard at an address](yards.md#a-yard-at-an-address) has. The open yard's card
  carries start at login where it is a yard of this machine.
- Agents & you: how this yard's agents work with you, and the line they
  know about you. On a yard at an address they are the operator's there,
  read-only.
- Connection: the web view's base address, through which a note's pages
  open in the browser, and the run base, through which they run. Each
  names the address in effect, the default where nothing is set, with a
  reset once it is overridden.
- Voice: talking with the yard by voice, off until it is switched on,
  which is the opt-in. Choose OpenAI GPT-Live or ElevenLabs Agent. Existing
  settings keep OpenAI. ElevenLabs uses an existing authenticated Agent,
  configured manually with the template below; an explicit start checks that
  configuration and obtains a fresh signed WebSocket URL.
  Both providers use the same masked key field, cleared on provider switch.
  Saving a key works while off and makes no validation request. Only an
  explicit session start can read a credential, and only the selected
  provider's. Off makes no provider requests, including voice or agent lists.
  Switching provider, saving a key, changing the saved agent configuration,
  or changing yard/conversation stops the session; another needs a new tap.
  The OpenAI key is your own project's, typed
  into its masked field and kept in `voice.key` beside the settings,
  for you alone, never in the settings file; where
  `FOOTPLATE_OPENAI_KEY` is set it is read first and the file is not
  read. A GPT-Live session sends microphone audio, what you say, the yard's
  name, the counterpart's name and their conversation replies to OpenAI.
  ElevenLabs instead uses `elevenlabs.key` and
  `FOOTPLATE_ELEVENLABS_KEY`; the files use distinct markers and the same
  keeper. On macOS and Linux it writes mode 0600 and refuses files readable
  by others; on Windows it relies on the profile directory's permissions.
  The ElevenLabs key authenticates with ElevenLabs; it never enters settings JSON, yard mail or
  telemetry. The non-secret `elevenlabs_agent` and `elevenlabs_voice`
  IDs live in settings JSON, separately from OpenAI's `voice_name`. Empty voice
  ID leaves the Agent's configured voice in place. Enter or the save button
  keeps the two IDs. Configure an already existing workspace agent manually
  with the [Agent template](#elevenlabs-agent-template); Settings never creates,
  modifies or enumerates hosted agents. Active audio, transcripts, tool inputs,
  open-conversation context and replies reach ElevenLabs; text also reaches the Agent's LLM provider. The Agent
  may paraphrase and written yard mail remains the record. Footplate adds no
  saved audio or transcript history. Workspace retention and model-improvement
  settings are unverified: check them before live use rather than assuming
  account opt-outs or zero retention. The section's disclosure describes the
  selected provider's data flow and retention. With OpenAI selected, the `voice` picker
  chooses which of OpenAI's voices speaks: a field with a chevron, which
  opens the list of the thirty-one the Live reference gives, with a note of
  language and accent on each OpenAI's guide describes, and `default` first,
  which leaves the choice to OpenAI (marin at present) and keeps nothing in
  the settings file. OpenAI fixes a session's voice when it starts, so a pick
  is heard from the next session, and one that stands is left as it was; a
  voice OpenAI refuses, from a settings file edited by hand, is refused in
  its own words as a refused key is. The `microphone` picker under it
  chooses which microphone a session hears through, in the same manner:
  `System default` first, which is what a session uses where nothing is
  chosen and keeps nothing in the settings file, then each microphone the
  machine lists, by what the system calls it with its id quiet beside it,
  the name and the id each ended by an ellipsis where it is too long for
  the row. The machine is asked when the section shows and each time the
  list opens, so a headset plugged in since is there. The pick is kept by
  that id, and the devices open as a session starts, so it is used from the
  next session and one that stands is left as it was. A microphone that is
  not plugged in when a session starts is not used: the system's default is,
  and the field says `not found` beside its id until it is back. With voice
  on, the composer of an
  open conversation has one button at its bottom right: Voice while the
  line is empty, and a tap starts the selected provider with whom you are talking;
  Send once the line holds words or a file; and Stop while a session
  stands, whatever the line holds, which a second tap ends without
  sending what you typed (it stays in the line, and Enter sends it).
  With ElevenLabs selected, the yard connected and no conversation open,
  Voice in the rail opens yardmaster-chris and starts a session there. If that
  conversation is absent, a notice asks you to open a conversation instead.
  While a session stands, quiet words beside the button say what is going
  on (`starting…`, `listening`, `yardmaster is working`, `speaking`), a
  line under the head shows the latest ElevenLabs transcript, or with GPT-Live,
  what you have said that no request has taken yet (a pause does not clear it;
  a long one shows its last words), and
  the head's right counts connected time and a call-only estimate (ElevenLabs), or the minutes and
  what they have cost at OpenAI's
  rate, $0.05 a minute. When the voice sends something to the yard, it goes
  as your own mail in that conversation, as Enter would send it. With GPT-Live,
  it sends once you have stopped talking, with the requests and replies it has
  held since its previous send, in the order you said them. Its awaited answers are spoken
  in the voice's own words. ElevenLabs instead sends through its client tool;
  see the [Agent template](#elevenlabs-agent-template) for pending results and
  contextual mail updates. With either provider, while the yard is away a
  send is refused, once, and nothing is queued or sent when the yard returns.
  Written mail stays the record. A decision that comes up in the window is
  never answered by voice; GPT-Live says it waits there. A session ends
  when you tap Stop, open another conversation or yard, switch voice off,
  hide, minimize or close the window, or when nobody has spoken and
  nothing waits for the yard for the minutes the section's `ends after`
  line says, five until you choose another. On Linux, under X11 the window
  manager says when the window is minimized. A Wayland compositor says
  nothing of it, but sends no frames to a window it shows nowhere, so there
  the session ends when two seconds pass with none, and that is so while
  other windows cover this one wholly or it stands on a workspace out of
  view as well: the microphone is off wherever nothing shows, and a talk cut
  short is one tap to start again. The microphone is open only
  while a session stands: no session, no microphone. On a Mac, macOS asks
  once, the first time a session opens it. Where the system refuses the
  microphone (on a Mac at that prompt, or later in System Settings, where
  macOS would give silence and no error), the section and the head say so
  in words, and where to change it (on a Mac, System Settings, Privacy &
  Security, Microphone), and no session stands. On Linux, footplate needs
  ALSA's `libasound.so.2` on the machine ([Installing](../README.md#linux)).
- Telemetry: the yard's opt-in telemetry switch, for the operator's
  window alone on a yard that serves the calls. Off, the consent words
  and Turn on; on, the install id with a copy of it, the next payload
  as the yard returns it, and Turn off. Where the yard takes no change
  from where the window reaches it, the section stands read-only, with
  the yard's web view to change the switch in.

They are kept in one file footplate writes on every change:
`$XDG_CONFIG_HOME/footplate/settings.json` when
`XDG_CONFIG_HOME` is set, `~/Library/Application Support/footplate/settings.json`
on macOS otherwise, and `~/.config/footplate/settings.json` on Linux
otherwise; on Windows `%APPDATA%\footplate\settings.json`, and nothing
else. A relative `XDG_CONFIG_HOME` is ignored. Without the file, or
with one footplate cannot read, the window is as it was before there were
settings: 14 pixels, auto.

## About

The About card says which footplate and which yardr you run. It stands over
the window, and esc, its close button or a click outside it shuts it. Two
doors open it: "About footplate" in the application's menu on a Mac, and
the "about footplate" row under "This window" in the
[settings](#settings), which is the door on Linux and on Windows, where
footplate has no application menu.

- footplate: the release this is, as its build names it (`v0.8.0`).
- yardr: what the yardr on this machine says to `yardr --version`, asked
  when the card opens (`v0.14.0 (cbceebd22919)`: the release, and the
  commit it was built from), and under it its path, found as
  [yardr, a dependency](yards.md#yardr-a-dependency) has it. With none, the card says
  "not installed"; a yardr that will not say gives the reason. The card moves
  and installs nothing. On Windows there is none, and the card names footplate
  alone.

The card looks for nothing newer, of either, and offers no update.

## Feedback

The Feedback button stands at the foot of the rail, above the settings'
gear, and opens a form over the window: a message, an email if you would
like an answer, and what it sends, said plainly under them: "Sends your
message, the footplate version and this system (OS), and your email if you
add it, to yardr's PostHog in the EU. Nothing else is sent." Enter in either
field, or Send, sends it; Tab moves between the fields, and esc, Cancel or a
click outside the form shuts it, and what was typed goes with it. A message
is required, and the email is not.

A message goes as one answer to yardr's PostHog survey "footplate feedback":
a `survey sent` event, one POST to `https://eu.i.posthog.com/i/v0/e/` with
the project's write key (a `phc_` key, public, which only writes) and
nothing of a yard's: no token, no session, no device. Its properties are the
survey's id (`$survey_id`), the message (`$survey_response`, the form of a
one-question survey), the footplate version (`footplate_version`), the
system (`os`: `macos`, `linux` or `windows`) and your email (`email`) where
you gave one. Its sender is an id made for that one message and kept
nowhere, so that nothing ties two messages of one person together. The yard
hears the answers through the posthog pack's feedback destination, which its
setup has. (The shape is PostHog's, read 28 Oct 2026 in its Capture API,
"Survey", and "Implementing custom surveys".)

It goes once, when you press Send, off the window's thread and with the
usual wait for an answer: nothing is queued, retried or kept, in the
settings or on disk. A send PostHog does not take leaves the form as it was,
with what you wrote in it, and says it could not be sent, why, and to try
again; one it takes closes the fields to say thanks, with Close.

The key and the survey's id are two constants in `src/feedback.rs`. While
either is empty the button is not shown, so that no build offers a form that
sends nowhere, and there is no override at build time: a fork or a build of
the source sends to yardr's project too, on purpose, as the place feedback
goes. The phones are not this footplate.

## Closing the window

On macOS, closing the window hides footplate and does not quit it: it goes
on following every yard, keeps what you typed and did not send, counts
what waits on the dock's icon and notifies as the settings say. Its item
in the menu bar, a click on its icon in the dock, `cmd+tab` to it, or
opening `Footplate.app` again shows the window as it was. `cmd+q` and "Quit
footplate", in the application's menu or the item's, quit, and nothing else
does.

This goes with the menu bar item, a setting that is on to begin with.
Switch it off in the settings (`cmd+,`), under "menu bar or tray item", and
the item is gone and closing the window quits. On Linux closing the window
quits whatever the setting says: there it switches
[the item in the tray](#the-tray-item-on-linux) alone.

### The menu bar item

On macOS footplate has an item in the menu bar: its mark, which is always
there, and beside it the number of things that wait for you, while any do.
It is the number on the dock's icon (mail you have not read plus the cards
to decide, over every yard footplate follows), and changes as that does. A
click opens a menu:

```text
mac · 1 mail
plantime · 1 mail, 1 decision
─────────────
Show window
Quit footplate
```

There is a row for each yard of the settings, in their order, with what the
switcher says waits in it, or that it is not reached and why. A yard's row
shows the window on that yard, as the switcher opens it: on the row you
left it on, or the yard's own where that is gone. The menu opens yards, and
never a card or a conversation by name; nothing is answered from it. "Show
window" shows the window on whatever is open, also when closing hid it.

Unlike the notifications the item needs no bundle: it is there from a bare
binary (`cargo run`) too (`src/tray.rs`).

### The tray item on Linux

On Linux footplate has an item in the desktop's tray, with the same menu,
and it follows the same setting ("menu bar or tray item", on to begin
with). It needs a tray that takes StatusNotifier items, over the session's
D-Bus: waybar's `tray` module under sway, KDE's panel, GNOME with the
AppIndicator extension. Where Telegram or Discord show an icon, footplate
does. Without such a tray, or without a session bus, there is no item and
nothing else changes; a bar that starts later, or again, gets the item
then.

A tray draws the icon and no title, so the count is on the icon:

- nothing waits: footplate's mark alone;
- something waits: the mark with the number on it, white on a red badge
  in its lower right corner. The number is the one of the Mac's item:
  mail you have not read plus the cards to decide, over every yard;
- a dot in place of the number where it would not be read: on an icon of
  16 pixels, three digits on one under 32, and four digits or more.

The icon is offered at 16, 22, 24, 32, 48 and 64 pixels, each with its own
sign, and the tray takes the one it draws. The tooltip says it in words:
"3 wait: 2 mail, 1 decision", or "nothing waiting".

A click on the item shows the window, as "Show window" in its menu does; a
yard's row shows it on that yard. Whether the window then comes to the
front is the compositor's to say: footplate asks, and under Wayland a
compositor may answer by marking the window urgent and leaving the focus
where it is (sway's `focus_on_window_activation` says which). Closing the
window still quits on Linux: the item is there while the window is.

It is spoken by the `ksni` crate, in Rust alone, over the D-Bus library
gpui's tree already has: nothing of the system's is built against
(`src/tray.rs`).

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
- With several yards in the settings it names the yard too, first
  ("plantime · yardmaster asks"), of the open yard as of any other: what
  arrives in a yard that is not open is told of by the same kinds. A burst
  from several yards says each yard's part by itself ("3 new in 2 yards",
  "mac: 1 from assembly; plantime: 1 from assembly, 1 decision").
- Nothing while the window is in front and shows the row it belongs to:
  you read it there. In front on another conversation or another yard,
  behind another window, or with the settings open, you are told.
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
  ([Building](building.md#building): `scripts/bundle-mac.sh`); from `cargo run` the
  settings say that instead and nothing is shown. Clicking a notification
  brings the window forward on that conversation, card or the yard row,
  in the yard it told of: where another yard is open, that one is opened.
  One of a yard that is no longer in the settings brings the window
  forward and opens nothing: the row of that name in the open yard is
  another.
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

Not in this slice, and in `yardr chat`: a crew member's pane. Nor here:
saving an image to disk, or resizing one that is over the cap.

## ElevenLabs Agent template

Apply this manually in your own workspace before starting an Agent session.
Footplate reads this agent's configuration only on explicit start; it creates
or modifies no account agent. Live language quality and workspace settings
still need commissioning; tests use a fake provider, not a model.

- Require authentication. Choose a German/English-capable voice; enable German
  and English language switching and interruptions. Try Swiss German during
  commissioning; it is not required.
- Define one client tool, `send_to_yard`, with required string `text` and
  **Wait for response** enabled and no other parameters. The desktop implements
  this tool; use no webhook, workflow, other action tools or imported yard history.
- Permit the system prompt override (`agent.prompt.prompt` in the conversation
  initiation overrides), and the `tts.voice_id` override only if a desktop
  voice ID is selected. Footplate supplies its prompt bound to the
  conversation and yard chosen at start (see [Voice settings](#settings)).
  No full history, attachments or secret ask contents are uploaded.
- Before commissioning, check and record retention, audio saving and training
  choices. Recommend audio saving off, zero-day scheduled deletion where
  available, and training opt-out; these are recommendations, not configured
  account settings or Enterprise Zero Retention Mode (ZRM).

ElevenLabs' [documented default retention](https://elevenlabs.io/docs/eleven-agents/customization/privacy/retention)
is two years, with separate transcript and audio controls that can be changed.
Training opt-out affects future submissions and is separate from deletion.
[ZRM](https://elevenlabs.io/docs/eleven-api/resources/zero-retention-mode) and
[EU residency](https://elevenlabs.io/docs/overview/administration/data-residency)
need eligible Enterprise access and configuration. EU storage alone does not
ensure EU-only processing. Footplate verifies none of these account choices and
adds no audio archive or duplicate transcript store; written mail follows the
yard's existing storage rules.

The meter shows connected minutes and seconds, a **call-only estimate** at
$0.08/min before discounts, excluding burst pricing, and **LLM extra**. Five
minutes gives $0.40 for calls at that reference rate. It is no balance or exact
bill: plan entitlement, included minutes and LLM tokens are unknown. Published
[call billing](https://elevenlabs.io/docs/help-center/product/eleven-agents/how-much-does-eleven-agents-cost)
discounts silence longer than ten seconds by 95%; pending work can keep an idle
session connected until its finite wait ends. Context grows with conversation
length and LLM charges pass through separately. The ADR's starting assumption
is Gemini 2.5 Flash-Lite ($0.10/M input and $0.40/M output tokens; regional
pricing may differ); model availability and actual rates need commissioning.
Check the workspace's Agents entitlement rather than assuming a subscription
includes it. Above quota requires purchased PAYG credits or legacy usage
billing. Footplate never buys credits, enables overage, selects a costlier model
or switches to GPT-Live automatically.

Missing or unsafe keys are refused locally. Authentication refusal, forbidden
or unavailable agent/model, expired signed address, concurrency/rate limit,
quota exhaustion and network loss end voice with local redacted words. Unknown
provider prose is never displayed. There is no reconnect, signed-URL refresh
loop or mail replay. Starting again is explicit; written mail remains readable,
including late replies after a session ends.

The session prompt is supplied by `hello` in
[`src/voice/elevenlabs.rs`](../src/voice/elevenlabs.rs); no prompt template or
session variables need to be copied into the hosted Agent.

Each `send_to_yard(text)` call submits that text as ordinary written mail to
the captured conversation, using the same sender as the composer. It is not
labelled a verbatim microphone transcript. Destination fields from the tool
are refused; approvals and secrets remain in their cards.

This iteration returns **pending** after the mail call succeeds. The yard's
current mail contract has no explicit reply relation, so no arriving mail can
be proved to answer that tool call. New selected-counterpart mail is sent as
`contextual_update`, labelled with its source and mail sequence ID, never as a
`user_message` or a successful tool reply. Context can inform the next turn;
it does not promise to interrupt speech. Duplicated and older events are
ignored. No unrelated history or other conversation is included.

Only one send may be outstanding. Repeated tool-call IDs reuse their original
result without another send; overlapping calls return busy, including while an
uncertain write is still in flight after its tool wait expires. A lost or refused
send returns an explicit unconfirmed-delivery result, with no automatic retry.
The local wait ends after 20 seconds, independently of the Agent's response
timeout, releasing the idle exception even if the mail callback is late.
A late written reply still appears in the conversation
and may reach the Agent as labelled context, without claiming correlation.
A session retains at most 256 tool-call IDs, then ends rather than forgetting
an ID that could send duplicate mail.
Voice off, stop, a hidden window or a yard/conversation/provider switch closes
the session and its audio; a new session needs a new explicit start.

The native app requests a signed URL with `xi-api-key`, then connects without
a key header. It permits only TLS at `api.elevenlabs.io`, follows no redirects,
never logs error bodies or signed URLs, and never reconnects automatically.
Signed URLs are consumed immediately in memory; expiry or refusal requires a
new explicit start. Mono PCM16 negotiated at 8, 16, 22.05, 24, 44.1 or 48 kHz is
converted to and from the existing 24 kHz audio seam. Compressed formats are
refused. Managed interruptions flush speaker audio and discard old event IDs.

See the [signed-URL reference](https://elevenlabs.io/docs/api-reference/conversations/get-signed-url),
[WebSocket guide](https://elevenlabs.io/docs/eleven-agents/libraries/web-sockets),
and [client tools](https://elevenlabs.io/docs/eleven-agents/customization/tools/client-tools)
for the provider contract. No paid calls, real microphone, workspace agent or
subscription settings were exercised by the offline verification. German,
English and Swiss German tool strings are preserved in fake tests; recognition,
pronunciation and bilingual model quality remain for live commissioning.
