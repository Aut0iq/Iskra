#!/usr/bin/env python3
"""Open a dev-to-main PR only for new content with a successful exact-SHA gate."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('delivery', ROOT / 'scripts/release-control.py')
delivery = importlib.util.module_from_spec(spec); spec.loader.exec_module(delivery)
api = delivery.api
api_all = delivery.api_all
repo = 'repos/Energy-Quant/Iskra'
if any(r['draft'] and r['tag_name'].startswith('v') for r in api_all(repo + '/releases?per_page=100')):
    print('Stable candidate pending: keep main frozen until author signing/publication completes.')
    raise SystemExit(0)
dev = api(repo + '/git/ref/heads/dev')['object']['sha']
main = api(repo + '/git/ref/heads/main')['object']['sha']
if api(repo + '/git/commits/' + dev)['tree']['sha'] == api(repo + '/git/commits/' + main)['tree']['sha']:
    print('No content difference; do not create a promotion/synchronization loop.')
    raise SystemExit(0)
runs = api_all(repo + '/actions/workflows/ci.yml/runs?branch=dev&per_page=50', 'workflow_runs')
run = next((r for r in runs if r['head_sha'] == dev and r['event'] in ('push', 'workflow_dispatch')), None)
if run is None or run['conclusion'] != 'success':
    print('Current dev is not green; wait for its latest CI.')
    raise SystemExit(0)
delivery.source_run(run, 'nightly')
jobs = api_all(repo + f'/actions/runs/{run["id"]}/jobs?per_page=100', 'jobs')
if not any(j['name'] == 'CI Gate' and j['conclusion'] == 'success' for j in jobs):
    raise ValueError('CI Gate missing')
existing = api_all(repo + '/pulls?state=open&base=main&head=Energy-Quant:dev')
if existing:
    print('Existing promotion PR:', existing[0]['html_url'])
    if os.environ.get('RESUME_PROMOTION') == '1':
        previous = api_all(repo + '/actions/workflows/ci.yml/runs?branch=dev&event=pull_request&per_page=50', 'workflow_runs')
        retry = next((r for r in previous if r['head_sha'] == dev and r['status'] == 'completed'), None)
        if retry:
            subprocess.run(['gh', 'run', 'rerun', str(retry['id']), '--repo', 'Energy-Quant/Iskra'], check=True)
else:
    subprocess.run(['gh', 'pr', 'create', '--repo', 'Energy-Quant/Iskra', '--base', 'main', '--head', 'dev',
                    '--title', 'Promote green dev to main', '--body', 'Exact dev CI: ' + run['html_url']], check=True)
