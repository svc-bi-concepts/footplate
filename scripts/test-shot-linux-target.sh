#!/bin/sh
# Each checkout's shot builds in its own directory under the shared target
# volume, launches that binary, and prunes stale trees before building: no
# shot ever runs another checkout's binary.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/shot-linux.sh
tmp=$(mktemp -d)
shot_dirs=""
# The fresh directories are named by the script under test, so they are
# never removed with what they hold: only the shot is, and then the
# directory if that left it empty.
cleanup() {
    for dir in $shot_dirs; do
        rm -f "$dir/footplate-shot.png"
        rmdir "$dir" 2>/dev/null || true
    done
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$tmp/bin" "$tmp/t1/checkout/scripts" "$tmp/t2/checkout/scripts"

# Two checkouts with the same directory name, as /tmp/a/footplate and
# /tmp/b/footplate: the script under test runs from each through a link,
# so each shot derives its own root.
ln -s "$script" "$tmp/t1/checkout/scripts/shot-linux.sh"
ln -s "$script" "$tmp/t2/checkout/scripts/shot-linux.sh"

# The shot script only needs Docker to answer whether a stale container is
# left and to report a visible window here. Stub it so this checks the real
# target-directory behavior without provisioning a container or building
# Footplate; the run and exec lines are logged.
cat > "$tmp/bin/docker" <<'MOCK_DOCKER'
#!/bin/sh
set -eu
command=$1
shift
case "$command" in
    inspect)
        # The only inspect asks whether a stale container of the name is left.
        exit 1
        ;;
    run)
        printf 'run %s\n' "$*" >> "$MOCK_LOG"
        exit 0
        ;;
    rm)
        printf 'rm %s\n' "$*" >> "$MOCK_LOG"
        exit 0
        ;;
    exec)
        shift
        printf '%s\n' "$*" >> "$MOCK_LOG"
        case "$*" in
            *swaymsg*-t\ get_tree*) echo '{"app_id":"footplate"}' ;;
        esac
        exit 0
        ;;
    cp)
        destination=$2
        printf 'mock screenshot\n' > "$destination"
        ;;
    *)
        echo "unexpected docker command: $command $*" >&2
        exit 1
        ;;
esac
MOCK_DOCKER
cat > "$tmp/bin/sleep" <<'MOCK_SLEEP'
#!/bin/sh
exit 0
MOCK_SLEEP
chmod +x "$tmp/bin/docker" "$tmp/bin/sleep"

fail() {
    echo "test-shot-linux-target.sh: $1" >&2
    exit 1
}

note_shot() {
    case "$1" in
        */footplate-shot.png) shot_dirs="$shot_dirs $(dirname "$1")" ;;
        *) fail "unexpected shot path: $1" ;;
    esac
}

target_of() {
    grep -o -- 'CARGO_TARGET_DIR=[^ ]*' "$1" | head -n 1
}

first=$(MOCK_LOG=$tmp/log-t1 PATH="$tmp/bin:$PATH" "$tmp/t1/checkout/scripts/shot-linux.sh")
note_shot "$first"
second=$(MOCK_LOG=$tmp/log-t1-again PATH="$tmp/bin:$PATH" "$tmp/t1/checkout/scripts/shot-linux.sh")
note_shot "$second"
other=$(MOCK_LOG=$tmp/log-t2 PATH="$tmp/bin:$PATH" "$tmp/t2/checkout/scripts/shot-linux.sh")
note_shot "$other"

# The build tree names the checkout's directory and a hash of its path.
dir1=$(target_of "$tmp/log-t1")
dir2=$(target_of "$tmp/log-t2")
again=$(target_of "$tmp/log-t1-again")
key1=$(printf '%s' "$tmp/t1/checkout" | cksum | cut -d' ' -f1)
key2=$(printf '%s' "$tmp/t2/checkout" | cksum | cut -d' ' -f1)
[ "$dir1" = "CARGO_TARGET_DIR=/target/checkout-$key1" ] \
    || fail "the first checkout built at $dir1"
[ "$dir2" = "CARGO_TARGET_DIR=/target/checkout-$key2" ] \
    || fail "the second checkout built at $dir2"
[ "$dir1" = "$again" ] || fail "one checkout's tree moved: $dir1 then $again"
[ "$dir1" != "$dir2" ] || fail "two checkouts share $dir1"

# Footplate launches from the checkout's own tree, never the shared root.
grep -F -q -- "-- /target/checkout-$key1/debug/footplate" "$tmp/log-t1" \
    || fail "the first checkout launched no binary of its own"
if grep -F -q -- '-- /target/debug/footplate' "$tmp/log-t1" "$tmp/log-t2"; then
    fail "a shot launched the shared root's binary"
fi

# Stale trees are pruned before the build, never after.
prune=$(grep -n -F 'shot-linux-prune.sh' "$tmp/log-t1" | cut -d: -f1 | head -n 1)
build=$(grep -n -F 'cargo build' "$tmp/log-t1" | cut -d: -f1 | head -n 1)
[ -n "$prune" ] || fail "no shot pruned stale trees"
[ -n "$build" ] || fail "no shot built"
[ "$prune" -lt "$build" ] || fail "the prune ran after the build"
grep -F -q -- "/target/.shot-active/checkout-$key1-" "$tmp/log-t1" \
    || fail "the shot registered no active target marker"
grep -F -q -- "rm -f /target/.shot-active/checkout-$key1-" "$tmp/log-t1" \
    || fail "the shot did not remove its active target marker"

echo "test-shot-linux-target.sh: pass"
