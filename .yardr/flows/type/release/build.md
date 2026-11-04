summary: The release group builds the archives, tags the base, makes the GitHub release and notes the five signing commands.

The release group works this stage
(`scripts/release-stage build`, docs/building.md "Building"): it
builds the seven archives and `checksums.txt` from the depot's base, tags
that commit, pushes the tag, and makes the GitHub release with the bead's
body as its notes. It then notes the five signing commands on the bead, for
the version, and reports done. No agent works this stage by hand.

Before the bead leaves as `done`:

- the release exists, with the seven archives and `checksums.txt`, unsigned;
- a note on the bead has the five signing commands.

Outcomes:

- `done`: both hold. The bead goes to `sign`, where mckean is asked to sign
  the checksums.
- `failed`: the version, the tag, the tree, the build or the release stood
  in the way; the note says which. The bead goes to `decide`, to a human.
- `question`: the bead itself is wrong, as a rule its title names no
  version. Note what is wrong first; the bead goes to `decide`, to a human.
