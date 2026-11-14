#!/usr/bin/env python3
"""Reconcile native branch/tag rules. App IDs are public; this never reads a key."""
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPO = 'Energy-Quant/Iskra'


def api(path, method='GET', data=None):
    args = ['gh', 'api', path, '--method', method]
    if data is not None:
        args += ['--input', '-']
    return json.loads(subprocess.check_output(args, input=None if data is None else json.dumps(data).encode()))


def desired(app_id):
    rules = [json.loads(p.read_text()) for p in sorted((ROOT / '.github/rulesets').glob('*.json'))]
    if app_id:
        actor = {'actor_id': app_id, 'actor_type': 'Integration', 'bypass_mode': 'always'}
        for rule in rules:
            if rule['name'] in ('dev-owner-and-ci-updates', 'release-tags-ci-creation'):
                rule['bypass_actors'].append(actor)
        rules.append({'name': 'main-ci-updates-only', 'target': 'branch', 'enforcement': 'active',
                      'bypass_actors': [actor], 'conditions': {'ref_name': {'include': ['refs/heads/main'], 'exclude': []}},
                      'rules': [{'type': 'update'}]})
    return rules


def contains(actual, wanted):
    if isinstance(wanted, dict):
        return isinstance(actual, dict) and all(k in actual and contains(actual[k], v) for k, v in wanted.items())
    if isinstance(wanted, list):
        return isinstance(actual, list) and len(actual) == len(wanted) and all(any(contains(a, w) for a in actual) for w in wanted)
    return actual == wanted


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--app-id', type=int, default=0)
    p.add_argument('--check', action='store_true')
    args = p.parse_args()
    collaborators = api(f'repos/{REPO}/collaborators')
    if any(c['login'] != 'Energy-Quant' and c.get('permissions', {}).get('push') for c in collaborators):
        p.error('fork isolation requires upstream write access to remain owner-only')
    existing = api(f'repos/{REPO}/rulesets')
    failures = []
    for wanted in desired(args.app_id):
        current = next((r for r in existing if r['name'] == wanted['name']), None)
        if args.check:
            actual = api(f"repos/{REPO}/rulesets/{current['id']}") if current else {}
            if not contains(actual, wanted):
                failures.append(wanted['name'])
        else:
            path = f'repos/{REPO}/rulesets' + (f"/{current['id']}" if current else '')
            result = api(path, 'PUT' if current else 'POST', wanted)
            print(result['name'], result['enforcement'])
    if failures:
        p.error('rule drift: ' + ', '.join(failures))
    if not args.app_id:
        print('BOOTSTRAP: main requires PR + CI; all new tags are blocked. Provision the automation App for CI-only tags/merges.')


if __name__ == '__main__':
    main()
