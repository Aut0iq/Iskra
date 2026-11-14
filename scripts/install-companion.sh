#!/bin/sh
# install.sh — install the iskra-desktop companion (zeron-pet).
#
# Transactional: every platform / version / prefix / destination / link
# condition is validated BEFORE anything changes; the new tree is staged
# in a private mktemp dir; a pre-existing managed destination is moved to
# a preserved backup (never deleted), and any failure rolls the previous
# destination and link back. No autostart is installed and nothing is
# started.
#
#   ./install.sh                  prefix = ~/.local  (default)
#   ./install.sh --prefix DIR     install under DIR (absolute or relative)
#   ISKRA_DESKTOP_VERSION=x.y.z   override the version dir name
set -eu

SRC_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
PREFIX="$HOME/.local"

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX=$2; shift 2 ;;
        --help|-h)
            sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

fail() { echo "error: $*" >&2; exit 3; }
die()  { echo "error: $*" >&2; exit 2; }

# --- validate everything up front ----------------------------------------

os=$(uname -s); arch=$(uname -m)
if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
    die "iskra-desktop supports Linux x86_64 (Wayland) only"
fi

# Required runtime: GTK4 + gtk4-layer-shell (checked via ldconfig when the
# system provides it; absence of ldconfig is a warning, not a verdict).
if command -v ldconfig >/dev/null 2>&1; then
    missing=""
    ldconfig -p 2>/dev/null | grep -q "libgtk-4\.so\.1" ||
        missing="$missing gtk4"
    ldconfig -p 2>/dev/null | grep -q "libgtk4-layer-shell\.so\.0" ||
        missing="$missing gtk4-layer-shell"
    [ -z "$missing" ] ||
        die "missing required runtime libraries:$missing
       (e.g. 'pacman -S gtk4 gtk4-layer-shell' on Arch)"
else
    echo "warning: no ldconfig — cannot verify GTK4/gtk4-layer-shell" >&2
fi

# Version: strict semver-ish, safe for paths.
VERSION=${ISKRA_DESKTOP_VERSION:-}
if [ -z "$VERSION" ]; then
    base=$(basename "$SRC_DIR")
    case "$base" in
        iskra-desktop-*) VERSION=${base#iskra-desktop-} ;;
        *) VERSION=$base ;;  # installed dirs are named by bare version
    esac
fi
case "$VERSION" in
    *[[:space:]]*|*..*) die "unsafe version string: '$VERSION'" ;;
esac
printf '%s' "$VERSION" |
    grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z._-]+)?$' ||
    die "unsafe version string: '$VERSION'"

# Prefix: must normalize to an absolute path (a relative one would make a
# broken link target).
mkdir -p "$PREFIX"
PREFIX=$(CDPATH='' cd -- "$PREFIX" && pwd -P)
case "$PREFIX" in /*) : ;; *) die "could not resolve --prefix to an absolute path" ;; esac

LIB_PARENT="$PREFIX/lib/iskra-desktop"
DEST="$LIB_PARENT/$VERSION"
BIN_DIR="$PREFIX/bin"
LINK="$BIN_DIR/zeron-pet"
MARKER=".iskra-desktop-install"

[ -f "$SRC_DIR/bin/zeron-pet" ] ||
    die "$SRC_DIR/bin/zeron-pet missing — run install.sh from the extracted iskra-desktop-<version> directory"

# Destination: reject symlinks outright; reject anything we did not
# install; an owned dir may be updated (it is preserved, not deleted).
if [ -L "$DEST" ]; then
    fail "$DEST is a symlink — refusing to touch it"
fi
if [ -e "$DEST" ]; then
    [ -d "$DEST" ] || fail "$DEST exists and is not a directory"
    [ -f "$DEST/$MARKER" ] ||
        fail "$DEST exists and was not installed by this installer — refusing"
    [ "$(cat "$DEST/$MARKER" 2>/dev/null)" = "$VERSION" ] ||
        fail "$DEST has a mismatched ownership marker — refusing"
fi

# Link: a regular file is never touched; a symlink is only accepted when
# it points exactly at a managed install under THIS prefix whose version
# is safe and whose ownership marker matches.
OLDLINK=""
if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
    fail "$LINK exists and is not a symlink — not overwriting it"
fi
if [ -L "$LINK" ]; then
    OLDLINK=$(readlink "$LINK")
    case "$OLDLINK" in
        "$LIB_PARENT"/*/bin/zeron-pet)
            ver=${OLDLINK#"$LIB_PARENT"/}
            ver=${ver%/bin/zeron-pet}
            ok=""
            case "$ver" in */*|*..*|*[[:space:]]*) ;; *)
                if printf '%s' "$ver" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z._-]+)?$' &&
                   [ -d "$LIB_PARENT/$ver" ] && [ ! -L "$LIB_PARENT/$ver" ] &&
                   [ -f "$LIB_PARENT/$ver/$MARKER" ] &&
                   [ "$(cat "$LIB_PARENT/$ver/$MARKER" 2>/dev/null)" = "$ver" ]; then
                    ok=yes
                fi ;;
            esac
            [ -n "$ok" ] ||
                fail "$LINK points to unmanaged target '$OLDLINK' — refusing" ;;
        *)
            fail "$LINK points to '$OLDLINK' — not ours, leaving it" ;;
    esac
fi

# --- stage the new tree (before touching the destination) -----------------

mkdir -p "$LIB_PARENT" "$BIN_DIR"
TMP=$(mktemp -d "$LIB_PARENT/.stage-XXXXXX")
trap 'rm -rf "$TMP"' EXIT INT TERM
for item in bin assets README.md THIRD_PARTY_NOTICES.md install.sh \
    LICENSE LICENSE-ARTWORK ARTWORK.md TRADEMARKS.md licenses; do
    [ -e "$SRC_DIR/$item" ] && cp -a "$SRC_DIR/$item" "$TMP/$item"
done
printf '%s\n' "$VERSION" > "$TMP/$MARKER"

# Atomically repoint LINK at the destination: build the symlink inside a
# fresh private dir in BIN_DIR, then rename it over LINK with mv -T — no
# unlink/link gap, and a failed repoint leaves the previous link in
# place, so rollback never has to touch it.
update_link() {
    ld=$(mktemp -d "$BIN_DIR/.link-XXXXXX") || return 1
    if ln -s "$DEST/bin/zeron-pet" "$ld/zeron-pet" &&
        mv -T "$ld/zeron-pet" "$LINK"; then
        rmdir "$ld"
    else
        rm -rf "$ld"
        return 1
    fi
}

# Idempotent: an owned destination identical to the staged tree is a
# no-op for files — but a missing link is still installed and a stale
# managed link repointed to this destination.
if [ -d "$DEST" ] && diff -r -q "$TMP" "$DEST" >/dev/null 2>&1; then
    rm -rf "$TMP"
    trap - EXIT INT TERM
    update_link ||
        die "unchanged, but the link update failed"
    echo "iskra-desktop $VERSION already installed and unchanged"
    exit 0
fi

# --- install + repoint, with rollback -------------------------------------

BACKUP=""
BACKUP_HOLDER=""
INSTALLED=""
if [ -d "$DEST" ]; then
    [ ! -L "$LIB_PARENT/.backups" ] ||
        fail "$LIB_PARENT/.backups is a symlink — refusing to use it"
    mkdir -p "$LIB_PARENT/.backups"
    BACKUP_HOLDER=$(mktemp -d "$LIB_PARENT/.backups/$VERSION-XXXXXXXX")
    BACKUP="$BACKUP_HOLDER/$VERSION"
    mv "$DEST" "$BACKUP"
fi

restore() {
    # Roll back only what this run created: the freshly installed
    # destination (the staged tree we moved in — never a preexisting
    # path), then move the preserved previous version back. The link
    # needs no restore: a failed atomic repoint leaves it unchanged.
    [ -z "$INSTALLED" ] || rm -rf "$DEST"
    if [ -n "$BACKUP" ]; then
        mv "$BACKUP" "$DEST"
        rmdir "$BACKUP_HOLDER" 2>/dev/null || true
    fi
}

if [ "${ISKRA_DESKTOP_TEST_FAIL_AT:-}" = "dest" ]; then
    echo "test hook: forced failure before destination install" >&2
    restore; rm -rf "$TMP"; trap - EXIT INT TERM; exit 2
fi
if ! mv "$TMP" "$DEST"; then
    restore; rm -rf "$TMP"; trap - EXIT INT TERM
    die "install failed — previous version restored"
fi
INSTALLED=yes
trap - EXIT INT TERM

if [ "${ISKRA_DESKTOP_TEST_FAIL_AT:-}" = "link" ]; then
    echo "test hook: forced failure before link update" >&2
    restore; exit 2
fi
if ! update_link; then
    restore
    die "link update failed — previous install restored"
fi

# --- report ----------------------------------------------------------------
echo "installed iskra-desktop $VERSION"
echo "  program:  $LINK -> $DEST/bin/zeron-pet"
echo "  assets:   $DEST/assets/v6"
[ -n "$BACKUP" ] && echo "  previous version preserved: $BACKUP"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo
       echo "note: $BIN_DIR is not on your PATH — add it, or run the"
       echo "      program as $LINK" ;;
esac
cat <<EOF

Nothing was started and no autostart was installed. Run it when you want:

    zeron-pet            # launch the desktop pet
    zeron-pet --status   # one-line status (forwarded to the running pet)
    zeron-pet --quit     # stop it

Removing or disabling the Zeron plugin does not stop a companion you
launched; use 'zeron-pet --quit'. To uninstall: remove
$DEST and the link $LINK.
EOF
