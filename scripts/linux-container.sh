#!/bin/sh
# Provisioning for footplate's Debian container, shared by the Linux
# screenshot and the release build: sourced, not run. Call linux_provision
# with the container's name, after setting what it provisions: root (the
# checkout, mounted read-only at /src), cargo_vol (the volume holding
# cargo's homes), target_vol (the volume holding the build trees),
# cargo_home (cargo's home in the container), target_dir
# (CARGO_TARGET_DIR), platform (docker's --platform, empty for the host's
# own, which the provisioning passes explicitly), extra_bins (binaries
# beyond cc and cargo that must answer before the provisioning is
# skipped) and extra_pkgs (the apt packages beyond the build's own that
# provide them).

have() {
    docker exec "$1" sh -c "command -v $2" >/dev/null 2>&1
}

linux_provision() {
    name=$1
    # A container of the name may be left by a killed run: remove it
    # before a new one starts.
    if docker inspect "$name" >/dev/null 2>&1; then
        docker rm -f "$name" >/dev/null
    fi
    # An empty platform is the host's own, passed explicitly: without the
    # flag docker starts whatever variant the local tag happens to hold
    # (an emulated leg leaves debian:13 as amd64), over volumes whose
    # binaries are the host's arch. An unknown host keeps no flag.
    if [ -z "$platform" ]; then
        case "$(uname -m)" in
            aarch64 | arm64) platform=linux/arm64 ;;
            x86_64 | amd64) platform=linux/amd64 ;;
        esac
    fi
    if [ -n "$platform" ]; then
        set -- --platform "$platform"
    else
        set --
    fi
    docker run "$@" -d --name "$name" --init \
        -v "$root:/src:ro" \
        -v "$cargo_vol:/cargo" -v "$target_vol:/target" \
        -e CARGO_HOME="$cargo_home" -e RUSTUP_HOME="$cargo_home/rustup" \
        -e CARGO_TARGET_DIR="$target_dir" \
        -e PATH="$cargo_home/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
        debian:13 sleep infinity >/dev/null
    # The checkout is mounted read-only: cargo reads it and writes the
    # build to a directory in the target volume, so a build never dirties
    # the checkout it builds.
    # Missing tools are installed: the build's (cc, the libraries gpui
    # builds against, and ALSA's for the voice session's audio, which the
    # binary is linked to: libasound2-dev brings libasound.so.2 to run it)
    # and the caller's extra ones.
    missing=false
    for bin in cc cargo $extra_bins; do
        have "$name" "$bin" || missing=true
    done
    if [ "$missing" = true ]; then
        docker exec "$name" sh -c 'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
            build-essential curl ca-certificates pkg-config \
            libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev \
            libwayland-dev libxcb1-dev libx11-dev libvulkan-dev libasound2-dev '"$extra_pkgs"' >/tmp/apt.log 2>&1 || { tail -n 20 /tmp/apt.log >&2; exit 1; }' >&2
    fi
    if ! docker exec "$name" sh -c 'id shot' >/dev/null 2>&1; then
        docker exec "$name" useradd -m -s /bin/bash shot
    fi
    if ! have "$name" cargo; then
        docker exec "$name" sh -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable -q' >&2
    fi
}
