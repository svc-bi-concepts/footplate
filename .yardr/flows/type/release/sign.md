summary: mckean signs the release's checksums by hand and says so; the signature is checked next.

The release's `checksums.txt` is signed by hand here, with the minisign key
that stays on mckean's machine and is on no runner. The build's note on the
bead has the five commands for this version: make a directory of its own,
download `checksums.txt`, sign it, check the signature against the release
key, upload `checksums.txt.minisig`. Run them in order; each names the
version, so a stale file or a wrong release shows itself. The signing never
rebuilds: it signs the checksums the build uploaded, as they stand.

Only this moves the bead: Done, once `checksums.txt.minisig` is uploaded to
the release. The verify stage checks the signature next, and a bad one asks
again here. Later is not yet; the ask comes back.

## Ask
kind: check
question: Is this release's checksums.txt.minisig uploaded?
context:
- The build's note on this bead has the five signing commands for this version: make a directory of its own, download checksums.txt, sign it with the minisign key, check the signature against the release key, upload checksums.txt.minisig.
- Run them in order. Each names the version, so a stale file or a wrong release shows itself.
- Done means the signature is uploaded; the verify stage checks it next. Not yet is Later: the ask comes back.
move: done
