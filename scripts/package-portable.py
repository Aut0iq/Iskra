#!/usr/bin/env python3
"""Deterministic OS-labelled archives of unsigned plugin staging and avatar tools."""
import argparse
import stat
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def package(source, output):
    source, output = Path(source), Path(output)
    files = sorted(p for p in source.rglob('*') if p.is_file())
    if source.is_symlink() or any(p.is_symlink() for p in source.rglob('*')):
        raise ValueError('symlinks are not allowed')
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in files:
            info = zipfile.ZipInfo(path.relative_to(source).as_posix(), (2000, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | 0o644) << 16
            archive.writestr(info, path.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', required=True)
    p.add_argument('--output', required=True)
    a = p.parse_args()
    package(a.source, a.output)


if __name__ == '__main__':
    main()
