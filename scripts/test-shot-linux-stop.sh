#!/bin/sh
# Run in a Linux container to verify the previous-shot process is stopped.
set -eu

if [ ! -r /proc/self/stat ]; then
    echo "test-shot-linux-stop.sh: requires Linux /proc" >&2
    exit 77
fi

helper=$(dirname "$0")/shot-linux-stop.sh
tmp=$(mktemp -d)
session=
app=
cleanup() {
    if [ -n "$app" ]; then
        kill "$app" 2>/dev/null || true
    fi
    if [ -n "$session" ]; then
        kill "$session" 2>/dev/null || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

cat > "$tmp/footplate-sleeper" <<'EOF'
#!/bin/bash
exec -a /target/debug/footplate sleep 60
EOF
chmod +x "$tmp/footplate-sleeper"
cat > "$tmp/dbus-run-session" <<'EOF'
#!/bin/bash
"$1" &
wait
EOF
chmod +x "$tmp/dbus-run-session"

bash "$tmp/dbus-run-session" "$tmp/footplate-sleeper" &
session=$!
printf '%s\n' "$session" > "$tmp/footplate.pid"
sleep 1
app=$(cat "/proc/$session/task/$session/children")
if [ -z "$app" ]; then
    echo "test-shot-linux-stop.sh: test Footplate process did not start" >&2
    exit 1
fi

sh "$helper" "$tmp/footplate.pid"
wait "$session" 2>/dev/null || true
if kill -0 "$app" 2>/dev/null; then
    echo "test-shot-linux-stop.sh: previous Footplate process is still running" >&2
    exit 1
fi
if kill -0 "$session" 2>/dev/null; then
    echo "test-shot-linux-stop.sh: previous dbus session is still running" >&2
    exit 1
fi
echo "test-shot-linux-stop.sh: pass"
