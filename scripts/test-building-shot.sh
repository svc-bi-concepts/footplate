#!/bin/sh
# The recipe for a bead's screenshot, in docs/building.md, must not name one
# fixed file in /tmp: every builder on the Mac would write, attach and remove
# the same one, and a note would carry another bead's picture.
set -eu

building=$(dirname "$0")/../docs/building.md
if grep -n '/tmp/[A-Za-z0-9_.-]*\.png' "$building" >&2; then
    echo "test-building-shot.sh: docs/building.md names a fixed PNG in /tmp" >&2
    exit 1
fi
if ! grep -q 'shot=\$(mktemp -d)/' "$building"; then
    echo "test-building-shot.sh: the shot in docs/building.md is not in a directory of its own" >&2
    exit 1
fi
echo "test-building-shot.sh: pass"
