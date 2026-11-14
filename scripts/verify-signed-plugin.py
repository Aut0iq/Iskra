#!/usr/bin/env python3
"""Verify exported Zeron payload and Ed25519 signature; never sign or load keys."""
import argparse
import base64
import hashlib
import json
import re
import subprocess
import tempfile
from pathlib import Path


def decode(s):
    if not isinstance(s, str) or not re.fullmatch(r'[A-Za-z0-9_-]+', s):
        raise ValueError('invalid unpadded base64url')
    return base64.urlsafe_b64decode(s + '=' * ((4 - len(s) % 4) % 4))


def verify_signature(manifest):
    # Public contract: zeronsh/zeron crates/engine/src/plugins/signing.rs.
    sig = manifest['signature']
    key, signature = decode(sig['key']), decode(sig['sig'])
    if len(key) != 32 or len(signature) != 64:
        raise ValueError('invalid Ed25519 signature shape')
    signed = dict(manifest, signature={'key': sig['key'], 'files': sig['files']})
    payload = b'zeron-plugin:v1\n' + json.dumps(signed, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
    with tempfile.TemporaryDirectory(prefix='iskra-verify-') as temp:
        root = Path(temp)
        (root / 'public.der').write_bytes(bytes.fromhex('302a300506032b6570032100') + key)
        (root / 'signature').write_bytes(signature)
        (root / 'payload').write_bytes(payload)
        result = subprocess.run(['openssl', 'pkeyutl', '-verify', '-pubin', '-keyform', 'DER',
                                 '-inkey', str(root / 'public.der'), '-rawin', '-in', str(root / 'payload'),
                                 '-sigfile', str(root / 'signature')], capture_output=True, timeout=15)
        if result.returncode:
            raise ValueError('Ed25519 verification failed')


def verify(package, staging, trusted):
    if Path(package).stat().st_size > 64 * 1024 * 1024:
        raise ValueError('oversized package')
    data = json.loads(Path(package).read_bytes())
    if data.get('format') != 'zeron-plugin:v1':
        raise ValueError('unsupported package format')
    manifest, files = data['manifest'], data['files']
    if manifest['signature']['key'] not in trusted['public_keys']:
        raise ValueError('unreviewed signing public key')
    verify_signature(manifest)
    staging = Path(staging)
    expected = {p.relative_to(staging).as_posix(): p for p in staging.rglob('*') if p.is_file()}
    if set(files) != set(expected) - {'zeron-plugin.json'} or set(files) != set(manifest['signature']['files']):
        raise ValueError('package membership differs from reviewed staging')
    for name, text in files.items():
        if name.startswith('/') or '\\' in name or '..' in Path(name).parts:
            raise ValueError('unsafe package path')
        value = base64.b64decode(text, validate=True)
        if value != expected[name].read_bytes() or hashlib.sha256(value).hexdigest() != manifest['signature']['files'][name]:
            raise ValueError('payload mismatch: ' + name)
    if {k: v for k, v in manifest.items() if k != 'signature'} != json.loads(expected['zeron-plugin.json'].read_bytes()):
        raise ValueError('manifest differs from reviewed staging')
    return manifest


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('package')
    p.add_argument('--staging', required=True)
    p.add_argument('--trust', default='.github/trusted-signers.json')
    args = p.parse_args()
    try:
        m = verify(args.package, args.staging, json.loads(Path(args.trust).read_text()))
        print('Payload and Ed25519 signature verified:', m['id'], m['version'])
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError) as error:
        p.exit(2, str(error) + '\n')
