#!/bin/sh
# The release script refuses a version that is not Cargo.toml's and a dirty
# tree, before it builds anything: nothing is written to the out dir. Its
# macOS half is run whole, on any system, against stand-ins for what a Mac
# and Apple do: the apps signed ad hoc and the disk images not signed with
# neither signing variable set, both signed, notarized and stapled with both,
# refused with one.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
script=$script_dir/release.sh
lib=$script_dir/linux-container.sh
bundle=$script_dir/bundle-mac.sh
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
    cp "$bundle" "$1/scripts/bundle-mac.sh"
    chmod +x "$1/scripts/release.sh" "$1/scripts/bundle-mac.sh"
    printf '[package]\nname = "footplate"\nversion = "%s"\n' "$2" > "$1/Cargo.toml"
    # What a Linux archive holds beside the binary.
    for f in README.md LICENSE NOTICE; do
        echo "$f" > "$1/$f"
    done
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
bin=$tmp/bin
mkdir -p "$bin"
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
    if [ "$prev" = --output ]; then echo icns >"$a"; fi
    prev=$a
done
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
# will not write over an image that is there.
cat >"$bin/hdiutil" <<'EOF'
#!/bin/sh
echo "hdiutil $*" >>"$CALLS"
[ "$1" = create ] || {
    echo "hdiutil: the stand-in makes images and does no more" >&2
    exit 1
}
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
# given, empty for none, so that the caller's own never reach it.
release_run() {
    name=$1
    sign_with=$2
    notary_with=$3
    shift 3
    make_checkout "$tmp/$name/checkout" "0.3.0"
    out=$tmp/$name/out
    calls=$tmp/$name/calls
    if text=$(env PATH="$bin:$PATH" CALLS="$calls" \
        FOOTPLATE_RELEASE_SIGN_IDENTITY="$sign_with" \
        FOOTPLATE_RELEASE_NOTARY_PROFILE="$notary_with" \
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

# archives_in_out: the six archives and the checksums of a release, and
# nothing else, and the checksums are of what is there.
archives_in_out() {
    for f in Footplate_0.3.0_darwin_arm64.zip Footplate_0.3.0_darwin_amd64.zip \
        Footplate_0.3.0_darwin_arm64.dmg Footplate_0.3.0_darwin_amd64.dmg \
        footplate_0.3.0_linux_arm64.tar.gz footplate_0.3.0_linux_amd64.tar.gz checksums.txt; do
        [ -f "$out/$f" ] || fail "$name: no $f in the out dir: $(find "$out" -mindepth 1)"
    done
    [ "$(find "$out" -mindepth 1 | wc -l | tr -d ' ')" = 7 ] || fail "$name: the out dir holds: $(find "$out" -mindepth 1)"
    (cd "$out" && shasum -a 256 -c checksums.txt >/dev/null) || fail "$name: checksums.txt is not of the archives"
    # And the checksums are of all six: none is left out of them.
    [ "$(wc -l <"$out/checksums.txt" | tr -d ' ')" = 6 ] || fail "$name: checksums.txt is: $(cat "$out/checksums.txt")"
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
one="codesign sign
ditto
hdiutil create"
[ "$(steps)" = "$(printf '%s\n%s' "$one" "$one")" ] ||
    fail "adhoc: the calls were: $(steps | tr '\n' ',')"
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
done
archives_in_out

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
stapler staple image"
[ "$(steps)" = "$(printf '%s\n%s' "$one" "$one")" ] ||
    fail "signed: the calls were: $(steps | tr '\n' ',')"
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
