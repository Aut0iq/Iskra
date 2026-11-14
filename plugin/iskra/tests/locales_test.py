#!/usr/bin/env python3
"""Parity check for plugin/iskra/locales/*.json.

Every locale must carry the same flattened (dotted) keys, and each key's
{placeholder} set must match across locales — a translated message may
reorder or repeat a placeholder but never gain or drop one.

Run:  python3 plugin/iskra/tests/locales_test.py
Exits 0 on parity, 1 listing every divergence.
"""

import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
LOCALES = HERE.parent / "locales"
REQUIRED = ("en", "ru", "zh-Hans")
PLACEHOLDER = re.compile(r"\{([^{}]+)\}")


def flatten(node, prefix=""):
    out = {}
    if isinstance(node, dict):
        for key, value in node.items():
            out.update(flatten(value, f"{prefix}{key}." if prefix else f"{key}."))
    else:
        out[prefix.rstrip(".")] = node
    return out


def main() -> int:
    failures = 0

    catalogs = {}
    for locale in REQUIRED:
        path = LOCALES / f"{locale}.json"
        if not path.is_file():
            print(f"FAIL: missing locale file {path}")
            failures += 1
            continue
        try:
            catalogs[locale] = flatten(json.loads(path.read_text(encoding="utf-8")))
        except json.JSONDecodeError as exc:
            print(f"FAIL: {path.name} is not valid JSON: {exc}")
            failures += 1
    if "en" not in catalogs:
        print(f"{failures} failure(s)")
        return 1

    en_keys = set(catalogs["en"])
    for locale, catalog in catalogs.items():
        missing = sorted(en_keys - set(catalog))
        extra = sorted(set(catalog) - en_keys)
        for key in missing:
            print(f"FAIL {locale}: missing key {key}")
            failures += 1
        for key in extra:
            print(f"FAIL {locale}: extra key {key}")
            failures += 1
        for key in sorted(en_keys & set(catalog)):
            en_ph = sorted(set(PLACEHOLDER.findall(str(catalogs["en"][key]))))
            ph = sorted(set(PLACEHOLDER.findall(str(catalog[key]))))
            if ph != en_ph:
                print(f"FAIL {locale}: {key} placeholders {ph} != en {en_ph}")
                failures += 1

    # every catalog value must be a string the host can format
    for locale, catalog in catalogs.items():
        for key, value in catalog.items():
            if not isinstance(value, str):
                print(f"FAIL {locale}: {key} is {type(value).__name__}, not a string")
                failures += 1

    if failures:
        print(f"{failures} failure(s)")
        return 1
    print(f"OK: {len(en_keys)} keys identical across {sorted(catalogs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
