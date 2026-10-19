#!/bin/sh
# The prune helper drops only build trees with nothing written in the last
# week, never this shot's own, and says what went on stderr. With a live
# list, a removed checkout's tree goes at once, a live checkout's keeps the
# 7-day rule, and markers of removed checkouts go with their trees.
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

# With a live list, a removed checkout's tree goes at once, however fresh;
# a live checkout's tree keeps the 7-day rule; and an active marker still
# guards its tree for the run, while markers of removed checkouts go.
live=$tmp/live
live_own=$live/own-tree-111
live_active=$live/.shot-active
mkdir -p "$live_own/debug" "$live/gone-fresh-222/debug" \
    "$live/live-fresh-333/debug" "$live/live-stale-444/debug" \
    "$live/gone-active-555/debug" "$live/live-active-666/debug"
mkdir -p "$live_active"
for t in own-tree-111 gone-fresh-222 live-fresh-333 live-stale-444 gone-active-555 live-active-666; do
    echo binary > "$live/$t/debug/footplate"
done
find "$live/live-stale-444" "$live_own" -exec touch -t 200001010000 {} +
printf '%s\n' own-tree-111 live-fresh-333 live-stale-444 live-active-666 > "$tmp/live-list"
echo active > "$live_active/gone-active-555-123"
echo active > "$live_active/live-active-666-123"
echo orphan > "$live_active/vanished-tree-999-123"

out=$(sh "$helper" "$live" "$live_own" "$live_active/own-tree-111-456" shot-linux.sh "$tmp/live-list" 2>"$tmp/live-stderr") \
    || fail "the helper failed with a live list"
[ -z "$out" ] || fail "the helper wrote to stdout: $out"
[ -d "$live_own" ] || fail "this shot's own tree was pruned"
[ ! -e "$live/gone-fresh-222" ] || fail "a removed checkout's fresh tree survived"
[ -d "$live/live-fresh-333" ] || fail "a live checkout's fresh tree was pruned"
[ ! -e "$live/live-stale-444" ] || fail "a live checkout's stale tree survived"
[ -d "$live/gone-active-555" ] || fail "an active-marked tree with no live checkout was pruned"
[ ! -e "$live_active/gone-active-555-123" ] || fail "a removed checkout's marker survived"
[ ! -e "$live_active/vanished-tree-999-123" ] || fail "an orphaned marker survived"
[ -f "$live_active/live-active-666-123" ] || fail "a live checkout's marker was pruned"
grep -F -q "shot-linux.sh: pruned build tree of a removed checkout $live/gone-fresh-222" "$tmp/live-stderr" \
    || fail "the helper did not say what it pruned at once: $(cat "$tmp/live-stderr")"
grep -F -q "shot-linux.sh: pruned stale build tree $live/live-stale-444" "$tmp/live-stderr" \
    || fail "the helper did not say what went stale: $(cat "$tmp/live-stderr")"

# The guarded tree goes on the next run, once its marker is gone.
out=$(sh "$helper" "$live" "$live_own" "$live_active/own-tree-111-456" shot-linux.sh "$tmp/live-list" 2>>"$tmp/live-stderr") \
    || fail "the helper failed on the next run"
[ ! -e "$live/gone-active-555" ] || fail "an unguarded removed tree survived"

# A live list that never arrived is no list at all: prune as before, and
# say so, never from an empty list.
mkdir -p "$live/gone-fresh-777/debug"
echo binary > "$live/gone-fresh-777/debug/footplate"
out=$(sh "$helper" "$live" "$live_own" "$live_active/own-tree-111-456" shot-linux.sh "$live/no-such-list" 2>"$tmp/live-stderr2") \
    || fail "the helper failed with a missing live list"
[ -d "$live/gone-fresh-777" ] || fail "a missing live list pruned a fresh tree"
grep -F -q "pruning as before" "$tmp/live-stderr2" \
    || fail "the helper did not say it pruned as before: $(cat "$tmp/live-stderr2")"

echo "test-shot-linux-prune.sh: pass"
