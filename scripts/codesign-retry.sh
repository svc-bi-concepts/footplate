#!/bin/sh
# Retried timestamped codesigning, shared by the app bundling and the disk
# image release: sourced, not run. Call codesign_retry with codesign's
# arguments. Apple's timestamp server now and then answers no secure
# timestamp ("A timestamp was expected but was not found"), where a second
# try is answered, so a failure naming only that is tried again, up to 3
# attempts; any other failure stops at once with codesign's own words and
# status. Between tries it sleeps FOOTPLATE_SIGN_RETRY_SLEEP seconds,
# 5 when unset.

codesign_retry() {
    for target; do :; done
    attempt=1
    err=$(mktemp)
    while :; do
        if codesign "$@" 2>"$err"; then
            rm -f "$err"
            return 0
        else
            status=$?
        fi
        if [ "$attempt" -ge 3 ] || ! grep -q -F 'A timestamp was expected but was not found' "$err"; then
            cat "$err" >&2
            rm -f "$err"
            return "$status"
        fi
        echo "codesign-retry.sh: $target: the timestamp was not found; trying again, try $attempt of 3" >&2
        sleep "${FOOTPLATE_SIGN_RETRY_SLEEP:-5}"
        attempt=$((attempt + 1))
    done
}
