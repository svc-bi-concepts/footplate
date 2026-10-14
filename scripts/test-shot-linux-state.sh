#!/bin/sh
# A state reaches footplate as FOOTPLATE_SHOT, and the picture still goes
# where it is told: `shot-linux.sh <state> [file]`, and with no state the
# first screen, as ever. An unknown state with a file is refused.
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
mkdir -p "$tmp/bin" "$tmp/cwd"

# The shot script only needs Docker to report a running container and a
# visible window here. Stub it so this checks the real argument behavior
# without provisioning a container or building Footplate; every exec line
# is logged, so this sees what footplate would start with.
cat > "$tmp/bin/docker" <<'MOCK_DOCKER'
#!/bin/sh
set -eu
command=$1
shift
case "$command" in
    inspect)
        shift
        if [ "${1:-}" = --format ] && [ "${2:-}" = '{{.State.Status}}' ]; then
            echo running
            exit 0
        fi
        # The first inspect asks whether a checkout-specific container exists.
        exit 1
        ;;
    run|start)
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
    echo "test-shot-linux-state.sh: $1" >&2
    exit 1
}

expect_shot() {
    what=$1
    want=$2
    [ -f "$want" ] || fail "$what wrote no screenshot at $want"
    case "$want" in
        */footplate-shot.png) shot_dirs="$shot_dirs $(dirname "$want")" ;;
    esac
}

expect_launch() {
    what=$1
    log=$2
    want=$3
    grep -F -q "FOOTPLATE_SHOT=$want dbus-run-session" "$log" \
        || fail "$what started footplate without FOOTPLATE_SHOT='$want'"
}

export MOCK_LOG=$tmp/log-default
out=$(PATH="$tmp/bin:$PATH" "$script")
expect_shot "no arguments" "$out"
expect_launch "no arguments" "$MOCK_LOG" ""

export MOCK_LOG=$tmp/log-run
out=$(PATH="$tmp/bin:$PATH" "$script" run)
expect_shot "a state alone" "$out"
expect_launch "a state alone" "$MOCK_LOG" "run"

export MOCK_LOG=$tmp/log-plain
out=$(PATH="$tmp/bin:$PATH" "$script" plain "$tmp/shot-plain.png")
[ "$out" = "$tmp/shot-plain.png" ] || fail "a state with a file printed $out"
expect_shot "a state with a file" "$out"
expect_launch "a state with a file" "$MOCK_LOG" "plain"

export MOCK_LOG=$tmp/log-file
out=$(PATH="$tmp/bin:$PATH" "$script" "$tmp/shot-first.png")
[ "$out" = "$tmp/shot-first.png" ] || fail "a file alone printed $out"
expect_shot "a file alone" "$out"
expect_launch "a file alone" "$MOCK_LOG" ""

# One argument that is no state is a file, as before the states.
export MOCK_LOG=$tmp/log-bogus-file
out=$(cd "$tmp/cwd" && PATH="$tmp/bin:$PATH" MOCK_LOG="$MOCK_LOG" "$script" bogus)
[ "$out" = "$tmp/cwd/bogus" ] || fail "a lone name printed $out"
[ -f "$tmp/cwd/bogus" ] || fail "a lone name wrote no screenshot"
expect_launch "a lone name" "$MOCK_LOG" ""

# Two arguments name a state first: an unknown one is refused, before
# anything is written.
export MOCK_LOG=$tmp/log-bogus-state
if err=$(PATH="$tmp/bin:$PATH" "$script" bogus "$tmp/shot-bogus.png" 2>&1); then
    fail "an unknown state shot anyway: $err"
fi
case "$err" in
    *"no such state: bogus"*) ;;
    *) fail "an unknown state said: $err" ;;
esac
[ ! -e "$tmp/shot-bogus.png" ] || fail "an unknown state wrote its file"

echo "test-shot-linux-state.sh: pass"
