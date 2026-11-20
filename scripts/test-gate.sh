#!/bin/sh
# Tests for .yardr/test, the gate: its slot on the build host, and its warm
# cache: a worktree seeded from the cache compiles footplate from its own
# sources, whatever their times, in every build directory the seed holds (the
# debug one the gate builds, a release, a target's), and takes nothing but its
# dependencies from the seed.
# The gate is run for real in worktrees of a throwaway crate named footplate
# (the package the gate cleans) with a dependency of its own to stand for
# gpui's tree, and in a home of its own: nothing here compiles gpui, and the
# machine's own cache is neither read nor written.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
gate=$root/.yardr/test
tmp=$(cd "$(mktemp -d)" && pwd -P)
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fail() {
    echo "test-gate.sh: $1" >&2
    exit 1
}

# The gate puts $HOME/.cargo/bin first on PATH, and the home below has none:
# the toolchain is found where it is now, with cargo's and rustup's homes
# kept.
PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null 2>&1 || {
    echo "test-gate.sh: requires cargo" >&2
    exit 77
}
CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
CARGO_TERM_COLOR=never
export PATH CARGO_HOME RUSTUP_HOME CARGO_TERM_COLOR
# A target dir of the person's own would put the targets outside the worktrees
# in $tmp, where the tests look for them.
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR GITHUB_ACTIONS
real_cargo=$(command -v cargo)
mkdir "$tmp/home"
HOME=$tmp/home
export HOME

# A crate named footplate, with a dependency on a path. It prints the name of
# its directory, so that a binary says which tree built it.
make_tree() {
    mkdir -p "$1/src" "$1/dep/src"
    cat > "$1/Cargo.toml" <<'EOF'
[package]
name = "footplate"
version = "0.1.0"
edition = "2021"

[dependencies]
dep = { path = "dep" }
EOF
    cat > "$1/dep/Cargo.toml" <<'EOF'
[package]
name = "dep"
version = "0.1.0"
edition = "2021"
EOF
    cat > "$1/dep/build.rs" <<'EOF'
fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=DEP_OUT={out}");
}
EOF
    cat > "$1/dep/src/lib.rs" <<'EOF'
pub const OUT: &str = env!("DEP_OUT");

pub fn answer() -> u32 {
    42
}
EOF
    sed "s/@name@/${1##*/}/" > "$1/src/main.rs" <<'EOF'
fn main() {
    println!("@name@");
}

#[cfg(test)]
mod tests {
    #[test]
    fn first() {
        assert_eq!(dep::answer(), 42);
    }
}
EOF
    (cd "$1" && cargo generate-lockfile --offline -q)
}

# What a branch adds to the crate: a test the cache's build does not have.
add_test() {
    cat >> "$1/src/main.rs" <<'EOF'

#[cfg(test)]
mod more {
    #[test]
    fn second() {}
}
EOF
}

# A worktree held for days, or a branch behind main: none of its files is
# newer than the build the cache holds.
age() {
    find "$1" -type f -exec touch -t 200001010000 {} +
}

# Runs the gate in the worktree $1 and keeps what it said in $tmp/$2.log.
gate_in() {
    (cd "$tmp/$1" && sh "$gate") > "$tmp/$2.log" 2>&1 || {
        tail -n 40 "$tmp/$2.log" >&2
        fail "the gate failed in $1"
    }
}

# Runs cargo with the arguments after the second in the worktree $1 and keeps
# what it said in $tmp/$2.log. Its input is closed: it is run in a loop that
# reads its lines from a here-document, which a program reading its input
# would take them from.
cargo_in() {
    dir=$1
    log=$2
    shift 2
    (cd "$tmp/$dir" && cargo "$@") < /dev/null > "$tmp/$log.log" 2>&1 || {
        tail -n 40 "$tmp/$log.log" >&2
        fail "cargo $* failed in $dir"
    }
}

# Whether the log $1 has the line $2 in it.
said() {
    grep -q -F -- "$2" "$tmp/$1.log"
}

# A cargo in $tmp/bin, for the front of PATH, that fails a clean whose
# arguments match the case pattern $1 and is the real one for anything else.
refusing_cargo() {
    mkdir -p "$tmp/bin"
    cat > "$tmp/bin/cargo" <<EOF
#!/bin/sh
if [ "\$1" = clean ]; then
    case "\$*" in $1) exit 1 ;; esac
fi
exec "$real_cargo" "\$@"
EOF
    chmod +x "$tmp/bin/cargo"
}

n=0
ok() {
    echo "ok - $1"
    n=$((n + 1))
}

# The first gate has no cache to start from, and leaves one.
make_tree "$tmp/a"
gate_in a a
[ -f "$HOME/.cache/footplate-warm-target/from" ] || fail "the gate left no cache"
ok "a gate that passes leaves the cache"

# The next worktree is older than the cache's build and has a test that
# build lacks. Its tests are its own, and the dependency is the seed's.
make_tree "$tmp/b"
add_test "$tmp/b"
age "$tmp/b"
gate_in b b
said b "test more::second ... ok" || fail "a seeded worktree ran the cache's tests, not its own"
said b "Compiling footplate " || fail "a seeded worktree did not compile footplate"
! said b "Compiling dep " || fail "a seeded worktree compiled the dependency of the seed again"
ok "a seeded worktree compiles footplate and keeps the seed's dependency"

# What is not seeded is not cleaned: the next run in it is incremental.
gate_in b b-again
! said b-again "Compiling footplate " || fail "a worktree's own build was compiled again"
! said b-again "Checking footplate " || fail "a worktree's own check was run again"
ok "a worktree with a target of its own keeps it"

# A clean that fails drops the seed, so the build is from nothing.
refusing_cargo '*'
make_tree "$tmp/c"
age "$tmp/c"
(PATH=$tmp/bin:$PATH && gate_in c c)
said c "Compiling dep " || fail "a failed clean kept the seed"
ok "a clean that fails drops the seed"

# The seed holds builds of footplate besides the debug one the gate makes: a
# release (what scripts/bundle-mac.sh is run after) and builds for a target
# (the Windows cross-check, the release's Macs), each in a directory of its
# own; the host stands for the target here. Cargo goes by the same rule for
# each, so a build of them outside the gate would hand on the seed's binary.
# The first tree makes them before its gate, which leaves its whole target in
# the cache; the second is older than all of that and has no target.
host=$(rustc -vV | sed -n 's/^host: //p')
make_tree "$tmp/d"
cargo_in d d-release build --release --locked
cargo_in d d-target build --target "$host" --locked
cargo_in d d-target-release build --release --target "$host" --locked
gate_in d d
make_tree "$tmp/e"
age "$tmp/e"
gate_in e e
while read -r binary args; do
    # $args is several words.
    cargo_in e e-build build --locked $args
    says=$("$tmp/e/target/$binary" 2>&1 || true)
    [ "$says" = e ] || fail "a seeded worktree's build for '$args' is not its own: it says '$says'"
    ! said e-build "Compiling dep " || fail "a seeded worktree compiled the dependency of the seed again for '$args'"
done <<EOF
release/footplate --release
$host/debug/footplate --target $host
$host/release/footplate --release --target $host
EOF
ok "a seeded worktree's release and target builds are its own"

# The cache is now the second tree's, with all of those directories: a clean
# that fails after the first, here the release's, drops the seed as well.
refusing_cargo '*--release*'
make_tree "$tmp/f"
age "$tmp/f"
(PATH=$tmp/bin:$PATH && gate_in f f)
said f "Compiling dep " || fail "a clean that failed after the first kept the seed"
ok "a clean that fails after the first drops the seed"

# The cache may be a link to another volume, as the yard keeps it on the
# mini's SSD: a gate reads it through the link and replaces what the link
# points to, beside it, and the link stays. $tmp/ssd stands for the volume.
cache=$HOME/.cache/footplate-warm-target
# Whether the cache is a link to $1 with the build of the worktree $2 in it,
# and nothing of the cache is a real directory in the home.
linked_to() {
    [ -L "$cache" ] || fail "the gate in $2 replaced the link with a directory"
    [ "$(readlink "$cache")" = "$1" ] || fail "the gate in $2 changed the link to $(readlink "$cache")"
    [ "$(cat "$cache/from")" = "$tmp/$2" ] || fail "the cache behind the link is not $2's build"
    [ -z "$(find "$HOME/.cache" -name 'footplate-warm-target*' -type d)" ] || fail "the gate in $2 left a directory of the cache in the home"
}
mkdir -p "$tmp/ssd/cache"
mv "$cache" "$tmp/ssd/cache/footplate-warm-target"
ln -s "$tmp/ssd/cache/footplate-warm-target" "$cache"
make_tree "$tmp/g"
age "$tmp/g"
gate_in g g
! said g "Compiling dep " || fail "a worktree was not seeded through the link"
linked_to "$tmp/ssd/cache/footplate-warm-target" g
ok "a cache that is a link is read through it and stays a link"

# A link may name its target from the link's own directory.
rm "$cache"
ln -s ../../ssd/cache/footplate-warm-target "$cache"
make_tree "$tmp/h"
age "$tmp/h"
gate_in h h
! said h "Compiling dep " || fail "a worktree was not seeded through a relative link"
linked_to ../../ssd/cache/footplate-warm-target h
ok "a cache that is a relative link stays the same link"

# A link may name its target with slashes after it, as a shell completes a
# directory: the cache is still replaced beside the target, not inside it.
rm "$cache"
ln -s "$tmp/ssd/cache/footplate-warm-target//" "$cache"
make_tree "$tmp/h2"
age "$tmp/h2"
gate_in h2 h2
! said h2 "Compiling dep " || fail "a worktree was not seeded through a link with a slash after it"
linked_to "$tmp/ssd/cache/footplate-warm-target//" h2
[ -z "$(find "$tmp/ssd/cache/footplate-warm-target" -mindepth 1 -maxdepth 1 -name '.*')" ] || fail "the gate worked inside the target of a link with a slash after it"
ok "a cache that is a link with a slash after it is replaced beside its target"

# A link to a volume that is not mounted: the gate runs without the cache and
# makes nothing where the volume would be, which is the internal disk.
rm "$cache"
ln -s "$tmp/gone/cache/footplate-warm-target" "$cache"
make_tree "$tmp/i"
age "$tmp/i"
gate_in i i
said i "Compiling dep " || fail "a worktree was seeded from a link to nothing"
said i "warm cache: $tmp/gone/cache is missing" || fail "the gate did not say why it ran without the cache"
[ ! -e "$tmp/gone" ] || fail "the gate made the missing volume's directory"
[ "$(readlink "$cache")" = "$tmp/gone/cache/footplate-warm-target" ] || fail "the gate changed a link to nothing"
[ -z "$(find "$HOME/.cache" -name 'footplate-warm-target*' -type d)" ] || fail "the gate left a directory of the cache in the home"
ok "a link to a missing volume switches the cache off"

# Links that lead round to themselves never name a directory: the gate runs
# without the cache and leaves them as they are.
rm "$cache"
ln -s "$cache.loop" "$cache"
ln -s "$cache" "$cache.loop"
make_tree "$tmp/j"
age "$tmp/j"
gate_in j j
said j "Compiling dep " || fail "a worktree was seeded through links that do not end"
said j "do not end, so the gate runs without the cache" || fail "the gate did not say the links do not end"
[ "$(readlink "$cache")" = "$cache.loop" ] || fail "the gate changed links that do not end"
[ -z "$(find "$HOME/.cache" -name 'footplate-warm-target*' -type d)" ] || fail "the gate left a directory of the cache in the home"
ok "links that do not end switch the cache off"

# The volume may go while the gate builds: the refresh after it makes nothing
# where the volume was, and the gate still passes. A cargo in $tmp/bin takes
# the volume away once the gate's last build is done.
rm "$cache" "$cache.loop"
mkdir -p "$tmp/vol/cache"
ln -s "$tmp/vol/cache/footplate-warm-target" "$cache"
mkdir -p "$tmp/bin"
cat > "$tmp/bin/cargo" <<EOF
#!/bin/sh
"$real_cargo" "\$@" || exit
if [ "\$1" = build ]; then
    rm -rf "$tmp/vol"
fi
EOF
chmod +x "$tmp/bin/cargo"
make_tree "$tmp/k"
(PATH=$tmp/bin:$PATH && gate_in k k)
said k "warm cache: $tmp/vol/cache is missing, so the gate leaves the cache as it was" || fail "the gate did not say why it left the cache"
[ ! -e "$tmp/vol" ] || fail "the gate made the directory of a volume that went"
[ "$(readlink "$cache")" = "$tmp/vol/cache/footplate-warm-target" ] || fail "the gate changed the link to a volume that went"
[ -z "$(find "$HOME/.cache" -name 'footplate-warm-target*' -type d)" ] || fail "the gate left a directory of the cache in the home"
ok "a volume that goes during the gate is not made again"

# An explicit target dir, as the yard gives a bead's builds on the SSD: the gate
# seeds it from the cache and refreshes the cache from it, the cache names the
# directory it was built in, and the worktree gets no target/ of its own. The
# cache is the link to $tmp/ssd again, and the targets are beside it there.
rm "$cache"
ln -s "$tmp/ssd/cache/footplate-warm-target" "$cache"
# Runs the gate in the worktree $1 with the environment variable $3 set to the
# directory $4, and keeps what it said in $tmp/$2.log.
gate_td() {
    (cd "$tmp/$1" && env "$3=$4" sh "$gate") > "$tmp/$2.log" 2>&1 || {
        tail -n 40 "$tmp/$2.log" >&2
        fail "the gate failed in $1 with $3 set"
    }
}
make_tree "$tmp/m"
age "$tmp/m"
gate_td m m CARGO_TARGET_DIR "$tmp/ssd/cargo-target/footplate/m"
! said m "Compiling dep " || fail "a worktree with an explicit target was not seeded"
[ -f "$tmp/ssd/cargo-target/footplate/m/.rustc_info.json" ] || fail "the explicit target was not built on the volume"
[ -f "$cache/target/.rustc_info.json" ] || fail "the gate with an explicit target left no cache"
[ "$(cat "$cache/target-dir")" = "$tmp/ssd/cargo-target/footplate/m" ] || fail "the cache does not name the explicit target it was built in: $(cat "$cache/target-dir")"
[ "$(cat "$cache/from")" = "$tmp/m" ] || fail "the cache does not name the worktree it was built in"
[ ! -e "$tmp/m/target" ] || fail "the gate made a target/ in its worktree beside an explicit one"
ok "a gate with an explicit target dir is seeded, and leaves the cache and its target on the volume"

# A worktree and a target whose names hold the cache's as a prefix: the build
# script's recorded out dir must name this target once, not the cache's and not
# a name made of both.
make_tree "$tmp/m2"
age "$tmp/m2"
gate_td m2 m2 CARGO_TARGET_DIR "$tmp/ssd/cargo-target/footplate/m2"
! said m2 "Compiling dep " || fail "a worktree whose name holds the cache's was not seeded"
td2=$tmp/ssd/cargo-target/footplate/m2
found=0
for f in "$td2"/debug/build/dep-*/root-output; do
    [ -f "$f" ] || continue
    found=1
    case $(cat "$f") in
        "$td2"/debug/build/dep-*/out) ;;
        *) fail "the seed's build script out dir names '$(cat "$f")', not the new target" ;;
    esac
done
[ "$found" = 1 ] || fail "the seeded target has no build script out dir"
ok "a seeded target's build script paths name the new target once"

# A seeded explicit target builds footplate from its own sources, and its own
# build stays as it is for the next run. CARGO_BUILD_TARGET_DIR, the other
# variable cargo reads, is seeded the same way.
make_tree "$tmp/n"
add_test "$tmp/n"
age "$tmp/n"
gate_td n n CARGO_BUILD_TARGET_DIR "$tmp/ssd/cargo-target/footplate/n"
said n "test more::second ... ok" || fail "an explicit target's gate ran the cache's tests, not its own"
said n "Compiling footplate " || fail "an explicit target's gate did not compile footplate"
! said n "Compiling dep " || fail "an explicit target's gate compiled the dependency of the seed again"
[ ! -e "$tmp/n/target" ] || fail "the gate made a target/ in its worktree beside CARGO_BUILD_TARGET_DIR"
[ "$(cat "$cache/target-dir")" = "$tmp/ssd/cargo-target/footplate/n" ] || fail "the cache does not name CARGO_BUILD_TARGET_DIR's target"
gate_td n n-again CARGO_BUILD_TARGET_DIR "$tmp/ssd/cargo-target/footplate/n"
! said n-again "Compiling footplate " || fail "an explicit target's own build was compiled again"
ok "a seeded explicit target builds footplate from its own sources and keeps its build"

# A target that holds something already is not seeded, and nothing in it goes.
make_tree "$tmp/q"
age "$tmp/q"
mkdir -p "$tmp/ssd/cargo-target/footplate/q"
echo mine > "$tmp/ssd/cargo-target/footplate/q/keep"
gate_td q q CARGO_TARGET_DIR "$tmp/ssd/cargo-target/footplate/q"
[ "$(cat "$tmp/ssd/cargo-target/footplate/q/keep")" = mine ] || fail "the gate removed what an explicit target held"
said q "Compiling dep " || fail "a target that held something was seeded"
ok "a target that holds something is built in as it is"

# On GitHub Actions the runner is thrown away: the gate neither seeds from the
# cache nor writes it.
before=$(cat "$cache/from" "$cache/target-dir")
make_tree "$tmp/r"
age "$tmp/r"
(GITHUB_ACTIONS=true && export GITHUB_ACTIONS && gate_td r r CARGO_TARGET_DIR "$tmp/ssd/cargo-target/footplate/r")
said r "Compiling dep " || fail "a gate on GitHub Actions was seeded"
[ "$(cat "$cache/from" "$cache/target-dir")" = "$before" ] || fail "a gate on GitHub Actions wrote the cache"
ok "a gate on GitHub Actions leaves the cache alone"

# A clean that fails with an explicit target drops that seed, so the build is
# from nothing.
refusing_cargo '*'
make_tree "$tmp/o"
age "$tmp/o"
(PATH=$tmp/bin:$PATH && gate_td o o CARGO_TARGET_DIR "$tmp/ssd/cargo-target/footplate/o")
said o "Compiling dep " || fail "a failed clean with an explicit target kept the seed"
ok "a clean that fails with an explicit target drops the seed"

# On the build host a shared slot script stands at a fixed place in the home:
# the gate hands itself to it, and its exit code is the gate's. The stand-in
# below says what it was given and exits 7 without running the gate.
slot=$HOME/ws_yardr/integrations/gate-slot.sh
mkdir -p "${slot%/*}"
cat > "$slot" <<EOF
#!/bin/sh
echo "slot: \$*" > "$tmp/slot.args"
exit 7
EOF
# This test may itself run in a slot, which marks what it starts as held.
rc=0
(unset YARD_GATE_SLOT_HELD && cd "$tmp/a" && sh "$gate") > "$tmp/slot.log" 2>&1 || rc=$?
[ "$rc" = 7 ] || fail "the gate's exit code through the slot is $rc, not 7"
[ "$(cat "$tmp/slot.args")" = "slot: -- sh $gate" ] || fail "the slot was given '$(cat "$tmp/slot.args")'"
ok "the gate runs itself in a slot where the slot script stands"

# Inside a slot the gate goes straight on and asks for no second one.
rm "$tmp/slot.args"
(YARD_GATE_SLOT_HELD=1 && export YARD_GATE_SLOT_HELD && gate_in a a-held)
[ ! -e "$tmp/slot.args" ] || fail "a gate inside a slot asked for another"
ok "a gate inside a slot runs at once"

echo "test-gate.sh: pass ($n cases)"
