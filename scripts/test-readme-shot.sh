#!/bin/sh
# The README's recipe for a bead's screenshot must not name one fixed file
# in /tmp: every builder on the Mac would write, attach and remove the same
# one, and a note would carry another bead's picture.
set -eu

readme=$(dirname "$0")/../README.md
if grep -n '/tmp/[A-Za-z0-9_.-]*\.png' "$readme" >&2; then
    echo "test-readme-shot.sh: the README names a fixed PNG in /tmp" >&2
    exit 1
fi
if ! grep -q 'shot=\$(mktemp -d)/' "$readme"; then
    echo "test-readme-shot.sh: the README's shot is not in a directory of its own" >&2
    exit 1
fi
echo "test-readme-shot.sh: pass"
