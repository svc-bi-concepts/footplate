#!/bin/sh
# Two shots without an argument must write to separate files; every shot
# mounts the two volumes all shots share and removes its container when it
# ends, and a stale container is removed before a new one starts.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/shot-linux.sh
tmp=$(mktemp -d)
first_dir=
second_dir=
stale_dir=
# The two directories are named by the script under test, so they are never
# removed with what they hold: only the shot is, and then the directory if
# that left it empty. A script that went wrong and wrote into a directory it
# shares (/tmp, the checkout) costs this test nothing but its own files.
remove_shot_dir() {
    [ -n "$1" ] || return 0
    rm -f "$1/footplate-shot.png"
    rmdir "$1" 2>/dev/null || true
}
cleanup() {
    remove_shot_dir "$first_dir"
    remove_shot_dir "$second_dir"
    remove_shot_dir "$stale_dir"
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$tmp/bin"

# The shot script only needs Docker to answer whether a stale container is
# left and to report a visible window here. Stub it so this checks the real
# output-path and cleanup behavior without provisioning a container or
# building Footplate; the run and rm lines are logged, so this sees what a
# shot mounts and leaves.
cat > "$tmp/bin/docker" <<'MOCK_DOCKER'
#!/bin/sh
set -eu
command=$1
shift
case "$command" in
    inspect)
        # The only inspect asks whether a stale container of the name is
        # left; MOCK_HAVE_CONTAINER=1 says one is.
        if [ "${MOCK_HAVE_CONTAINER:-0}" = 1 ]; then
            exit 0
        fi
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
        case "$*" in
            *swaymsg*-t\ get_tree*) echo '{"app_id":"footplate"}' ;;
        esac
        exit 0
        ;;
    cp)
        destination=$2
        if [ "${MOCK_FAIL_CP:-0}" = 1 ]; then
            exit 1
        fi
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
    echo "test-shot-linux-default.sh: $1" >&2
    exit 1
}

# The container's name, as the script under test derives it from the
# checkout: one container per checkout.
root=$(CDPATH= cd "$script_dir/.." && pwd)
suffix=$(basename "$root")
suffix=$(printf '%s' "$suffix" | tr -c 'a-zA-Z0-9_.-' '-' | tr 'A-Z' 'a-z')
name=footplate-shot-$suffix

run_shot() {
    MOCK_LOG=$1 PATH="$tmp/bin:$PATH" "$script"
}

# A shot mounts the checkout read-only and the two volumes all shots
# share, and removes its container when it ends: the log holds a run line
# and then the rm line, and with a stale container one rm line before that.
expect_run() {
    what=$1
    log=$2
    lines=$3
    grep -F -q -- "run -d --name $name " "$log" \
        || fail "$what started no container of its own"
    grep -F -q -- ":/src:ro" "$log" \
        || fail "$what did not mount the checkout read-only"
    grep -F -q -- "-v footplate-shot-cargo:/cargo -v footplate-shot-target:/target" "$log" \
        || fail "$what did not mount the shared volumes"
    [ "$(tail -n 1 "$log")" = "rm -f $name" ] \
        || fail "$what left its container behind"
    [ "$(grep -c '' "$log")" = "$lines" ] \
        || fail "$what ran unexpected docker commands: $(cat "$log")"
}

first=$(run_shot "$tmp/log-first")
second=$(run_shot "$tmp/log-second")
expect_run "the first shot" "$tmp/log-first" 2
expect_run "the second shot" "$tmp/log-second" 2

# A failure after the container has started still removes it.
if MOCK_FAIL_CP=1 MOCK_LOG=$tmp/log-failure PATH="$tmp/bin:$PATH" \
        "$script" "$tmp/shot-failure.png" >"$tmp/failure.out" 2>&1; then
    fail "a failed screenshot unexpectedly succeeded"
fi
grep -F -q -- "run -d --name $name " "$tmp/log-failure" \
    || fail "the failing shot started no container"
[ "$(tail -n 1 "$tmp/log-failure")" = "rm -f $name" ] \
    || fail "the failing shot left its container behind"

# A container of the name left by a killed run is removed before a new one
# starts.
export MOCK_HAVE_CONTAINER=1
stale=$(run_shot "$tmp/log-stale")
unset MOCK_HAVE_CONTAINER
stale_dir=$(dirname "$stale")
[ "$(head -n 1 "$tmp/log-stale")" = "rm -f $name" ] \
    || fail "a stale container was not removed before the new one started"
expect_run "the stale shot" "$tmp/log-stale" 3
if [ "$first" = "$second" ]; then
    echo "test-shot-linux-default.sh: two default shots used the same path: $first" >&2
    exit 1
fi
first_dir=$(dirname "$first")
second_dir=$(dirname "$second")
if [ "$first_dir" = "$second_dir" ]; then
    echo "test-shot-linux-default.sh: two default shots used the same directory: $first_dir" >&2
    exit 1
fi
for shot in "$first" "$second"; do
    case "$shot" in
        */footplate-shot.png) ;;
        *)
            echo "test-shot-linux-default.sh: unexpected default path: $shot" >&2
            exit 1
            ;;
    esac
    if [ ! -f "$shot" ]; then
        echo "test-shot-linux-default.sh: no screenshot at $shot" >&2
        exit 1
    fi
done
echo "test-shot-linux-default.sh: pass"
