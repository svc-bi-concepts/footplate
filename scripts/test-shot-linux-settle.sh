#!/bin/sh
# grim shoots until the frame settles: changing bytes are shot again,
# stable bytes are kept, and past ten rounds the last frame is kept with a
# warning and the shot still succeeds. Stubbed docker serves the frames, so
# this runs without a display or a container.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/shot-linux.sh
tmp=$(mktemp -d)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$tmp/bin"

# Stub docker: grim's shots are counted, and cmp answers whether the frame
# has settled yet, MOCK_SETTLE_AFTER shots in. Everything else answers as
# the state test's stub does.
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
            *WAYLAND_DISPLAY=wayland-1\ grim*)
                printf 'shot\n' >> "$MOCK_GRIM_LOG"
                ;;
            *cmp\ -s*)
                n=$(grep -c '' "$MOCK_GRIM_LOG" 2>/dev/null || true)
                if [ "$n" -ge "${MOCK_SETTLE_AFTER:-2}" ]; then
                    exit 0
                fi
                exit 1
                ;;
        esac
        exit 0
        ;;
    cp)
        printf 'cp %s\n' "$*" >> "$MOCK_LOG"
        printf 'mock screenshot\n' > "$2"
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
    echo "test-shot-linux-settle.sh: $1" >&2
    exit 1
}

# One shot of the weighty state: grim serves changing bytes until the
# settle-after shot, and the script must shoot want_shots times, write its
# file once, and warn only where told.
run_case() {
    tag=$1
    settle_after=$2
    want_shots=$3
    want_warning=$4
    : > "$tmp/grim-$tag"
    set +e
    got=$(MOCK_SETTLE_AFTER=$settle_after MOCK_GRIM_LOG=$tmp/grim-$tag MOCK_LOG=$tmp/log-$tag \
        PATH="$tmp/bin:$PATH" "$script" weighty "$tmp/shot-$tag.png" 2>"$tmp/stderr-$tag")
    code=$?
    set -e
    [ "$code" = 0 ] || fail "$tag exited $code"
    [ "$got" = "$tmp/shot-$tag.png" ] || fail "$tag printed $got"
    [ -f "$tmp/shot-$tag.png" ] || fail "$tag wrote no shot"
    shots=$(grep -c '' "$tmp/grim-$tag")
    [ "$shots" = "$want_shots" ] || fail "$tag shot $shots times, want $want_shots"
    if [ "$want_warning" = yes ]; then
        grep -F -q "frame never settled after 10 tries; look at the picture" "$tmp/stderr-$tag" \
            || fail "$tag warned nothing: $(cat "$tmp/stderr-$tag")"
    else
        if grep -F -q "frame never settled" "$tmp/stderr-$tag"; then
            fail "$tag warned on a settled frame: $(cat "$tmp/stderr-$tag")"
        fi
    fi
    [ "$(grep -c '^cp ' "$tmp/log-$tag")" = 1 ] \
        || fail "$tag copied $(grep -c '^cp ' "$tmp/log-$tag") times"
}

run_case settled-first 2 2 no
run_case settled-late 3 3 no
run_case never-settled 999 10 yes

echo "test-shot-linux-settle.sh: pass"
