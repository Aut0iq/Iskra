#!/usr/bin/env python3
"""Vendor license/notice texts for the audited Linux dependency closure.

Reads the audit metadata (artifacts/license-linux-target.json, produced
from `cargo metadata`/`cargo tree` for x86_64-unknown-linux-gnu) and
copies every listed package's license/notice texts into
third_party/rust-licenses/ together with a manifest.json that pins the
SHA-256 of Cargo.lock and of each vendored file.

Fail closed: an existing snapshot is refused (preserved for review), a
symlinked/special file anywhere in the copied set aborts, a package with
no license text aborts, and the audited package set must match the
frozen count — this script vendors evidence, it does not re-audit.

    python3 scripts/vendor-license-texts.py [--metadata PATH]
"""

import argparse
import hashlib
import json
import pathlib
import shutil
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_METADATA = ROOT / "artifacts" / "license-linux-target.json"

TARGET = "x86_64-unknown-linux-gnu"
EXPECTED_PACKAGES = 100  # frozen audit scope; do not recompute here

NOTICE_PREFIXES = (
    "license",
    "licence",
    "copying",
    "notice",
    "unlicense",
    "copyright",
)

SCOPE = (
    "Linux normal/build registry dependency closure; not a list of all "
    "files linked into the executable"
)


def _notice_files(src: pathlib.Path, name: str) -> list:
    """License/notice files under a crate source dir, fail closed."""
    if src.is_symlink():
        raise RuntimeError("symlinked crate source")
    files = []
    for f in sorted(src.iterdir()):
        if not f.name.lower().startswith(NOTICE_PREFIXES):
            continue
        if f.is_symlink():
            raise RuntimeError("symlinked notice")
        if f.is_file():
            files.append(f)
        elif f.is_dir():
            for nested in sorted(f.rglob("*")):
                if nested.is_symlink():
                    raise RuntimeError("symlinked nested notice")
                if nested.is_file():
                    files.append(nested)
                elif not nested.is_dir():
                    raise RuntimeError("special nested notice")
        else:
            raise RuntimeError("special notice")
    if not files:
        raise RuntimeError("No license text for " + name)
    return files


def vendor(metadata_path: pathlib.Path) -> pathlib.Path:
    meta = json.loads(pathlib.Path(metadata_path).read_text())
    if meta["target"] != TARGET or len(meta["packages"]) != EXPECTED_PACKAGES:
        raise SystemExit(
            "metadata does not match the audited Linux target closure "
            f"(target={meta.get('target')!r}, "
            f"packages={len(meta.get('packages', []))})"
        )

    parent = ROOT / "third_party"
    parent.mkdir(exist_ok=True)
    dest = parent / "rust-licenses"
    if dest.exists() or dest.is_symlink():
        raise SystemExit(
            "Refusing existing license snapshot; preserve it for review"
        )
    stage = pathlib.Path(tempfile.mkdtemp(prefix=".rust-licenses-", dir=parent))
    try:
        records = []
        packages = sorted(
            meta["packages"], key=lambda x: (x["name"], x["version"])
        )
        for p in packages:
            src = pathlib.Path(p["manifest_path"]).parent
            package = p["name"] + "-" + p["version"]
            entries = []
            for f in sorted(set(_notice_files(src, p["name"]))):
                rel = pathlib.Path(package) / f.relative_to(src)
                out = stage / rel
                out.parent.mkdir(parents=True, exist_ok=True)
                data = f.read_bytes()
                out.write_bytes(data)
                entries.append(
                    {
                        "path": rel.as_posix(),
                        "sha256": hashlib.sha256(data).hexdigest(),
                    }
                )
            records.append(
                {
                    "name": p["name"],
                    "version": p["version"],
                    "license": p["license"],
                    "files": entries,
                }
            )
        manifest = {
            "schema": 1,
            "target": meta["target"],
            "scope": SCOPE,
            "cargo_lock_sha256": hashlib.sha256(
                (ROOT / "Cargo.lock").read_bytes()
            ).hexdigest(),
            "packages": records,
        }
        (stage / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n"
        )
        stage.rename(dest)
    except BaseException:
        if stage.exists():
            shutil.rmtree(stage)
        raise
    return dest


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--metadata",
        type=pathlib.Path,
        default=DEFAULT_METADATA,
        help="audit metadata JSON (default %(default)s)",
    )
    args = ap.parse_args(argv)
    print(vendor(args.metadata))
    return 0


if __name__ == "__main__":
    sys.exit(main())
