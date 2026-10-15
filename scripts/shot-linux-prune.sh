#!/bin/sh
# Drop checkout build trees under a Linux target volume when nothing in
# them was written in the last week. Active builds leave markers so another
# build cannot prune a tree while it is in use. Says what went, on stderr;
# files at the volume's root are left alone.
set -e

root=${1:?target volume is required}
own=${2:?own build tree is required}
marker=${3:?active build marker is required}
label=${4:-shot-linux.sh}
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
        echo "$label: pruned stale build tree $d" >&2
    else
        echo "$label: could not prune stale build tree $d" >&2
    fi
done
