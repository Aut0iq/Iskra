#!/bin/sh
# Build the optional Iskra desktop companion tarball.
#
#   dist/iskra-desktop-<version>-linux-x86_64.tar.gz
#     iskra-desktop-<version>/
#       bin/zeron-pet
#       assets/v6/manifest.json + the V6 frames the manifest references
#       install.sh  README.md  THIRD_PARTY_NOTICES.md
#       LICENSE  LICENSE-ARTWORK  ARTWORK.md  TRADEMARKS.md
#       licenses/  vendored Rust-dependency license texts (Cargo.lock-pinned)
#
# Linux/x86_64 only — the companion is a GTK4 + gtk4-layer-shell Wayland
# app; other platforms fail honestly rather than silently mislabeled.
#
# Usage:
#   scripts/build-companion.sh [--repo-root DIR] [--output DIR]
#                              [--binary /path/to/zeron-pet]
#
# Without --binary it runs:
#   cargo build --release --offline --bin zeron-pet
# honoring CARGO_TARGET_DIR when set (otherwise cargo's default
# <repo>/target). --binary accepts an existing release binary —
# intended for mechanical packaging verification before the current
# sources have been rebuilt; the tarball then ships that binary as-is
# and the output is clearly marked as a fixture build.
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)
OUTPUT_DIR=""
BINARY=""

usage() {
    sed -n '2,21p' "$0"
    exit "${1:-0}"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT=$2; shift 2 ;;
        --output)    OUTPUT_DIR=$2; shift 2 ;;
        --binary)    BINARY=$2; shift 2 ;;
        --help|-h)   usage 0 ;;
        *) echo "unknown argument: $1" >&2; usage 1 ;;
    esac
done

[ -n "$OUTPUT_DIR" ] || OUTPUT_DIR="$REPO_ROOT/dist"

os=$(uname -s)
arch=$(uname -m)
[ "$os" = "Linux" ] || { echo "error: unsupported OS '$os' (Linux only)" >&2; exit 2; }
[ "$arch" = "x86_64" ] || { echo "error: unsupported arch '$arch' (x86_64 only)" >&2; exit 2; }

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO_ROOT/Cargo.toml" | head -n 1)
[ -n "$VERSION" ] || { echo "error: no version in $REPO_ROOT/Cargo.toml" >&2; exit 2; }

# --- release license files (fail closed) ---------------------------------
# Every artifact ships the repo-root license files verbatim; they must
# exist as regular files — a missing file or a symlink aborts the build
# before anything is staged.
for f in LICENSE LICENSE-ARTWORK ARTWORK.md TRADEMARKS.md; do
    p="$REPO_ROOT/$f"
    [ -L "$p" ] && { echo "error: release file is a symlink: $p" >&2; exit 2; }
    if [ ! -f "$p" ] || [ ! -r "$p" ]; then
        echo "error: required release file missing or not regular: $p" >&2
        exit 2
    fi
done

# --- binary -------------------------------------------------------------
FIXTURE=0
if [ -n "$BINARY" ]; then
    FIXTURE=1
    if [ ! -f "$BINARY" ] || [ ! -x "$BINARY" ]; then
        echo "error: --binary '$BINARY' is not an executable file" >&2
        exit 2
    fi
    if ! head -c 4 "$BINARY" | grep -q 'ELF'; then
        echo "warning: --binary is not an ELF; packaging it anyway (fixture mode)" >&2
    fi
else
    if [ "${CARGO_TARGET_DIR:-}" ]; then
        echo "building: cargo build --release --offline --bin zeron-pet"
        echo "  CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
        BINARY="$CARGO_TARGET_DIR/release/zeron-pet"
    else
        echo "building: cargo build --release --offline --bin zeron-pet"
        echo "  CARGO_TARGET_DIR unset — cargo default <repo>/target"
        BINARY="$REPO_ROOT/target/release/zeron-pet"
    fi
    (cd "$REPO_ROOT" && cargo build --release --offline --bin zeron-pet)
    [ -x "$BINARY" ] || { echo "error: built binary not found at $BINARY" >&2; exit 2; }
fi

PKG="iskra-desktop-$VERSION"
ARCHIVE="$OUTPUT_DIR/$PKG-linux-x86_64.tar.gz"

# --- stage --------------------------------------------------------------
mkdir -p "$OUTPUT_DIR"
STAGE_PARENT=$(mktemp -d "$OUTPUT_DIR/.pkg-XXXXXX")
trap 'rm -rf "$STAGE_PARENT"' EXIT INT TERM
STAGE="$STAGE_PARENT/$PKG"
mkdir -p "$STAGE/bin" "$STAGE/assets"

cp -f "$BINARY" "$STAGE/bin/zeron-pet"
chmod 0755 "$STAGE/bin/zeron-pet"

# V6 assets: exactly the manifest-referenced frames, validated.
python3 "$SCRIPT_DIR/lib_assets.py" stage \
    --manifest "$REPO_ROOT/assets/v6/manifest.json" \
    --dest "$STAGE/assets/v6"

# Vendored Rust-dependency license texts, validated against Cargo.lock
# (manifest schema, lock pin, file set, hashes — all before copying).
python3 "$SCRIPT_DIR/lib_licenses.py" stage \
    --source "$REPO_ROOT/third_party/rust-licenses" \
    --dest "$STAGE/licenses" \
    --lockfile "$REPO_ROOT/Cargo.lock"

cp "$SCRIPT_DIR/install-companion.sh" "$STAGE/install.sh"
chmod 0755 "$STAGE/install.sh"

# Repo-root release files, verbatim (checked fail-closed above).
cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/LICENSE-ARTWORK" \
   "$REPO_ROOT/ARTWORK.md" "$REPO_ROOT/TRADEMARKS.md" "$STAGE/"

if [ "$FIXTURE" = 1 ]; then
    cat > "$STAGE/FIXTURE-BUILD.txt" <<'EOF'
This archive was packaged from a pre-existing binary via --binary,
NOT from a fresh build of the current sources. Rebuild with
scripts/build-companion.sh (no --binary) before release.
EOF
fi

cat > "$STAGE/README.md" <<EOF
# Iskra desktop companion $VERSION (Linux / Wayland)

Optional desktop companion for the Iskra Zeron plugin: a transparent
always-on-top animated 2D illustrated pet that reports Zeron chat status. It needs a
wlroots/Hyprland-style compositor (wlr layer-shell) plus GTK4 and
gtk4-layer-shell. Wayland only — no X11, macOS or Windows.

Install (never automatic, never autostart):

    tar -xzf $PKG-linux-x86_64.tar.gz
    cd $PKG
    ./install.sh                 # into ~/.local (default)
    ./install.sh --prefix DIR    # staged/test install under DIR

The installer copies the tree to
<prefix>/lib/iskra-desktop/$VERSION and links
<prefix>/bin/zeron-pet to it. Old versions are left in place.

Run:   zeron-pet                 (uses the bundled assets automatically)
Stop:  zeron-pet --quit
Info:  zeron-pet --status, zeron-pet --help

Disabling or removing the Zeron plugin does not stop a companion you
launched yourself — stop it explicitly with \`zeron-pet --quit\`.
EOF

cat > "$STAGE/THIRD_PARTY_NOTICES.md" <<EOF
# Third-party notices — iskra-desktop $VERSION

This archive contains the \`zeron-pet\` binary, the Iskra V6 artwork
produced for this project, a manual installer (\`install.sh\`), a short
README, the project license and artwork-provenance files (\`LICENSE\`,
\`LICENSE-ARTWORK\`, \`ARTWORK.md\`, \`TRADEMARKS.md\`) and the vendored
Rust-dependency license texts under \`licenses/\`.

## System libraries (not distributed)

The binary dynamically links libraries that must be installed on the
target system; they remain under their own licenses:

- GTK 4 and friends (glib, gio, gdk-pixbuf, cairo, pango) — LGPL-2.1-or-later
- gtk4-layer-shell — MIT (verified against the installed
  /usr/share/licenses/gtk4-layer-shell text)

## Statically linked Rust crates

Exact crate versions are pinned in Cargo.lock of the zeron-pet source
tree. The full license and notice texts for the audited Linux
dependency closure ship inside this archive under \`licenses/\`
(\`licenses/manifest.json\` lists every package, its declared SPDX
expression and the SHA-256 of each text). The dependency set is
permissively licensed: mostly MIT OR Apache-2.0, plus crates carrying
BSD-3-Clause, Zlib, Unlicense, 0BSD, Unicode-3.0 or Apache-2.0 WITH
LLVM-exception — see the included texts.

## Zeron plugin SDK

\`zeron.luau\` in the separate plugin staging folder is generated by
Zeron itself and carries Zeron's MIT attribution; it is not part of
this archive.
EOF

# No symlinks or host paths may enter the archive.
if find "$STAGE" -type l -o -type p -o -type s | grep -q .; then
    echo "error: staged tree contains links/special files" >&2; exit 2
fi

# --- archive (deterministic) --------------------------------------------
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-946684800}
export SOURCE_DATE_EPOCH
# Sequential tar then gzip inside the private mktemp dir — no pipeline
# (which would mask a tar failure behind gzip's status) and no
# predictable next-to-final tmp name (which a preexisting symlink could
# hijack). Any failure is fatal via set -e + the trap, and a previous
# final archive is only replaced by the atomic mv at the very end.
TMP_TAR="$STAGE_PARENT/archive.tar"
TMP_ARCHIVE="$STAGE_PARENT/archive.tar.gz"
tar --sort=name --format=ustar --numeric-owner --owner=0 --group=0 \
    --mtime="@$SOURCE_DATE_EPOCH" \
    -cf "$TMP_TAR" -C "$STAGE_PARENT" "$PKG"
gzip -n -c "$TMP_TAR" > "$TMP_ARCHIVE"

# Verify members: only relative paths under the package root.
tar -tzf "$TMP_ARCHIVE" >/dev/null
if tar -tzf "$TMP_ARCHIVE" | grep -v "^$PKG/" | grep -q .; then
    echo "error: archive contains paths outside $PKG/" >&2
    exit 2
fi
if tar -tzf "$TMP_ARCHIVE" | grep -E '(^|/)\.\.(/|$)|^/' | grep -q .; then
    echo "error: archive contains absolute or traversal paths" >&2
    exit 2
fi

mv -f "$TMP_ARCHIVE" "$ARCHIVE"
rm -rf "$STAGE_PARENT"
trap - EXIT INT TERM

echo "built $ARCHIVE"
echo
echo "Install (manual): tar -xzf $(basename "$ARCHIVE") && cd $PKG && ./install.sh"
echo "The companion is a separate artifact from the unsigned plugin"
echo "staging; it never installs or starts anything by itself."
