summary: The release group files the yardr.dev sync bead and advances it to new.

The release group works this stage (`scripts/release-stage publish`): the
release is built, signed and verified, and what is left is the site. It
files the yardr.dev sync bead (`scripts/sync-release footplate <version>`,
priority 1, `difficulty:1 size:small`), advances it to `new`, and notes its
id on this bead. No agent works this stage by hand.

Outcomes:

- `done`: the sync bead is filed and on its way. This bead goes to
  `published`; the release is the site's now.
- `failed`: the sync bead could not be filed or advanced; the note says
  what stopped it and the id, where one was made. The bead goes to
  `decide`, to a human.
- `question`: filing it needs a person's decision; to `decide`.
