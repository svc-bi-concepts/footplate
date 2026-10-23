#!/bin/sh
# The real footplate window on this Mac, isolated from the session it is run
# from: env -i with only what the run needs, its own home, config, data and
# temp in one short directory (the fake yard's socket lives there, and a unix
# socket path is capped at 104 bytes), the canned yardr of
# scripts/shot-linux.sh on PATH ahead of the system's, and a fake yard
# serving the state (src/shot.rs, a debug build's alone). Nothing of the
# caller's yard is in reach. What it started is stopped by pid, and its
# directory removed, when it ends. Run it on the Mac, from anywhere in the
# checkout.
#
#   scripts/shot-mac.sh <state> <light|dark> <out.png> [binary] [seconds]
#       takes a picture of the window with screencapture and quits it.
#   scripts/shot-mac.sh <state> <light|dark> hold [binary]
#       leaves the window up for trying by hand (drag it by its top,
#       double-click there); quit it with cmd+q.
#
#   state    a FOOTPLATE_SHOT state, as the README names them (yardmaster, crew, ...)
#   binary   default: target/debug/footplate under the checkout
#   seconds  how long the window stands before the picture (default 4)
#
# The display is woken if it sleeps and held awake for as long as this runs
# (caffeinate -d -u). On a Mac that asks for no password a sleeping display is
# what reads as a locked screen: the session says it is locked, with
# loginwindow in front, while the display is off, and says so no longer once
# it wakes. Where the session is still locked with the display awake (one that
# asks for a password), every window is occluded and gpui stops painting one,
# and no process can be brought forward, so the script stops with that said:
# the picture would be the first frame the window drew (the away screen), with
# the traffic lights grey. The window is brought to the front before the
# picture, key, so that its traffic lights are coloured: shoot one at a time,
# since a second shot would take the key window from the first.
set -eu

caller=$PWD
# The states src/shot.rs serves, kept as that file's own list has them: a
# state added there is added here, and refused here until it is. The same
# list as scripts/shot-linux.sh keeps.
states="yardmaster crew thread column plain plain-select lines run weighty weighty-mapping stopped far settings settings-appearance settings-yards settings-yards-rename settings-agents settings-notifications settings-connection reconnecting page mail image refusal undelivered answered setup hosted signedout away arrivals departures arrival flow old-lines grants secret secret-shown secret-refused switcher switcher-shut refs decisions decisions-filtered keys about about-unfetched reactions reactions-hover reactions-zoom notice drawer delivery"
is_state() {
    case " $states " in
        *" $1 "*) return 0 ;;
        *) return 1 ;;
    esac
}
if [ $# -lt 3 ]; then
    echo "shot-mac.sh: usage: shot-mac.sh <state> <light|dark> <out.png|hold> [binary] [seconds]" >&2
    exit 2
fi
state=$1
look=$2
out=$3
want_bin=${4:-}
wait_s=${5:-4}
if ! is_state "$state"; then
    echo "shot-mac.sh: no such state: $state ($states)" >&2
    exit 2
fi
case "$look" in light|dark) ;; *)
    echo "shot-mac.sh: the look must be light or dark" >&2
    exit 2
    ;;
esac
# No hold file: hold leaves the window up, it names no picture.
case "$out" in
    /*|hold) ;;
    *) out=$caller/$out ;;
esac
cd "$(dirname "$0")/.."
root=$PWD
bin=${want_bin:-$root/target/debug/footplate}

T=$(mktemp -d /tmp/fp-shot.XXXXXX)
pid=""
awake=""
cleanup() {
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    if [ -n "$awake" ]; then
        kill "$awake" 2>/dev/null || true
        wait "$awake" 2>/dev/null || true
    fi
    rm -rf "${T:?}"
}
trap cleanup EXIT INT TERM

# The display is woken if it sleeps and kept awake for as long as this runs:
# -u wakes it, -d keeps it from going to sleep again in the middle of a wait,
# and -w ends both with this script where the trap cannot run (kill -9).
caffeinate -d -u -w $$ &
awake=$!

mkdir -p "$T/home/.local/bin" "$T/config/footplate/bin" "$T/data" "$T/tmp" "$T/yard"
bin_dir="$T/config/footplate/bin"
cat > "$bin_dir/yardr" <<'EOF'
#!/bin/sh
# yardr for a shot alone: doctor, status and settings answer canned JSON,
# and --version the version of a release, which the About card shows. (Its
# version file says 99 so that the fetch keeps it; what it says to --version
# is a real release's.)
D=$(dirname "$0")
case "$1" in
doctor) cat "$D/doctor.json" ;;
status) cat "$D/status.json" ;;
--version) echo "v0.14.0 (cbceebd22919)" ;;
settings) case "$2" in get) cat "$D/settings-get.json" ;; set) echo {} ;; *) echo "{\"error\":\"a shot takes no settings $2\"}"; exit 1 ;; esac ;;
*) echo "{\"error\":\"a shot takes no $1\"}"; exit 1 ;;
esac
EOF
chmod +x "$bin_dir/yardr"
printf '%s\n' "v99.0.0" > "$bin_dir/yardr.version"
printf '%s\n' '{"yard": false, "machine": []}' > "$bin_dir/doctor.json"
printf '%s\n' '[{"key": "operator_style", "value": "show", "from": "set"}, {"key": "operator_note", "value": "I review every merge myself", "from": "set"}]' > "$bin_dir/settings-get.json"
printf '%s\n' '{"home": "/tmp/shot-yard", "name": "shot", "operator": "Jo", "server": {"running": true, "cli_build": "shot"}, "daemon": {"running": true, "pid": 0, "service": {"supported": true, "manager": "launchd", "name": "dev.yardr.shot", "file": "/tmp/shot.plist", "installed": false, "loaded": false}}, "herdr_login_service": {"supported": true, "manager": "launchd", "name": "dev.herdr.shot", "file": "/tmp/shot-herdr.plist", "installed": false, "loaded": false}, "web": {"views": [], "service": {"supported": true, "manager": "launchd", "name": "dev.yardr.shot-web", "file": "/tmp/shot-web.plist", "installed": false, "loaded": false}}}' > "$bin_dir/status.json"
# The look is the window's own setting, so the system's stays as it is. An
# absolute XDG_CONFIG_HOME wins over ~/Library on macOS. A yard at an address
# opens from the settings: one far yard, open, with the look in the same file,
# as scripts/shot-linux.sh writes it. A state that writes its own settings
# (src/shot.rs) gets no file here, since it refuses one that stands; it reads
# the look from SHOT_LOOK instead.
case "$state" in
    far|settings-yards-rename)
        printf '{"yards": [{"name": "far", "address": "https://yard.example.net", "token_file": "far.token"}], "open": "far", "look": "%s"}\n' "$look" > "$T/config/footplate/settings.json"
        ;;
    away|switcher|switcher-shut|signedout) ;;
    *)
        printf '{"look": "%s"}\n' "$look" > "$T/config/footplate/settings.json"
        ;;
esac

# The run sees nothing of the caller's: no YARDR_*, no agent socket, a PATH of
# its own directory and the system's. The fake yard knows the login "shot".
env -i \
    HOME="$T/home" \
    USER=shot \
    LOGNAME=shot \
    XDG_CONFIG_HOME="$T/config" \
    XDG_DATA_HOME="$T/data" \
    YARDR_HOME="$T/yard" \
    TMPDIR="$T/tmp" \
    PATH="$T/home/.local/bin:/usr/bin:/bin" \
    SHOT_LOOK="$look" \
    FOOTPLATE_SHOT="$state" \
    "$bin" > "$T/footplate.log" 2>&1 &
pid=$!

if [ "$out" = hold ]; then
    echo "shot-mac.sh: footplate (pid $pid) is up in $T; cmd+q quits it" >&2
    wait "$pid" || true
    pid=""
    exit 0
fi

# The window's number: its process's windows, which CoreGraphics lists
# without any permission. Onscreen reads 1. With "front" after the process it
# brings that process forward instead, which needs no permission either, so
# that its window is the key one: a window started from a shell is not, and a
# window that is not draws its traffic lights grey. Since macOS 14 a request
# to bring another process forward is dropped, though the call answers true,
# unless an application makes it, so the tool becomes one first. It exits 1
# when the process is not frontmost two seconds after.
cat > "$T/winid.swift" <<'EOF'
import Cocoa
guard CommandLine.arguments.count > 1, let pid = Int32(CommandLine.arguments[1]) else { exit(2) }
if CommandLine.arguments.count > 2 && CommandLine.arguments[2] == "front" {
    _ = NSApplication.shared
    guard let app = NSRunningApplication(processIdentifier: pid) else { exit(1) }
    _ = app.activate(options: [])
    for _ in 0..<20 {
        if app.isActive { exit(0) }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.1))
    }
    exit(1)
}
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
    print("\(w[kCGWindowNumber as String] ?? "?")\t\(w[kCGWindowLayer as String] ?? "?")\t\(w[kCGWindowIsOnscreen as String] ?? false)")
}
EOF
swiftc -O "$T/winid.swift" -o "$T/winid"

# The window is waited for, not slept for.
num=""
i=0
while [ "$i" -lt 120 ]; do
    if ! kill -0 "$pid" 2>/dev/null; then
        echo "shot-mac.sh: footplate exited; its log:" >&2
        cat "$T/footplate.log" >&2
        exit 1
    fi
    num=$("$T/winid" "$pid" | awk -F'\t' '$2 == 0 && ($3 == "1" || $3 == "true") { print $1; exit }')
    if [ -n "$num" ]; then
        break
    fi
    i=$((i + 1))
    sleep 0.5
done
if [ -z "$num" ]; then
    echo "shot-mac.sh: no window in 60 seconds; its log:" >&2
    cat "$T/footplate.log" >&2
    exit 1
fi
# Forward before the wait, so the window has drawn itself as the key one by
# the picture. Where it cannot be brought forward the picture would show its
# traffic lights grey, so there is none.
if ! "$T/winid" "$pid" front; then
    echo "shot-mac.sh: footplate did not come to the front, so its window is not the key one (is the screen locked?)" >&2
    exit 1
fi
sleep "$wait_s"
screencapture -l"$num" -o "$out"
echo "$out"
