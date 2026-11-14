#!/bin/sh
# stage-plugin.sh — explicitly stage a reviewed unsigned plugin folder
# into Zeron's plugin directory, backing the live folder up OUTSIDE the
# scanned plugins root.
#
#   scripts/stage-plugin.sh --dist dist/iskra-0.1.0
#
# What it does:
#   * verifies the staged tree is complete and structurally UNSIGNED
#     (python3 JSON check: id == "iskra", no "signature" key)
#   * refuses any symlink or special file in the staged tree
#   * moves the live plugins/iskra aside to
#     <plugins-parent>/plugin-backups/iskra/<unique> — kept, never
#     deleted, and never inside the scanned plugins root
#   * copies the staged tree into place via a private mktemp staging
#     dir; if the final swap fails the live folder is restored
#
# What it never does: sign anything, grant consent, touch Zeron's
# .state/key material, or restart the engine. After staging, signing is
# `zeron plugin sign iskra` (device key) or the consent sheet's
# "It's mine…"; the "commands" permission for zeron-pet is granted only
# by the person in Zeron.
set -eu

DIST=""
PLUGINS_DIR="$HOME/.zeron/plugins"

while [ $# -gt 0 ]; do
    case "$1" in
        --dist) DIST=$2; shift 2 ;;
        --plugins-dir) PLUGINS_DIR=$2; shift 2 ;;  # test fixture override
        --help|-h) sed -n '2,21p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

die() { echo "error: $*" >&2; exit 2; }

[ -n "$DIST" ] || die "--dist <staged folder> required"
[ -d "$DIST" ] || die "staged folder not found: $DIST"
DIST=$(CDPATH='' cd -- "$DIST" && pwd -P)

for f in zeron-plugin.json init.luau zeron.luau; do
    [ -f "$DIST/$f" ] || die "staged tree missing $f"
done

# Structural manifest check (id + unsigned), not a grep.
python3 - "$DIST/zeron-plugin.json" <<'PY' || die "manifest check failed"
import json, sys
m = json.load(open(sys.argv[1], encoding="utf-8"))
if m.get("id") != "iskra":
    sys.exit("id != 'iskra'")
if "signature" in m:
    sys.exit("staged manifest carries a 'signature' — must stay unsigned")
PY

# No symlink or special files may enter the live plugin folder.
if find "$DIST" \( -type l -o -type p -o -type s -o -type b -o -type c \) |
    grep -q .; then
    die "staged tree contains symlinks or special files"
fi
case "$(basename "$DIST")" in
    iskra-*) : ;;
    *) echo "warning: '$DIST' does not look like iskra-<version>" >&2 ;;
esac

LIVE="$PLUGINS_DIR/iskra"
mkdir -p "$PLUGINS_DIR"
# Private staging dir inside the plugins root (same fs → atomic rename).
NEW=$(mktemp -d "$PLUGINS_DIR/.iskra-stage-XXXXXX")
trap 'rm -rf "$NEW"' EXIT INT TERM
cp -a "$DIST/." "$NEW/"

if [ -e "$LIVE" ] || [ -L "$LIVE" ]; then
    # Backup OUTSIDE the scanned plugins root so the old copy can never
    # load as a duplicate plugin id.
    BK_PARENT="$(dirname "$PLUGINS_DIR")/plugin-backups/iskra"
    mkdir -p "$BK_PARENT"
    BAK=$(mktemp -d "$BK_PARENT/iskra-XXXXXXXX")
    rmdir "$BAK"
    if ! mv "$LIVE" "$BAK"; then
        rm -rf "$NEW"; trap - EXIT INT TERM
        die "could not move live plugin aside — nothing changed"
    fi
    echo "live plugin preserved: $BAK"
fi

if ! mv "$NEW" "$LIVE"; then
    [ -n "${BAK:-}" ] && mv "$BAK" "$LIVE"
    trap - EXIT INT TERM
    die "staging failed — previous plugin restored"
fi
trap - EXIT INT TERM

cat <<EOF
staged unsigned plugin -> $LIVE

Next (manual — no script can grant consent):
  1. sign with this device's key:  zeron plugin sign iskra
     (or open the plugin's consent sheet in Zeron and choose
     "It's mine…" — the "commands" permission for zeron-pet is
     granted only by the person, in Zeron)
  2. export for distribution:  zeron plugin export iskra [--to <path>]
     -> attach the .zeron-plugin to a GitHub release, or commit the
     signed folder. See docs/PLUGIN.md.
EOF
