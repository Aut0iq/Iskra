#!/usr/bin/env python3
"""Stage a vendored Rust-dependency license snapshot into a package tree.

The snapshot (third_party/rust-licenses/) is produced by
scripts/vendor-license-texts.py and pinned to a Cargo.lock SHA-256 in
its manifest.json. Staging validates the whole snapshot first — schema,
lockfile pin, declared file set, paths and hashes — and only then copies
it into a fresh destination directory. A missing, stale or tampered
snapshot fails loudly; the source is never modified.

CLI:
    python3 lib_licenses.py stage --source third_party/rust-licenses \
        --dest <dir> --lockfile Cargo.lock

Importable: stage_licenses(), LicenseError.
"""

import argparse
import hashlib
import json
import posixpath
import shutil
import sys
import tempfile
from pathlib import Path, PurePosixPath


class LicenseError(Exception):
    """A license snapshot failed validation."""


_LOWER_HEX = frozenset("0123456789abcdef")


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _safe_component(value, what):
    """A manifest name/version must be one safe path component."""
    if not isinstance(value, str) or not value:
        raise LicenseError(f"manifest: package {what} missing or not a string")
    if (
        value in (".", "..")
        or "/" in value
        or "\\" in value
        or "\x00" in value
    ):
        raise LicenseError(f"manifest: unsafe package {what} {value!r}")
    return value


def _canonical_rel(path, pkg_dir):
    """A declared file path must be a canonical relative posix path
    inside the package's own directory: no absolute paths, no empty or
    dot/dot-dot components, no doubled separators, no backslashes, no
    traversal."""
    if not isinstance(path, str) or not path:
        raise LicenseError(f"{pkg_dir}: entry path missing or not a string")
    if "\\" in path or "\x00" in path:
        raise LicenseError(f"{pkg_dir}: unsafe declared path {path!r}")
    p = PurePosixPath(path)
    if p.is_absolute():
        raise LicenseError(f"{pkg_dir}: absolute declared path {path!r}")
    if any(part in (".", "..") for part in p.parts):
        raise LicenseError(f"{pkg_dir}: dot/traversal in declared path {path!r}")
    if p.as_posix() != path:
        raise LicenseError(f"{pkg_dir}: non-canonical declared path {path!r}")
    if len(p.parts) < 2 or p.parts[0] != pkg_dir:
        raise LicenseError(
            f"{pkg_dir}: declared path outside package dir {path!r}"
        )
    return path


def _check_sha256(sha, pkg_dir, path):
    if (
        not isinstance(sha, str)
        or len(sha) != 64
        or any(c not in _LOWER_HEX for c in sha)
    ):
        raise LicenseError(
            f"{pkg_dir}: sha256 for {path!r} is not 64 lowercase hex"
        )
    return sha


def _declared_files(source: Path, manifest: dict) -> dict:
    """Manifest package entries -> {relpath: sha256}, fully validated."""
    packages = manifest.get("packages")
    if not isinstance(packages, list) or not packages:
        raise LicenseError(f"{source}/manifest.json: no packages")
    declared = {}
    pkg_ids = set()
    seen_paths = {}
    for p in packages:
        if not isinstance(p, dict):
            raise LicenseError("manifest: package entry is not an object")
        name = _safe_component(p.get("name"), "name")
        version = _safe_component(p.get("version"), "version")
        pkg_dir = f"{name}-{version}"
        if pkg_dir in pkg_ids:
            raise LicenseError(
                f"manifest: duplicate package id {pkg_dir!r}"
            )
        pkg_ids.add(pkg_dir)
        files = p.get("files")
        if not isinstance(files, list) or not files:
            raise LicenseError(f"{pkg_dir}: no license files declared")
        for f in files:
            if not isinstance(f, dict):
                raise LicenseError(
                    f"{pkg_dir}: file entry is not an object"
                )
            path = _canonical_rel(f.get("path"), pkg_dir)
            sha = _check_sha256(f.get("sha256"), pkg_dir, path)
            canonical = posixpath.normpath(path)
            if canonical in seen_paths:
                prev = seen_paths[canonical]
                if prev == path:
                    raise LicenseError(
                        f"{pkg_dir}: duplicate declared path {path!r}"
                    )
                raise LicenseError(
                    f"manifest: declared path {path!r} is a canonical "
                    f"alias of {prev!r}"
                )
            seen_paths[canonical] = path
            declared[path] = sha
    return declared


def stage_licenses(source, dest, lockfile) -> Path:
    """Validate `source` (a vendor-license-texts.py snapshot) completely,
    then copy it into a fresh `dest`. Raises LicenseError on any
    violation; `source` is never modified and an existing `dest` is
    refused, not touched."""
    source = Path(source)
    dest = Path(dest)
    lockfile = Path(lockfile)

    if source.is_symlink():
        raise LicenseError(f"license snapshot root is a symlink: {source}")
    if not source.is_dir():
        raise LicenseError(f"license snapshot missing: {source}")
    manifest_path = source / "manifest.json"
    if manifest_path.is_symlink():
        raise LicenseError(f"manifest is a symlink: {manifest_path}")
    if not manifest_path.is_file():
        raise LicenseError(f"license snapshot missing: {manifest_path}")
    if lockfile.is_symlink() or not lockfile.is_file():
        raise LicenseError(f"lockfile is not a regular file: {lockfile}")

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise LicenseError(f"{manifest_path}: invalid JSON: {exc}")
    if not isinstance(manifest, dict):
        raise LicenseError(f"{manifest_path}: top level is not an object")
    if manifest.get("schema") != 1:
        raise LicenseError(
            f"{manifest_path}: schema {manifest.get('schema')!r} != 1"
        )
    want_lock = manifest.get("cargo_lock_sha256")
    got_lock = _sha256(lockfile)
    if want_lock != got_lock:
        raise LicenseError(
            "license snapshot is stale: Cargo.lock changed since it was "
            f"vendored ({manifest_path} pins {want_lock})"
        )

    declared = _declared_files(source, manifest)

    # The source tree must contain exactly the declared files plus the
    # manifest — no extras, no missing, nothing but files and dirs.
    actual = set()
    for entry in sorted(source.rglob("*")):
        if entry.is_symlink():
            raise LicenseError(f"symlink in license snapshot: {entry}")
        if entry.is_dir():
            continue
        if not entry.is_file():
            raise LicenseError(f"special file in snapshot: {entry}")
        actual.add(entry.relative_to(source).as_posix())
    expected = set(declared) | {"manifest.json"}
    for missing in sorted(expected - actual):
        raise LicenseError(f"declared file missing from snapshot: {missing}")
    for extra in sorted(actual - expected):
        raise LicenseError(f"unexpected file in snapshot: {extra}")

    for path, want in declared.items():
        if _sha256(source / path) != want:
            raise LicenseError(
                f"license text changed since vendoring: {path}"
            )

    # Fully validated — copy into a fresh destination atomically.
    if dest.exists() or dest.is_symlink():
        raise LicenseError(f"refusing existing destination: {dest}")
    try:
        dest.resolve().relative_to(source.resolve())
    except ValueError:
        pass
    else:
        raise LicenseError(f"destination inside license snapshot: {dest}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix=".licenses-", dir=dest.parent))
    try:
        for rel in sorted(expected):
            out = tmp / rel
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / rel, out)
        tmp.rename(dest)
    except BaseException:
        if tmp.exists():
            shutil.rmtree(tmp)
        raise
    return dest


def _main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    st = sub.add_parser("stage", help="validate + copy a license snapshot")
    st.add_argument("--source", required=True)
    st.add_argument("--dest", required=True)
    st.add_argument("--lockfile", required=True)
    args = ap.parse_args()
    if args.cmd == "stage":
        try:
            out = stage_licenses(args.source, args.dest, args.lockfile)
        except LicenseError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 2
        print(f"staged license texts: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(_main())
