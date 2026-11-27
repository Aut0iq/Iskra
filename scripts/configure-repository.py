#!/usr/bin/env python3
"""Reconcile native branch/tag rules. App IDs are public; this never reads a key."""
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPO = 'Energy-Quant/Iskra'
WRITERS = {'Energy-Quant': 128940291, 'Aut0iq': 71998777}
APP_KEY = 'ISKRA_RELEASE_APP_PRIVATE_KEY'


def api(path, method='GET', data=None):
    args = ['gh', 'api', path, '--method', method]
    if data is not None:
        args += ['--input', '-']
    response = subprocess.check_output(args, input=None if data is None else json.dumps(data).encode())
    return json.loads(response) if response.strip() else {}


def api_all(path, key=None):
    pages = json.loads(subprocess.check_output(['gh', 'api', '--paginate', '--slurp', path], text=True))
    return [item for page in pages for item in (page[key] if key else page)]


def check_writers(collaborators, app_id):
    for user in collaborators:
        if not user.get('permissions', {}).get('push'):
            continue
        login = user['login']
        if login not in WRITERS or user['id'] != WRITERS[login] or (login != 'Energy-Quant' and not app_id):
            raise ValueError(f'unexpected upstream writer: {login}')
        if login == 'Aut0iq' and user.get('role_name') != 'write':
            raise ValueError('Aut0iq must have Write, not broader repository administration')


def check_app_secret_scope():
    repo = f'repos/{REPO}'
    if api(repo + '/actions/secrets')['total_count'] != 0:
        raise ValueError('repository-level Actions secrets must be empty before enabling maintainer Write')
    for name in ('release', 'stable-release'):
        path = repo + f'/environments/{name}'
        policy = api(path).get('deployment_branch_policy') or {}
        branches = api_all(path + '/deployment-branch-policies?per_page=100', 'branch_policies')
        if (policy.get('custom_branch_policies') is not True or policy.get('protected_branches') is not False
                or [(r['name'], r['type']) for r in branches] != [('main', 'branch')]):
            raise ValueError(f'{name} must allow deployments from the main branch only')
        if not any(s['name'] == APP_KEY for s in api_all(path + '/secrets?per_page=100', 'secrets')):
            raise ValueError(f'{name} has no App key environment secret')


def desired(app_id):
    rules = [json.loads(p.read_text()) for p in sorted((ROOT / '.github/rulesets').glob('*.json'))]
    if app_id:
        actor = {'actor_id': app_id, 'actor_type': 'Integration', 'bypass_mode': 'always'}
        for rule in rules:
            if rule['name'] in ('dev-owner-and-ci-updates', 'dev-required-ci', 'release-tags-ci-creation'):
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
    if args.app_id <= 0:
        p.error('--app-id is required for this protected repository')
    try:
        check_writers(api_all(f'repos/{REPO}/collaborators?per_page=100'), args.app_id)
        check_app_secret_scope()
    except ValueError as error:
        p.error(str(error))
    existing = api_all(f'repos/{REPO}/rulesets?per_page=100')
    failures = []
    for wanted in sorted(desired(args.app_id), key=lambda r: r['name'] == 'dev-owner-and-ci-updates'):
        current = next((r for r in existing if r['name'] == wanted['name']), None)
        if args.check:
            actual = api(f"repos/{REPO}/rulesets/{current['id']}") if current else {}
            if not contains(actual, wanted):
                failures.append(wanted['name'])
        else:
            path = f'repos/{REPO}/rulesets' + (f"/{current['id']}" if current else '')
            result = api(path, 'PUT' if current else 'POST', wanted)
            print(result['name'], result['enforcement'])
    if args.check:
        if not api(f'repos/{REPO}/immutable-releases').get('enabled'):
            failures.append('immutable releases')
    else:
        api(f'repos/{REPO}/immutable-releases', 'PUT')
    if failures:
        p.error('rule drift: ' + ', '.join(failures))


if __name__ == '__main__':
    main()
