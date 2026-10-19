#!/bin/sh
# Drop checkout build trees under a Linux target volume when nothing in
# them was written in the last week. Active builds leave markers so another
# build cannot prune a tree while it is in use. Says what went, on stderr;
# files at the volume's root are left alone.
# With a live list (a fifth argument naming a file with one live tree name
# per line), a removed checkout's tree goes at once instead of after a
# week, and its markers go with it; a live checkout's tree keeps the 7-day
# rule. Without one, prune as before.
set -e

root=${1:?target volume is required}
own=${2:?own build tree is required}
marker=${3:?active build marker is required}
label=${4:-shot-linux.sh}
live_list=${5:-}
active=$root/.shot-active
mkdir -p "$active"

# Register before pruning and keep the marker until the caller no longer uses
# the tree. Markers left by a killed run age out with the build tree.
# The build starts only after this helper returns, so a concurrent prune that
# wins the marker-creation race can only discard this build's old cache.
touch "$marker"
for stale_marker in "$active"/*; do
    [ -f "$stale_marker" ] || continue
    if find "$stale_marker" -mtime +7 -print 2>/dev/null | grep -q .; then
        rm -f "$stale_marker"
    fi
done

# A missing live list is an old caller, or a caller whose list never
# arrived: prune as before, never from an empty list.
have_live=false
if [ -n "$live_list" ]; then
    if [ -f "$live_list" ] && [ -r "$live_list" ]; then
        have_live=true
    else
        echo "$label: no live list at $live_list; pruning as before" >&2
    fi
fi
live() {
    live_wanted=$1
    while IFS= read -r live_name || [ -n "$live_name" ]; do
        if [ "$live_name" = "$live_wanted" ]; then
            return 0
        fi
    done < "$live_list"
    return 1
}

for d in "$root"/*; do
    [ -d "$d" ] || continue
    if [ "$d" = "$own" ]; then
        continue
    fi
    name=${d##*/}
    if [ "$name" = ".shot-active" ]; then
        continue
    fi
    in_use=false
    for shot in "$active/$name-"*; do
        if [ -f "$shot" ]; then
            in_use=true
            break
        fi
    done
    if [ "$in_use" = true ]; then
        continue
    fi
    if [ "$have_live" = true ] && ! live "$name"; then
        if rm -rf "$d"; then
            echo "$label: pruned build tree of a removed checkout $d" >&2
        else
            echo "$label: could not prune build tree of a removed checkout $d" >&2
        fi
        continue
    fi
    if find "$d" -mtime -7 -print 2>/dev/null | grep -q .; then
        continue
    fi
    if rm -rf "$d"; then
        echo "$label: pruned stale build tree $d" >&2
    else
        echo "$label: could not prune stale build tree $d" >&2
    fi
done

# Markers of removed checkouts go with their trees: a crashed shot's marker
# must not guard its tree forever. After the loop, so a marker still guards
# its tree for this run.
if [ "$have_live" = true ]; then
    for m in "$active"/*; do
        [ -f "$m" ] || continue
        if [ "$m" = "$marker" ]; then
            continue
        fi
        tree=${m##*/}
        tree=${tree%-*}
        if ! live "$tree"; then
            rm -f "$m"
        fi
    done
fi
