#!/bin/sh
# zeron-pet launcher. Remote-control flags are passed through unchanged
# (no forced startup flags); on a cold start it builds once if needed and
# prepares assets once if missing — all offline. No autostart.
set -eu
cd "$(dirname "$0")"

BIN=target/release/zeron-pet

is_control_only() {
    prev=
    for a in "$@"; do
        case "$a" in
            --status|--quit|--pause|--resume|--live|--click-through|--help|-h|--pose)
                return 0 ;;
            --demo)
                # blush/dragged are display-only controls, never a launch.
                prev="$a" ;;
            *)
                if [ "$prev" = "--demo" ]; then
                    case "$a" in
                        blush|dragged) return 0 ;;
                    esac
                fi
                prev= ;;
        esac
    done
    return 1
}

needs_build() {
    [ ! -x "$BIN" ] && return 0
    for f in Cargo.toml build.rs src/*.rs src/bin/*.rs; do
        [ "$f" -nt "$BIN" ] && return 0
    done
    return 1
}

if ! is_control_only "$@"; then
    if needs_build; then
        cargo build --release --offline --bin zeron-pet
    fi
    # Assets: any prepared manifest (v6 > v5 > v4 > v3 > v2 > base) means
    # ready; only a complete absence triggers the one-time legacy preparation.
    if [ ! -f assets/v6/manifest.json ] && \
       [ ! -f assets/v5/manifest.json ] && \
       [ ! -f assets/v4/manifest.json ] && \
       [ ! -f assets/v3/manifest.json ] && \
       [ ! -f assets/v2/manifest.json ] && \
       [ ! -f assets/manifest.json ]; then
        cargo run --release --offline --bin zeron-pet-prepare-assets
    fi
fi

exec "$BIN" "$@"
