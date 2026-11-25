#!/usr/bin/env python3
"""Compare all platform plugin payloads and validate the native package data."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('delivery', ROOT / 'scripts/release-control.py')
delivery = importlib.util.module_from_spec(spec); spec.loader.exec_module(delivery)
artifacts = Path(sys.argv[1])
stages = list(artifacts.rglob('iskra-*/zeron-plugin.json'))
if len(stages) != 1:
    raise ValueError('exactly one native unsigned plugin staging tree is required')
stage = stages[0].parent
expected = {p.relative_to(stage).as_posix(): p.read_bytes() for p in stage.rglob('*') if p.is_file()}
if 'signature' in json.loads(expected['zeron-plugin.json']):
    raise ValueError('CI staging must remain unsigned')
archives = list(artifacts.rglob('iskra-plugin-UNSIGNED-*.zip'))
if len(archives) != 3 or {p.name for p in archives} != {f'iskra-plugin-UNSIGNED-{p}.zip' for p in ('linux', 'windows', 'macos')}:
    raise ValueError('missing or duplicated portable platform package')
with tempfile.TemporaryDirectory(prefix='iskra-contract-') as temp:
    for i, archive in enumerate(archives):
        dest = Path(temp) / str(i); delivery.unzip(archive, dest)
        actual = {p.relative_to(dest).as_posix(): p.read_bytes() for p in dest.rglob('*') if p.is_file()}
        if actual != expected:
            raise ValueError('cross-platform payload mismatch: ' + archive.name)
manifest = json.loads((ROOT / 'assets/v6/manifest.json').read_bytes())
# Zero matches here must never read as "all checks passed".
desktops = list(artifacts.rglob('iskra-desktop-*.tar.gz'))
if len(desktops) != 1:
    raise ValueError('exactly one native desktop tarball is required')
for archive in desktops:
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        if len(members) != len({m.name for m in members}):
            raise ValueError('duplicate tar member')
        version = json.loads(expected['zeron-plugin.json'])['version']
        prefix = 'iskra-desktop-' + version + '/'
        paths = {'bin/zeron-pet', 'README.md', 'install.sh', 'THIRD_PARTY_NOTICES.md',
                 'LICENSE', 'LICENSE-ARTWORK', 'ARTWORK.md', 'TRADEMARKS.md', 'assets/v6/manifest.json'}
        paths.update('assets/v6/' + f for a in manifest['animations'].values() for f in a['frames'])
        paths.update('licenses/' + p.relative_to(ROOT / 'third_party/rust-licenses').as_posix()
                     for p in (ROOT / 'third_party/rust-licenses').rglob('*') if p.is_file())
        if {m.name for m in members if m.isfile()} != {prefix + p for p in paths}:
            raise ValueError('unexpected native archive membership')
        binary = tar.extractfile(prefix + 'bin/zeron-pet').read()
        if binary[:6] != b'\x7fELF\x02\x01' or binary[18:20] != b'\x3e\x00':
            raise ValueError('native binary is not ELF x86_64')
        if tar.getmember(prefix + 'bin/zeron-pet').mode != 0o755 or not tar.getmember(prefix + 'install.sh').mode & 0o111:
            raise ValueError('native executable modes lost')
        for m in members:
            if (m.name.startswith('/') or '..' in Path(m.name).parts or not (m.isfile() or m.isdir())
                    or m.name.endswith('FIXTURE-BUILD.txt')):
                raise ValueError('unsafe or fixture tar member')
        for animation in manifest['animations'].values():
            for frame in animation['frames']:
                if tar.extractfile(prefix + 'assets/v6/' + frame).read() != (ROOT / 'assets/v6' / frame).read_bytes():
                    raise ValueError('native frame differs from source')
        for name in ('LICENSE', 'LICENSE-ARTWORK', 'ARTWORK.md', 'TRADEMARKS.md'):
            if tar.extractfile(prefix + name).read() != (ROOT / name).read_bytes():
                raise ValueError('native notice mismatch')
        for path in (ROOT / 'third_party/rust-licenses').rglob('*'):
            if path.is_file() and tar.extractfile(prefix + 'licenses/' + path.relative_to(ROOT / 'third_party/rust-licenses').as_posix()).read() != path.read_bytes():
                raise ValueError('native dependency license mismatch')
checksums = list(artifacts.rglob('SHA256SUMS'))
if len(checksums) != 1:
    raise ValueError('exactly one SHA256SUMS is required')
for manifest_path in checksums:
    for line in manifest_path.read_text(encoding='utf-8').splitlines():
        expected_hash, name = line.split(maxsplit=1); name = name.strip()
        if '/' in name or '\\' in name or name in ('.', '..'):
            raise ValueError('unsafe checksum path')
        if hashlib.sha256((manifest_path.parent / name).read_bytes()).hexdigest() != expected_hash:
            raise ValueError('artifact checksum mismatch')
print('All three portable payloads match; native frames/notices/licenses/checksums pass.')
