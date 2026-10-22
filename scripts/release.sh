#!/bin/sh
# Builds everything a footplate release carries, on this Mac: the two macOS
# zips, one per darwin target, each holding the Footplate.app bundle, and
# the two Linux archives, one per arch, each holding the binary, README.md,
# LICENSE and NOTICE, with one checksums.txt (sha256) over the four.
# Run it from anywhere: `scripts/release.sh <version> [out-dir]`.
# <version> is the tag (v0.4.0) and must equal Cargo.toml's version, and the
# tree must be clean, else the script refuses. The archives go to out-dir,
# else to dist/ under the checkout. The Linux builds run in the shot's
# Debian container (scripts/linux-container.sh): arm64 natively, amd64
# emulated, which takes a while the first time.
#
# FOOTPLATE_RELEASE_SIGN_IDENTITY and FOOTPLATE_RELEASE_NOTARY_PROFILE, set
# together, sign each macOS app with that "Developer ID Application"
# identity of the keychain (scripts/bundle-mac.sh), have Apple notarize it
# under that notarytool keychain profile, and staple the ticket to it before
# it is zipped for the release (README.md, "Building"). With neither, the
# apps are signed ad hoc and not notarized. They are named after yardr's
# YARDR_RELEASE_SIGN_IDENTITY and YARDR_RELEASE_NOTARY_PROFILE, and take the
# same values.
set -e

caller=$PWD
if [ $# -ne 1 ] && [ $# -ne 2 ]; then
    echo "release.sh: usage: scripts/release.sh <version> [out-dir]" >&2
    exit 2
fi
version=$1
out=${2:-dist}
case "$out" in
    /*) ;;
    *) out=$caller/$out ;;
esac
cd "$(dirname "$0")/.."
root=$PWD
. ./scripts/linux-container.sh

number=${version#v}
toml=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
if [ "$number" != "$toml" ]; then
    echo "release.sh: $version is not Cargo.toml's version (v$toml)" >&2
    exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
    echo "release.sh: the tree is dirty; commit or stash before a release" >&2
    exit 1
fi

# Signing is asked for whole or not at all, and refused here, before the
# first build: a signed app that is not notarized is one Gatekeeper refuses
# like an unsigned one, and a release that was meant to be signed must not
# come out ad hoc for a variable that was forgotten, as yardr's
# contrib/release/build has it.
identity=${FOOTPLATE_RELEASE_SIGN_IDENTITY:-}
profile=${FOOTPLATE_RELEASE_NOTARY_PROFILE:-}
if [ -n "$identity" ] && [ -z "$profile" ]; then
    echo "release.sh: FOOTPLATE_RELEASE_SIGN_IDENTITY is set and FOOTPLATE_RELEASE_NOTARY_PROFILE is not; set both to sign and notarize the macOS apps, or neither" >&2
    exit 2
fi
if [ -z "$identity" ] && [ -n "$profile" ]; then
    echo "release.sh: FOOTPLATE_RELEASE_NOTARY_PROFILE is set and FOOTPLATE_RELEASE_SIGN_IDENTITY is not; set both to sign and notarize the macOS apps, or neither" >&2
    exit 2
fi

# Both variables on another system are refused here too: codesign and
# notarytool are macOS's, and so is the whole release.
if [ "$(uname -s)" != Darwin ]; then
    echo "release.sh: a release is built on macOS" >&2
    exit 1
fi
for tool in cargo docker ditto shasum file; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "release.sh: $tool is not on PATH" >&2
        exit 1
    fi
done
if [ -z "$identity" ]; then
    echo "release.sh: the macOS apps are signed ad hoc and not notarized (FOOTPLATE_RELEASE_SIGN_IDENTITY and FOOTPLATE_RELEASE_NOTARY_PROFILE are not set)" >&2
fi

work=$(mktemp -d)
stage_out=$work/out
mkdir -p "$stage_out"
release_container=""
release_marker=""
# Each Linux container goes when the release ends, however it ends, and the
# staging with it.
cleanup() {
    if [ -n "$release_container" ] && [ -n "$release_marker" ]; then
        docker exec "$release_container" rm -f "$release_marker" >/dev/null 2>&1 || true
    fi
    if [ -n "$release_container" ]; then
        docker rm -f "$release_container" >/dev/null 2>&1 || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# A binary of the wrong arch is the failure this script risks: each is
# checked as file(1) sees it before it is packed.
expect_binary() {
    if ! file "$1" | grep -q -F "$2"; then
        echo "release.sh: $1 is no $2 binary: $(file "$1")" >&2
        exit 1
    fi
}

# The app as the zip a Mac unpacks: ditto keeps the bundle's modes and links,
# and leaves out the resource forks and extended attributes it would pack
# beside them. $1 is the directory the app stands in, $2 the zip.
zip_app() {
    (cd "$1" && ditto -c -k --keepParent --norsrc --noextattr Footplate.app "$2")
}

# The string field $1 of notarytool's JSON answer $2.
notary_field() {
    printf '%s\n' "$2" | sed -n 's/.*"'"$1"'" *: *"\([^"]*\)".*/\1/p' | head -n 1
}

# Has Apple notarize the signed app whose zip is $2, and staples the ticket to
# the app $3, so that it opens without asking Apple and the zip that ships
# holds it. $1 names the app in the messages.
notarize() {
    echo "release.sh: $1: signed; waiting for Apple's notary service (minutes, as a rule)" >&2
    # json, as the normal format is for reading: the id and the status are
    # taken from it. A refusal is a status, which notarytool need not exit
    # non-zero for, and some versions exit non-zero along with a finished
    # submission: the answer is read either way, so that a refusal still
    # gets its log.
    submit_exit=0
    answer=$(xcrun notarytool submit "$2" --keychain-profile "$profile" --wait --output-format json) || submit_exit=$?
    status=$(notary_field status "$answer")
    id=$(notary_field id "$answer")
    if [ "$submit_exit" -ne 0 ] || [ "$status" != Accepted ]; then
        echo "release.sh: $1: Apple did not notarize the app (status \"$status\", submission $id); its log:" >&2
        if [ -n "$id" ]; then
            xcrun notarytool log "$id" --keychain-profile "$profile" >&2 || true
        else
            echo "release.sh: $1: notarytool submit failed (exit $submit_exit)" >&2
            printf '%s\n' "$answer" >&2
        fi
        exit 1
    fi
    echo "release.sh: $1: notarized (submission $id)" >&2
    xcrun stapler staple "$3" >&2
}

# Both darwin targets are installed: a release builds each.
if command -v rustup >/dev/null 2>&1; then
    for triple in aarch64-apple-darwin x86_64-apple-darwin; do
        if ! rustup target list --installed 2>/dev/null | grep -q -F -x "$triple"; then
            echo "release.sh: installing rust target $triple" >&2
            rustup target add "$triple" >&2
        fi
    done
fi

for triple in aarch64-apple-darwin x86_64-apple-darwin; do
    case "$triple" in
        aarch64-*) arch=arm64 ;;
        *) arch=amd64 ;;
    esac
    echo "release.sh: macOS $arch: building ($triple)" >&2
    cargo build --release --locked --target "$triple" >&2
    binary=target/$triple/release/footplate
    case "$arch" in
        arm64) expect_binary "$binary" "Mach-O 64-bit executable arm64" ;;
        *) expect_binary "$binary" "Mach-O 64-bit executable x86_64" ;;
    esac
    echo "release.sh: macOS $arch: bundling Footplate.app" >&2
    scripts/bundle-mac.sh "$binary" "$work/mac-$arch/Footplate.app" >/dev/null
    zip=$stage_out/Footplate_${number}_darwin_${arch}.zip
    echo "release.sh: macOS $arch: zipping $zip" >&2
    zip_app "$work/mac-$arch" "$zip"
    if [ -n "$identity" ]; then
        # Apple takes the zip as it stands. The ticket it answers with goes to
        # the app, and the zip is made again from the stapled app, so that
        # the one that ships, and its line in checksums.txt, holds the ticket.
        notarize "macOS $arch" "$zip" "$work/mac-$arch/Footplate.app"
        echo "release.sh: macOS $arch: zipping the stapled app again" >&2
        zip_app "$work/mac-$arch" "$zip"
    fi
done

# One container per checkout and arch. The path hash keeps same-named
# worktrees' containers and build trees apart.
suffix=$(basename "$root")
suffix=$(printf '%s' "$suffix" | tr -c 'a-zA-Z0-9_.-' '-' | tr 'A-Z' 'a-z')
path_hash=$(printf '%s' "$root" | cksum | cut -d' ' -f1)
cargo_vol=footplate-shot-cargo
extra_bins=""
extra_pkgs=""
for arch in arm64 amd64; do
    case "$arch" in
        arm64)
            platform=""
            cargo_home=/cargo
            target_vol=footplate-shot-target
            ;;
        amd64)
            platform=linux/amd64
            # rustup's proxies and toolchains are arch-specific: one cargo
            # home for both would build the wrong arch, so each keeps its
            # own home in the shared volume, and the emulated build keeps
            # its own target volume besides, so its fingerprints never
            # answer for a native build.
            cargo_home=/cargo/amd64
            target_vol=footplate-release-target-amd64
            ;;
    esac
    # Its own tree in the shared volume, apart from the shot's: a release
    # must not contend with a shot building beside it. The prune helper
    # protects it while this build runs and drops it after a week of quiet.
    target_dir=/target/release-$suffix-$path_hash
    # The path hash keeps same-named worktrees' containers apart, just as
    # it keeps their build trees apart.
    name=footplate-release-$path_hash-$arch
    release_container=$name
    echo "release.sh: Linux $arch: provisioning the container" >&2
    linux_provision "$name"
    release_marker=/target/.shot-active/release-$suffix-$path_hash-$$
    docker exec "$name" sh /src/scripts/shot-linux-prune.sh \
        /target "$target_dir" "$release_marker" release.sh >&2
    echo "release.sh: Linux $arch: building (release; the first build takes a while, longer under emulation)" >&2
    docker exec "$name" sh -c "cd /src && $cargo_home/bin/cargo build --release --locked" >&2
    stage=$work/linux-$arch
    mkdir -p "$stage"
    docker cp "$name:$target_dir/release/footplate" "$stage/footplate"
    case "$arch" in
        arm64) expect_binary "$stage/footplate" "ARM aarch64" ;;
        *) expect_binary "$stage/footplate" "x86-64" ;;
    esac
    cp README.md LICENSE NOTICE "$stage/"
    archive=$stage_out/footplate_${number}_linux_${arch}.tar.gz
    echo "release.sh: Linux $arch: packing $archive" >&2
    (cd "$stage" && tar -czf "$archive" LICENSE NOTICE README.md footplate)
    docker exec "$name" rm -f "$release_marker" >/dev/null
    docker rm -f "$name" >/dev/null
    release_container=""
    release_marker=""
done

# The emulated build leaves the debian:13 image in the local store as
# amd64, under which the next native `docker run` would emulate with a
# warning: pull the host's own back. Past the builds, so a failure here
# warns and the release still stands.
docker pull -q debian:13 >&2 \
    || echo "release.sh: could not restore the native debian:13 image" >&2

(cd "$stage_out" && shasum -a 256 \
    "Footplate_${number}_darwin_arm64.zip" \
    "Footplate_${number}_darwin_amd64.zip" \
    "footplate_${number}_linux_arm64.tar.gz" \
    "footplate_${number}_linux_amd64.tar.gz" > checksums.txt)

mkdir -p "$out"
rm -f "$out/checksums.txt"
for f in \
        "Footplate_${number}_darwin_arm64.zip" \
        "Footplate_${number}_darwin_amd64.zip" \
        "footplate_${number}_linux_arm64.tar.gz" \
        "footplate_${number}_linux_amd64.tar.gz"; do
    mv "$stage_out/$f" "$out/$f"
done
mv "$stage_out/checksums.txt" "$out/checksums.txt"

for f in \
        "$out/Footplate_${number}_darwin_arm64.zip" \
        "$out/Footplate_${number}_darwin_amd64.zip" \
        "$out/footplate_${number}_linux_arm64.tar.gz" \
        "$out/footplate_${number}_linux_amd64.tar.gz" \
        "$out/checksums.txt"; do
    echo "$f"
done
