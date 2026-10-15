#!/bin/sh
# Drop every checkout's build tree under the shot target volume but this
# shot's own when nothing in it was written in the last week: a deleted or
# idle checkout's tree costs GBs no shot will warm from. Says what went, on
# stderr. Files at the volume's root are left alone, and so is a tree being
# built this minute: a build writes constantly.
set -e

root=${1:?target volume is required}
own=${2:?own build tree is required}
for d in "$root"/*; do
    [ -d "$d" ] || continue
    if [ "$d" = "$own" ]; then
        continue
    fi
    if find "$d" -mtime -7 -print 2>/dev/null | grep -q .; then
        continue
    fi
    if rm -rf "$d"; then
        echo "shot-linux.sh: pruned stale build tree $d" >&2
    else
        echo "shot-linux.sh: could not prune stale build tree $d" >&2
    fi
done
