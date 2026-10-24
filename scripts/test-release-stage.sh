#!/bin/sh
# Tests for scripts/release-stage, the release flow's scripted stages. The
# script is run the way the exec group runs it, against a throwaway git
# repository with a local bare remote, with stand-ins for gh, minisign and
# yardr first on PATH; what it asked of them is asserted from their shared
# call log. The build step runs a stand-in build script (the real
# scripts/release.sh is test-release.sh's), so no test compiles footplate.
# Verify's look at the macOS apps has stand-ins for xcrun, spctl and ditto,
# which only a Mac has, so it is tried the same on any system.
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
stage=$script_dir/release-stage
tmp=$(mktemp -d)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fail() {
    echo "test-release-stage.sh: $1" >&2
    exit 1
}

n=0
ok() {
    echo "ok - $1"
    n=$((n + 1))
}

if ! command -v git >/dev/null 2>&1; then
    echo "test-release-stage.sh: requires git" >&2
    exit 77
fi
if ! command -v jq >/dev/null 2>&1; then
    echo "test-release-stage.sh: requires jq" >&2
    exit 77
fi
if command -v shasum >/dev/null 2>&1; then
    SUM="shasum -a 256"
elif command -v sha256sum >/dev/null 2>&1; then
    SUM="sha256sum"
else
    echo "test-release-stage.sh: requires shasum or sha256sum" >&2
    exit 77
fi

# The key the script verifies against, and the signer's key path its note
# names: each stands in the note's commands.
KEY="RWSzUPGBbv7MDqH+nnbNmYdOm20UTkuk7eC5e6XuVtYRwZtaEZRlV/WB"

export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t
export GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
export FAKE_SUM="$SUM"

bin=$tmp/bin
mkdir -p "$bin"

# fake yardr answers bead show and prime from canned files, logs every call
# with the note or body it was given on stdin, and mints one sync bead id.
# Like the real one it reads stdin only where an argument asks it to: "-" after
# --note (bead done, bead advance) or after -b or --body (bead create).
# Publish advances a bead without a note, and a stand-in that read anyway
# would wait for ever on a stdin that stands open. The arguments are scanned,
# not the line they make: a note whose text holds the words is no request.
cat >"$bin/yardr" <<'EOF'
#!/bin/sh
printf 'yardr %s\n' "$*" >>"$FAKE_DIR/calls.log"
# asked <flag> <argument>...: whether "-" follows flag among the arguments.
asked() {
  flag=$1
  shift
  prev=
  for a in "$@"; do
    if [ "$prev" = "$flag" ] && [ "$a" = - ]; then return 0; fi
    prev=$a
  done
  return 1
}
case "$1 $2" in
"bead show")
  cat "$FAKE_DIR/show.json"
  ;;
"prime --bead")
  cat "$FAKE_DIR/prime.json"
  ;;
"bead done" | "bead advance")
  { echo "--note--"; if asked --note "$@"; then cat; fi; echo "--end--"; } >>"$FAKE_DIR/calls.log"
  ;;
"bead create")
  { echo "--create--"; if asked -b "$@" || asked --body "$@"; then cat; fi; echo "--end--"; } >>"$FAKE_DIR/calls.log"
  printf '{"id": "%s"}\n' "${FAKE_CREATE_ID:-yardr.dev-t1st}"
  ;;
esac
EOF

# fake gh answers release view from an exit status, saves the notes file a
# release create was given, and serves release downloads from a directory.
cat >"$bin/gh" <<'EOF'
#!/bin/sh
printf 'gh %s\n' "$*" >>"$FAKE_DIR/calls.log"
case "$1 $2" in
"release view")
  exit "${FAKE_GH_VIEW_RC:-1}"
  ;;
"release create")
  prev=""
  for a in "$@"; do
    if [ "$prev" = "--notes-file" ]; then cp "$a" "$FAKE_DIR/notes.md"; fi
    prev="$a"
  done
  exit "${FAKE_GH_CREATE_RC:-0}"
  ;;
"release download")
  rc="${FAKE_GH_DOWNLOAD_RC:-0}"
  if [ "$rc" -eq 0 ]; then
    prev=""
    for a in "$@"; do
      if [ "$prev" = "--dir" ]; then dir="$a"; fi
      prev="$a"
    done
    cp -R "$FAKE_RELEASE_DIR"/. "$dir"/
  fi
  exit "$rc"
  ;;
esac
exit 0
EOF

# fake minisign verifies as its exit status says, printing its canned line.
cat >"$bin/minisign" <<'EOF'
#!/bin/sh
printf 'minisign %s\n' "$*" >>"$FAKE_DIR/calls.log"
printf '%s\n' "${FAKE_MINISIGN_OUT:-Signature and comment signature verified}"
exit "${FAKE_MINISIGN_RC:-0}"
EOF

# fakebuild stands in for scripts/release.sh: the seven footplate archives
# and a checksums.txt over them, and a log of where it ran, with what cache
# and with which of the signing variables the group's environment gave it.
cat >"$bin/fakebuild" <<'EOF'
#!/bin/sh
# fakebuild <version> <out-dir>
{
  echo "fakebuild $*"
  echo "pwd: $PWD"
  echo "gocache: $GOCACHE"
  echo "out: $2"
  echo "signing: identity=[${FOOTPLATE_RELEASE_SIGN_IDENTITY:-}] profile=[${FOOTPLATE_RELEASE_NOTARY_PROFILE:-}]"
} >>"$FAKE_DIR/calls.log"
number=${1#v}
printf 'fake footplate darwin arm64 for %s\n' "$1" >"$2/Footplate_${number}_darwin_arm64.zip"
printf 'fake footplate darwin arm64 image for %s\n' "$1" >"$2/Footplate_${number}_darwin_arm64.dmg"
if [ "${FAKE_BUILD_OMIT:-}" != "Footplate_${number}_darwin_amd64.zip" ]; then
    printf 'fake footplate darwin amd64 for %s\n' "$1" >"$2/Footplate_${number}_darwin_amd64.zip"
fi
if [ "${FAKE_BUILD_OMIT:-}" != "Footplate_${number}_darwin_amd64.dmg" ]; then
    printf 'fake footplate darwin amd64 image for %s\n' "$1" >"$2/Footplate_${number}_darwin_amd64.dmg"
fi
printf 'fake footplate linux arm64 for %s\n' "$1" >"$2/footplate_${number}_linux_arm64.tar.gz"
printf 'fake footplate linux amd64 for %s\n' "$1" >"$2/footplate_${number}_linux_amd64.tar.gz"
if [ "${FAKE_BUILD_OMIT:-}" != "footplate_${number}_windows_amd64.zip" ]; then
    printf 'fake footplate windows amd64 for %s\n' "$1" >"$2/footplate_${number}_windows_amd64.zip"
fi
if [ "${FAKE_BUILD_EXTRA:-}" = yes ]; then
    printf 'unexpected release asset\n' >"$2/unexpected.txt"
fi
# The checksums are over the archives that were made, whichever one a case left out.
(
    cd "$2"
    files=""
    for f in Footplate_*_darwin_*.zip Footplate_*_darwin_*.dmg footplate_*_linux_*.tar.gz footplate_*_windows_*.zip; do
        [ ! -f "$f" ] || files="$files $f"
    done
    # shellcheck disable=SC2086 # $FAKE_SUM and $files are lists
    $FAKE_SUM $files >checksums.txt
)
EOF
# fake xcrun answers stapler's validate from a canned line and exit status.
cat >"$bin/xcrun" <<'EOF'
#!/bin/sh
printf 'xcrun %s\n' "$*" >>"$FAKE_DIR/calls.log"
printf '%s\n' "${FAKE_STAPLER_OUT:-The validate action worked!}"
exit "${FAKE_STAPLER_RC:-0}"
EOF

# fake spctl assesses as its exit status says, printing on stderr as the real
# one does: its canned text, else "accepted" from a notarized Developer ID
# for the app it was given last.
cat >"$bin/spctl" <<'EOF'
#!/bin/sh
printf 'spctl %s\n' "$*" >>"$FAKE_DIR/calls.log"
for app; do :; done
if [ -n "${FAKE_SPCTL_OUT:-}" ]; then
  printf '%s\n' "$FAKE_SPCTL_OUT" >&2
else
  printf '%s: accepted\nsource=Notarized Developer ID\n' "$app" >&2
fi
exit "${FAKE_SPCTL_RC:-0}"
EOF

# fake ditto "unpacks" a zip into the directory it is given: the one app a
# footplate zip holds, or none, or a failure, when it is told so.
cat >"$bin/ditto" <<'EOF'
#!/bin/sh
printf 'ditto %s\n' "$*" >>"$FAKE_DIR/calls.log"
# ditto -x -k <zip> <dest>
dest=$4
if [ "${FAKE_DITTO_RC:-0}" -ne 0 ]; then
  echo "ditto: cannot unpack"
  exit "$FAKE_DITTO_RC"
fi
if [ "${FAKE_DITTO_EMPTY:-}" != yes ]; then
  mkdir -p "$dest/Footplate.app"
fi
EOF

# fake hdiutil "mounts" a disk image at the mountpoint it is given: the app
# and the Applications link a footplate image holds (with the .DS_Store a
# real image may carry), or a volume with junk / without the link, when it
# is told so. Detaching only logs.
cat >"$bin/hdiutil" <<'EOF'
#!/bin/sh
printf 'hdiutil %s\n' "$*" >>"$FAKE_DIR/calls.log"
# hdiutil attach -readonly -nobrowse -noautoopen -mountpoint <dir> <dmg>
if [ "$1" = attach ]; then
  prev=""
  for a in "$@"; do
    if [ "$prev" = "-mountpoint" ]; then dir="$a"; fi
    prev="$a"
  done
  mkdir -p "$dir/Footplate.app"
  printf 'finder\n' >"$dir/.DS_Store"
  if [ "${FAKE_HDIUTIL_NOLINK:-}" != yes ]; then
    ln -s /Applications "$dir/Applications"
  fi
  if [ "${FAKE_HDIUTIL_EXTRA:-}" = yes ]; then
    # A name beginning with two dots must still count as an extra entry.
    printf 'junk\n' >"$dir/..junk.txt"
  fi
fi
EOF
chmod +x "$bin/yardr" "$bin/gh" "$bin/minisign" "$bin/fakebuild" "$bin/xcrun" "$bin/spctl" "$bin/ditto" "$bin/hdiutil"

# nomac: a PATH with what verify needs but no xcrun, as a system that is no
# Mac has it. These tests run on a Mac as well, where xcrun stands in
# /usr/bin among everything else, so the dir is made of links to the tools
# verify uses and to the stand-ins but xcrun.
nomac=$tmp/nomac
mkdir -p "$nomac"
for tool in jq git sed grep mktemp rm cat mkdir cp env tail head date shasum sha256sum; do
    found=$(command -v "$tool" 2>/dev/null) || continue
    ln -s "$found" "$nomac/$tool"
done
for tool in yardr gh minisign spctl ditto; do
    ln -s "$bin/$tool" "$nomac/$tool"
done

# nohdiutil: a PATH with what verify needs but no hdiutil, as a system
# without the mounter has it. Made of links like nomac, since a Mac's
# /usr/bin holds the real one.
nohdiutil=$tmp/nohdiutil
mkdir -p "$nohdiutil"
for tool in jq git sed grep mktemp rm cat mkdir cp env tail head date shasum sha256sum; do
    found=$(command -v "$tool" 2>/dev/null) || continue
    ln -s "$found" "$nohdiutil/$tool"
done
for tool in yardr gh minisign xcrun spctl ditto; do
    ln -s "$bin/$tool" "$nohdiutil/$tool"
done

# The fake reads nothing for a call that asks for nothing, however stdin
# stands: a fifo that this shell holds a write end of stands open until it is
# closed, and the call must end at once. A fake that blocks fails the test
# after five seconds, and gets its end of file when this shell exits.
mkfifo "$tmp/open-stdin"
exec 3<>"$tmp/open-stdin"
mkdir "$tmp/no-read"
# ends_at_once <yardr argument>...: the fake, called so, ends on the open stdin.
ends_at_once() {
    rm -f "$tmp/no-read/ended"
    (
        exec 3>&-
        FAKE_DIR=$tmp/no-read "$bin/yardr" "$@" <"$tmp/open-stdin" >/dev/null
        { : >"$tmp/no-read/ended"; } 2>/dev/null
    ) &
    tenths=0
    while [ ! -e "$tmp/no-read/ended" ] && [ "$tenths" -lt 50 ]; do
        sleep 0.1
        tenths=$((tenths + 1))
    done
    [ -e "$tmp/no-read/ended" ] ||
        fail "the fake yardr waits on stdin for: yardr $*"
}
# The first start of a script this Mac has not run takes seconds at a high load,
# and the five seconds allowed below must not pay for it: the fake is started
# once first, on a stdin that ends.
FAKE_DIR=$tmp/no-read "$bin/yardr" bead advance footplate-rel1 --to new </dev/null
# The advance publish makes, which gives no note; a note whose text holds the
# words that ask for one, which is one argument; a body given in the call.
ends_at_once bead advance footplate-rel1 --to new
ends_at_once bead advance footplate-rel1 --to new --note "see --note - below"
ends_at_once bead create Title -b "a body"
exec 3>&-
ok "the fake yardr reads a note or a body only where it is asked for"

# Where it is asked to read, what it read lands between the markers: a note
# after --note -, and a body after -b - or --body -.
mkdir "$tmp/reads"
printf 'a note\n' | FAKE_DIR=$tmp/reads "$bin/yardr" bead advance footplate-rel1 --to new --note -
printf 'a body\n' | FAKE_DIR=$tmp/reads "$bin/yardr" bead create Short -b - >/dev/null
printf 'another body\n' | FAKE_DIR=$tmp/reads "$bin/yardr" bead create Long --body - >/dev/null
printf '%s\n' \
    "yardr bead advance footplate-rel1 --to new --note -" "--note--" "a note" "--end--" \
    "yardr bead create Short -b -" "--create--" "a body" "--end--" \
    "yardr bead create Long --body -" "--create--" "another body" "--end--" |
    cmp -s - "$tmp/reads/calls.log" ||
    fail "the fake yardr does not log what it is asked to read: $(cat "$tmp/reads/calls.log")"
ok "the fake yardr logs a note or a body it is asked to read"

# begin <name>: a fresh case with the flow's outcomes from here, in the
# shape has_outcome greps.
begin() {
    name=$1
    casedir=$tmp/$1
    mkdir -p "$casedir"
    cat >"$casedir/prime.json" <<'EOF'
{
  "next": {
    "stage": "here",
    "outcomes": [
      {
        "name": "done",
        "to": "somewhere"
      },
      {
        "name": "failed",
        "to": "somewhere"
      },
      {
        "name": "question",
        "to": "somewhere"
      }
    ]
  }
}
EOF
}

# show <title> <body>: cans the bead through real JSON encoding, so the
# script's jq reads what the yard's would.
show() {
    jq -n --arg title "$1" --arg body "$2" '{title: $title, body: $body}' \
        >"$casedir/show.json" || fail "$name: cannot write show.json"
}

# repo: a clean checkout on main with a bare origin that has it, at
# Cargo.toml 0.3.0; prints the checkout, and ORIGIN is the remote.
repo() {
    checkout=$casedir/repo
    ORIGIN=$casedir/origin.git
    git init --bare -q "$ORIGIN"
    git init -b main -q "$checkout"
    git -C "$checkout" remote add origin "$ORIGIN"
    printf '[package]\nname = "footplate"\nversion = "0.3.0"\n' >"$checkout/Cargo.toml"
    printf 'v1\n' >"$checkout/file"
    git -C "$checkout" add -A
    git -C "$checkout" commit -qm one
    git -C "$checkout" push -q origin main
}

# real_release: the build script itself in the checkout, committed and pushed,
# for the cases that run it in place of fakebuild.
real_release() {
    mkdir -p "$checkout/scripts"
    cp "$script_dir/release.sh" "$script_dir/linux-container.sh" "$script_dir/codesign-retry.sh" "$script_dir/bundle-mac.sh" "$checkout/scripts/"
    git -C "$checkout" add -A
    git -C "$checkout" commit -qm scripts
    git -C "$checkout" push -q origin main
}

# fixture <sig|nosig> <ok|corrupt> [mac] [dmg]: a release for verify to
# download, of two archives with a checksums.txt over them, and with mac
# the two zips of footplate's macOS apps as well (their bytes are no
# zips: ditto is a stand-in), and with dmg the two disk images beside
# whatever else was asked for (their bytes are no images: hdiutil is a
# stand-in).
fixture() {
    rdir=$casedir/release
    mkdir -p "$rdir"
    printf 'a\n' >"$rdir/a.tar.gz"
    printf 'b\n' >"$rdir/b.tar.gz"
    archives="a.tar.gz b.tar.gz"
    if [ "${3:-}" = mac ]; then
        for arch in arm64 amd64; do
            printf 'footplate darwin %s\n' "$arch" >"$rdir/Footplate_0.3.0_darwin_$arch.zip"
            archives="$archives Footplate_0.3.0_darwin_$arch.zip"
        done
    fi
    if [ "${4:-}" = dmg ]; then
        for arch in arm64 amd64; do
            printf 'footplate darwin %s image\n' "$arch" >"$rdir/Footplate_0.3.0_darwin_$arch.dmg"
            archives="$archives Footplate_0.3.0_darwin_$arch.dmg"
        done
    fi
    # shellcheck disable=SC2086 # $SUM and $archives are lists
    (cd "$rdir" && $SUM $archives >checksums.txt)
    if [ "$1" = sig ]; then printf 'sig\n' >"$rdir/checksums.txt.minisig"; fi
    if [ "$2" != ok ]; then printf 'corrupted\n' >>"$rdir/a.tar.gz"; fi
}

# run <mode> [VAR=value ...]: release-stage the way the exec group runs it.
# The signing variables are empty unless a case gives them, so that the
# caller's own never reach it.
run() {
    mode=$1
    shift
    FAKE_DIR=$casedir PATH="$bin:$PATH" \
        YARDR_BIN=yardr YARDR_BEAD=footplate-rel1 YARDR_BASE=main \
        YARDR_RELEASE_BUILD="$bin/fakebuild" \
        env FOOTPLATE_RELEASE_SIGN_IDENTITY= FOOTPLATE_RELEASE_NOTARY_PROFILE= \
        "$@" "$stage" "$mode" >"$casedir/out.log" 2>&1 ||
        fail "$name: release-stage $mode exited $?: $(tail -n 20 "$casedir/out.log")"
}

log() {
    cat "$casedir/calls.log" 2>/dev/null || true
}

want() {
    log | grep -q -F -- "$1" || fail "$name: want [$1] in: $(log)"
}

lack() {
    if log | grep -q -F -- "$1"; then fail "$name: want no [$1] in: $(log)"; fi
}

# count <n> <prefix>: exactly n logged calls begin with prefix, which stands
# for text a call line starts with and a note does not.
count() {
    got=$(log | grep -c -- "^$2" || true)
    [ "$got" -eq "$1" ] || fail "$name: want $1 calls of [$2], got $got in: $(log)"
}

# A bad title is questioned, and nothing is released.
begin bad-title
repo
show "cut a release sometime" "notes"
run build "YARDR_DEPOT_PATH=$checkout"
want "yardr bead done footplate-rel1 --outcome question --note -"
want "names no version"
lack "gh release"
lack "fakebuild"
if git -C "$checkout" rev-parse --verify -q "refs/tags/v0.3.0" >/dev/null; then
    fail "bad-title: a tag was made for a refused build"
fi
ok "build refuses a bad title"

# A version that is not Cargo.toml's is refused, before anything builds.
begin cargo-mismatch
repo
show "footplate v9.9.9" "notes"
run build "YARDR_DEPOT_PATH=$checkout"
want "yardr bead done footplate-rel1 --outcome failed --note -"
want "is not Cargo.toml's version (v0.3.0)"
want "the bead titles v9.9.9"
lack "fakebuild"
lack "gh release create"
ok "build refuses a version that is not Cargo.toml's"

# A partial archive set is refused before the tag or release is created,
# whether it lacks a zip, a disk image or the Windows zip.
for omitted in Footplate_0.3.0_darwin_amd64.zip Footplate_0.3.0_darwin_amd64.dmg footplate_0.3.0_windows_amd64.zip; do
    begin "missing-$omitted"
    repo
    show "footplate v0.3.0" "notes"
    run build "YARDR_DEPOT_PATH=$checkout" \
        "FAKE_BUILD_OMIT=$omitted"
    want "yardr bead done footplate-rel1 --outcome failed --note -"
    want "the build left no $omitted"
    lack "gh release create"
    if git -C "$checkout" rev-parse --verify -q "refs/tags/v0.3.0" >/dev/null; then
        fail "$name: a tag was made for a partial build"
    fi
    ok "build refuses a missing archive ($omitted)"
done

# An extra file is refused instead of riding along as a release asset.
begin extra-asset
repo
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout" "FAKE_BUILD_EXTRA=yes"
want "yardr bead done footplate-rel1 --outcome failed --note -"
want "unexpected file unexpected.txt"
lack "gh release create"
if git -C "$checkout" rev-parse --verify -q "refs/tags/v0.3.0" >/dev/null; then
    fail "extra-asset: a tag was made for an unexpected asset"
fi
ok "build refuses an extra asset"

# A dirty tree is refused.
begin dirty-tree
repo
echo stray >"$checkout/stray"
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout"
want "yardr bead done footplate-rel1 --outcome failed --note -"
want "uncommitted or untracked files"
lack "gh release create"
lack "fakebuild"
ok "build refuses a dirty tree"

# An existing release is refused.
begin existing-release
repo
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout" "FAKE_GH_VIEW_RC=0"
want "--outcome failed"
want "already exists"
lack "gh release create"
lack "fakebuild"
ok "build refuses an existing release"

# A tag at another commit is refused.
begin tag-elsewhere
repo
git -C "$checkout" tag v0.3.0
printf 'v2\n' >"$checkout/file"
git -C "$checkout" commit -qam two
git -C "$checkout" push -q origin main
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout"
want "--outcome failed"
want "at another commit"
lack "fakebuild"
ok "build refuses a tag at another commit"

# A release: the tag here and on the remote, the release with the body's
# bytes as its notes, done with the five commands, each naming the version.
begin releases
repo
body='Release "v0.3.0"

- fixes <bugs> & things'
show "footplate v0.3.0" "$body"
mkdir "$tmp/temp dir"
run build "YARDR_DEPOT_PATH=$checkout" "TMPDIR=$tmp/temp dir"
at=$(git -C "$checkout" rev-parse refs/tags/v0.3.0)
[ "$at" = "$(git -C "$checkout" rev-parse refs/heads/main)" ] ||
    fail "releases: tag v0.3.0 is at $at, not at main"
remote=$(git --git-dir="$ORIGIN" rev-parse refs/tags/v0.3.0) &&
    [ "$remote" = "$at" ] ||
    fail "releases: tag v0.3.0 is not pushed"
want "fakebuild v0.3.0"
want "pwd: $checkout"
want "gocache: "
[ -n "$(log | sed -n 's/^gocache: //p')" ] ||
    fail "releases: the build ran with an empty GOCACHE"
want "gh release create v0.3.0"
want "--repo yardr-dev/footplate"
want "--title v0.3.0"
want "--verify-tag"
want "Footplate_0.3.0_darwin_arm64.zip"
want "Footplate_0.3.0_darwin_amd64.zip"
want "Footplate_0.3.0_darwin_arm64.dmg"
want "Footplate_0.3.0_darwin_amd64.dmg"
want "footplate_0.3.0_linux_arm64.tar.gz"
want "footplate_0.3.0_linux_amd64.tar.gz"
want "footplate_0.3.0_windows_amd64.zip"
want "checksums.txt"
want "7 archives and checksums.txt"
printf '%s\n' "$body" | cmp -s - "$casedir/notes.md" ||
    fail "releases: the notes file is not the body's bytes"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
want "mkdir -p ~/releases/v0.3.0"
want "gh release download v0.3.0 --repo yardr-dev/footplate --pattern checksums.txt --dir ~/releases/v0.3.0 --clobber"
want "minisign -S -s ~/.minisign/yardr-release.key -m ~/releases/v0.3.0/checksums.txt"
want "minisign -V -P $KEY -m ~/releases/v0.3.0/checksums.txt"
want "gh release upload v0.3.0 --repo yardr-dev/footplate ~/releases/v0.3.0/checksums.txt.minisig --clobber"
# With neither signing variable the build is given none, and the note says
# the macOS apps are ad hoc and the disk images unsigned: a group without
# them would release apps another Mac blocks, and this is where it shows.
want "signing: identity=[] profile=[]"
want "The macOS apps are signed ad hoc and not notarized, and the disk images are not signed"
lack "notarized by Apple"
ok "build releases"

# Both signing variables reach the build as the group's environment gave
# them, a value with spaces and brackets whole, and the note says the apps
# and the disk images are signed and notarized.
begin releases-signed
repo
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout" \
    "FOOTPLATE_RELEASE_SIGN_IDENTITY=Developer ID Application: Stand In (ABCDE12345)" \
    "FOOTPLATE_RELEASE_NOTARY_PROFILE=stand-in-profile"
want "signing: identity=[Developer ID Application: Stand In (ABCDE12345)] profile=[stand-in-profile]"
want "The macOS apps and disk images are signed with Developer ID, notarized by Apple and stapled."
lack "signed ad hoc"
want "gh release create v0.3.0"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
ok "build gives the build its signing variables"

# One signing variable without the other is refused by the build script
# itself, the real one, before it builds anything: the stage fails with what
# it said, and nothing is tagged or released.
begin half-pair
repo
real_release
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout" "YARDR_RELEASE_BUILD=scripts/release.sh" \
    "FOOTPLATE_RELEASE_SIGN_IDENTITY=Developer ID Application: Stand In (ABCDE12345)"
want "yardr bead done footplate-rel1 --outcome failed --note -"
want "building v0.3.0 failed"
want "FOOTPLATE_RELEASE_SIGN_IDENTITY is set and FOOTPLATE_RELEASE_NOTARY_PROFILE is not"
lack "gh release create"
lack "fakebuild"
if git -C "$checkout" rev-parse --verify -q "refs/tags/v0.3.0" >/dev/null; then
    fail "half-pair: a tag was made for a refused build"
fi
ok "build refuses half a signing pair"

# A signed release verifies.
begin verify-ok
fixture sig ok
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir"
want "gh release download v0.3.0 --repo yardr-dev/footplate"
want "minisign -V -P $KEY"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
want "the signature on v0.3.0 is good"
want "2 archives match checksums.txt"
# With no macOS archive among them there is no app to look at, and the note
# says so; no Apple tool is asked.
want "holds no macOS archive"
lack "xcrun"
lack "ditto"
ok "verify checks a signed release"

# A bad signature fails, so sign is asked again, with what minisign said.
begin verify-bad-signature
fixture sig ok
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_MINISIGN_RC=1" \
    "FAKE_MINISIGN_OUT=Signature verification failed"
want "yardr bead done footplate-rel1 --outcome failed --note -"
want "the signature on release v0.3.0 is bad"
want "Signature verification failed"
ok "verify fails a bad signature"

# A missing signature fails, so sign is asked again.
begin verify-missing-signature
fixture nosig ok
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir"
want "--outcome failed"
want "checksums.txt.minisig is not on release v0.3.0"
ok "verify fails a missing signature"

# Archives that do not match are questioned: re-signing cannot fix them.
begin verify-mismatch
fixture sig corrupt
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir"
want "--outcome question"
want "do not match its checksums.txt"
ok "verify questions mismatched archives"

# The macOS archives are unpacked, each into a directory of its own (both
# hold a Footplate.app), and each app passes stapler and Gatekeeper: done to
# publish, and the note names the apps it looked at.
begin verify-mac-ok
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
want "its 4 archives match checksums.txt"
count 2 "ditto -x -k "
want "mac-Footplate_0.3.0_darwin_arm64"
want "mac-Footplate_0.3.0_darwin_amd64"
count 2 "xcrun stapler validate "
count 2 "spctl -a -vv -t exec "
want "The macOS apps are notarized"
want "Footplate.app in Footplate_0.3.0_darwin_arm64.zip"
want "Footplate.app in Footplate_0.3.0_darwin_amd64.zip"
ok "verify checks the macOS apps"

# An app with no ticket stapled to it is questioned, with what stapler said,
# and Gatekeeper is not asked.
begin verify-mac-unstapled
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_STAPLER_RC=65" \
    "FAKE_STAPLER_OUT=Footplate.app does not have a ticket stapled to it."
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "does not pass xcrun stapler validate"
want "Footplate.app does not have a ticket stapled to it."
want "it is remade by hand"
lack "spctl -a"
ok "verify questions an app with no ticket stapled"

# Gatekeeper that takes the app from another source than a notarized
# Developer ID is questioned, with what spctl said.
begin verify-mac-not-notarized
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" \
    "FAKE_SPCTL_OUT=Footplate.app: accepted
source=Developer ID"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "does not name source=Notarized Developer ID"
want "source=Developer ID"
ok "verify questions an app Gatekeeper does not call notarized"

# Gatekeeper that refuses the app outright is questioned too, and an
# "Unnotarized Developer ID" source is no "Notarized Developer ID" one.
begin verify-mac-rejected
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_SPCTL_RC=3" \
    "FAKE_SPCTL_OUT=Footplate.app: rejected
source=Unnotarized Developer ID"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "Gatekeeper refuses the Footplate.app"
want "source=Unnotarized Developer ID"
ok "verify questions an app Gatekeeper rejects"

# A zip that cannot be unpacked leaves nothing to look at: questioned, not
# passed.
begin verify-mac-unpacking
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_DITTO_RC=1"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "unpacking Footplate_0.3.0_darwin_amd64.zip of release v0.3.0 failed"
want "ditto: cannot unpack"
lack "xcrun"
ok "verify questions a zip it cannot unpack"

# Nor does a zip with no app in it.
begin verify-mac-no-app
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_DITTO_EMPTY=yes"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "holds no .app to check"
lack "xcrun"
ok "verify questions a zip that holds no app"

# Off a Mac (no xcrun) the apps are not looked at, and the done note says
# that: the release still goes to publish, with nothing unpacked.
begin verify-mac-no-xcrun
fixture sig ok mac
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "PATH=$nomac"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
want "its 4 archives match checksums.txt"
want "were not checked for notarization: PATH here lacks xcrun, which a Mac has"
lack "ditto -x -k"
lack "spctl -a"
ok "verify says it skipped the macOS apps off a Mac"

# The disk images beside the zips are checked too: each is stapled and
# notarized itself, mounts read-only at a mount point of its own, holds
# the app and the Applications link (a .DS_Store may ride along), and
# holds a notarized app; then each is detached. Done to publish, and the
# note names the images beside the apps.
begin verify-mac-dmg-ok
fixture sig ok mac dmg
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
want "its 6 archives match checksums.txt"
count 2 "hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "
want "mount-Footplate_0.3.0_darwin_arm64"
want "mount-Footplate_0.3.0_darwin_amd64"
count 2 "hdiutil detach "
count 6 "xcrun stapler validate "
count 2 "spctl -a -vv -t open --context context:primary-signature "
count 4 "spctl -a -vv -t exec "
count 2 "ditto -x -k "
want "The macOS apps and disk images are notarized"
want "Footplate_0.3.0_darwin_arm64.dmg"
want "Footplate_0.3.0_darwin_amd64.dmg"
want "Footplate.app in Footplate_0.3.0_darwin_arm64.zip"
ok "verify checks the macOS disk images"

# An image with no ticket stapled to it is questioned, with what stapler
# said, and nothing is mounted.
begin verify-mac-dmg-unstapled
fixture sig ok mac dmg
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_STAPLER_RC=65" \
    "FAKE_STAPLER_OUT=Footplate_0.3.0_darwin_amd64.dmg does not have a ticket stapled to it."
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "the disk image Footplate_0.3.0_darwin_amd64.dmg of release v0.3.0 does not pass xcrun stapler validate"
want "does not have a ticket stapled to it."
want "it is remade by hand"
lack "spctl -a"
lack "hdiutil attach"
ok "verify questions an image with no ticket stapled"

# Gatekeeper that takes the image from another source than a notarized
# Developer ID is questioned, with what spctl said.
begin verify-mac-dmg-not-notarized
fixture sig ok mac dmg
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" \
    "FAKE_SPCTL_OUT=Footplate_0.3.0_darwin_amd64.dmg: accepted
source=Developer ID"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "does not name source=Notarized Developer ID"
want "Footplate_0.3.0_darwin_amd64.dmg"
want "source=Developer ID"
lack "hdiutil attach"
ok "verify questions an image Gatekeeper does not call notarized"

# An image holding more than the app and the link is questioned, and the
# mount is still let go of.
begin verify-mac-dmg-extra
fixture sig ok mac dmg
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_HDIUTIL_EXTRA=yes"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "holds more than the app and the Applications link (..junk.txt)"
count 1 "hdiutil attach "
count 1 "hdiutil detach "
ok "verify questions an image with an extra file, and detaches it"

# As is an image without the Applications link.
begin verify-mac-dmg-no-link
fixture sig ok mac dmg
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "FAKE_HDIUTIL_NOLINK=yes"
want "yardr bead done footplate-rel1 --outcome question --note -"
lack "yardr bead done footplate-rel1 --note -"
want "holds no Applications link to /Applications"
count 1 "hdiutil attach "
count 1 "hdiutil detach "
ok "verify questions an image without the Applications link, and detaches it"

# Without hdiutil the macOS archives are not looked at, and the done note
# says that: the release still goes to publish, with nothing mounted or
# unpacked.
begin verify-mac-no-hdiutil
fixture sig ok mac dmg
show "footplate v0.3.0" "notes"
run verify "FAKE_RELEASE_DIR=$rdir" "PATH=$nohdiutil"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
want "its 6 archives match checksums.txt"
want "were not checked for notarization: PATH here lacks hdiutil, which a Mac has"
lack "hdiutil attach"
lack "ditto -x -k"
lack "spctl -a"
ok "verify says it skipped the macOS check without hdiutil"

# Publish files the footplate sync bead and advances it to new.
begin publish-ok
repo
git -C "$checkout" tag v0.3.0
at=$(git -C "$checkout" rev-parse --short refs/tags/v0.3.0)
show "footplate v0.3.0" "notes"
run publish "YARDR_DEPOT_PATH=$checkout" "FAKE_CREATE_ID=yardr.dev-s9nc"
want "yardr bead create Sync footplate v0.3.0 to yardr.dev: scripts/sync-release footplate v0.3.0 -d yardr.dev -p 1 -b - -l difficulty:1,size:small --json"
want "https://github.com/yardr-dev/footplate/releases/tag/v0.3.0"
want "built from $at"
want "scripts/sync-release footplate v0.3.0"
want "public/releases/footplate/latest.json"
want "https://yardr.dev/releases/footplate/latest.json"
want "yardr bead advance yardr.dev-s9nc --to new"
want "publish: filed yardr.dev-s9nc"
want "yardr bead done footplate-rel1 --note -"
lack "--outcome"
ok "publish files the sync bead"

# Without the tag publish has nothing to point at.
begin publish-missing-tag
repo
show "footplate v0.3.0" "notes"
run publish "YARDR_DEPOT_PATH=$checkout"
want "--outcome failed"
want "tag v0.3.0 is not in"
lack "bead create"
ok "publish refuses a missing tag"

echo "test-release-stage.sh: pass ($n cases)"
