#!/bin/sh
# Each checkout's shot builds in its own directory under the shared target
# volume, launches that binary, and prunes stale trees before building: no
# shot ever runs another checkout's binary. The live list it hands the prune
# names each checkout's tree exactly as its own.
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
# so each shot derives its own root. Its container provisioning comes
# along the same way, since the script sources it beside itself.
ln -s "$script" "$tmp/t1/checkout/scripts/shot-linux.sh"
ln -s "$script" "$tmp/t2/checkout/scripts/shot-linux.sh"
ln -s "$script_dir/linux-container.sh" "$tmp/t1/checkout/scripts/linux-container.sh"
ln -s "$script_dir/linux-container.sh" "$tmp/t2/checkout/scripts/linux-container.sh"

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
            *cat*.shot-live-*) cat > "$MOCK_LOG.stdin" ;;
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
# The depot's live checkouts, as `git worktree list --porcelain` prints
# them: both checkouts under test, so each shot's live list names both.
# GIT_FAIL=1 says git cannot list them.
cat > "$tmp/bin/git" <<MOCK_GIT
#!/bin/sh
if [ "\${GIT_FAIL:-0}" = 1 ]; then
    exit 1
fi
printf 'worktree %s\nHEAD 0000000\nbranch refs/heads/main\n\n' "$tmp/t1/checkout"
printf 'worktree %s\nHEAD 0000000\nbranch refs/heads/yard/x\n\n' "$tmp/t2/checkout"
MOCK_GIT
chmod +x "$tmp/bin/docker" "$tmp/bin/sleep" "$tmp/bin/git"

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

# The run pins the host's own platform, whatever variant the local tag
# holds; on a host uname -m says nothing known of, it keeps no flag.
case "$(uname -m)" in
    aarch64 | arm64) want_platform=linux/arm64 ;;
    x86_64 | amd64) want_platform=linux/amd64 ;;
    *) want_platform="" ;;
esac
if [ -n "$want_platform" ]; then
    for log in "$tmp/log-t1" "$tmp/log-t1-again" "$tmp/log-t2"; do
        grep -F -q -- "run --platform $want_platform " "$log" \
            || fail "a run names no --platform $want_platform: $(cat "$log")"
    done
elif grep -F -q -- "run --platform " "$tmp/log-t1" "$tmp/log-t2"; then
    fail "a run on an unknown host names a platform"
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

# The live list reaches the prune helper: the shot writes its depot's live
# trees (each checkout's tree named exactly as its own, plus the release
# tree sharing the volume) and hands the file to the prune, then removes it.
grep -F -q -- "cat > /target/.shot-live-" "$tmp/log-t1" \
    || fail "the shot wrote no live list"
prune_line=$(grep -F 'shot-linux-prune.sh' "$tmp/log-t1" | head -n 1)
case "$prune_line" in
    *"/target/.shot-live-"*) ;;
    *) fail "the prune took no live list: $prune_line" ;;
esac
for want in "checkout-$key1" "release-checkout-$key1" "checkout-$key2" "release-checkout-$key2"; do
    grep -F -x -q -- "$want" "$tmp/log-t1.stdin" \
        || fail "the live list names no $want: $(cat "$tmp/log-t1.stdin" 2>/dev/null)"
done
grep -F -q -- "rm -f /target/.shot-live-" "$tmp/log-t1" \
    || fail "the shot did not remove its live list"

# Where git cannot list the depot, the shot prunes as before, with no live
# list, and says so.
fallback=$(GIT_FAIL=1 MOCK_LOG=$tmp/log-fallback PATH="$tmp/bin:$PATH" "$tmp/t1/checkout/scripts/shot-linux.sh" 2>"$tmp/stderr-fallback")
note_shot "$fallback"
fallback_prune=$(grep -F 'shot-linux-prune.sh' "$tmp/log-fallback" | head -n 1)
case "$fallback_prune" in
    *".shot-live-"*) fail "a shot with no live list pruned with one: $fallback_prune" ;;
esac
if grep -F -q -- "cat > /target/.shot-live-" "$tmp/log-fallback"; then
    fail "a shot with no live list wrote one"
fi
grep -F -q "could not list the depot's checkouts" "$tmp/stderr-fallback" \
    || fail "a shot with no live list did not say so: $(cat "$tmp/stderr-fallback")"

echo "test-shot-linux-target.sh: pass"
