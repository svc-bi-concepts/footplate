summary: The release group checks the signature against the release key and the archives against the checksums.

The release group works this stage (`scripts/release-stage verify`): it
downloads `checksums.txt` and `checksums.txt.minisig` from the release,
checks the signature against the release key, and checks the release's
archives against the checksums. No agent works this stage by hand.

Outcomes:

- `done`: the signature is good and the archives match the checksums. The
  bead goes to `publish`.
- `failed`: the signature is missing or bad; the note carries what minisign
  said. The bead goes back to `sign`, where mckean is asked again.
- `question`: the release itself is wrong (the archives do not match the
  checksums) or the check could not run at all. The note says which. The
  bead goes to `decide`, to a human: a release whose archives are wrong is
  remade by hand (the release and the tag deleted, the bead back to
  `build`).
