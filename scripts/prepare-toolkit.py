#!/usr/bin/env python3
"""Stage the portable, code-only avatar authoring toolkit."""
import argparse
import shutil
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('output', type=Path)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
for relative in ('scripts/avatars.py', 'scripts/lib_assets.py', 'LICENSE', 'LICENSE-ARTWORK', 'ARTWORK.md', 'docs/AVATARS.md'):
    dest = a.output / relative
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / relative, dest)
(a.output / 'templates').mkdir()
shutil.copy2(ROOT / 'assets/v6/manifest.json', a.output / 'templates/v6-manifest.json')
(a.output / 'requirements.txt').write_text('Pillow==13.0.0\n')
(a.output / 'README.md').write_text('Iskra avatar toolkit. Install requirements.txt, then follow docs/AVATARS.md. No Zeron keys or signing tools included.\n')
