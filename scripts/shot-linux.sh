#!/bin/sh
# A picture of footplate's first screen, drawn on Linux with no display: a
# Debian container holds a headless sway with one fixed output, footplate
# runs in it against a throwaway home, and grim writes the PNG. gpui draws
# through Vulkan, and the container has no GPU, so footplate is pointed at
# Mesa's software driver (lavapipe) alone: the picture proves that is enough.
# Run it on the Mac, from anywhere: `scripts/shot-linux.sh [<state>] [output.png]`.
# A state opens the window against a fake yard serving it (src/shot.rs),
# in the debug build alone, or with no yard where the state says none:
# `yardmaster` (the conversation with the
# yardmaster), `crew` (chat with the sidebar's groups full), `thread`
# (a long wrapped thread from alternating senders,
# the newest mail with an image), `column` (a long conversation over
# three days, grouped, with short links, a correction and a code block
# of one line), `column-hover` (that conversation with the pointer on
# the code block, a letter under its head), `column-copy` (that with the
# pointer on the block itself: its copy control shows beside the bar),
# `column-top` (that conversation scrolled to stand a letter's head at the
# top of the list, with the pointer on it: its bar of reactions stops at
# the list's top, clear of the Screen control in the header),
# `older-unread` (the conversation of 120 mails, ninety of them
# unread and forty of those older than its newest fifty, opened: the
# window reads back to the oldest unread mail, so the yardmaster's row
# shows no count while the assembly's, not open, shows its three),
# `plain` (an open decision card of a plain ask), `plain-select`
# (that card with a selection posed over its context), `lines` (that
# card with one of jo's beads changed, so Lines carries its badge), `page` (an
# open card whose note carries a page, an image and a file), `mail` (a mail with
# a page and an image), `image` (that mail's image open full size over
# the window), `run` (an
# open card of a run ask), `teammates` (an open card of a teammate
# invite), `teammates-list` (the teammates view open on its row),
# `stopped` (a stopped yard), `far`
# (a yard at an address, away), `settings` (the settings on each of
# their sections: `settings` is this window, and `settings-appearance`,
# `settings-yards`, `settings-agents`, `settings-notifications`,
# `settings-connection`, `settings-voice` and `settings-telemetry` the
# rest, with `settings-telemetry-off` the telemetry section off, and
# `settings-voice-list` the voice section on, with a voice chosen and the
# list of voices open under its field, and `settings-voice-microphone` the
# same with a microphone chosen among a stand-in's and its list open),
# `voice` (the yardmaster's conversation with a voice session standing in
# it, against a stand-in for OpenAI's socket: the composer's button
# saying Stop, `listening` beside it, the session's minutes and the
# caption of what is said: two asks with a pause between them, held
# together), `voice-ready` and `voice-draft` (that
# conversation with voice on and no session: the composer's button
# saying Voice, and Send with a line typed), `settings-yards-rename` (the yards
# with the far yard's rename form open in its card), `reconnecting` (a
# running yard whose server
# is then stopped, with a draft in the composer, its Send held), `refusal` (the
# first screen's way to a yard at an address, with a pairing's http
# address refused under its fields), `undelivered` (the conversation
# with one of a linked yard whose mail was refused over the hour),
# `answered` (the first screen with a way's fields open after the yard
# answered, with the line that esc opens it), `setup` (step 2 of
# setting up a yard here, posed: a canned doctor of two providers and
# their model offers, and the working styles as a numbered scale),
# `setup-terminal` (the step that asks whether the line that puts
# `~/.local/bin` on the PATH goes in the shell's startup file, posed on zsh
# with yardr, herdr and claude in that directory),
# `hosted` (the first screen with the hosted way's fields open and
# filled), `signedout` (the away screen of a hosted yard that is
# signed out), `away` (the away screen of a yard at an address that
# no longer lets the device in, with a way's fields open),
# `arrivals` (the arrivals panel open on the arrivals
# row, with its lamps, headcodes and quiet lines),
# `departures` (the departures board open under the toggle, with its
# banner), `arrival` (one arrival's bead open from the panel), `flow`
# (yardr's line open on its flow), `old-lines` (the Lines pane against
# a yard too old to serve it, saying so in plain words), `grants` (the
# grants view open on its row, with a read, an update and an `--as`
# grant, and a source's named read call and update call), `pairing`
# (the Pair a phone pane open on its row, with a pairing's QR, code
# and minutes left), `secret` (an
# open card of a secret
# ask, posed with a value typed and hidden), `secret-shown` (that
# card where the yard marks the value no secret: typed and shown),
# `secret-refused` (that card with the yard's refusal of its answer),
# `switcher` (the switcher's list open over four yards, one away),
# `switcher-shut` (that list, shut a second after it drew as a click
# outside it shuts it: the script drives no pointer, so the state asks the
# window for what the click asks), `switcher-member` (that list over an own
# and a member hosted yard, the member marked with its operator's handle),
# `member-pair` (the settings' yards, with a member yard's pair this device
# pressed while the operator holds a pairing open: the edge names its yard
# user), `member-paired` (the same with none open: the device paired as the
# member's yard user),
# `refs` (the conversation naming an ask and two bead ids, the ask's
# chip with its preview under the mail), `decisions` (the decisions
# list with one open ask and the 156-item history open: day groups
# and a folded look-alike row), `decisions-filtered` (that history
# with `docs` in its filter), `keys` (the cheatsheet open over an
# open card), `about` (the About card open over the yardmaster's
# conversation, naming footplate's version and the kept yardr's),
# `about-missing` (that card with no yardr on the machine), `feedback` (the
# Feedback form open over the yardmaster's conversation, with a message
# typed, against a capture of this machine's that stands in for
# PostHog), `reactions` (the
# conversation with the parties' reactions under three mails),
# `reactions-hover` (that
# conversation with the bar of reactions to tap floating over one mail),
# `reactions-zoom` (that bar with the pointer on one emoji),
# `notice` (the conversation with the notice a flows view that did not
# answer leaves under the mail), `drawer` (the conversation with the
# bead its chip names open in the drawer beside it), `delivery` (the
# yard row open on a hook delivery whose photo the yard holds, with a
# document it did not take under the photo), `screen` (the
# yardmaster's screen open over the window, as its Screen control
# opens it), `screen-hosted` (that card on a hosted yard, saying
# the screen is read on the yard's machine, with no Screen offered)
# and `screen-refused` (that card on a yard that refuses the read in
# many words, which wrap inside it).
# With no state the first
# screen is shot, as ever.
# The switch's yardr answers canned JSON, so the status row draws the
# same on every shot, and a release's version to --version, which the
# About card shows.
# The picture goes to the file named, else to footplate-shot.png under a
# fresh directory of its own; its path is printed at the end either way.
# Every run provisions a fresh container and builds footplate in it; the
# container is removed when the shot ends, so a shot leaves nothing behind
# but the picture. Cargo's home lives in a volume every shot shares, and
# each checkout keeps its own build tree in another, so a checkout's first
# shot builds for minutes and its later ones only rebuild what changed. One
# shared build tree would shoot another checkout's binary: cargo takes a
# package as fresh from its sources' mtimes, not their contents.
set -e

caller=$PWD
# The states src/shot.rs serves, kept as that file's own list has them: a
# state added there is added here, and refused here until it is.
states="yardmaster crew thread column column-hover column-copy column-top older-unread plain plain-select lines run teammates teammates-list weighty weighty-mapping stopped far settings settings-appearance settings-yards settings-yards-rename settings-agents settings-notifications settings-connection settings-voice settings-voice-list settings-voice-microphone settings-telemetry settings-telemetry-off voice voice-ready voice-draft reconnecting page mail image refusal undelivered answered setup setup-terminal hosted signedout away arrivals departures arrival flow old-lines grants pairing secret secret-shown secret-refused switcher switcher-shut switcher-member member-pair member-paired refs decisions decisions-filtered keys about about-missing feedback reactions reactions-hover reactions-zoom notice drawer delivery screen screen-hosted screen-refused"
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
. ./scripts/linux-container.sh
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
# SHOT_LOOK draws the window in that look instead of the desktop's: light
# or dark, as the settings name them. Empty is the desktop's, as ever.
look=${SHOT_LOOK:-}
case "$look" in ""|light|dark) ;; *)
    echo "shot-linux.sh: SHOT_LOOK must be light or dark" >&2
    exit 2
    ;;
esac
# SHOT_TEXT draws the window at that text size, 11 to 20 as the settings
# name them. Empty is the usual 14, as ever.
text=${SHOT_TEXT:-}
case "$text" in
    ""|11|12|13|14|15|16|17|18|19|20) ;;
    *)
        echo "shot-linux.sh: SHOT_TEXT must be 11 to 20" >&2
        exit 2
        ;;
esac
# Away, signedout and screen-hosted write their own settings
# (src/shot.rs), which carry no text size, so one asked of them would
# be dropped without a word: it is refused here, before any container
# is touched.
case "$state" in
    away|signedout|screen-hosted)
        if [ -n "$text" ]; then
            echo "shot-linux.sh: SHOT_TEXT is not read for the $state state, which writes its own settings: shoot it at the usual text size" >&2
            exit 2
        fi
        ;;
esac
if ! command -v docker >/dev/null 2>&1; then
    echo "shot-linux.sh: docker is not on PATH" >&2
    exit 1
fi

# The build tree's name for a checkout's path: its directory's name through
# the container's alphabet, then a hash of the path, so two checkouts with
# the same directory name stay apart. One function for this shot's own tree
# and the live list below, so the two can never disagree.
tree_name() {
    _base=$(basename "$1")
    _base=$(printf '%s' "$_base" | tr -c 'a-zA-Z0-9_.-' '-' | tr 'A-Z' 'a-z')
    _hash=$(printf '%s' "$1" | cksum | cut -d' ' -f1)
    printf '%s-%s\n' "$_base" "$_hash"
}
# One container per checkout, from its directory's name: two builders' shots
# never share one, and a container always looks at the checkout that made it.
# SHOT_CONTAINER names another one. The container is removed when the shot
# ends; cargo's home lives in a volume every shot shares instead, so each
# build warms from the last one's registry, and the build trees live in
# another volume with one directory per checkout, so no shot builds or runs
# another checkout's binary.
suffix=$(basename "$root")
suffix=$(printf '%s' "$suffix" | tr -c 'a-zA-Z0-9_.-' '-' | tr 'A-Z' 'a-z')
name=${SHOT_CONTAINER:-footplate-shot-$suffix}
cargo_vol=footplate-shot-cargo
target_vol=footplate-shot-target
# The checkout's own build tree under the shared target volume. The hash of
# the checkout's path keeps two checkouts with the same directory name
# apart; only the registry and git checkouts in cargo's home are still
# shared, and cargo locks those.
tree=$(tree_name "$root")
target_dir=/target/$tree
target_marker=/target/.shot-active/$tree-$$
live_file=/target/.shot-live-$$

# The shot leaves nothing behind: the container goes when the script ends,
# however it ends.
cleanup() {
    # Let later shots prune this tree once this shot has finished using it.
    docker exec "$name" rm -f "$target_marker" >/dev/null 2>&1 || true
    docker exec "$name" rm -f "$live_file" >/dev/null 2>&1 || true
    docker rm -f "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# Beyond the build's own packages, which the provisioning brings itself,
# the shot wants the compositor's (sway, grim, seatd), Vulkan's (the
# loader, the software driver, vulkaninfo to say which driver answers) and
# fonts, so the window's words are drawn as words and not as boxes: one
# for text, and one for emoji, which mail and reactions carry.
platform=""
cargo_home=/cargo
extra_bins="sway grim seatd vulkaninfo dbus-run-session"
extra_pkgs="sway grim seatd mesa-vulkan-drivers libvulkan1 vulkan-tools dbus fonts-dejavu-core fonts-noto-color-emoji"
linux_provision "$name"
echo "shot-linux.sh: container $name, volumes $cargo_vol $target_vol" >&2

# Build trees of checkouts long gone are pruned before building: each costs
# GBs no shot will warm from. The depot's live checkouts go to the helper,
# so a removed checkout's tree goes at once; a live checkout's tree keeps
# the 7-day rule. Never this shot's own or another active shot's, and the
# helper says what went.
live_arg=""
if live_paths=$(git -C "$root" worktree list --porcelain 2>/dev/null); then
    live_names=$(printf '%s\n' "$live_paths" | sed -n 's/^worktree //p' | {
        while IFS= read -r live_checkout; do
            [ -n "$live_checkout" ] || continue
            live_tree=$(tree_name "$live_checkout")
            # A live checkout's release tree shares the volume
            # (scripts/release.sh): list it too, so a shot never drops a
            # release it warms from.
            printf '%s\nrelease-%s\n' "$live_tree" "$live_tree"
        done
        # This checkout runs this shot, so it is live even where git did
        # not list it.
        live_own=$(tree_name "$root")
        printf '%s\nrelease-%s\n' "$live_own" "$live_own"
    })
    if [ -n "$live_names" ] \
        && printf '%s\n' "$live_names" | docker exec -i "$name" sh -c "cat > $live_file" >/dev/null 2>&1; then
        live_arg=$live_file
    else
        echo "shot-linux.sh: could not write the live list; pruning as before" >&2
    fi
else
    echo "shot-linux.sh: could not list the depot's checkouts; pruning as before" >&2
fi
if [ -n "$live_arg" ]; then
    docker exec "$name" sh /src/scripts/shot-linux-prune.sh /target "$target_dir" "$target_marker" shot-linux.sh "$live_arg" >&2
    docker exec "$name" rm -f "$live_arg" >/dev/null 2>&1 || true
else
    docker exec "$name" sh /src/scripts/shot-linux-prune.sh /target "$target_dir" "$target_marker" >&2
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
docker exec -e "SHOT_LOOK=$look" -e "SHOT_TEXT=$text" "$name" sh -c 'export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1
    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
    scratch=$(mktemp -d /tmp/shots/scratch.XXXXXX)
    mkdir -p "$scratch/home" "$scratch/config" "$scratch/data"
    chown -R shot:shot "$scratch"
    echo "$scratch" > /tmp/shots/last-scratch
    # The yardr of the shot, where the installer puts it: doctor and
    # status answer canned JSON, so the status row draws the same on
    # every shot. Stopped says nothing runs; every other state says the
    # yard runs, and none starts at login.
    bin="$scratch/home/.local/bin"
    mkdir -p "$bin"
    printf "%s\n" "#!/bin/sh" "# yardr for a shot alone: doctor, status and settings answer canned JSON, and --version the version of a release." "# Anything else is refused: a shot takes no click." "D=\$(dirname \"\$0\")" "case \"\$1\" in" "doctor) cat \"\$D/doctor.json\" ;;" "status) cat \"\$D/status.json\" ;;" "--version) echo \"v0.14.0 (cbceebd22919)\" ;;" "settings) case \"\$2\" in get) cat \"\$D/settings-get.json\" ;; set) echo {} ;; *) echo \"{\\\"error\\\":\\\"a shot takes no settings \$2\\\"}\"; exit 1 ;; esac ;;" "*) echo \"{\\\"error\\\":\\\"a shot takes no \$1\\\"}\"; exit 1 ;;" "esac" > "$bin/yardr"
    chmod +x "$bin/yardr"
    printf "%s\n" "{\"yard\": false, \"machine\": []}" > "$bin/doctor.json"
    printf "%s\n" "[{\"key\": \"operator_style\", \"value\": \"show\", \"from\": \"set\"}, {\"key\": \"operator_note\", \"value\": \"I review every merge myself\", \"from\": \"set\"}]" > "$bin/settings-get.json"
    case "'"$state"'" in stopped|setup) up=false ;; *) up=true ;; esac
    printf "%s\n" "{\"home\": \"/tmp/shot-yard\", \"name\": \"shot\", \"operator\": \"Jo\", \"server\": {\"running\": $up, \"cli_build\": \"shot\"}, \"daemon\": {\"running\": $up, \"pid\": 0, \"service\": {\"supported\": true, \"manager\": \"systemd\", \"name\": \"dev.yardr.shot\", \"file\": \"/tmp/shot.service\", \"installed\": false, \"loaded\": false}}, \"herdr_login_service\": {\"supported\": true, \"manager\": \"systemd\", \"name\": \"dev.herdr.shot\", \"file\": \"/tmp/shot-herdr.service\", \"installed\": false, \"loaded\": false}, \"web\": {\"views\": [], \"service\": {\"supported\": true, \"manager\": \"systemd\", \"name\": \"dev.yardr.shot-web\", \"file\": \"/tmp/shot-web.service\", \"installed\": false, \"loaded\": false}}}" > "$bin/status.json"
    # A yard at an address opens from the settings: one far yard, open.
    # The rename shot opens that same far yard, with its form open in
    # its card. A look or a text size for the shot rides in the same
    # file where one is asked for. The switcher states, away, signedout
    # and screen-hosted write their own settings (src/shot.rs) and
    # refuse a file that stands, so none is written for them: the look
    # is theirs to read from SHOT_LOOK in the window environment, and a
    # text size was refused above.
    shot_settings=""
    if [ -n "$SHOT_LOOK" ]; then shot_settings="\"look\": \"$SHOT_LOOK\""; fi
    if [ -n "$SHOT_TEXT" ]; then
        if [ -n "$shot_settings" ]; then shot_settings="$shot_settings, "; fi
        shot_settings="$shot_settings\"text\": $SHOT_TEXT"
    fi
    if [ -n "$shot_settings" ]; then shot_json=", $shot_settings"; else shot_json=""; fi
    if [ "'"$state"'" = far ] || [ "'"$state"'" = settings-yards-rename ]; then printf "%s\n" "{\"yards\": [{\"name\": \"far\", \"address\": \"https://yard.example.net\", \"token_file\": \"far.token\"}], \"open\": \"far\"$shot_json}" > "$scratch/config/footplate/settings.json"; fi
    if [ -n "$shot_settings" ] && [ "'"$state"'" != far ] && [ "'"$state"'" != settings-yards-rename ] && [ "'"$state"'" != switcher ] && [ "'"$state"'" != switcher-shut ] && [ "'"$state"'" != switcher-member ] && [ "'"$state"'" != member-pair ] && [ "'"$state"'" != member-paired ] && [ "'"$state"'" != away ] && [ "'"$state"'" != signedout ] && [ "'"$state"'" != screen-hosted ]; then printf "%s\n" "{$shot_settings}" > "$scratch/config/footplate/settings.json"; fi
    chown -R shot:shot "$scratch"
    su shot -c "HOME=$scratch/home XDG_CONFIG_HOME=$scratch/config XDG_DATA_HOME=$scratch/data YARDR_HOME=$scratch/yard PATH=$scratch/home/.local/bin:/usr/bin:/bin XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json SHOT_LOOK='$SHOT_LOOK' FOOTPLATE_SHOT='"$state"' dbus-run-session -- '"$target_dir"'/debug/footplate >$scratch/footplate.log 2>&1 & echo \$!" > /tmp/shots/footplate.pid' >&2

# The window is waited for, not slept for: sway's tree names it by app_id.
# Three seconds past that, the first paint is up, and grim shoots until
# the frame settles.
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
# The first paint is not the settled frame: under load the card's bead
# read answers after grim's first shot, so shoot until two frames in a row
# agree (grim's bytes are deterministic for identical pixels: two shots of
# a settled window cmp equal). A round compares two frames; at most ten
# rounds, then the last frame is kept with a warning, since a loaded
# machine must still get a picture.
docker exec "$name" sh -c 'su shot -c "XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 grim /tmp/shots/footplate.png"' >&2
docker exec "$name" sh -c 'cp /tmp/shots/footplate.png /tmp/shots/prev.png' >&2
i=0
settled=0
while [ "$i" -lt 10 ]; do
    sleep 1
    docker exec "$name" sh -c 'su shot -c "XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 grim /tmp/shots/footplate.png"' >&2
    i=$((i + 1))
    if docker exec "$name" cmp -s /tmp/shots/prev.png /tmp/shots/footplate.png; then
        settled=1
        break
    fi
    docker exec "$name" sh -c 'cp /tmp/shots/footplate.png /tmp/shots/prev.png' >&2
done
if [ "$settled" -eq 0 ]; then
    echo "shot-linux.sh: frame never settled after 10 tries; look at the picture" >&2
fi
docker cp "$name:/tmp/shots/footplate.png" "$out"

echo "$out"
