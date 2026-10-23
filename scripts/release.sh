#!/bin/sh
# Builds everything a footplate release carries, on this Mac: the two macOS
# zips, one per darwin target, each holding the Footplate.app bundle, the two
# disk images (dmg) of the same apps, each holding the bundle and a link to
# /Applications to drag it onto, the two Linux archives, one per arch, each
# holding the binary, its desktop entry and icon (footplate.desktop,
# footplate.png), README.md, LICENSE and NOTICE, and the Windows zip, holding
# footplate.exe, README.md, LICENSE and NOTICE, with one checksums.txt
# (sha256) over the seven.
# No archive holds yardr, which footplate fetches itself at its first run:
# each is listed as it is made, the disk image mounted read-only for it, and
# a member whose path names yardr fails the release.
#
# The Windows zip is not built here. gpui compiles its shaders for Windows
# with fxc.exe of the Windows SDK, which a Mac does not have, so the release
# job of .github/workflows/windows.yml builds the zip on a Windows runner for
# every commit that is pushed, and keeps it as an artifact of the run. This
# script takes the zip of the run for the commit it releases (HEAD) with gh,
# logged in, and starts no run itself. Before it builds anything it refuses a
# commit that has no run, or whose run ended in anything but success. After
# the builds it waits for a run that is still going, 75 minutes at most
# (GitHub stops a run at 60). It lists the zip and checks its footplate.exe as
# it does the other archives. YARDR_RELEASE_REPO names the repository the run
# is in, owner/name, as it does for scripts/release-stage (default
# yardr-dev/footplate).
#
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
# it is zipped for the release; the disk image is made from the stapled app,
# and is signed, notarized and stapled in its turn (README.md, "Building").
# With neither, the apps are signed ad hoc and the images not signed, and
# nothing is notarized. The two variables are named after yardr's
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
for tool in cargo docker ditto hdiutil shasum file gh; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "release.sh: $tool is not on PATH" >&2
        exit 1
    fi
done
if [ -z "$identity" ]; then
    echo "release.sh: the macOS apps are signed ad hoc and not notarized; the disk images are neither signed nor notarized (FOOTPLATE_RELEASE_SIGN_IDENTITY and FOOTPLATE_RELEASE_NOTARY_PROFILE are not set)" >&2
fi

# The Windows zip, and the artifact of the workflow's run that holds it.
windows_zip=footplate_${number}_windows_amd64.zip
windows_artifact=footplate-windows-amd64
repo=${YARDR_RELEASE_REPO:-yardr-dev/footplate}
commit=$(git rev-parse HEAD)

# What a release carries, in the order checksums.txt lists it.
archives="Footplate_${number}_darwin_arm64.zip
Footplate_${number}_darwin_amd64.zip
Footplate_${number}_darwin_arm64.dmg
Footplate_${number}_darwin_amd64.dmg
footplate_${number}_linux_arm64.tar.gz
footplate_${number}_linux_amd64.tar.gz
$windows_zip"

work=$(mktemp -d)
stage_out=$work/out
mkdir -p "$stage_out"
release_container=""
release_marker=""
# The disk image, while it is mounted to be listed.
mounted=""
# The mounted image and each Linux container go when the release ends,
# however it ends, and the staging with them.
cleanup() {
    if [ -n "$mounted" ]; then
        hdiutil detach -force "$mounted" >/dev/null 2>&1 || true
    fi
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

# The app as the disk image a Mac mounts: the app and a link to /Applications
# beside it, for the drag that installs it. The image holds what the directory
# $1 holds, which is the app and nothing else, so the link is made there, once
# the zips are done with it; $2 is the image. The filesystem is named, as
# hdiutil's default is its own to change and a release should come out the
# same image on any Mac.
dmg_app() {
    ln -s /Applications "$1/Applications"
    hdiutil create -volname Footplate -srcfolder "$1" -fs HFS+ -format UDZO "$2" >&2
}

# The members of the archive $1 as whoever opens it finds them, a path to a
# line. tar reads the zip and the tarball alike. An image has to be mounted to
# be read: read-only, at a mount point of its own in the work dir, and let go
# before this returns (cleanup lets go of it, were the release to end here).
list_archive() {
    case "$1" in
        *.dmg)
            mounted=$(mktemp -d "$work/mount.XXXXXX")
            if ! hdiutil attach -readonly -nobrowse -noverify -noautoopen -mountpoint "$mounted" "$1" >/dev/null; then
                echo "release.sh: could not open $1 to list it" >&2
                exit 1
            fi
            if ! (cd "$mounted" && find . -mindepth 1 -print >"$work/members"); then
                echo "release.sh: could not list mounted image $1" >&2
                exit 1
            fi
            if ! sed 's|^\./||' "$work/members"; then
                echo "release.sh: could not read mounted image listing for $1" >&2
                exit 1
            fi
            if hdiutil detach "$mounted" >/dev/null 2>&1; then
                mounted=""
            elif hdiutil detach -force "$mounted" >/dev/null 2>&1; then
                mounted=""
            else
                echo "release.sh: could not detach mounted image $1" >&2
                exit 1
            fi
            ;;
        *) tar -tf "$1" ;;
    esac
}

# Fails the release when the archive $1, called $2 in the messages, holds a
# member whose path names yardr, in any case: footplate fetches yardr at its
# first run (README.md, "Installing") and carries none, and the listing is how
# that is known. A listing that is empty proves nothing, and fails too.
assert_no_yardr() {
    listing=$work/listing
    list_archive "$1" >"$listing" || {
        echo "release.sh: could not list $2" >&2
        exit 1
    }
    count=$(wc -l <"$listing" | tr -d ' ')
    if [ "$count" -eq 0 ]; then
        echo "release.sh: $2 lists nothing, which proves nothing" >&2
        exit 1
    fi
    stray=$(grep -i yardr "$listing" || true)
    if [ -n "$stray" ]; then
        echo "release.sh: $2 carries yardr, which a release leaves out; it holds:" >&2
        printf '%s\n' "$stray" >&2
        exit 1
    fi
    echo "release.sh: $2: lists $count members, none of yardr" >&2
}

# Has Apple notarize the signed file $2 and staples the ticket to $3, so that
# it opens without asking Apple. For an app, $2 is its zip and $3 the app, and
# the zip that ships is made again from it, to hold the ticket; for a disk
# image both are the image, which is the file that ships. $1 names what is
# notarized in the messages, as each arch has two submissions.
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
        echo "release.sh: $1: Apple did not notarize it (status \"$status\", submission $id); its log:" >&2
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

# The field $1, a string or a number, of the JSON $2 that gh prints.
gh_field() {
    printf '%s\n' "$2" | sed -n 's/.*"'"$1"'": *"\{0,1\}\([^",}]*\).*/\1/p' | head -n 1
}

# The newest run of the Windows workflow for the commit that is released, as
# $run_id, $run_status and $run_conclusion: all three are empty when GitHub
# knows no run of it, and the conclusion is empty while the run is going.
windows_look() {
    if ! runs=$(gh run list --repo "$repo" --workflow windows.yml --commit "$commit" --limit 1 \
        --json databaseId,status,conclusion 2>"$work/gh.err"); then
        echo "release.sh: could not ask GitHub for the Windows run of $commit (gh, logged in, is needed): $(cat "$work/gh.err")" >&2
        exit 1
    fi
    run_id=$(gh_field databaseId "$runs")
    run_status=$(gh_field status "$runs")
    run_conclusion=$(gh_field conclusion "$runs")
    run_url=https://github.com/$repo/actions/runs/$run_id
}

# Refuses the release for a commit without a Windows run, and for a run that is
# over and did not succeed. A run that is going passes: windows_fetch waits for
# it.
windows_check() {
    windows_look
    if [ -z "$run_id" ]; then
        echo "release.sh: GitHub has no run of windows.yml for $commit, so there is no Windows zip to release; push the commit (the workflow runs on every push), then release again" >&2
        exit 1
    fi
    if [ "$run_status" = completed ] && [ "$run_conclusion" != success ]; then
        echo "release.sh: the Windows run for $commit ended $run_conclusion ($run_url); fix Windows and push the fix, or run its failed jobs again (gh run rerun $run_id --failed), then release again" >&2
        exit 1
    fi
}

# Takes the Windows zip of the run into the stage dir, waiting for the run if it
# is going, and checks the zip as it will ship. It is named for this version,
# holds the four files yardr.dev's sync-release allows and nothing of yardr,
# and its footplate.exe is a Windows program for x86-64.
windows_fetch() {
    waited=0
    windows_check
    while [ "$run_status" != completed ]; do
        if [ "$waited" -ge 4500 ]; then
            echo "release.sh: the Windows run for $commit is still $run_status after 75 minutes ($run_url); look at it, then release again" >&2
            exit 1
        fi
        if [ "$waited" -eq 0 ]; then
            echo "release.sh: Windows: the run for $commit is $run_status ($run_url); waiting for it" >&2
        fi
        sleep 60
        waited=$((waited + 60))
        windows_check
    done
    echo "release.sh: Windows: taking $windows_zip from run $run_id" >&2
    mkdir -p "$work/windows"
    if ! gh run download "$run_id" --repo "$repo" --name "$windows_artifact" --dir "$work/windows" >"$work/gh.out" 2>&1; then
        echo "release.sh: could not take the Windows zip from run $run_id ($run_url); its artifact is kept for 30 days, and gh run rerun $run_id builds it again: $(cat "$work/gh.out")" >&2
        exit 1
    fi
    if [ ! -f "$work/windows/$windows_zip" ]; then
        echo "release.sh: the artifact of run $run_id ($run_url) holds no $windows_zip; it holds: $(find "$work/windows" -type f | sed 's|.*/||' | tr '\n' ' ')" >&2
        exit 1
    fi
    mv "$work/windows/$windows_zip" "$stage_out/$windows_zip"
    assert_no_yardr "$stage_out/$windows_zip" "Windows archive"
    held=$(tar -tf "$stage_out/$windows_zip" | LC_ALL=C sort | paste -s -d ' ' -)
    if [ "$held" != "LICENSE NOTICE README.md footplate.exe" ]; then
        echo "release.sh: the Windows archive holds: $held; it should hold LICENSE NOTICE README.md footplate.exe and nothing else" >&2
        exit 1
    fi
    tar -xf "$stage_out/$windows_zip" -C "$work/windows" footplate.exe
    expect_binary "$work/windows/footplate.exe" "PE32+ executable"
    expect_binary "$work/windows/footplate.exe" "x86-64"
}

# The zip is taken at the end, but its run is looked at now. A commit with no
# run, or whose run failed, is refused before the builds below, which take the
# better part of an hour, and not after them.
windows_check
echo "release.sh: Windows: run $run_id for $commit: $run_status${run_conclusion:+, $run_conclusion} ($run_url); its zip is taken after the builds" >&2

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
        notarize "macOS $arch app" "$zip" "$work/mac-$arch/Footplate.app"
        echo "release.sh: macOS $arch: zipping the stapled app again" >&2
        zip_app "$work/mac-$arch" "$zip"
    fi
    assert_no_yardr "$zip" "macOS $arch zip"
    # The image is made from the app as it ships, stapled or not.
    dmg=$stage_out/Footplate_${number}_darwin_${arch}.dmg
    echo "release.sh: macOS $arch: imaging $dmg" >&2
    dmg_app "$work/mac-$arch" "$dmg"
    if [ -n "$identity" ]; then
        # Apple takes an image only when its maker has signed it, and the
        # signature is checked here as the app's is, before the wait. The
        # ticket is stapled to the image itself, whose line in checksums.txt
        # is of it. The hardened runtime is for code, which an image is not.
        codesign --force --sign "$identity" --timestamp "$dmg" >&2
        codesign --verify --strict "$dmg" >&2
        notarize "macOS $arch disk image" "$dmg" "$dmg"
    fi
    # Listed last, as the file that ships, ticket and all.
    assert_no_yardr "$dmg" "macOS $arch disk image"
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
    cp README.md LICENSE NOTICE assets/linux/footplate.desktop "$stage/"
    # The icon the entry names, and the file the program itself installs
    # as that icon (src/taskbar.rs), byte for byte.
    cp assets/icon-512.png "$stage/footplate.png"
    archive=$stage_out/footplate_${number}_linux_${arch}.tar.gz
    echo "release.sh: Linux $arch: packing $archive" >&2
    (cd "$stage" && tar -czf "$archive" LICENSE NOTICE README.md footplate footplate.desktop footplate.png)
    assert_no_yardr "$archive" "Linux $arch archive"
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

windows_fetch

# The names have no spaces, so the list splits on its lines.
# shellcheck disable=SC2086
(cd "$stage_out" && shasum -a 256 $archives > checksums.txt)

mkdir -p "$out"
rm -f "$out/checksums.txt"
for f in $archives; do
    mv "$stage_out/$f" "$out/$f"
done
mv "$stage_out/checksums.txt" "$out/checksums.txt"

for f in $archives checksums.txt; do
    echo "$out/$f"
done
