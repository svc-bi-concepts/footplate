#!/bin/sh
# The shot with no argument must not go to one fixed file in /tmp: every
# builder on the Mac shares it, two shots at once would overwrite each
# other, and a note would carry another bead's picture.
set -eu

script=$(dirname "$0")/shot-linux.sh
if grep -n 'out=${1:-/tmp/' "$script" >&2; then
    echo "test-shot-linux-default.sh: the shot defaults to one fixed file in /tmp" >&2
    exit 1
fi
if ! grep -q 'out=${1:-$(mktemp -d)/' "$script"; then
    echo "test-shot-linux-default.sh: the shot's default is not in a directory of its own" >&2
    exit 1
fi
echo "test-shot-linux-default.sh: pass"
