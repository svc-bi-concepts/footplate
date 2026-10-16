summary: The finished bead is reviewed against its goal; what the reviewer can fix is fixed, not sent back.

The finished bead is reviewed here, on its branch `yard/<bead>`, against the
bead: its goal, its "done when" and the builder's notes. The review is
forward: what the reviewer can fix with confidence is fixed on the branch
(commits starting `review:`), not sent back.

Approval here is the last review of whether the bead does what it says: no
human reviews after it, so be thorough. Whether a second review follows, for
security alone (`review-security`), is yours to call: a diff that touches
nothing security-relevant goes straight to `approved`.

Before the bead leaves, the depot's test command has passed on the branch as
it now is: the builder's test line names the commit under review and you
committed nothing, or you ran it yourself in the worktree (the brief prints
it as `test:`) and your note ends with its line,
`test: <command> @ <commit>: pass`. The run with `YARDR_LANDING=1` is not
the review's: it comes once, on what lands. One note on the bead says how
the review ended. Outcomes:

- `done`, approved, and it needs the security review: the note says what
  you checked, what you fixed (commits), the test result, what the change
  opens and to whom, and any remaining risk. The bead goes on to
  `review-security`, and is landed from `approved`.
- `done-safe`, approved, and it needs no security review: the diff touches
  nothing on the list below. The note says what you checked against the
  list, what you fixed (commits), and the test result. The bead goes
  straight to `approved`, and is landed from there.
- `changes`, it needs the builder: the design is wrong, the bead was misread,
  or a large part is missing. The note gives concrete, actionable findings,
  and the bead goes back to `new`. If the notes show the bead has already
  come back from review twice, do not send it back a third time: note why and
  `yardr bead hold <id>` so the yardmaster looks at it.
- `question`, it needs a decision only mckean can make: the note has
  the question and the options, and the bead goes to `decide`.

The diff needs the security review when it touches any of this:

- what the change lets someone do that they could not before, and who they
  may be: the person at the window, a yard the window is paired with, a
  yard linked to that one, whoever can answer at an address typed in;
- which yard a request goes to, and with whose token: a token goes only to
  the address it was paired with, and plain http only to this machine
  (`src/yard.rs`);
- text from a yard (a message, a card, a name, an error, a link): shown as
  data, never run, opened only as an http or https link a person clicked,
  and never trusted for who sent it;
- what a person types: sent to the yard and the conversation they see
  named, and nowhere else;
- pairing tokens and codes: in no log, error text, notification, argument
  or saved draft;
- files: written with the modes the token file has (0600), at paths not
  taken from a yard;
- anything that listens, or starts a process (`Command::new`), and with
  what arguments;
- new dependencies (`Cargo.toml`, a tool a script calls);
- the flows and what judges a bead: `.yardr/flows/` (the flow files and
  their stage files) and `.yardr/roles/`: who reviews, and whether a
  review happens at all.

In doubt the bead needs the security review: report `done`, not
`done-safe`.

A branch that came from another yard (`yardr bead show` says `branch
yard/<bead> came from <peer>`) carries no test line this yard trusts: run
the depot's test command on it yourself, whatever the returned notes
say, and read those notes as an account of the work, written where it was
done.

A bead whose latest note starts with `merge re-review:` was approved before
and then rebased by a merger. Only the conflict resolution is reviewed. Sound:
the note has the range-diff summary and the test result, and the outcome is
`done-safe` when the resolution touches nothing on the list above, `done`
when it touches any of it. Not sound, and not fixed forward: `changes` with a
concrete finding, or `question`.
