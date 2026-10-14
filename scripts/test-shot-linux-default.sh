#!/bin/sh
# Two shots without an argument must write to separate files.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/shot-linux.sh
tmp=$(mktemp -d)
first_dir=
second_dir=
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
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$tmp/bin"

# The shot script only needs Docker to report a running container and a
# visible window here. Stub it so this checks the real output-path behavior
# without provisioning a container or building Footplate.
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

run_shot() {
    PATH="$tmp/bin:$PATH" "$script"
}

first=$(run_shot)
second=$(run_shot)
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
