#!/usr/bin/env python3
"""Create and validate declarative Iskra avatar packs; never execute pack code."""
import argparse
import json
import re
import shutil
from pathlib import Path

import lib_assets

ROOT = Path(__file__).resolve().parents[1]
FORMAT = 'iskra-avatar:v1'


def validate(folder):
    folder = Path(folder)
    if folder.is_symlink() or not folder.is_dir():
        raise ValueError('avatar must be a regular directory')
    for p in folder.rglob('*'):
        if p.is_symlink() or not (p.is_file() or p.is_dir()):
            raise ValueError('symlinks and special files are not allowed')
    meta = json.loads((folder / 'avatar.json').read_text(encoding='utf-8'))
    if not isinstance(meta, dict) or meta.get('format') != FORMAT:
        raise ValueError('unsupported avatar format')
    if set(meta) - {'format', 'id', 'name', 'author', 'license', 'attribution', 'mode', 'personalization'}:
        raise ValueError('unknown avatar metadata fields')
    if meta.get('mode') not in ('portrait', 'animated'):
        raise ValueError('avatar mode must be portrait or animated')
    prefs = meta.get('personalization', {'schema': 1, 'theme': None})
    if not isinstance(prefs, dict) or set(prefs) - {'schema', 'theme'} or prefs.get('schema') != 1:
        raise ValueError('unsupported personalization metadata')
    if prefs.get('theme') is not None and not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,47}', str(prefs['theme'])):
        raise ValueError('theme is an ID, not a path or URL')
    if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,47}', meta.get('id', '')):
        raise ValueError('invalid avatar id')
    for key in ('name', 'author', 'license', 'attribution'):
        if not isinstance(meta.get(key), str) or not meta[key].strip() or len(meta[key]) > 1024:
            raise ValueError('missing or oversized ' + key)
    manifest = lib_assets.load_manifest(folder / 'manifest.json')
    frames = lib_assets.validate_frames(manifest, folder)
    allowed = set(frames) | {'avatar.json', 'manifest.json'}
    actual = {str(p.relative_to(folder).as_posix()) for p in folder.rglob('*') if p.is_file()}
    if actual != allowed:
        raise ValueError('unexpected or missing avatar files')
    if sum((folder / f).stat().st_size for f in actual) > 64 * 1024 * 1024:
        raise ValueError('avatar exceeds 64 MiB')
    return meta, manifest


def initialize(portrait, output, avatar_id, author, license_name, attribution):
    # Pillow is only a build-time dependency; the runtime never loads it.
    from PIL import Image, ImageOps
    portrait, output = Path(portrait), Path(output)
    if portrait.is_symlink() or not portrait.is_file() or portrait.stat().st_size > 16 * 1024 * 1024:
        raise ValueError('portrait must be a regular image up to 16 MiB')
    if output.exists():
        raise ValueError('output already exists; preserve it or choose a new directory')
    if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,47}', avatar_id):
        raise ValueError('invalid avatar id')
    with Image.open(portrait) as source:
        if source.width * source.height > 16_000_000:
            raise ValueError('portrait dimensions too large')
        image = ImageOps.contain(source.convert('RGBA'), (240, 240))
    if image.getbbox() is None:
        raise ValueError('portrait is fully transparent')
    canvas = Image.new('RGBA', (256, 256))
    canvas.alpha_composite(image, ((256 - image.width) // 2, 248 - image.height))
    template = ROOT / 'assets/v6/manifest.json'
    if not template.exists():
        template = ROOT / 'templates/v6-manifest.json'
    manifest = json.loads(template.read_text())
    output.mkdir(parents=True)
    try:
        for name, animation in manifest['animations'].items():
            animation['frames'] = [f'frames/{name}/{i:02}.png' for i in range(8)]
            for frame in animation['frames']:
                path = output / frame
                path.parent.mkdir(parents=True, exist_ok=True)
                canvas.save(path, optimize=False)
        meta = {'format': FORMAT, 'id': avatar_id, 'name': avatar_id,
                'author': author, 'license': license_name, 'attribution': attribution,
                'mode': 'portrait', 'personalization': {'schema': 1, 'theme': None}}
        (output / 'avatar.json').write_text(json.dumps(meta, indent=2) + '\n')
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        validate(output)
    except Exception:
        shutil.rmtree(output)
        raise
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    init = commands.add_parser('init')
    for name in ('portrait', 'output', 'id', 'author', 'license', 'attribution'):
        init.add_argument('--' + name, required=True)
    check = commands.add_parser('validate')
    check.add_argument('folder')
    args = parser.parse_args()
    try:
        if args.command == 'init':
            print(initialize(args.portrait, args.output, args.id, args.author, args.license, args.attribution))
        else:
            meta, _ = validate(args.folder)
            print('Valid avatar:', meta['id'])
    except (ValueError, OSError, json.JSONDecodeError, lib_assets.AssetError) as error:
        parser.exit(2, str(error) + '\n')


if __name__ == '__main__':
    main()
