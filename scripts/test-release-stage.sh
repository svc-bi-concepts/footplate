#!/bin/sh
# Tests for scripts/release-stage, the release flow's scripted stages. The
# script is run the way the exec group runs it, against a throwaway git
# repository with a local bare remote, with stand-ins for gh, minisign and
# yardr first on PATH; what it asked of them is asserted from their shared
# call log. The build step runs a stand-in build script (the real
# scripts/release.sh is test-release.sh's), so no test compiles footplate.
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
# with the note it carried on stdin, and mints one sync bead id.
cat >"$bin/yardr" <<'EOF'
#!/bin/sh
printf 'yardr %s\n' "$*" >>"$FAKE_DIR/calls.log"
case "$1 $2" in
"bead show")
  cat "$FAKE_DIR/show.json"
  ;;
"prime --bead")
  cat "$FAKE_DIR/prime.json"
  ;;
"bead done" | "bead advance")
  { echo "--note--"; cat; echo "--end--"; } >>"$FAKE_DIR/calls.log"
  ;;
"bead create")
  { echo "--create--"; cat; echo "--end--"; } >>"$FAKE_DIR/calls.log"
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

# fakebuild stands in for scripts/release.sh: the four footplate archives
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
if [ "${FAKE_BUILD_OMIT:-}" != "Footplate_${number}_darwin_amd64.zip" ]; then
    printf 'fake footplate darwin amd64 for %s\n' "$1" >"$2/Footplate_${number}_darwin_amd64.zip"
fi
printf 'fake footplate linux arm64 for %s\n' "$1" >"$2/footplate_${number}_linux_arm64.tar.gz"
printf 'fake footplate linux amd64 for %s\n' "$1" >"$2/footplate_${number}_linux_amd64.tar.gz"
if [ "${FAKE_BUILD_EXTRA:-}" = yes ]; then
    printf 'unexpected release asset\n' >"$2/unexpected.txt"
fi
# shellcheck disable=SC2086 # $FAKE_SUM is a list
(cd "$2" && $FAKE_SUM Footplate_*_darwin_*.zip footplate_*_linux_*.tar.gz >checksums.txt)
EOF
chmod +x "$bin/yardr" "$bin/gh" "$bin/minisign" "$bin/fakebuild"

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
    cp "$script_dir/release.sh" "$script_dir/linux-container.sh" "$script_dir/bundle-mac.sh" "$checkout/scripts/"
    git -C "$checkout" add -A
    git -C "$checkout" commit -qm scripts
    git -C "$checkout" push -q origin main
}

# fixture <sig|nosig> <ok|corrupt>: a release for verify to download, of
# two archives with a checksums.txt over them.
fixture() {
    rdir=$casedir/release
    mkdir -p "$rdir"
    printf 'a\n' >"$rdir/a.tar.gz"
    printf 'b\n' >"$rdir/b.tar.gz"
    # shellcheck disable=SC2086 # $SUM is a list
    (cd "$rdir" && $SUM a.tar.gz b.tar.gz >checksums.txt)
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

# A partial archive set is refused before the tag or release is created.
begin missing-archive
repo
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout" \
    "FAKE_BUILD_OMIT=Footplate_0.3.0_darwin_amd64.zip"
want "yardr bead done footplate-rel1 --outcome failed --note -"
want "the build left no Footplate_0.3.0_darwin_amd64.zip"
lack "gh release create"
if git -C "$checkout" rev-parse --verify -q "refs/tags/v0.3.0" >/dev/null; then
    fail "missing-archive: a tag was made for a partial build"
fi
ok "build refuses a missing archive"

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
want "footplate_0.3.0_linux_arm64.tar.gz"
want "footplate_0.3.0_linux_amd64.tar.gz"
want "checksums.txt"
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
# the macOS apps are ad hoc: a group without them would release apps another
# Mac blocks, and this is where it shows.
want "signing: identity=[] profile=[]"
want "The macOS apps are signed ad hoc and not notarized"
lack "notarized by Apple"
ok "build releases"

# Both signing variables reach the build as the group's environment gave
# them, a value with spaces and brackets whole, and the note says the apps
# are signed and notarized.
begin releases-signed
repo
show "footplate v0.3.0" "notes"
run build "YARDR_DEPOT_PATH=$checkout" \
    "FOOTPLATE_RELEASE_SIGN_IDENTITY=Developer ID Application: Stand In (ABCDE12345)" \
    "FOOTPLATE_RELEASE_NOTARY_PROFILE=stand-in-profile"
want "signing: identity=[Developer ID Application: Stand In (ABCDE12345)] profile=[stand-in-profile]"
want "The macOS apps are signed with Developer ID, notarized by Apple and stapled."
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
