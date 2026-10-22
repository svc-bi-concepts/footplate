#!/bin/sh
# The release script refuses a version that is not Cargo.toml's and a dirty
# tree, before it builds anything: nothing is written to the out dir. Its
# macOS half is run whole, on any system, against stand-ins for what a Mac
# and Apple do: signed ad hoc with neither signing variable set, signed,
# notarized and stapled with both, refused with one.
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
# A signed app is one that holds the mark a signature leaves; --verify asks
# for it, so that a check before the signing, or of another app, fails.
cat >"$bin/codesign" <<'EOF'
#!/bin/sh
echo "codesign $*" >>"$CALLS"
for last; do :; done
case " $* " in
    *" --verify "*)
        [ -f "$last/Contents/_CodeSignature/CodeResources" ] || {
            echo "$last: code object is not signed at all" >&2
            exit 1
        }
        ;;
    *)
        mkdir -p "$last/Contents/_CodeSignature"
        echo sealed >"$last/Contents/_CodeSignature/CodeResources"
        ;;
esac
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
# Apple is sent a signed app that has no ticket yet, and answers with one:
# the ticket is the file a stapled app holds.
cat >"$bin/xcrun" <<'EOF'
#!/bin/sh
echo "xcrun $*" >>"$CALLS"
case "$1 $2" in
    "notarytool submit")
        tar -tf "$3" | grep -q -x 'Footplate.app/Contents/_CodeSignature/CodeResources' || {
            echo "the submission is not of a signed app" >&2
            exit 1
        }
        if tar -tf "$3" | grep -q -x 'Footplate.app/Contents/CodeResources'; then
            echo "the submission holds a ticket already" >&2
            exit 1
        fi
        echo '{"id":"2efe2717-52ef-43a5-96dc-0797e4ca1041","message":"Processing complete","path":"'"$3"'","status":"'"${STANDIN_STATUS:-Accepted}"'"}'
        exit "${STANDIN_SUBMIT_EXIT:-0}"
        ;;
    "notarytool log")
        echo '{"issues":[{"message":"The signature of the binary is invalid."}]}'
        ;;
    "stapler staple")
        echo ticket >"$3/Contents/CodeResources"
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
# some call is such a line; lack <text>: none was, which holds when there is
# no log at all.
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

# steps: the calls of the stand-ins that matter to the order of a release,
# one or two words each.
steps() {
    awk '
        $1 == "codesign" { print ($0 ~ / --verify /) ? "codesign verify" : "codesign sign" }
        $1 == "ditto" { print "ditto" }
        $1 == "xcrun" { print $2 " " $3 }
    ' "$calls"
}

# in_zip <zip> <member>: whether the app that was zipped holds the member.
in_zip() {
    tar -tf "$1" | grep -q -x "Footplate.app/$2"
}

# archives_in_out: the four archives and the checksums of a release, and
# nothing else, and the checksums are of what is there.
archives_in_out() {
    for f in Footplate_0.3.0_darwin_arm64.zip Footplate_0.3.0_darwin_amd64.zip \
        footplate_0.3.0_linux_arm64.tar.gz footplate_0.3.0_linux_amd64.tar.gz checksums.txt; do
        [ -f "$out/$f" ] || fail "$name: no $f in the out dir: $(find "$out" -mindepth 1)"
    done
    [ "$(find "$out" -mindepth 1 | wc -l | tr -d ' ')" = 5 ] || fail "$name: the out dir holds: $(find "$out" -mindepth 1)"
    (cd "$out" && shasum -a 256 -c checksums.txt >/dev/null) || fail "$name: checksums.txt is not of the archives"
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

# With neither variable the apps are signed ad hoc, as they were, and Apple
# is not asked: no hardened runtime, no verify, no notary, no ticket, and
# each zip made once.
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
[ "$(steps)" = "$(printf 'codesign sign\nditto\ncodesign sign\nditto')" ] ||
    fail "adhoc: the calls were: $(steps | tr '\n' ',')"
for arch in arm64 amd64; do
    zip=$out/Footplate_0.3.0_darwin_$arch.zip
    in_zip "$zip" Contents/_CodeSignature/CodeResources || fail "adhoc: the $arch zip holds no signature"
    ! in_zip "$zip" Contents/CodeResources || fail "adhoc: the $arch zip holds a ticket"
done
archives_in_out

# With both, each app is signed with the identity, hardened and timestamped,
# and the signature is checked; sent to Apple in a zip, waited for; stapled;
# and only then zipped for the release, so that the zip that ships holds the
# ticket and checksums.txt is of it. (The stand-in for notarytool refuses a
# submission that is not of a signed app, or that holds a ticket already.)
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
ditto"
[ "$(steps)" = "$(printf '%s\n%s' "$one" "$one")" ] ||
    fail "signed: the calls were: $(steps | tr '\n' ',')"
for arch in arm64 amd64; do
    zip=$out/Footplate_0.3.0_darwin_$arch.zip
    in_zip "$zip" Contents/_CodeSignature/CodeResources || fail "signed: the $arch zip holds no signature"
    in_zip "$zip" Contents/CodeResources || fail "signed: the $arch zip holds no ticket"
done
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
    for said in "status \"$status\"" "2efe2717-52ef-43a5-96dc-0797e4ca1041" "The signature of the binary is invalid."; do
        case "$text" in
            *"$said"*) ;;
            *) fail "$name: it did not say [$said]: $text" ;;
        esac
    done
    want "xcrun notarytool log 2efe2717-52ef-43a5-96dc-0797e4ca1041 --keychain-profile $notary"
    lack "stapler"
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
