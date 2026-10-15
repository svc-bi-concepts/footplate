#!/bin/sh
# Drop checkout build trees under the shot target volume when nothing in
# them was written in the last week. A deleted or idle checkout's tree costs
# GBs no shot will warm from. Active shots leave markers so another shot
# cannot prune a tree between its build and launch. Says what went, on
# stderr; files at the volume's root are left alone.
set -e

root=${1:?target volume is required}
own=${2:?own build tree is required}
marker=${3:?active shot marker is required}
active=$root/.shot-active
mkdir -p "$active"

# Register before pruning and keep the marker until shot-linux.sh has copied
# the picture. Markers left by a killed run age out with the build tree.
# The build starts only after this helper returns, so a concurrent prune that
# wins the marker-creation race can only discard this shot's old cache.
touch "$marker"
for stale_marker in "$active"/*; do
    [ -f "$stale_marker" ] || continue
    if find "$stale_marker" -mtime +7 -print 2>/dev/null | grep -q .; then
        rm -f "$stale_marker"
    fi
done

for d in "$root"/*; do
    [ -d "$d" ] || continue
    if [ "$d" = "$own" ]; then
        continue
    fi
    if find "$d" -mtime -7 -print 2>/dev/null | grep -q .; then
        continue
    fi
    name=${d##*/}
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
    if rm -rf "$d"; then
        echo "shot-linux.sh: pruned stale build tree $d" >&2
    else
        echo "shot-linux.sh: could not prune stale build tree $d" >&2
    fi
done
