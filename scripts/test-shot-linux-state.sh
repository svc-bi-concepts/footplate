#!/bin/sh
# A state reaches footplate as FOOTPLATE_SHOT, and the picture still goes
# where it is told: `shot-linux.sh <state> [file]`, and with no state the
# first screen, as ever. An unknown state with a file is refused. A look or
# a text size asked for rides in the settings file the container's script
# writes, except for the states that write their own (src/shot.rs): those
# are given no file, and away and signedout refuse a text size.
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

# The shot script only needs Docker to answer whether a stale container is
# left and to report a visible window here. Stub it so this checks the real
# argument and cleanup behavior without provisioning a container or building
# Footplate; the run, rm and exec lines are logged, so this sees what a
# shot mounts and leaves, and what footplate would start with.
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
    echo "test-shot-linux-state.sh: $1" >&2
    exit 1
}

# The container's name, as the script under test derives it from the
# checkout: one container per checkout.
root=$(CDPATH= cd "$script_dir/.." && pwd)
suffix=$(basename "$root")
suffix=$(printf '%s' "$suffix" | tr -c 'a-zA-Z0-9_.-' '-' | tr 'A-Z' 'a-z')
name=footplate-shot-$suffix

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

# A shot mounts the two volumes all shots share and removes its container
# when it ends: the run line carries them, and the rm line is last.
expect_cleanup() {
    what=$1
    log=$2
    grep -F -q -- "run -d --name $name " "$log" \
        || fail "$what started no container of its own"
    grep -F -q -- "-v footplate-shot-cargo:/cargo -v footplate-shot-target:/target" "$log" \
        || fail "$what did not mount the shared volumes"
    [ "$(tail -n 1 "$log")" = "rm -f $name" ] \
        || fail "$what left its container behind"
}

export MOCK_LOG=$tmp/log-default
out=$(PATH="$tmp/bin:$PATH" "$script")
expect_shot "no arguments" "$out"
expect_launch "no arguments" "$MOCK_LOG" ""
expect_cleanup "no arguments" "$MOCK_LOG"

export MOCK_LOG=$tmp/log-run
out=$(PATH="$tmp/bin:$PATH" "$script" run)
expect_shot "a state alone" "$out"
expect_launch "a state alone" "$MOCK_LOG" "run"
expect_cleanup "a state alone" "$MOCK_LOG"

export MOCK_LOG=$tmp/log-plain
out=$(PATH="$tmp/bin:$PATH" "$script" plain "$tmp/shot-plain.png")
[ "$out" = "$tmp/shot-plain.png" ] || fail "a state with a file printed $out"
expect_shot "a state with a file" "$out"
expect_launch "a state with a file" "$MOCK_LOG" "plain"
expect_cleanup "a state with a file" "$MOCK_LOG"

export MOCK_LOG=$tmp/log-file
out=$(PATH="$tmp/bin:$PATH" "$script" "$tmp/shot-first.png")
[ "$out" = "$tmp/shot-first.png" ] || fail "a file alone printed $out"
expect_shot "a file alone" "$out"
expect_launch "a file alone" "$MOCK_LOG" ""
expect_cleanup "a file alone" "$MOCK_LOG"

# One argument that is no state is a file, as before the states.
export MOCK_LOG=$tmp/log-bogus-file
out=$(cd "$tmp/cwd" && PATH="$tmp/bin:$PATH" MOCK_LOG="$MOCK_LOG" "$script" bogus)
[ "$out" = "$tmp/cwd/bogus" ] || fail "a lone name printed $out"
[ -f "$tmp/cwd/bogus" ] || fail "a lone name wrote no screenshot"
expect_launch "a lone name" "$MOCK_LOG" ""
expect_cleanup "a lone name" "$MOCK_LOG"

# Two arguments name a state first: an unknown one is refused, before
# anything is written or any container is touched.
export MOCK_LOG=$tmp/log-bogus-state
if err=$(PATH="$tmp/bin:$PATH" "$script" bogus "$tmp/shot-bogus.png" 2>&1); then
    fail "an unknown state shot anyway: $err"
fi
case "$err" in
    *"no such state: bogus"*) ;;
    *) fail "an unknown state said: $err" ;;
esac
[ ! -e "$tmp/shot-bogus.png" ] || fail "an unknown state wrote its file"
[ ! -e "$tmp/log-bogus-state" ] || fail "an unknown state reached docker"

# settings_given <state> <look> <text>: shoots the state against the stub,
# with the look and the text size asked for (either may be empty), then runs
# the lines of the container's script that write the settings file against
# an empty directory, and prints what they wrote, or nothing where they
# wrote nothing. The stub runs none of that script, so this is where the
# rule of which states are given a file, and what is in it, is run.
settings_given() {
    st=$1
    lk=$2
    tx=$3
    key=$st-${lk:-nolook}-${tx:-notext}
    MOCK_LOG=$tmp/log-given-$key PATH="$tmp/bin:$PATH" SHOT_LOOK=$lk SHOT_TEXT=$tx \
        "$script" "$st" "$tmp/shot-given-$key.png" >/dev/null 2>&1 ||
        fail "$st at look [$lk] and text size [$tx] did not shoot"
    scratch=$tmp/scratch-$key
    mkdir -p "$scratch/config/footplate"
    sed -n '/^    shot_settings=""$/,/^    chown -R shot:shot /p' "$tmp/log-given-$key" |
        sed '$d' >"$scratch/section.sh"
    [ -s "$scratch/section.sh" ] || fail "the script run for $st holds no lines that write settings"
    env SHOT_LOOK="$lk" SHOT_TEXT="$tx" scratch="$scratch" sh "$scratch/section.sh"
    if [ -f "$scratch/config/footplate/settings.json" ]; then
        cat "$scratch/config/footplate/settings.json"
    fi
}

# A state with no settings of its own is given the look and the text size in
# a file, and far its yard with them: these prove that the lines run here
# write anything at all, which the rows after them rely on.
got=$(settings_given plain dark "")
[ "$got" = '{"look": "dark"}' ] || fail "plain at a look was given [$got]"
got=$(settings_given plain dark 16)
[ "$got" = '{"look": "dark", "text": 16}' ] || fail "plain at a look and a text size was given [$got]"
got=$(settings_given far dark "")
case "$got" in
    *'"open": "far"'*'"look": "dark"'*) ;;
    *) fail "far at a look was given [$got]" ;;
esac

# The states that write their own settings (src/shot.rs refuses a file that
# stands, with exit 2) are given none, with a look asked for or not: the
# switcher and the state that shuts it, which have long been so, and now
# away and signedout.
for st in switcher switcher-shut away signedout; do
    got=$(settings_given "$st" dark "")
    [ -z "$got" ] || fail "$st at a look was given a settings file: $got"
    got=$(settings_given "$st" "" "")
    [ -z "$got" ] || fail "$st with neither look nor text was given a settings file: $got"
done

# A text size asked of away or signedout, which have no text reader, is
# refused at once and in words, before anything is written or any container
# is touched; another state is not.
for st in away signedout; do
    export MOCK_LOG=$tmp/log-$st-text
    rc=0
    err=$(PATH="$tmp/bin:$PATH" SHOT_TEXT=16 "$script" "$st" "$tmp/shot-$st-text.png" 2>&1) || rc=$?
    [ "$rc" -eq 2 ] || fail "$st at a text size exited $rc, not 2: $err"
    case "$err" in
        "shot-linux.sh: SHOT_TEXT is not read for the $st state, which writes its own settings: shoot it at the usual text size") ;;
        *) fail "$st at a text size said: $err" ;;
    esac
    [ ! -e "$tmp/shot-$st-text.png" ] || fail "$st at a text size wrote its file"
    [ ! -e "$MOCK_LOG" ] || fail "$st at a text size reached docker"
done

echo "test-shot-linux-state.sh: pass"
