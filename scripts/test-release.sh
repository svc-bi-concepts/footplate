#!/bin/sh
# The release script refuses a version that is not Cargo.toml's and a dirty
# tree, before it builds anything: nothing is written to the out dir. Its
# macOS half is run whole, on any system, against stand-ins for what a Mac
# and Apple do: the apps signed ad hoc and the disk images not signed with
# neither signing variable set, both signed, notarized and stapled with both,
# refused with one. The Linux archive's launcher is the repository's own, run
# as a desktop runs it; and every archive is listed as it ships, a yardr in
# the zip, the image, the Linux archive or the Windows zip refused. The Windows
# zip comes from the run of the Windows workflow for the commit, against a
# stand-in for gh. A run that succeeded gives it, a run that is going is waited
# for, a commit with no run gets one started, and a run that failed or was
# cancelled refuses the release, before any build when that is known by then.
# With FOOTPLATE_RELEASE_WINDOWS=off the Windows zip is left out instead: no
# run is started or asked about, and the out dir holds the six other archives.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/release.sh
lib=$script_dir/linux-container.sh
retry=$script_dir/codesign-retry.sh
bundle=$script_dir/bundle-mac.sh
desktop=$script_dir/../assets/linux/footplate.desktop
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
if ! command -v shasum >/dev/null 2>&1 && ! command -v sha256sum >/dev/null 2>&1; then
    echo "test-release.sh: requires shasum or sha256sum" >&2
    exit 77
fi

# A checkout of the script's own shape: the script reads Cargo.toml and the
# tree's state beside itself.
make_checkout() {
    mkdir -p "$1/scripts"
    cp "$script" "$1/scripts/release.sh"
    cp "$lib" "$1/scripts/linux-container.sh"
    cp "$retry" "$1/scripts/codesign-retry.sh"
    cp "$bundle" "$1/scripts/bundle-mac.sh"
    chmod +x "$1/scripts/release.sh" "$1/scripts/bundle-mac.sh"
    printf '[package]\nname = "footplate"\nversion = "%s"\n' "$2" > "$1/Cargo.toml"
    # What a Linux archive holds beside the binary: the launcher is the
    # repository's own, and the icon only stands in for the icon.
    for f in README.md LICENSE NOTICE; do
        echo "$f" > "$1/$f"
    done
    mkdir -p "$1/assets/linux"
    cp "$desktop" "$1/assets/linux/footplate.desktop"
    echo icon-512 > "$1/assets/icon-512.png"
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

# Stand-ins for what a release asks of a Mac and of Apple, on a PATH of
# their own: no test has a Developer ID or may call Apple, and the Linux half
# would need docker and a cargo build. Each writes its call to $CALLS, and
# the ones that make something leave what the real one leaves, so that a
# later call can be asked about it. uname makes any system a Mac, or the
# Linux a case asks for.
#
#   STANDIN_UNAME         what uname -s prints (Darwin)
#   STANDIN_STATUS        what the notary service answers (Accepted)
#   STANDIN_SUBMIT_EXIT   notarytool submit's exit status (0)
#   STANDIN_ONLY          the one kind of submission, zip or dmg, those two are
#                         for; the other is accepted (both are for each)
#   STANDIN_STRAY         a file of that name in the app's Resources, so in its
#                         zip and its image: yardr, smuggled in
#   STANDIN_DMG_STRAY     a member of that path in the image alone
#   STANDIN_TAR_STRAY     a file of that name in the Linux archive alone
#   STANDIN_ATTACH_FAILS  hdiutil attach fails, as it does for an image a Mac
#                         cannot open
#   STANDIN_DETACH_FAILS  hdiutil cannot let go of a mounted image
#   STANDIN_FIND_FAILS    find returns an incomplete image listing
#   STANDIN_RUNS          what gh says of the Windows run on each ask, one word
#                         for each, the last for those after: status:conclusion
#                         (completed:success, in_progress:), or none
#   STANDIN_GH_FAILS      gh cannot ask, as when it is not logged in
#   STANDIN_WORKFLOW_FAILS gh cannot start a run, as for a ref it refuses
#   STANDIN_NO_ARTIFACT   the run holds no artifact, as one that has expired
#   STANDIN_ZIP_VERSION   the version in the name of the zip the artifact holds
#   STANDIN_ZIP_STRAY     a file of that name in the Windows zip, besides its four
#   STANDIN_EXE           what the zip's footplate.exe is, as file below reads it
#   STANDIN_CODESIGN_FAIL_SKIP/COUNT
#                         the timestamped signings that fail with a missing
#                         secure timestamp, attempts SKIP+1 through SKIP+COUNT
#                         (0 and 0: none fail)
bin=$tmp/bin
mkdir -p "$bin"
REAL_FIND=$(command -v find)
export REAL_FIND
cat >"$bin/find" <<'EOF'
#!/bin/sh
case "$PWD" in
    */mount.*)
        if [ -n "${STANDIN_FIND_FAILS:-}" ]; then
            echo ./Footplate.app
            exit 1
        fi
        ;;
esac
exec "$REAL_FIND" "$@"
EOF
cat >"$bin/uname" <<'EOF'
#!/bin/sh
echo "${STANDIN_UNAME:-Darwin}"
EOF
# The binary of a target is a line naming it, which file below reads back.
cat >"$bin/cargo" <<'EOF'
#!/bin/sh
echo "cargo $*" >>"$CALLS"
prev=""
for a in "$@"; do
    if [ "$prev" = --target ]; then triple=$a; fi
    prev=$a
done
mkdir -p "target/$triple/release"
echo "$triple" >"target/$triple/release/footplate"
chmod +x "target/$triple/release/footplate"
EOF
cat >"$bin/rustup" <<'EOF'
#!/bin/sh
printf 'aarch64-apple-darwin\nx86_64-apple-darwin\n'
EOF
cat >"$bin/file" <<'EOF'
#!/bin/sh
case "$(cat "$1")" in
    aarch64-apple-darwin) echo "$1: Mach-O 64-bit executable arm64" ;;
    x86_64-apple-darwin) echo "$1: Mach-O 64-bit executable x86_64" ;;
    linux-arm64) echo "$1: ELF 64-bit LSB pie executable, ARM aarch64" ;;
    linux-amd64) echo "$1: ELF 64-bit LSB pie executable, x86-64" ;;
    windows-amd64) echo "$1: PE32+ executable (GUI) x86-64, for MS Windows" ;;
    *) echo "$1: data" ;;
esac
EOF
# The container has everything and no leftover of an earlier run; what is
# copied out of it is the Linux binary of the container's arch (its name
# ends with it).
cat >"$bin/docker" <<'EOF'
#!/bin/sh
case "$1" in
    inspect) exit 1 ;;
    cp)
        name=${2%%:*}
        echo "linux-${name##*-}" >"$3"
        ;;
esac
EOF
cat >"$bin/sips" <<'EOF'
#!/bin/sh
prev=""
for a in "$@"; do
    if [ "$prev" = --out ]; then echo icon >"$a"; fi
    prev=$a
done
EOF
cat >"$bin/iconutil" <<'EOF'
#!/bin/sh
prev=""
for a in "$@"; do
    if [ "$prev" = --output ]; then
        echo icns >"$a"
        [ -z "${STANDIN_STRAY:-}" ] || echo stray >"$(dirname "$a")/$STANDIN_STRAY"
    fi
    prev=$a
done
EOF
# The real tar, and with STANDIN_TAR_STRAY the Linux archive (the one made
# with -czf) is made with a file of that name, made beside the others, too.
REAL_TAR=$(command -v tar)
export REAL_TAR
cat >"$bin/tar" <<'EOF'
#!/bin/sh
if [ "$1" = -czf ] && [ -n "${STANDIN_TAR_STRAY:-}" ]; then
    echo stray >"$STANDIN_TAR_STRAY"
    exec "$REAL_TAR" "$@" "$STANDIN_TAR_STRAY"
fi
exec "$REAL_TAR" "$@"
EOF
# A signed app is one that holds the mark a signature leaves, and a signed
# image one with a signature line of its own (an image is a manifest, below);
# --verify asks for it, so that a check before the signing, or of another one,
# fails, as does signing what is not there.
cat >"$bin/codesign" <<'EOF'
#!/bin/sh
echo "codesign $*" >>"$CALLS"
for last; do :; done
[ -e "$last" ] || {
    echo "$last: No such file or directory" >&2
    exit 1
}
case " $* " in
    *" --verify "*)
        if [ -f "$last" ]; then
            grep -q -x 'signature: sealed' "$last"
        else
            [ -f "$last/Contents/_CodeSignature/CodeResources" ]
        fi || {
            echo "$last: code object is not signed at all" >&2
            exit 1
        }
        ;;
    *)
        # A timestamped signing alone fails as Apple's timestamp server
        # fails it, attempts SKIP+1 through SKIP+COUNT of the timestamped
        # signings in the calls, this one already logged among them; a
        # verify and an ad hoc signing never fail. A failed attempt seals
        # nothing, so only a signing tried again leaves its mark.
        case " $* " in
            *" --timestamp "*)
                n=$(grep -c '^codesign .*--timestamp' "$CALLS")
                if [ "$n" -gt "${STANDIN_CODESIGN_FAIL_SKIP:-0}" ] && [ "$n" -le "$((${STANDIN_CODESIGN_FAIL_SKIP:-0} + ${STANDIN_CODESIGN_FAIL_COUNT:-0}))" ]; then
                    echo "$last: A timestamp was expected but was not found" >&2
                    exit 1
                fi
                ;;
        esac
        if [ -f "$last" ]; then
            echo 'signature: sealed' >>"$last"
        else
            mkdir -p "$last/Contents/_CodeSignature"
            echo sealed >"$last/Contents/_CodeSignature/CodeResources"
        fi
        ;;
esac
EOF
# The image hdiutil makes is a manifest of what it holds, a line for each
# member (a link says where it points) after three lines for how it was made.
# codesign and stapler add a line to it, which is how notarytool below and a
# test ask what the image that ships has had done to it. Like the real one it
# will not write over an image that is there. Attached at an empty folder, the
# image is a volume of its members, as empty files and links; detached, the
# folder is empty again.
cat >"$bin/hdiutil" <<'EOF'
#!/bin/sh
echo "hdiutil $*" >>"$CALLS"
case "$1" in
    attach)
        shift
        for last; do :; done
        mnt=""
        while [ $# -gt 1 ]; do
            case "$1" in
                -mountpoint) mnt=$2; shift ;;
            esac
            shift
        done
        [ -z "${STANDIN_ATTACH_FAILS:-}" ] || {
            echo "hdiutil: attach failed - no mountable file systems" >&2
            exit 1
        }
        [ -f "$last" ] && [ -d "$mnt" ] && [ -z "$(ls -A "$mnt")" ] || {
            echo "hdiutil: attach failed - no image $last, or no empty folder $mnt" >&2
            exit 1
        }
        # Children before the members they are in, so that those are folders.
        grep -v -E '^(volname|fs|format|signature|ticket): ' "$last" | sort -r | while read -r member; do
            case "$member" in
                *" -> "*)
                    mkdir -p "$mnt/$(dirname "${member%% -> *}")"
                    ln -s "${member#* -> }" "$mnt/${member%% -> *}"
                    ;;
                *)
                    [ -e "$mnt/$member" ] || {
                        mkdir -p "$mnt/$(dirname "$member")"
                        : >"$mnt/$member"
                    }
                    ;;
            esac
        done
        printf '/dev/disk9\tApple_HFS\t%s\n' "$mnt"
        exit 0
        ;;
    detach)
        shift
        [ "$1" != -force ] || shift
        [ -z "${STANDIN_DETACH_FAILS:-}" ] || {
            echo "hdiutil: detach failed - the volume is busy" >&2
            exit 1
        }
        [ -d "$1" ] || {
            echo "hdiutil: detach failed - no such volume $1" >&2
            exit 1
        }
        find "$1" -mindepth 1 -delete
        exit 0
        ;;
    create) ;;
    *)
        echo "hdiutil: the stand-in makes images, mounts them and lets them go, and does no more" >&2
        exit 1
        ;;
esac
shift
for last; do :; done
while [ $# -gt 1 ]; do
    case "$1" in
        -volname) volname=$2; shift ;;
        -srcfolder) src=$2; shift ;;
        -fs) fs=$2; shift ;;
        -format) format=$2; shift ;;
    esac
    shift
done
[ -d "$src" ] || {
    echo "hdiutil: create failed - no such folder $src" >&2
    exit 1
}
[ ! -e "$last" ] || {
    echo "hdiutil: create failed - File exists" >&2
    exit 1
}
{
    echo "volname: $volname"
    echo "fs: $fs"
    echo "format: $format"
    (cd "$src" && find . -mindepth 1 | sort | while read -r member; do
        if [ -L "$member" ]; then
            echo "${member#./} -> $(readlink "$member")"
        else
            echo "${member#./}"
        fi
    done)
    [ -z "${STANDIN_DMG_STRAY:-}" ] || echo "$STANDIN_DMG_STRAY"
} >"$last"
EOF
# A tar of the app stands in for its zip, which a test can list.
cat >"$bin/ditto" <<'EOF'
#!/bin/sh
echo "ditto $*" >>"$CALLS"
app=""
zip=""
for a; do
    app=$zip
    zip=$a
done
tar -cf "$zip" "$app"
EOF
# The Windows run is gh's to say. `run list` answers as gh does with --json:
# the state in STANDIN_RUNS for this ask, the number of asks kept beside the
# calls, or none (an empty list). `workflow run` starts a run that the asks
# after it answer with, failing only as STANDIN_WORKFLOW_FAILS asks.
# `run download` leaves the artifact's zip in --dir, a tar of the four files
# standing in for it as ditto's does for the Mac's zips. sleep only says it
# was asked, so that a wait costs no time.
cat >"$bin/gh" <<'EOF'
#!/bin/sh
echo "gh $*" >>"$CALLS"
case "$1 $2" in
    "run list")
        [ -z "${STANDIN_GH_FAILS:-}" ] || {
            echo "To get started with GitHub CLI, please run:  gh auth login" >&2
            exit 1
        }
        asked=$(($(cat "$CALLS.asked" 2>/dev/null || echo 0) + 1))
        echo "$asked" >"$CALLS.asked"
        i=0
        for state in ${STANDIN_RUNS:-completed:success}; do
            i=$((i + 1))
            [ "$i" -lt "$asked" ] || break
        done
        if [ "$state" = none ]; then
            echo '[]'
        else
            printf '[{"conclusion":"%s","databaseId":4242,"status":"%s"}]\n' "${state#*:}" "${state%%:*}"
        fi
        ;;
    "workflow run")
        [ -z "${STANDIN_WORKFLOW_FAILS:-}" ] || {
            echo "could not create workflow dispatch event" >&2
            exit 1
        }
        ;;
    "run download")
        [ -z "${STANDIN_NO_ARTIFACT:-}" ] || {
            echo "no valid artifacts found to download" >&2
            exit 1
        }
        prev=""
        for a; do
            if [ "$prev" = --dir ]; then dir=$a; fi
            prev=$a
        done
        held=$(dirname "$CALLS")/zip-held
        mkdir -p "$held"
        echo "${STANDIN_EXE:-windows-amd64}" >"$held/footplate.exe"
        for f in README.md LICENSE NOTICE ${STANDIN_ZIP_STRAY:-}; do
            echo "$f" >"$held/$f"
        done
        (cd "$held" && tar -cf "$dir/footplate_${STANDIN_ZIP_VERSION:-0.3.0}_windows_amd64.zip" \
            footplate.exe README.md LICENSE NOTICE ${STANDIN_ZIP_STRAY:-})
        ;;
esac
EOF
cat >"$bin/sleep" <<'EOF'
#!/bin/sh
echo "sleep $*" >>"$CALLS"
EOF
# Apple is sent a signed app that has no ticket yet, in its zip, and answers
# with one: the ticket is the file a stapled app holds. It is sent a signed
# image of the stapled app and the link to drag it onto, with no ticket of its
# own yet, and answers with one for the image: the line a stapled image has.
cat >"$bin/xcrun" <<'EOF'
#!/bin/sh
echo "xcrun $*" >>"$CALLS"
case "$1 $2" in
    "notarytool submit")
        case "$3" in
            *.dmg)
                kind=dmg
                for held in 'signature: sealed' \
                    'Footplate.app/Contents/_CodeSignature/CodeResources' \
                    'Footplate.app/Contents/CodeResources' \
                    'Applications -> /Applications'; do
                    grep -q -x -F -- "$held" "$3" || {
                        echo "the submission is not of a signed image of the stapled app: no [$held]" >&2
                        exit 1
                    }
                done
                if grep -q -x 'ticket: stapled' "$3"; then
                    echo "the submission holds a ticket already" >&2
                    exit 1
                fi
                ;;
            *)
                kind=zip
                tar -tf "$3" | grep -q -x 'Footplate.app/Contents/_CodeSignature/CodeResources' || {
                    echo "the submission is not of a signed app" >&2
                    exit 1
                }
                if tar -tf "$3" | grep -q -x 'Footplate.app/Contents/CodeResources'; then
                    echo "the submission holds a ticket already" >&2
                    exit 1
                fi
                ;;
        esac
        status=Accepted
        exit_status=0
        if [ -z "${STANDIN_ONLY:-}" ] || [ "$STANDIN_ONLY" = "$kind" ]; then
            status=${STANDIN_STATUS:-Accepted}
            exit_status=${STANDIN_SUBMIT_EXIT:-0}
        fi
        echo '{"id":"2efe2717-52ef-43a5-96dc-0797e4ca1041","message":"Processing complete","path":"'"$3"'","status":"'"$status"'"}'
        exit "$exit_status"
        ;;
    "notarytool log")
        echo '{"issues":[{"message":"The signature of the binary is invalid."}]}'
        ;;
    "stapler staple")
        if [ -f "$3" ]; then
            echo 'ticket: stapled' >>"$3"
        else
            echo ticket >"$3/Contents/CodeResources"
        fi
        echo "The staple and validate action worked!"
        ;;
esac
EOF
chmod +x "$bin"/*
# The checksums are checked with the tool the release made them with.
if ! command -v shasum >/dev/null 2>&1; then
    cat >"$bin/shasum" <<'EOF'
#!/bin/sh
shift 2
exec sha256sum "$@"
EOF
    chmod +x "$bin/shasum"
fi

# release_run <case> <identity> <profile> [VAR=value ...]: release.sh v0.3.0
# whole, in a checkout and with an out dir of the case's own, the stand-ins
# first on PATH. $rc is its exit status, $text what it said, $out its out
# dir and $calls the stand-ins' log. The two signing variables are always
# given, empty for none, so that the caller's own never reach it, and
# FOOTPLATE_RELEASE_WINDOWS is empty unless a case gives it, for the same
# reason. With DETACH_HEAD set the checkout is left on no branch, for a
# release from a detached HEAD.
release_run() {
    name=$1
    sign_with=$2
    notary_with=$3
    shift 3
    make_checkout "$tmp/$name/checkout" "0.3.0"
    if [ -n "${DETACH_HEAD:-}" ]; then
        (cd "$tmp/$name/checkout" && git checkout -q --detach)
    fi
    out=$tmp/$name/out
    calls=$tmp/$name/calls
    if text=$(env PATH="$bin:$PATH" CALLS="$calls" \
        FOOTPLATE_RELEASE_SIGN_IDENTITY="$sign_with" \
        FOOTPLATE_RELEASE_NOTARY_PROFILE="$notary_with" \
        FOOTPLATE_RELEASE_WINDOWS= \
        "$@" "$tmp/$name/checkout/scripts/release.sh" v0.3.0 "$out" 2>&1); then
        rc=0
    else
        rc=$?
    fi
}

# want <text>: the stand-ins were called with it somewhere; want_line <regex>:
# some call is such a line; lack <text> and lack_line <regex>: none was, which
# holds when there is no log at all.
want() {
    grep -q -F -- "$1" "$calls" || fail "$name: no [$1] in the calls: $(cat "$calls" 2>&1)"
}
want_line() {
    grep -q -E -- "$1" "$calls" || fail "$name: no line like [$1] in the calls: $(cat "$calls" 2>&1)"
}
lack() {
    if grep -q -F -- "$1" "$calls" 2>/dev/null; then
        fail "$name: [$1] in the calls: $(cat "$calls")"
    fi
}
lack_line() {
    if grep -q -E -- "$1" "$calls" 2>/dev/null; then
        fail "$name: a line like [$1] in the calls: $(cat "$calls")"
    fi
}

# steps: the calls of the stand-ins that matter to the order of a release,
# one to three words each: what is done to a disk image (a file ending in
# .dmg) says so, so that the app's own calls and the image's can be told apart.
steps() {
    awk '
        $1 == "codesign" {
            what = ($NF ~ /\.dmg$/) ? " image" : ""
            print (($0 ~ / --verify /) ? "codesign verify" : "codesign sign") what
        }
        $1 == "ditto" { print "ditto" }
        $1 == "hdiutil" { print "hdiutil " $2 }
        $1 == "xcrun" { print $2 " " $3 (($4 ~ /\.dmg$/) ? " image" : "") }
    ' "$calls"
}

# in_zip <zip> <member>: whether the app that was zipped holds the member.
in_zip() {
    tar -tf "$1" | grep -q -x "Footplate.app/$2"
}

# in_dmg <dmg> <line>: whether the image's manifest holds the line, which
# names a member, or what was done to the image, or how it was made.
in_dmg() {
    grep -q -x -F -- "$2" "$1"
}

# archives_in_out [off]: the seven archives and the checksums of a release,
# and nothing else, and the checksums are of what is there. With off, the
# six without the Windows zip.
archives_in_out() {
    off=${1:-}
    set -- Footplate_0.3.0_darwin_arm64.zip Footplate_0.3.0_darwin_amd64.zip \
        Footplate_0.3.0_darwin_arm64.dmg Footplate_0.3.0_darwin_amd64.dmg \
        footplate_0.3.0_linux_arm64.tar.gz footplate_0.3.0_linux_amd64.tar.gz
    if [ "$off" = off ]; then
        want_n=6
    else
        set -- "$@" footplate_0.3.0_windows_amd64.zip
        want_n=7
    fi
    for f in "$@" checksums.txt; do
        [ -f "$out/$f" ] || fail "$name: no $f in the out dir: $(find "$out" -mindepth 1)"
    done
    [ "$(find "$out" -mindepth 1 | wc -l | tr -d ' ')" = $((want_n + 1)) ] || fail "$name: the out dir holds: $(find "$out" -mindepth 1)"
    (cd "$out" && shasum -a 256 -c checksums.txt >/dev/null) || fail "$name: checksums.txt is not of the archives"
    # And the checksums are of them all: none is left out of them.
    [ "$(wc -l <"$out/checksums.txt" | tr -d ' ')" = "$want_n" ] || fail "$name: checksums.txt is: $(cat "$out/checksums.txt")"
}

# refused <case> <exit> <says> <identity> <profile> [VAR=value ...]: the
# release is refused before anything is called or written, with that exit
# status and those words, and without repeating the identity or the profile,
# which name a key of the keychain.
refused() {
    case_name=$1
    want_rc=$2
    says=$3
    shift 3
    release_run "$case_name" "$@"
    [ "$rc" = "$want_rc" ] || fail "$name: exited $rc, not $want_rc: $text"
    case "$text" in
        *"$says"*) ;;
        *) fail "$name: it did not say [$says]: $text" ;;
    esac
    case "$text" in
        *"Stand In"* | *"stand-in-profile"*) fail "$name: the refusal repeats a value: $text" ;;
    esac
    [ ! -e "$calls" ] || fail "$name: a refused release called: $(cat "$calls")"
    [ ! -e "$out" ] || fail "$name: a refused release wrote its out dir"
}

signer="Developer ID Application: Stand In (ABCDE12345)"
notary=stand-in-profile
# The identity as a pattern, for the calls that want_line matches whole.
signer_re=$(printf '%s' "$signer" | sed 's/[][\.*^$()+?{|}]/\\&/g')

# image_holds <dmg>: the image is the one a drag installs from, made as a
# compressed read-only HFS+ volume named for the app, which holds the signed
# app and the link to /Applications to drag it onto.
image_holds() {
    [ -f "$1" ] || fail "$name: no image $1 in the out dir"
    for held in 'volname: Footplate' 'fs: HFS+' 'format: UDZO' Footplate.app \
        Footplate.app/Contents/_CodeSignature/CodeResources 'Applications -> /Applications'; do
        in_dmg "$1" "$held" || fail "$name: the image $1 does not hold [$held]: $(cat "$1")"
    done
}

# listed_clean [off]: the release said of each of its seven archives that it
# listed it and found no yardr in it, and let go of each image it mounted to
# do so. With off, the six without the Windows zip.
listed_clean() {
    if [ "${1:-}" = off ]; then
        set -- "macOS arm64 zip" "macOS amd64 zip" "macOS arm64 disk image" \
            "macOS amd64 disk image" "Linux arm64 archive" "Linux amd64 archive"
    else
        set -- "macOS arm64 zip" "macOS amd64 zip" "macOS arm64 disk image" \
            "macOS amd64 disk image" "Linux arm64 archive" "Linux amd64 archive" \
            "Windows archive"
    fi
    for what in "$@"; do
        case "$text" in
            *"$what: lists "*" members, none of yardr"*) ;;
            *) fail "$name: it did not say that the $what holds no yardr: $text" ;;
        esac
    done
    [ "$(grep -c '^hdiutil attach' "$calls")" = "$(grep -c '^hdiutil detach' "$calls")" ] ||
        fail "$name: an image was left mounted: $(cat "$calls")"
}

# linux_archive_holds <tar.gz>: the archive holds the binary, the three texts,
# the launcher and the icon, and nothing else. The icon is the checkout's. The
# launcher is an entry for the menu of applications (the program's own is not
# one), found by the window's id, naming that icon; and its Exec, run as a
# desktop runs it, with no PATH but the system's and the person's home for
# $HOME, starts the footplate that stands in ~/.local/bin.
linux_archive_holds() {
    members=$(tar -tzf "$1" | LC_ALL=C sort | tr '\n' ' ')
    [ "$members" = "LICENSE NOTICE README.md footplate footplate.desktop footplate.png " ] ||
        fail "$name: the archive $1 holds: $members"
    unpacked=$tmp/$name/unpacked-${1##*/}
    mkdir -p "$unpacked"
    tar -xzf "$1" -C "$unpacked"
    cmp -s "$unpacked/footplate.png" "$tmp/$name/checkout/assets/icon-512.png" ||
        fail "$name: the icon in $1 is not the checkout's"
    entry=$unpacked/footplate.desktop
    for held in '[Desktop Entry]' Type=Application Name='yardr footplate' \
        GenericName='Chat client' Keywords='footplate;yardr;yard;chat;' Icon=footplate \
        Terminal=false StartupWMClass=footplate; do
        grep -q -x -F -- "$held" "$entry" || fail "$name: the launcher in $1 lacks [$held]: $(cat "$entry")"
    done
    ! grep -q -i NoDisplay "$entry" || fail "$name: the launcher in $1 is hidden from the menu"
    [ "$(grep -c '^Exec=' "$entry")" = 1 ] || fail "$name: the launcher in $1 has not one Exec: $(cat "$entry")"
    # The entry format's escapes first, a pair of backslashes being one; what
    # is left is quoted as a shell reads it too.
    command=$(sed -n 's/^Exec=//p' "$entry" | sed 's/\\\\/\\/g')
    case "$command" in
        *'/.local/bin/footplate'*) ;;
        *) fail "$name: the Exec in $1 does not name the footplate in ~/.local/bin: $command" ;;
    esac
    home=$unpacked/home
    mkdir -p "$home/.local/bin"
    # shellcheck disable=SC2016 # the footplate expands $HOME when it runs
    printf '#!/bin/sh\necho started >"$HOME/started"\n' >"$home/.local/bin/footplate"
    chmod +x "$home/.local/bin/footplate"
    env -i HOME="$home" PATH=/usr/bin:/bin sh -c "$command" ||
        fail "$name: the Exec in $1 failed: $command"
    [ "$(cat "$home/started" 2>/dev/null)" = started ] ||
        fail "$name: the Exec in $1 did not start the footplate in ~/.local/bin: $command"
}

# windows_archive_holds <zip>: the archive holds the program and the three
# texts, at its root, and nothing else.
windows_archive_holds() {
    members=$(tar -tf "$1" | LC_ALL=C sort | tr '\n' ' ')
    [ "$members" = "LICENSE NOTICE README.md footplate.exe " ] ||
        fail "$name: the archive $1 holds: $members"
}

# windows_refused <case> <says> <built> [VAR=value ...]: the release is refused
# over the Windows zip or its run, with those words and nothing written to the
# out dir. With built no the refusal comes before the first build, which the
# calls show; with yes it comes after the builds, for a run that was going
# when they began.
windows_refused() {
    case_name=$1
    says=$2
    built=$3
    shift 3
    release_run "windows-$case_name" "" "" "$@"
    [ "$rc" = 1 ] || fail "$name: exited $rc, not 1: $text"
    case "$text" in
        *"$says"*) ;;
        *) fail "$name: it did not say [$says]: $text" ;;
    esac
    [ ! -e "$out" ] || fail "$name: a refused release wrote its out dir"
    if [ "$built" = yes ]; then
        want "cargo build --release --locked --target x86_64-apple-darwin"
    else
        lack "cargo"
        lack "hdiutil"
    fi
}

# stray_refused <case> <VAR=value> <archive> <member>: the knob puts a member
# named for yardr into the archive, in any case, and the release is refused:
# it says the archive carries yardr and names the member, writes nothing to the
# out dir, and lets go of every image it mounted.
stray_refused() {
    release_run "stray-$1" "" "" "$2"
    [ "$rc" = 1 ] || fail "$name: a release with yardr in the $3 exited $rc: $text"
    case "$text" in
        *"$3 carries yardr"*) ;;
        *) fail "$name: it did not say the $3 carries yardr: $text" ;;
    esac
    case "$text" in
        *"$4"*) ;;
        *) fail "$name: it did not name the member $4: $text" ;;
    esac
    [ ! -e "$out" ] || fail "$name: a release with yardr in it wrote its out dir"
    [ "$(grep -c '^hdiutil attach' "$calls")" = "$(grep -c '^hdiutil detach' "$calls")" ] ||
        fail "$name: an image was left mounted: $(cat "$calls")"
}

# With neither variable the apps are signed ad hoc, as they were, and Apple
# is not asked: no hardened runtime, no verify, no notary, no ticket, and
# each zip made once. An image is made of each app, and is not signed.
release_run adhoc "" ""
[ "$rc" = 0 ] || fail "adhoc: a release with neither variable failed: $text"
case "$text" in
    *"signed ad hoc and not notarized"*) ;;
    *) fail "adhoc: it did not say the apps are ad hoc: $text" ;;
esac
want_line '^codesign --force --sign - /.*/mac-arm64/Footplate\.app$'
want_line '^codesign --force --sign - /.*/mac-amd64/Footplate\.app$'
lack "--options"
lack "--timestamp"
lack "--verify"
lack "xcrun"
# The image is mounted to be listed once it is made, the file that ships.
one="codesign sign
ditto
hdiutil create
hdiutil attach
hdiutil detach"
[ "$(steps)" = "$(printf '%s\n%s' "$one" "$one")" ] ||
    fail "adhoc: the calls were: $(steps | tr '\n' ',')"
listed_clean
for arch in arm64 amd64; do
    zip=$out/Footplate_0.3.0_darwin_$arch.zip
    dmg=$out/Footplate_0.3.0_darwin_$arch.dmg
    in_zip "$zip" Contents/_CodeSignature/CodeResources || fail "adhoc: the $arch zip holds no signature"
    ! in_zip "$zip" Contents/CodeResources || fail "adhoc: the $arch zip holds a ticket"
    want_line "^hdiutil create -volname Footplate -srcfolder /.*/mac-$arch -fs HFS\+ -format UDZO /.*/Footplate_0\.3\.0_darwin_$arch\.dmg\$"
    image_holds "$dmg"
    ! in_dmg "$dmg" Footplate.app/Contents/CodeResources || fail "adhoc: the $arch image holds a ticket"
    ! in_dmg "$dmg" 'signature: sealed' || fail "adhoc: the $arch image is signed"
    ! in_dmg "$dmg" 'ticket: stapled' || fail "adhoc: the $arch image is stapled"
    linux_archive_holds "$out/footplate_0.3.0_linux_$arch.tar.gz"
done
archives_in_out
# The Windows zip is the run's for this commit, asked for before the builds and
# again after them, and downloaded from the run that succeeded; a run that is
# over is not waited for.
commit=$(git -C "$tmp/adhoc/checkout" rev-parse HEAD)
want_line "^gh run list --repo yardr-dev/footplate --workflow windows.yml --commit $commit --limit 1 --json databaseId,status,conclusion\$"
[ "$(grep -c '^gh run list ' "$calls")" = 2 ] || fail "adhoc: gh run list was asked: $(grep '^gh ' "$calls")"
want_line "^gh run download 4242 --repo yardr-dev/footplate --name footplate-windows-amd64 --dir /"
lack "sleep"
windows_archive_holds "$out/footplate_0.3.0_windows_amd64.zip"

# With both, each app is signed with the identity, hardened and timestamped,
# and the signature is checked; sent to Apple in a zip, waited for; stapled;
# and only then zipped for the release, so that the zip that ships holds the
# ticket and checksums.txt is of it. The image is made of the stapled app,
# signed with the identity (not hardened: an image is not code) and checked,
# sent to Apple itself, waited for, and stapled, so that the image that ships
# holds its own ticket and checksums.txt is of that too. (The stand-in for
# notarytool refuses a submission that is not of a signed app, or that holds
# a ticket already, and an image that is unsigned, that holds an app without
# its ticket or no link to /Applications, or that is stapled already.)
release_run signed "$signer" "$notary"
[ "$rc" = 0 ] || fail "signed: a release with both variables failed: $text"
case "$text" in
    *"signed ad hoc"*) fail "signed: it says the apps are ad hoc: $text" ;;
esac
want "codesign --force --options runtime --timestamp --sign $signer /"
want "codesign --verify --strict /"
want_line "^xcrun notarytool submit /.*/Footplate_0\.3\.0_darwin_arm64\.zip --keychain-profile $notary --wait --output-format json\$"
want_line "^xcrun notarytool submit /.*/Footplate_0\.3\.0_darwin_amd64\.zip --keychain-profile $notary --wait --output-format json\$"
want_line '^xcrun stapler staple /.*/mac-arm64/Footplate\.app$'
want_line '^xcrun stapler staple /.*/mac-amd64/Footplate\.app$'
lack "xcrun notarytool log"
one="codesign sign
codesign verify
ditto
notarytool submit
stapler staple
ditto
hdiutil create
codesign sign image
codesign verify image
notarytool submit image
stapler staple image
hdiutil attach
hdiutil detach"
[ "$(steps)" = "$(printf '%s\n%s' "$one" "$one")" ] ||
    fail "signed: the calls were: $(steps | tr '\n' ',')"
listed_clean
for arch in arm64 amd64; do
    zip=$out/Footplate_0.3.0_darwin_$arch.zip
    dmg=$out/Footplate_0.3.0_darwin_$arch.dmg
    in_zip "$zip" Contents/_CodeSignature/CodeResources || fail "signed: the $arch zip holds no signature"
    in_zip "$zip" Contents/CodeResources || fail "signed: the $arch zip holds no ticket"
    want_line "^hdiutil create -volname Footplate -srcfolder /.*/mac-$arch -fs HFS\+ -format UDZO /.*/Footplate_0\.3\.0_darwin_$arch\.dmg\$"
    want_line "^codesign --force --sign $signer_re --timestamp /.*/Footplate_0\.3\.0_darwin_$arch\.dmg\$"
    want_line "^codesign --verify --strict /.*/Footplate_0\.3\.0_darwin_$arch\.dmg\$"
    want_line "^xcrun notarytool submit /.*/Footplate_0\.3\.0_darwin_$arch\.dmg --keychain-profile $notary --wait --output-format json\$"
    want_line "^xcrun stapler staple /.*/Footplate_0\.3\.0_darwin_$arch\.dmg\$"
    image_holds "$dmg"
    in_dmg "$dmg" Footplate.app/Contents/CodeResources || fail "signed: the $arch image's app holds no ticket"
    in_dmg "$dmg" 'signature: sealed' || fail "signed: the $arch image is not signed"
    in_dmg "$dmg" 'ticket: stapled' || fail "signed: the $arch image holds no ticket"
done
lack_line '^codesign .*--options.* /.*\.dmg$'
archives_in_out

# A timestamp the server drops once is tried again: the arm64 app signs at
# the second try and the release passes whole, saying so.
release_run sign-retry-app "$signer" "$notary" FOOTPLATE_SIGN_RETRY_SLEEP=0 STANDIN_CODESIGN_FAIL_SKIP=0 STANDIN_CODESIGN_FAIL_COUNT=1
[ "$rc" = 0 ] || fail "$name: a release with one dropped timestamp failed: $text"
case "$text" in
    *"the timestamp was not found; trying again"*) ;;
    *) fail "$name: it did not say the timestamp was tried again: $text" ;;
esac
[ "$(grep -c -E "^codesign --force --options runtime --timestamp --sign $signer_re /.*/mac-arm64/Footplate\\.app\$" "$calls")" = 2 ] ||
    fail "$name: the arm64 app was signed: $(grep '^codesign' "$calls")"
archives_in_out

# A timestamp server that answers nothing fails the release after exactly 3
# signings of the arm64 app, with the tool's own words.
release_run sign-retry-dead "$signer" "$notary" FOOTPLATE_SIGN_RETRY_SLEEP=0 STANDIN_CODESIGN_FAIL_SKIP=0 STANDIN_CODESIGN_FAIL_COUNT=99
[ "$rc" = 1 ] || fail "$name: a release with no timestamp exited $rc: $text"
case "$text" in
    *"A timestamp was expected but was not found"*) ;;
    *) fail "$name: it did not name the missing timestamp: $text" ;;
esac
[ "$(grep -c -E "^codesign --force --options runtime --timestamp --sign $signer_re /.*/mac-arm64/Footplate\\.app\$" "$calls")" = 3 ] ||
    fail "$name: the arm64 app was signed: $(grep '^codesign' "$calls")"
lack_line '^codesign --verify --strict /.*/mac-arm64/Footplate\.app$'
[ ! -e "$out" ] || fail "$name: a release with no timestamp wrote its out dir"

# The first disk image's signing dropped once: the release still passes, the
# image signed at the second try and the app before it signed once. This
# proves the image's signing is wrapped: the calls log alone cannot show it,
# as a wrapped call logs the same argv.
release_run sign-retry-dmg "$signer" "$notary" FOOTPLATE_SIGN_RETRY_SLEEP=0 STANDIN_CODESIGN_FAIL_SKIP=1 STANDIN_CODESIGN_FAIL_COUNT=1
[ "$rc" = 0 ] || fail "$name: a release with one dropped image timestamp failed: $text"
case "$text" in
    *"the timestamp was not found; trying again"*) ;;
    *) fail "$name: it did not say the timestamp was tried again: $text" ;;
esac
[ "$(grep -c -E "^codesign --force --sign $signer_re --timestamp /.*/Footplate_0\\.3\\.0_darwin_arm64\\.dmg\$" "$calls")" = 2 ] ||
    fail "$name: the arm64 image was signed: $(grep '^codesign' "$calls")"
[ "$(grep -c -E "^codesign --force --options runtime --timestamp --sign $signer_re /.*/mac-arm64/Footplate\\.app\$" "$calls")" = 1 ] ||
    fail "$name: the arm64 app was signed: $(grep '^codesign' "$calls")"
archives_in_out

# A yardr in an archive is refused, whichever of the three kinds holds it: the
# zip is listed first, so one in the app stops there; one in the image alone is
# found once the image is mounted, and one in the Linux archive alone at the
# end of the Mac's half. An image that cannot be mounted cannot be listed, and
# a release that cannot show what it ships does not ship it.
stray_refused zip STANDIN_STRAY=Yardr-helper "macOS arm64 zip" \
    Footplate.app/Contents/Resources/Yardr-helper
stray_refused image STANDIN_DMG_STRAY=Footplate.app/Contents/Resources/yardr "macOS arm64 disk image" \
    Footplate.app/Contents/Resources/yardr
stray_refused linux STANDIN_TAR_STRAY=yardr "Linux arm64 archive" yardr
stray_refused windows STANDIN_ZIP_STRAY=Yardr.txt "Windows archive" Yardr.txt
release_run no-mount "" "" STANDIN_ATTACH_FAILS=1
[ "$rc" = 1 ] || fail "no-mount: a release whose image could not be opened exited $rc: $text"
case "$text" in
    *"could not open"*"to list it"*) ;;
    *) fail "no-mount: it did not say it could not open the image: $text" ;;
esac
[ ! -e "$out" ] || fail "no-mount: a release that could not list an image wrote its out dir"
release_run partial-list "" "" STANDIN_FIND_FAILS=1
[ "$rc" = 1 ] || fail "partial-list: a release with an incomplete image listing exited $rc: $text"
case "$text" in
    *"could not list mounted image"*) ;;
    *) fail "partial-list: it did not report the incomplete listing: $text" ;;
esac
[ ! -e "$out" ] || fail "partial-list: a release that could not list an image wrote its out dir"
release_run no-detach "" "" STANDIN_DETACH_FAILS=1
[ "$rc" = 1 ] || fail "no-detach: a release that could not detach its image exited $rc: $text"
case "$text" in
    *"could not detach mounted image"*) ;;
    *) fail "no-detach: it did not report the mounted image: $text" ;;
esac
grep -q '^hdiutil detach -force ' "$calls" || fail "no-detach: cleanup did not retry with force: $(cat "$calls")"
[ ! -e "$out" ] || fail "no-detach: a release with a mounted image wrote its out dir"

# A commit with no run of the Windows workflow gets one started, on its
# branch, and the run is taken as one that was going; from a detached HEAD the
# run starts on main. A gh that cannot start refuses the release before
# anything is built, as does a run that never shows after it was started, five
# minutes at most; so does a run that ended without success, cancelled or not,
# and a gh that cannot ask. A run that is going is waited for, a stretch at a
# time, and then is taken or refused as one that is over; one that never ends
# is given up after 75 minutes. The artifact must be there, be this version's,
# hold what a Windows archive holds, and hold a Windows program.
release_run windows-dispatched "" "" "STANDIN_RUNS=none queued: completed:success"
[ "$rc" = 0 ] || fail "windows-dispatched: a release that started its run failed: $text"
branch=$(git -C "$tmp/windows-dispatched/checkout" rev-parse --abbrev-ref HEAD)
case "$text" in
    *"no run for"*"; starting one on $branch"*) ;;
    *) fail "windows-dispatched: it did not say it started a run on $branch: $text" ;;
esac
want_line "^gh workflow run windows\\.yml --ref $branch --repo yardr-dev/footplate\$"
[ "$(grep -c '^gh run list ' "$calls")" = 3 ] || fail "windows-dispatched: gh run list was asked: $(grep '^gh ' "$calls")"
lack "sleep"
archives_in_out
windows_archive_holds "$out/footplate_0.3.0_windows_amd64.zip"
DETACH_HEAD=1
release_run windows-detached "" "" "STANDIN_RUNS=none completed:success"
DETACH_HEAD=
[ "$rc" = 0 ] || fail "windows-detached: a release from a detached HEAD failed: $text"
want_line "^gh workflow run windows\\.yml --ref main --repo yardr-dev/footplate\$"
archives_in_out
windows_archive_holds "$out/footplate_0.3.0_windows_amd64.zip"
windows_refused start-fails "could not start a Windows run" no STANDIN_RUNS=none STANDIN_WORKFLOW_FAILS=1
case "$text" in
    *"could not create workflow dispatch event"*) ;;
    *) fail "windows-start-fails: it did not pass on what gh said: $text" ;;
esac
windows_refused start-missing "shows none after 5 minutes" no STANDIN_RUNS=none
[ "$(grep -c '^sleep 10$' "$calls")" = 30 ] || fail "windows-start-missing: slept: $(grep '^sleep' "$calls")"
windows_refused red "ended failure" no STANDIN_RUNS=completed:failure
windows_refused cancelled "ended cancelled" no STANDIN_RUNS=completed:cancelled
windows_refused gh "could not ask GitHub for the Windows run" no STANDIN_GH_FAILS=1
case "$text" in
    *"gh auth login"*) ;;
    *) fail "windows-gh: it did not pass on what gh said: $text" ;;
esac
release_run windows-waits "" "" "STANDIN_RUNS=queued: in_progress: in_progress: completed:success"
[ "$rc" = 0 ] || fail "windows-waits: a run that finished green while waited for failed the release: $text"
case "$text" in
    *"is in_progress"*"waiting for it"*) ;;
    *) fail "windows-waits: it did not say it waited: $text" ;;
esac
[ "$(grep -c '^gh run list ' "$calls")" = 4 ] || fail "windows-waits: gh run list was asked: $(grep '^gh ' "$calls")"
[ "$(grep -c '^sleep 60$' "$calls")" = 2 ] || fail "windows-waits: slept: $(grep '^sleep' "$calls")"
archives_in_out
windows_archive_holds "$out/footplate_0.3.0_windows_amd64.zip"
windows_refused waits-red "ended failure" yes "STANDIN_RUNS=in_progress: in_progress: completed:failure"
windows_refused never "still in_progress after 75 minutes" yes STANDIN_RUNS=in_progress:
[ "$(grep -c '^sleep 60$' "$calls")" = 75 ] || fail "windows-never: slept $(grep -c '^sleep' "$calls") times"
windows_refused no-artifact "could not take the Windows zip from run 4242" yes STANDIN_NO_ARTIFACT=1
windows_refused other-version "holds no footplate_0.3.0_windows_amd64.zip" yes STANDIN_ZIP_VERSION=0.2.0
windows_refused more-members "the Windows archive holds: LICENSE NOTICE README.md footplate.exe notes.txt;" yes STANDIN_ZIP_STRAY=notes.txt
windows_refused not-windows "is no PE32+ executable binary" yes STANDIN_EXE=linux-amd64

# With FOOTPLATE_RELEASE_WINDOWS=off the release ships without the Windows
# zip: it says so once, no run is started or asked about (the commit has
# none, which would otherwise start one), and the out dir holds the six
# other archives with the checksums over those alone. Any other value is
# today's release, Windows zip and all.
release_run windows-off "" "" FOOTPLATE_RELEASE_WINDOWS=off STANDIN_RUNS=none
[ "$rc" = 0 ] || fail "windows-off: a release without Windows failed: $text"
case "$text" in
    *"release.sh: Windows: off, no Windows zip in this release"*) ;;
    *) fail "windows-off: it did not say Windows is off: $text" ;;
esac
said=$(printf '%s\n' "$text" | grep -c 'Windows: off' || true)
[ "$said" = 1 ] || fail "windows-off: it said Windows is off $said times: $text"
lack_line '^gh '
listed_clean off
archives_in_out off
[ ! -e "$out/footplate_0.3.0_windows_amd64.zip" ] || fail "windows-off: the out dir holds the Windows zip"
! grep -q windows "$out/checksums.txt" || fail "windows-off: checksums.txt names the Windows zip: $(cat "$out/checksums.txt")"
release_run windows-on "" "" FOOTPLATE_RELEASE_WINDOWS=on
[ "$rc" = 0 ] || fail "windows-on: a release with FOOTPLATE_RELEASE_WINDOWS=on failed: $text"
case "$text" in
    *"Windows: off"*) fail "windows-on: it said Windows is off: $text" ;;
esac
want_line "^gh run list "
archives_in_out
windows_archive_holds "$out/footplate_0.3.0_windows_amd64.zip"

# A submission Apple does not accept fails the release with the notary
# service's log of it, whether or not notarytool's own exit status says so,
# and so does a notarytool that fails with an answer that reads Accepted: a
# failed command is not trusted. Nothing is left in the out dir: no ticket,
# no zip, no checksums.
for answer in Invalid:1 Invalid:0 Accepted:1; do
    status=${answer%:*}
    submit_exit=${answer#*:}
    release_run "refused-$status-$submit_exit" "$signer" "$notary" \
        "STANDIN_STATUS=$status" "STANDIN_SUBMIT_EXIT=$submit_exit"
    [ "$rc" = 1 ] || fail "$name: a refused submission exited $rc: $text"
    for said in "macOS arm64 app: Apple did not notarize it" "status \"$status\"" "2efe2717-52ef-43a5-96dc-0797e4ca1041" "The signature of the binary is invalid."; do
        case "$text" in
            *"$said"*) ;;
            *) fail "$name: it did not say [$said]: $text" ;;
        esac
    done
    want "xcrun notarytool log 2efe2717-52ef-43a5-96dc-0797e4ca1041 --keychain-profile $notary"
    lack "stapler"
    [ ! -e "$out" ] || fail "$name: a refused release wrote its out dir"
done

# The same three answers to the image's submission alone, the app's having
# been accepted and stapled: the image is made and signed, its refusal fails
# the release as the app's does, naming the image, with the log of that
# submission, and nothing is stapled to it or left in the out dir.
for answer in Invalid:1 Invalid:0 Accepted:1; do
    status=${answer%:*}
    submit_exit=${answer#*:}
    release_run "refused-image-$status-$submit_exit" "$signer" "$notary" \
        STANDIN_ONLY=dmg "STANDIN_STATUS=$status" "STANDIN_SUBMIT_EXIT=$submit_exit"
    [ "$rc" = 1 ] || fail "$name: a refused image exited $rc: $text"
    for said in "macOS arm64 disk image: Apple did not notarize it" "status \"$status\"" "2efe2717-52ef-43a5-96dc-0797e4ca1041" "The signature of the binary is invalid."; do
        case "$text" in
            *"$said"*) ;;
            *) fail "$name: it did not say [$said]: $text" ;;
        esac
    done
    want "xcrun notarytool log 2efe2717-52ef-43a5-96dc-0797e4ca1041 --keychain-profile $notary"
    [ "$(steps)" = "$(printf '%s\n' "codesign sign" "codesign verify" ditto "notarytool submit" "stapler staple" ditto "hdiutil create" "codesign sign image" "codesign verify image" "notarytool submit image" "notarytool log")" ] ||
        fail "$name: the calls were: $(steps | tr '\n' ',')"
    [ ! -e "$out" ] || fail "$name: a refused release wrote its out dir"
done

# One variable without the other is an error before anything is built.
refused half-identity 2 \
    "FOOTPLATE_RELEASE_SIGN_IDENTITY is set and FOOTPLATE_RELEASE_NOTARY_PROFILE is not" \
    "$signer" ""
refused half-profile 2 \
    "FOOTPLATE_RELEASE_NOTARY_PROFILE is set and FOOTPLATE_RELEASE_SIGN_IDENTITY is not" \
    "" "$notary"

# Both on a system that is not a Mac are refused too, as a release is
# anywhere but on one.
refused linux 1 "a release is built on macOS" "$signer" "$notary" STANDIN_UNAME=Linux

# bundle-mac.sh on its own, as a builder or a contributor runs it, signs ad
# hoc whatever the keychain holds; with an identity it signs with it and
# checks the signature, and asks Apple for nothing, as that is the release's.
bundle_run() {
    name=bundle-$1
    calls=$tmp/$name.calls
    make_checkout "$tmp/$name" "0.3.0"
    echo binary >"$tmp/$name/footplate"
    chmod +x "$tmp/$name/footplate"
    env PATH="$bin:$PATH" CALLS="$calls" FOOTPLATE_RELEASE_SIGN_IDENTITY="$2" \
        "$tmp/$name/scripts/bundle-mac.sh" "$tmp/$name/footplate" "$tmp/$name/Footplate.app" >/dev/null ||
        fail "$name: bundle-mac.sh failed"
}
bundle_run adhoc ""
[ "$(steps)" = codesign\ sign ] || fail "$name: the calls were: $(cat "$calls")"
want_line '^codesign --force --sign - /.*/Footplate\.app$'
lack "--options"
bundle_run signed "$signer"
[ "$(steps)" = "$(printf 'codesign sign\ncodesign verify')" ] || fail "$name: the calls were: $(cat "$calls")"
want "codesign --force --options runtime --timestamp --sign $signer /"
want "codesign --verify --strict /"
lack "xcrun"

echo "test-release.sh: pass"
