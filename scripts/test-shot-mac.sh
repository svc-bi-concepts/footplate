#!/bin/sh
# shot-mac.sh refuses its bad arguments before it launches anything, and its
# hold and shoot modes run without a display: the window's binary, the
# compiler of its window lookup and screencapture are stubbed, as the linux
# tests stub docker. Nothing here needs an unlocked screen.
set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
script=$script_dir/shot-mac.sh
tmp=$(mktemp -d)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$tmp/bin" "$tmp/cwd"

[ -x "$script" ] || {
    echo "test-shot-mac.sh: shot-mac.sh is not executable" >&2
    exit 1
}

# A stand-in for footplate's binary: it reports what the run gave it, then
# either quits (hold mode's window, closed at once) or stands (the shoot
# mode's window, which the script stops by pid). It writes beside itself,
# since env -i leaves it no MOCK_LOG: $0 is the absolute path it was run
# with. The tail is appended by the caller.
make_footplate() {
    cat > "$1" <<'MOCK_FOOTPLATE'
#!/bin/sh
{
    printf 'FOOTPLATE_SHOT=%s\n' "$FOOTPLATE_SHOT"
    printf 'settings: '
    cat "$XDG_CONFIG_HOME/footplate/settings.json"
    printf 'PATH=%s\n' "$PATH"
    printf 'yardr_vars: '
    env | grep '^YARDR_' || printf '(none)\n'
} > "$0.log" 2>&1
MOCK_FOOTPLATE
    printf '%s\n' "$2" >> "$1"
    chmod +x "$1"
}
make_footplate "$tmp/bin/footplate-hold" "exit 0"
# An absolute sleep: PATH's sleep is stubbed to return at once, and this
# window must stand until the script stops it.
make_footplate "$tmp/bin/footplate-shoot" "exec /bin/sleep 30"

# swiftc answers with a window lookup that names window 7, onscreen.
cat > "$tmp/bin/swiftc" <<'MOCK_SWIFTC'
#!/bin/sh
out=""
prev=""
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then
        out=$arg
    fi
    prev=$arg
done
printf '#!/bin/sh\nprintf "7\\t0\\t1\\n"\n' > "$out"
chmod +x "$out"
MOCK_SWIFTC
cat > "$tmp/bin/screencapture" <<'MOCK_CAPTURE'
#!/bin/sh
id=""
out=""
for arg in "$@"; do
    case "$arg" in
        -l*) id=${arg#-l} ;;
        -o) ;;
        *) out=$arg ;;
    esac
done
printf 'screencapture %s\n' "$*" >> "$MOCK_LOG"
printf 'mock mac screenshot winid=%s\n' "$id" > "$out"
MOCK_CAPTURE
cat > "$tmp/bin/sleep" <<'MOCK_SLEEP'
#!/bin/sh
printf 'sleep %s\n' "$*" >> "$MOCK_LOG"
exit 0
MOCK_SLEEP
chmod +x "$tmp/bin/swiftc" "$tmp/bin/screencapture" "$tmp/bin/sleep"

fail() {
    echo "test-shot-mac.sh: $1" >&2
    exit 1
}

run_script() {
    PATH="$tmp/bin:$PATH" "$script" "$@"
}

# Fewer than three arguments are refused with the usage, before anything
# is launched.
set +e
err=$(run_script 2>&1)
code=$?
set -e
[ "$code" = 2 ] || fail "no arguments exited $code: $err"
case "$err" in
    *"usage: shot-mac.sh"*) ;;
    *) fail "no arguments said: $err" ;;
esac
set +e
err=$(run_script yardmaster dark 2>&1)
code=$?
set -e
[ "$code" = 2 ] || fail "two arguments exited $code: $err"
case "$err" in
    *"usage: shot-mac.sh"*) ;;
    *) fail "two arguments said: $err" ;;
esac

# An unknown state is refused, before anything is written or launched.
export MOCK_LOG="$tmp/log-bogus-state"
set +e
err=$(run_script bogus dark "$tmp/shot-bogus.png" "$tmp/bin/footplate-hold" 2>&1)
code=$?
set -e
[ "$code" = 2 ] || fail "an unknown state exited $code: $err"
case "$err" in
    *"no such state: bogus"*) ;;
    *) fail "an unknown state said: $err" ;;
esac
[ ! -e "$tmp/shot-bogus.png" ] || fail "an unknown state wrote its file"
[ ! -e "$tmp/bin/footplate-hold.log" ] || fail "an unknown state launched the binary"

# A look that is neither light nor dark is refused the same way.
export MOCK_LOG="$tmp/log-bogus-look"
set +e
err=$(run_script yardmaster sepia "$tmp/shot-sepia.png" "$tmp/bin/footplate-hold" 2>&1)
code=$?
set -e
[ "$code" = 2 ] || fail "a bad look exited $code: $err"
case "$err" in
    *"the look must be light or dark"*) ;;
    *) fail "a bad look said: $err" ;;
esac
[ ! -e "$tmp/shot-sepia.png" ] || fail "a bad look wrote its file"
[ ! -e "$tmp/bin/footplate-hold.log" ] || fail "a bad look launched the binary"

# What a launched window must have seen: its state, its look in the
# settings, and none of the caller's world.
expect_launch() {
    what=$1
    log=$2
    state=$3
    look=$4
    [ -f "$log" ] || fail "$what launched no window"
    grep -F -q "FOOTPLATE_SHOT=$state" "$log" \
        || fail "$what started footplate without FOOTPLATE_SHOT='$state'"
    grep -F -q "{\"look\": \"$look\"}" "$log" \
        || fail "$what did not write the look to the settings: $(cat "$log")"
    grep -F -q '/home/.local/bin:/usr/bin:/bin' "$log" \
        || fail "$what ran with the caller's PATH: $(cat "$log")"
    if grep -F -q "$tmp" "$log"; then
        fail "$what leaked the test's directory into the run: $(cat "$log")"
    fi
    # The run's own yard home is set; the caller's session is not in reach.
    grep -F -q 'YARDR_HOME=/tmp/fp-shot.' "$log" \
        || fail "$what ran without its own yard home: $(cat "$log")"
    for var in YARDR_BEAD YARDR_SESSION YARDR_ACTOR; do
        if grep -F -q "$var" "$log"; then
            fail "$what leaked the caller's $var into the run: $(cat "$log")"
        fi
    done
}

# Hold mode leaves the window up and quits with it: the stand-in quits at
# once, so this ends. It names no picture, so no file named hold is left.
export MOCK_LOG="$tmp/log-hold"
export YARDR_BEAD=sentinel-from-the-test
export YARDR_SESSION=sentinel-from-the-test
export YARDR_ACTOR=sentinel-from-the-test
set +e
err=$(cd "$tmp/cwd" && run_script yardmaster dark hold "$tmp/bin/footplate-hold" 2>&1)
code=$?
set -e
unset YARDR_BEAD YARDR_SESSION YARDR_ACTOR
[ "$code" = 0 ] || fail "hold exited $code: $err"
printf '%s\n' "$err" | grep -F -q "shot-mac.sh: footplate (pid " \
    || fail "hold said: $err"
printf '%s\n' "$err" | grep -F -q "; cmd+q quits it" \
    || fail "hold said: $err"
[ ! -e "$tmp/cwd/hold" ] || fail "hold left a file named hold"
expect_launch "hold" "$tmp/bin/footplate-hold.log" "yardmaster" "dark"

# A shot takes the window the lookup names and writes where it is told,
# after the seconds it was given.
export MOCK_LOG="$tmp/log-shoot"
export YARDR_BEAD=sentinel-from-the-test
export YARDR_SESSION=sentinel-from-the-test
export YARDR_ACTOR=sentinel-from-the-test
set +e
out=$(run_script yardmaster dark "$tmp/shot-mac.png" "$tmp/bin/footplate-shoot" 1)
code=$?
set -e
unset YARDR_BEAD YARDR_SESSION YARDR_ACTOR
[ "$code" = 0 ] || fail "a shot exited $code"
[ "$out" = "$tmp/shot-mac.png" ] || fail "a shot printed $out"
[ -f "$tmp/shot-mac.png" ] || fail "a shot wrote no screenshot"
grep -F -q 'mock mac screenshot winid=7' "$tmp/shot-mac.png" \
    || fail "a shot did not capture window 7: $(cat "$tmp/shot-mac.png")"
grep -F -q -- "-l7 -o $tmp/shot-mac.png" "$MOCK_LOG" \
    || fail "a shot captured no window: $(cat "$MOCK_LOG")"
[ "$(grep -c '^sleep ' "$MOCK_LOG")" = 1 ] \
    || fail "a shot slept awry: $(cat "$MOCK_LOG")"
grep -F -q 'sleep 1' "$MOCK_LOG" \
    || fail "a shot did not wait its second: $(cat "$MOCK_LOG")"
expect_launch "a shot" "$tmp/bin/footplate-shoot.log" "yardmaster" "dark"

# A relative file lands under the directory the script is run from, not
# the checkout it shoots from.
export MOCK_LOG="$tmp/log-relative"
set +e
out=$(cd "$tmp/cwd" && run_script crew light rel-shot.png "$tmp/bin/footplate-shoot" 2)
code=$?
set -e
[ "$code" = 0 ] || fail "a relative file exited $code"
[ "$out" = "$tmp/cwd/rel-shot.png" ] || fail "a relative file printed $out"
[ -f "$tmp/cwd/rel-shot.png" ] || fail "a relative file wrote no screenshot"
[ ! -e "$script_dir/../rel-shot.png" ] || fail "a relative file landed in the checkout"

echo "test-shot-mac.sh: pass"
