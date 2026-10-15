#!/bin/sh
# The prune helper drops only build trees with nothing written in the last
# week, never this shot's own, and says what went on stderr.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
helper=$script_dir/shot-linux-prune.sh
tmp=$(mktemp -d)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fail() {
    echo "test-shot-linux-prune.sh: $1" >&2
    exit 1
}

root=$tmp/target
own=$root/this-checkout-111
active=$root/.shot-active
mkdir -p "$own/debug" "$root/fresh-checkout-222/debug" \
    "$root/stale-checkout-333/debug" "$root/dead-checkout-444/debug"
mkdir -p "$active"
echo binary > "$own/debug/footplate"
echo binary > "$root/fresh-checkout-222/debug/footplate"
echo binary > "$root/stale-checkout-333/debug/footplate"
echo binary > "$root/dead-checkout-444/debug/footplate"
# A file at the volume's root, as the old shared build left: never a tree.
echo legacy > "$root/.rustc_info.json"
# The stale tree went quiet over a week ago, and this shot's own is older
# still: age never takes a shot's own tree.
find "$root/stale-checkout-333" "$root/dead-checkout-444" "$own" \
    -exec touch -t 200001010000 {} +
echo active > "$active/stale-checkout-333-123"
echo abandoned > "$active/dead-checkout-444-123"
touch -t 200001010000 "$active/dead-checkout-444-123"

out=$(sh "$helper" "$root" "$own" "$active/this-checkout-111-456" 2>"$tmp/stderr") \
    || fail "the helper failed"
[ -z "$out" ] || fail "the helper wrote to stdout: $out"
[ -d "$own" ] || fail "this shot's own tree was pruned"
[ -d "$root/fresh-checkout-222" ] || fail "a fresh tree was pruned"
[ -f "$root/.rustc_info.json" ] || fail "a file at the root was pruned"
[ -d "$root/stale-checkout-333" ] || fail "an active stale tree was pruned"
[ ! -e "$root/dead-checkout-444" ] || fail "a stale marker kept an abandoned tree"
[ ! -e "$active/dead-checkout-444-123" ] || fail "an abandoned marker did not age out"

# A stale tree can still be in use by a shot whose build has not written to
# it recently. Once that shot removes its marker, the tree can be pruned.
rm -f "$active/stale-checkout-333-123"
out=$(sh "$helper" "$root" "$own" "$active/this-checkout-111-456" release.sh 2>>"$tmp/stderr") \
    || fail "the helper failed after the active shot ended"
[ -z "$out" ] || fail "the helper wrote to stdout: $out"
[ ! -e "$root/stale-checkout-333" ] || fail "a stale tree survived"
grep -F -q "release.sh: pruned stale build tree $root/stale-checkout-333" "$tmp/stderr" \
    || fail "the helper did not say what it pruned: $(cat "$tmp/stderr")"

# With nothing stale the helper is silent, and a first shot's tree, not
# yet built, is no failure.
rm -rf "$own"
rm -f "$active/this-checkout-111-456"
out=$(sh "$helper" "$root" "$own" "$active/this-checkout-111-789" 2>"$tmp/stderr2") \
    || fail "a run with nothing stale failed"
[ -z "$out" ] || fail "a quiet run wrote to stdout: $out"
[ ! -s "$tmp/stderr2" ] || fail "a quiet run said: $(cat "$tmp/stderr2")"
[ -d "$root/fresh-checkout-222" ] || fail "a quiet run pruned a fresh tree"

echo "test-shot-linux-prune.sh: pass"
