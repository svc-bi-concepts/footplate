#!/bin/sh
# Tests for the warm cache of .yardr/test, the gate: a worktree seeded from
# the cache compiles footplate from its own sources, whatever their times,
# and takes nothing but its dependencies from the seed. The gate is run for
# real in worktrees of a throwaway crate named footplate (the package the
# gate cleans) with a dependency of its own to stand for gpui's tree, and in
# a home of its own: nothing here compiles gpui, and the machine's own cache
# is neither read nor written.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
gate=$root/.yardr/test
tmp=$(mktemp -d)
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
# kept. A target dir of the person's own would switch the seeding off.
PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null 2>&1 || {
    echo "test-gate.sh: requires cargo" >&2
    exit 77
}
CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
CARGO_TERM_COLOR=never
export PATH CARGO_HOME RUSTUP_HOME CARGO_TERM_COLOR
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR
real_cargo=$(command -v cargo)
mkdir "$tmp/home"
HOME=$tmp/home
export HOME

# A crate named footplate, with a dependency on a path.
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
    cat > "$1/dep/src/lib.rs" <<'EOF'
pub fn answer() -> u32 {
    42
}
EOF
    cat > "$1/src/main.rs" <<'EOF'
fn main() {
    println!("{}", dep::answer());
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

# Whether the log $1 has the line $2 in it.
said() {
    grep -q -F -- "$2" "$tmp/$1.log"
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
mkdir "$tmp/bin"
cat > "$tmp/bin/cargo" <<EOF
#!/bin/sh
[ "\$1" = clean ] && exit 1
exec "$real_cargo" "\$@"
EOF
chmod +x "$tmp/bin/cargo"
make_tree "$tmp/c"
age "$tmp/c"
(PATH=$tmp/bin:$PATH && gate_in c c)
said c "Compiling dep " || fail "a failed clean kept the seed"
ok "a clean that fails drops the seed"

echo "test-gate.sh: pass ($n cases)"
