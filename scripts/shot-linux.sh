#!/bin/sh
# A picture of footplate's first screen, drawn on Linux with no display: a
# Debian container holds a headless sway with one fixed output, footplate
# runs in it against a throwaway home, and grim writes the PNG. gpui draws
# through Vulkan, and the container has no GPU, so footplate is pointed at
# Mesa's software driver (lavapipe) alone: the picture proves that is enough.
# Run it on the Mac, from anywhere: `scripts/shot-linux.sh [<state>] [output.png]`.
# A state opens the window against a fake yard serving it (src/shot.rs),
# in the debug build alone: `yardmaster` (the conversation with the
# yardmaster), `plain` (an open decision card of a plain ask), `page` (an
# open card whose note carries a page and an image), `run` (an
# open card of a run ask), `stopped` (the head with a stopped yard), `far`
# (a yard at an address, away), `settings` (the settings with the
# start-at-login switch), `reconnecting` (a running yard whose server
# is then stopped, with a draft in the composer), `refusal` (the
# first screen's way to a yard at an address, with a pairing's http
# address refused under its fields) and `undelivered` (the conversation
# with one of a linked yard whose mail was refused over the hour).
# With no state the first
# screen is shot, as ever.
# The switch's yardr answers canned JSON, so the head draws the same on
# every shot.
# The picture goes to the file named, else to footplate-shot.png under a
# fresh directory of its own; its path is printed at the end either way.
# Every run provisions a fresh container and builds footplate in it; the
# container is removed when the shot ends, so a shot leaves nothing behind
# but the picture. Cargo's home and the build tree live in two volumes
# every shot shares, so the first build takes minutes and later ones only
# rebuild what changed.
set -e

caller=$PWD
# The states src/shot.rs serves, kept as that file's own list has them: a
# state added there is added here, and refused here until it is.
states="yardmaster plain run stopped far settings reconnecting page refusal undelivered"
is_state() {
    case " $states " in
        *" $1 "*) return 0 ;;
        *) return 1 ;;
    esac
}
state=""
if [ $# -ge 2 ]; then
    if is_state "$1"; then
        state=$1
        shift
    else
        echo "shot-linux.sh: no such state: $1 ($states)" >&2
        exit 2
    fi
elif [ $# -eq 1 ] && is_state "$1"; then
    state=$1
    shift
fi
# No file: one under a fresh directory of its own. Every builder on
# this Mac shares /tmp, and under one fixed name there two shots at once
# would overwrite each other, and a note could carry another bead's picture.
# The path is printed at the end, as with an argument.
out=${1:-$(mktemp -d)/footplate-shot.png}
case "$out" in
    /*) ;;
    *) out=$caller/$out ;;
esac
cd "$(dirname "$0")/.."
root=$PWD
size=${SHOT_SIZE:-1280x800}
case "$size" in
    *[!0-9x]*|x*|*x|*x*x*)
        echo "shot-linux.sh: SHOT_SIZE must be WIDTHxHEIGHT using positive integers" >&2
        exit 2
        ;;
esac
width=${size%x*}
height=${size#*x}
case "$width" in *[1-9]*) ;; *)
    echo "shot-linux.sh: SHOT_SIZE must be WIDTHxHEIGHT using positive integers" >&2
    exit 2
    ;;
esac
case "$height" in *[1-9]*) ;; *)
    echo "shot-linux.sh: SHOT_SIZE must be WIDTHxHEIGHT using positive integers" >&2
    exit 2
    ;;
esac

if ! command -v docker >/dev/null 2>&1; then
    echo "shot-linux.sh: docker is not on PATH" >&2
    exit 1
fi

# One container per checkout, from its directory's name: two builders' shots
# never share one, and a container always looks at the checkout that made it.
# SHOT_CONTAINER names another one. The container is removed when the shot
# ends; cargo's home and the build tree live in two volumes every shot
# shares instead, so each build warms from the last one. Two shots at once
# share them safely: cargo locks the target dir.
suffix=$(basename "$root")
suffix=$(printf '%s' "$suffix" | tr -c 'a-zA-Z0-9_.-' '-' | tr 'A-Z' 'a-z')
name=${SHOT_CONTAINER:-footplate-shot-$suffix}
cargo_vol=footplate-shot-cargo
target_vol=footplate-shot-target

# The shot leaves nothing behind: the container goes when the script ends,
# however it ends.
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
trap 'exit 1' HUP INT TERM

have() {
    docker exec "$name" sh -c "command -v $1" >/dev/null 2>&1
}

# A container of the name may be left by a killed run: remove it before a
# new one starts.
if docker inspect "$name" >/dev/null 2>&1; then
    docker rm -f "$name" >/dev/null
fi
docker run -d --name "$name" --init \
    -v "$root:/src:ro" \
    -v "$cargo_vol:/cargo" -v "$target_vol:/target" \
    -e CARGO_HOME=/cargo -e RUSTUP_HOME=/cargo/rustup \
    -e CARGO_TARGET_DIR=/target \
    -e PATH=/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    debian:13 sleep infinity >/dev/null
echo "shot-linux.sh: container $name, volumes $cargo_vol $target_vol" >&2

# The checkout is mounted read-only: cargo reads it and writes the build to
# the target volume, so a shot never dirties the checkout it shoots.
# Missing tools are installed: the build's (cc, the libraries gpui builds
# against), the compositor's (sway, grim, seatd), Vulkan's (the loader, the
# software driver, vulkaninfo to say which driver answers) and a font, so
# the window's words are drawn as words and not as boxes.
if ! have sway || ! have grim || ! have seatd || ! have vulkaninfo \
        || ! have dbus-run-session || ! have cc \
        || ! docker exec "$name" test -x /cargo/bin/cargo; then
    docker exec "$name" sh -c 'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
        build-essential curl ca-certificates pkg-config \
        libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev \
        libwayland-dev libxcb1-dev libx11-dev libvulkan-dev \
        sway grim seatd mesa-vulkan-drivers libvulkan1 vulkan-tools \
        dbus fonts-dejavu-core >/tmp/apt.log 2>&1 || { tail -n 20 /tmp/apt.log >&2; exit 1; }' >&2
fi
if ! docker exec "$name" sh -c 'id shot' >/dev/null 2>&1; then
    docker exec "$name" useradd -m -s /bin/bash shot
fi
if ! have cargo; then
    docker exec "$name" sh -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable -q' >&2
fi

docker exec "$name" sh -c 'cd /src && /cargo/bin/cargo build --locked' >&2

# seatd lends the device-less session its seat; sway will not start without
# one.
docker exec "$name" sh -c 'mkdir -p /run/user/1000 /tmp/shots && chown shot:shot /run/user/1000 /tmp/shots && chmod 700 /run/user/1000
    if ! { [ -f /tmp/shots/seatd.pid ] && kill -0 "$(cat /tmp/shots/seatd.pid)" 2>/dev/null; }; then
        seatd -u shot -g shot >/tmp/shots/seatd.log 2>&1 & echo $! > /tmp/shots/seatd.pid
        sleep 1
    fi' >&2
docker exec "$name" sh -c "printf 'output HEADLESS-1 resolution %sx%s\nxwayland disable\n' '$width' '$height' > /tmp/shots/sway-config && chown shot:shot /tmp/shots/sway-config" >&2
if ! docker exec "$name" sh -c '{ [ -f /tmp/shots/sway.pid ] && kill -0 "$(cat /tmp/shots/sway.pid)" 2>/dev/null; }'; then
    docker exec "$name" sh -c 'rm -f /run/user/1000/sway-ipc.*.sock; su shot -c "WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 XDG_RUNTIME_DIR=/run/user/1000 sway -c /tmp/shots/sway-config >/tmp/shots/sway.log 2>&1 & echo \$!" > /tmp/shots/sway.pid' >&2
    sleep 5
fi

# The pidfile holds dbus-run-session's PID. Stop its Footplate child first.
docker exec "$name" sh /src/scripts/shot-linux-stop.sh /tmp/shots/footplate.pid >&2

# The window opens against a throwaway home, as the README's "Trying the
# setup without touching your yard" has it: nothing of this yard's socket or
# pairing is in reach. VK_ICD_FILENAMES leaves lavapipe as the only Vulkan
# driver, so the picture is drawn in software and says so. A state opens it
# against a fake yard serving that state instead, which the debug build
# alone reads; empty, the first screen is shot.
docker exec "$name" sh -c 'export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1
    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
    scratch=$(mktemp -d /tmp/shots/scratch.XXXXXX)
    mkdir -p "$scratch/home" "$scratch/config" "$scratch/data"
    chown -R shot:shot "$scratch"
    echo "$scratch" > /tmp/shots/last-scratch
    # The yardr of the shot, kept beside the settings: doctor and
    # status answer canned JSON, so the head draws the same on every
    # shot. Kept as version 99, the fetch keeps it: the site is asked for
    # the number of the latest alone, and where even that fails the kept
    # one is still the answer. Stopped says nothing runs; every other
    # state says the yard runs, and none starts at login.
    bin="$scratch/config/footplate/bin"
    mkdir -p "$bin"
    printf "%s\n" "#!/bin/sh" "# yardr for a shot alone: doctor and status answer canned JSON." "# Anything else is refused: a shot takes no click." "D=\$(dirname \"\$0\")" "case \"\$1\" in" "doctor) cat \"\$D/doctor.json\" ;;" "status) cat \"\$D/status.json\" ;;" "*) echo \"{\\\"error\\\":\\\"a shot takes no \$1\\\"}\"; exit 1 ;;" "esac" > "$bin/yardr"
    chmod +x "$bin/yardr"
    printf "%s\n" "v99.0.0" > "$bin/yardr.version"
    printf "%s\n" "{\"yard\": false, \"machine\": []}" > "$bin/doctor.json"
    case "'"$state"'" in stopped) up=false ;; *) up=true ;; esac
    printf "%s\n" "{\"home\": \"/tmp/shot-yard\", \"name\": \"shot\", \"operator\": \"Jo\", \"server\": {\"running\": $up, \"cli_build\": \"shot\"}, \"daemon\": {\"running\": $up, \"pid\": 0, \"service\": {\"supported\": true, \"manager\": \"systemd\", \"name\": \"dev.yardr.shot\", \"file\": \"/tmp/shot.service\", \"installed\": false, \"loaded\": false}}, \"herdr_login_service\": {\"supported\": true, \"manager\": \"systemd\", \"name\": \"dev.herdr.shot\", \"file\": \"/tmp/shot-herdr.service\", \"installed\": false, \"loaded\": false}, \"web\": {\"views\": [], \"service\": {\"supported\": true, \"manager\": \"systemd\", \"name\": \"dev.yardr.shot-web\", \"file\": \"/tmp/shot-web.service\", \"installed\": false, \"loaded\": false}}}" > "$bin/status.json"
    # A yard at an address opens from the settings: one far yard, open.
    if [ "'"$state"'" = far ]; then printf "%s\n" "{\"yards\": [{\"name\": \"far\", \"address\": \"https://yard.example.net\", \"token_file\": \"far.token\"}], \"open\": \"far\"}" > "$scratch/config/footplate/settings.json"; fi
    chown -R shot:shot "$scratch"
    su shot -c "HOME=$scratch/home XDG_CONFIG_HOME=$scratch/config XDG_DATA_HOME=$scratch/data YARDR_HOME=$scratch/yard PATH=$scratch/home/.local/bin:/usr/bin:/bin XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json FOOTPLATE_SHOT='"$state"' dbus-run-session -- /target/debug/footplate >$scratch/footplate.log 2>&1 & echo \$!" > /tmp/shots/footplate.pid' >&2

# The window is waited for, not slept for: sway's tree names it by app_id.
# Three seconds past that, the first paint has settled and grim shoots.
i=0
while [ "$i" -lt 45 ]; do
    if docker exec "$name" sh -c 'SWAYSOCK=$(ls /run/user/1000/sway-ipc.*.sock | head -n 1); su shot -c "XDG_RUNTIME_DIR=/run/user/1000 SWAYSOCK=$SWAYSOCK swaymsg -t get_tree"' 2>/dev/null | grep -q '"app_id"'; then
        break
    fi
    i=$((i + 1))
    sleep 2
done
if [ "$i" -ge 45 ]; then
    echo "shot-linux.sh: no window in 90 seconds; footplate's log:" >&2
    docker exec "$name" sh -c 'cat "$(cat /tmp/shots/last-scratch)/footplate.log"' >&2
    exit 1
fi
sleep 3
docker exec "$name" sh -c 'su shot -c "XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 grim /tmp/shots/footplate.png"' >&2
docker cp "$name:/tmp/shots/footplate.png" "$out"

echo "$out"
