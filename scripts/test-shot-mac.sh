#!/bin/sh
# shot-mac.sh refuses its bad arguments before it launches anything, and its
# hold and shoot modes run without a display: the window's binary, the
# compiler of its window lookup, screencapture and the hold on the display
# are stubbed, as the linux tests stub docker. Nothing here needs an unlocked
# screen, and nothing wakes or keeps awake the display it runs on.
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

# A stand-in for footplate's binary: it reports what the run gave it (the
# state and the look, and the settings it would open from, "(none)" where the
# run left no file), marks the report done with $0.up, then either quits
# (hold mode's window, closed as soon as the stand-in caffeinate beside it
# says it is up, so that the script is not over before the display is held) or
# stands (the shoot mode's window, which the script stops by pid). It writes
# beside itself, since env -i leaves it no MOCK_LOG: $0 is the absolute path it
# was run with. The tail is appended by the caller.
make_footplate() {
    cat > "$1" <<'MOCK_FOOTPLATE'
#!/bin/sh
{
    printf 'FOOTPLATE_SHOT=%s\n' "$FOOTPLATE_SHOT"
    printf 'SHOT_LOOK=%s\n' "$SHOT_LOOK"
    printf 'settings: '
    cat "$XDG_CONFIG_HOME/footplate/settings.json" 2>/dev/null || printf '(none)\n'
    printf 'PATH=%s\n' "$PATH"
    printf 'yardr_vars: '
    env | grep '^YARDR_' || printf '(none)\n'
} > "$0.log" 2>&1
: > "$0.up"
MOCK_FOOTPLATE
    printf '%s\n' "$2" >> "$1"
    chmod +x "$1"
}
make_footplate "$tmp/bin/footplate-hold" 'n=0
while [ ! -e "$(dirname "$0")/caffeinate.up" ] && [ "$n" -lt 100 ]; do
    /bin/sleep 0.05
    n=$((n + 1))
done
exit 0'
# An absolute sleep: PATH's sleep is stubbed to return at once, and this
# window must stand until the script stops it.
make_footplate "$tmp/bin/footplate-shoot" "exec /bin/sleep 30"

# swiftc answers with a window lookup that names window 7, onscreen, and
# logs what it was asked: "winid <pid>" to list the windows, "winid <pid>
# front" to bring the process forward, which fails where MOCK_FRONT_FAILS
# is set, as it does on a locked screen. It answers once the stand-in has
# reported ($MOCK_UP) and the display is held ($MOCK_AWAKE), so a shot is
# never stopped before its stand-in has said what it saw, and the log has the
# hold before anything the window does. An absolute sleep: PATH's is stubbed.
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
cat > "$out" <<'MOCK_WINID'
#!/bin/sh
n=0
while { [ ! -e "$MOCK_AWAKE" ] || { [ -n "${MOCK_UP:-}" ] && [ ! -e "$MOCK_UP" ]; }; } && [ "$n" -lt 100 ]; do
    /bin/sleep 0.05
    n=$((n + 1))
done
printf 'winid %s\n' "$*" >> "$MOCK_LOG"
if [ "${2:-}" = front ]; then
    [ -z "${MOCK_FRONT_FAILS:-}" ] || exit 1
    exit 0
fi
printf '7\t0\t1\n'
MOCK_WINID
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
# caffeinate holds the display for as long as it runs, which a stand-in does
# by standing: it logs its arguments, says it is up ($MOCK_AWAKE, beside the
# stand-in window as caffeinate.up), and says it was stopped ($MOCK_STOPPED)
# when the script ends it, as the script must; left alone it ends by itself
# after 30 seconds, and says nothing. An absolute sleep: PATH's is stubbed.
cat > "$tmp/bin/caffeinate" <<'MOCK_CAFFEINATE'
#!/bin/sh
printf 'caffeinate %s\n' "$*" >> "$MOCK_LOG"
/bin/sleep 30 &
child=$!
trap 'kill "$child" 2>/dev/null; : > "$MOCK_STOPPED"; exit 0' TERM
: > "$MOCK_AWAKE"
wait "$child"
MOCK_CAFFEINATE
chmod +x "$tmp/bin/swiftc" "$tmp/bin/screencapture" "$tmp/bin/sleep" "$tmp/bin/caffeinate"

fail() {
    echo "test-shot-mac.sh: $1" >&2
    exit 1
}

export MOCK_AWAKE="$tmp/bin/caffeinate.up"
export MOCK_STOPPED="$tmp/bin/caffeinate.stopped"
run_script() {
    rm -f "$MOCK_AWAKE" "$MOCK_STOPPED"
    PATH="$tmp/bin:$PATH" "$script" "$@"
}

# A shot of <state> in <look> into <file> against the stand-in that stands,
# its last report cleared first and waited for.
export MOCK_UP="$tmp/bin/footplate-shoot.up"
shoot() {
    rm -f "$tmp/bin/footplate-shoot.log" "$MOCK_UP"
    run_script "$1" "$2" "$3" "$tmp/bin/footplate-shoot" "${4:-1}"
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
[ ! -e "$MOCK_LOG" ] || fail "an unknown state started something: $(cat "$MOCK_LOG")"

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
[ ! -e "$MOCK_LOG" ] || fail "a bad look started something: $(cat "$MOCK_LOG")"

# What a launched window must have seen: its state, its look, in the
# settings it is to open from, and none of the caller's world. A yard at an
# address opens from the settings the script writes, with the look in the
# same file; a state that writes its own settings (src/shot.rs) finds none
# standing, since it refuses one, and takes the look from SHOT_LOOK; the
# rest open from the look alone.
expect_launch() {
    what=$1
    log=$2
    state=$3
    look=$4
    [ -f "$log" ] || fail "$what launched no window"
    grep -F -q "FOOTPLATE_SHOT=$state" "$log" \
        || fail "$what started footplate without FOOTPLATE_SHOT='$state'"
    grep -F -q "SHOT_LOOK=$look" "$log" \
        || fail "$what did not pass the look in SHOT_LOOK: $(cat "$log")"
    case "$state" in
        far|settings-yards-rename)
            settings="{\"yards\": [{\"name\": \"far\", \"address\": \"https://yard.example.net\", \"token_file\": \"far.token\"}], \"open\": \"far\", \"look\": \"$look\"}"
            ;;
        away|switcher|signedout) settings="(none)" ;;
        *) settings="{\"look\": \"$look\"}" ;;
    esac
    grep -F -q "settings: $settings" "$log" \
        || fail "$what did not write the settings its state opens from: $(cat "$log")"
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

# A run holds the display awake while it stands, before it does anything the
# display is needed for, and lets go of it when it ends, by whatever way: the
# stand-in caffeinate of the run just ended was stopped by the script. -u
# wakes a display that sleeps, -d keeps it from sleeping again, and -w ends
# the hold with the script where its trap cannot run.
expect_display_held() {
    what=$1
    log=$(cat "$MOCK_LOG" 2>/dev/null || true)
    [ "$(printf '%s\n' "$log" | grep -c '^caffeinate ')" = 1 ] \
        || fail "$what did not start caffeinate once: $log"
    printf '%s\n' "$log" | grep -E -q '^caffeinate -d -u -w [0-9]+$' \
        || fail "$what held the display awake awry: $log"
    [ -e "$MOCK_STOPPED" ] || fail "$what left the display held awake"
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
expect_display_held "hold"

# A shot takes the window the lookup names and writes where it is told,
# after the seconds it was given. The window is found, then brought forward
# (so that it is the key one), then waited for, then taken.
export MOCK_LOG="$tmp/log-shoot"
export YARDR_BEAD=sentinel-from-the-test
export YARDR_SESSION=sentinel-from-the-test
export YARDR_ACTOR=sentinel-from-the-test
set +e
out=$(shoot yardmaster dark "$tmp/shot-mac.png" 1)
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
expect_display_held "a shot"
[ "$(grep -n '^caffeinate ' "$MOCK_LOG" | cut -d: -f1)" = 1 ] \
    || fail "a shot did something before it held the display awake: $(cat "$MOCK_LOG")"
[ "$(grep -c -E '^winid [0-9]+ front$' "$MOCK_LOG")" = 1 ] \
    || fail "a shot did not bring its window forward once: $(cat "$MOCK_LOG")"
[ "$(grep -n -E '^winid [0-9]+$' "$MOCK_LOG" | head -n 1 | cut -d: -f1)" -lt \
    "$(grep -n ' front$' "$MOCK_LOG" | cut -d: -f1)" ] \
    || fail "a shot brought its window forward before it found it: $(cat "$MOCK_LOG")"
[ "$(grep -n ' front$' "$MOCK_LOG" | cut -d: -f1)" -lt "$(grep -n '^sleep ' "$MOCK_LOG" | cut -d: -f1)" ] \
    || fail "a shot brought its window forward after the wait: $(cat "$MOCK_LOG")"
[ "$(grep -n '^sleep ' "$MOCK_LOG" | cut -d: -f1)" -lt "$(grep -n '^screencapture ' "$MOCK_LOG" | cut -d: -f1)" ] \
    || fail "a shot took its picture before the wait: $(cat "$MOCK_LOG")"

# A window that cannot be brought forward is no shot: its traffic lights
# would be grey. A locked screen is how that happens.
export MOCK_LOG="$tmp/log-front-fails"
export MOCK_FRONT_FAILS=1
set +e
err=$(shoot yardmaster dark "$tmp/shot-locked.png" 1 2>&1)
code=$?
set -e
unset MOCK_FRONT_FAILS
[ "$code" = 1 ] || fail "a window not brought forward exited $code: $err"
case "$err" in
    *"did not come to the front"*"locked"*) ;;
    *) fail "a window not brought forward said: $err" ;;
esac
[ ! -e "$tmp/shot-locked.png" ] || fail "a window not brought forward was shot anyway"
if grep -q '^screencapture ' "$MOCK_LOG"; then
    fail "a window not brought forward was captured: $(cat "$MOCK_LOG")"
fi
expect_display_held "a window not brought forward"

# A relative file lands under the directory the script is run from, not
# the checkout it shoots from.
export MOCK_LOG="$tmp/log-relative"
set +e
out=$(cd "$tmp/cwd" && shoot crew light rel-shot.png 2)
code=$?
set -e
[ "$code" = 0 ] || fail "a relative file exited $code"
[ "$out" = "$tmp/cwd/rel-shot.png" ] || fail "a relative file printed $out"
[ -f "$tmp/cwd/rel-shot.png" ] || fail "a relative file wrote no screenshot"
[ ! -e "$script_dir/../rel-shot.png" ] || fail "a relative file landed in the checkout"

# Each state opens from the settings it wants, in both looks. weighty-mapping
# is a state like the others, and the states that write their own settings
# are shot like them: they find no file standing, and take the look from
# SHOT_LOOK.
for state in yardmaster weighty-mapping far settings-yards-rename away switcher signedout; do
    for look in light dark; do
        export MOCK_LOG="$tmp/log-$state-$look"
        set +e
        out=$(shoot "$state" "$look" "$tmp/shot-$state-$look.png" 1 2>&1)
        code=$?
        set -e
        [ "$code" = 0 ] || fail "$state in $look exited $code: $out"
        [ -f "$tmp/shot-$state-$look.png" ] || fail "$state in $look wrote no screenshot"
        expect_launch "$state in $look" "$tmp/bin/footplate-shoot.log" "$state" "$look"
    done
done

echo "test-shot-mac.sh: pass"
