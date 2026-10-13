#!/bin/sh
# Stop the dbus-run-session and Footplate child from the previous shot.
set -e

pid_file=${1:?pid file is required}
[ -f "$pid_file" ] || exit 0
pid=$(cat "$pid_file")
case "$pid" in
    ''|*[!0-9]*)
        echo "shot-linux-stop.sh: invalid pid in $pid_file" >&2
        exit 1
        ;;
esac

# Do not signal a reused PID: only stop a process that is still our session.
if [ ! -r "/proc/$pid/cmdline" ] || ! grep -qa dbus-run-session "/proc/$pid/cmdline"; then
    rm -f "$pid_file"
    exit 0
fi

# /proc/<pid>/stat puts the command in parentheses, so remove through the
# final ") " before splitting fields. The second remaining field is its PPID.
for d in /proc/[0-9]*; do
    stat=$(cat "$d/stat" 2>/dev/null) || continue
    rest=${stat##*) }
    set -- $rest
    [ "${2-}" = "$pid" ] || continue
    if grep -qa footplate "$d/cmdline" 2>/dev/null; then
        child=${d##*/}
        kill "$child" 2>/dev/null || true
    fi
done

kill "$pid" 2>/dev/null || true
rm -f "$pid_file"
sleep 2
