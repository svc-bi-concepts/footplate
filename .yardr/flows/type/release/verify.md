summary: The release group checks the signature against the release key, the archives against the checksums, and the Mac archives for notarization.

The release group works this stage (`scripts/release-stage verify`): it
downloads `checksums.txt` and `checksums.txt.minisig` from the release,
checks the signature against the release key, and checks the release's
archives against the checksums. It then looks at each macOS archive: each
`*_darwin_*.zip` is unpacked (with `ditto -x -k`) and its app checked:
`xcrun stapler validate` must pass, and `spctl -a -vv -t exec` must name
`source=Notarized Developer ID`. Each `*_darwin_*.dmg` is checked itself
(`xcrun stapler validate`, and `spctl -a -vv -t open` naming
`source=Notarized Developer ID`), then mounted read-only and must hold
exactly `Footplate.app` and the `Applications` link to `/Applications`,
whose app passes `xcrun stapler validate` and `spctl -a -vv -t exec` in
turn; the image is detached after, whatever the check found. A release
built without the signing variables is signed ad hoc and fails that
check, as it should: another Mac blocks its apps. `xcrun`, `spctl`,
`ditto` and `hdiutil` are a Mac's; where one is missing the check is
skipped, and the done note says that it was and which tool was missing.
No agent works this stage by hand.

Outcomes:

- `done`: the signature is good, the archives match the checksums, and the
  note names the Mac apps and disk images that are notarized (or says that
  none was looked at, and why). The bead goes to `publish`.
- `failed`: the signature is missing or bad; the note carries what minisign
  said. The bead goes back to `sign`, where mckean is asked again.
- `question`: the release itself is wrong (the archives do not match the
  checksums, or a Mac app or disk image is not signed and notarized) or a
  required check could not run at all. A missing `xcrun`, `spctl`, `ditto`
  or `hdiutil` skips only the macOS check, as the `done` outcome says. The
  note says which, with what the tool said. The bead goes to `decide`, to a
  human: a release whose archives are wrong is remade by hand (the release
  and the tag deleted, the bead back to `build`).
