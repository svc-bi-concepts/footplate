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
    (cd "$work/mac-$arch" && ditto -c -k --keepParent --norsrc --noextattr Footplate.app "$zip")
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
