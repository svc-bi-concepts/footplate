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
mkdir -p "$own/debug" "$root/fresh-checkout-222/debug" "$root/stale-checkout-333/debug"
echo binary > "$own/debug/footplate"
echo binary > "$root/fresh-checkout-222/debug/footplate"
echo binary > "$root/stale-checkout-333/debug/footplate"
# A file at the volume's root, as the old shared build left: never a tree.
echo legacy > "$root/.rustc_info.json"
# The stale tree went quiet over a week ago, and this shot's own is older
# still: age never takes a shot's own tree.
find "$root/stale-checkout-333" "$own" -exec touch -t 200001010000 {} +

out=$(sh "$helper" "$root" "$own" 2>"$tmp/stderr") || fail "the helper failed"
[ -z "$out" ] || fail "the helper wrote to stdout: $out"
[ -d "$own" ] || fail "this shot's own tree was pruned"
[ -d "$root/fresh-checkout-222" ] || fail "a fresh tree was pruned"
[ ! -e "$root/stale-checkout-333" ] || fail "a stale tree survived"
[ -f "$root/.rustc_info.json" ] || fail "a file at the root was pruned"
grep -F -q "pruned stale build tree $root/stale-checkout-333" "$tmp/stderr" \
    || fail "the helper did not say what it pruned: $(cat "$tmp/stderr")"

# With nothing stale the helper is silent, and a first shot's tree, not
# yet built, is no failure.
rm -rf "$own"
out=$(sh "$helper" "$root" "$own" 2>"$tmp/stderr2") \
    || fail "a run with nothing stale failed"
[ -z "$out" ] || fail "a quiet run wrote to stdout: $out"
[ ! -s "$tmp/stderr2" ] || fail "a quiet run said: $(cat "$tmp/stderr2")"
[ -d "$root/fresh-checkout-222" ] || fail "a quiet run pruned a fresh tree"

echo "test-shot-linux-prune.sh: pass"
