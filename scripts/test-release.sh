#!/bin/sh
# The release script refuses a version that is not Cargo.toml's and a dirty
# tree, before it builds anything: nothing is written to the out dir.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/release.sh
lib=$script_dir/linux-container.sh
tmp=$(mktemp -d)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fail() {
    echo "test-release.sh: $1" >&2
    exit 1
}

if ! command -v git >/dev/null 2>&1; then
    echo "test-release.sh: requires git" >&2
    exit 77
fi

# A checkout of the script's own shape: the script reads Cargo.toml and the
# tree's state beside itself.
make_checkout() {
    mkdir -p "$1/scripts"
    cp "$script" "$1/scripts/release.sh"
    cp "$lib" "$1/scripts/linux-container.sh"
    chmod +x "$1/scripts/release.sh"
    printf '[package]\nname = "footplate"\nversion = "%s"\n' "$2" > "$1/Cargo.toml"
    (cd "$1" && git init -q && git add -A && git -c user.email=t@t -c user.name=t commit -qm init)
}

# A wrong version is refused from a clean tree.
make_checkout "$tmp/clean" "0.3.0"
if err=$("$tmp/clean/scripts/release.sh" v9.9.9 "$tmp/out-wrong" 2>&1); then
    fail "a wrong version released anyway"
fi
case "$err" in
    *"is not Cargo.toml's version"*) ;;
    *) fail "a wrong version said: $err" ;;
esac
[ ! -e "$tmp/out-wrong" ] || fail "a wrong version wrote its out dir"

# A dirty tree is refused for the right version, here an untracked file.
make_checkout "$tmp/dirty" "0.3.0"
echo note > "$tmp/dirty/note.txt"
if err=$("$tmp/dirty/scripts/release.sh" v0.3.0 "$tmp/out-dirty" 2>&1); then
    fail "a dirty tree released anyway"
fi
case "$err" in
    *"tree is dirty"*) ;;
    *) fail "a dirty tree said: $err" ;;
esac
[ ! -e "$tmp/out-dirty" ] || fail "a dirty tree wrote its out dir"

# Without a version the usage is printed.
if err=$("$tmp/clean/scripts/release.sh" 2>&1); then
    fail "no version released anyway"
fi
case "$err" in
    *"usage: scripts/release.sh"*) ;;
    *) fail "no version said: $err" ;;
esac

echo "test-release.sh: pass"
