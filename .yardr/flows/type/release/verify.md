summary: The release group checks the signature against the release key, the archives against the checksums, and the Mac apps for notarization.

The release group works this stage (`scripts/release-stage verify`): it
downloads `checksums.txt` and `checksums.txt.minisig` from the release,
checks the signature against the release key, and checks the release's
archives against the checksums. It then unpacks each macOS archive
(`*_darwin_*.zip`, with `ditto -x -k`) and checks its app: `xcrun stapler
validate` must pass, and `spctl -a -vv -t exec` must name `source=Notarized
Developer ID`. A release built without the signing variables is signed ad hoc
and fails that check, as it should: another Mac blocks its apps. `xcrun`,
`spctl` and `ditto` are a Mac's; where one is missing the check is skipped,
and the done note says that it was and which tool was missing. No agent works
this stage by hand.

Outcomes:

- `done`: the signature is good, the archives match the checksums, and the
  note names the Mac apps that are notarized (or says that none was looked
  at, and why). The bead goes to `publish`.
- `failed`: the signature is missing or bad; the note carries what minisign
  said. The bead goes back to `sign`, where mckean is asked again.
- `question`: the release itself is wrong (the archives do not match the
  checksums, or a Mac app is not signed and notarized) or a required check
  could not run at all. A missing `xcrun`, `spctl` or `ditto` skips only the
  macOS check, as the `done` outcome says. The note says which, with what the
  tool said. The bead goes to `decide`, to a human: a release whose archives
  are wrong is remade by hand (the release and the tag deleted, the bead back
  to `build`).
