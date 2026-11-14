#!/usr/bin/env python3
"""Open a dev-to-main PR only for new content with a successful exact-SHA gate."""
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('delivery', ROOT / 'scripts/release-control.py')
delivery = importlib.util.module_from_spec(spec); spec.loader.exec_module(delivery)
api = delivery.api
repo = 'repos/Energy-Quant/Iskra'
dev = api(repo + '/git/ref/heads/dev')['object']['sha']
main = api(repo + '/git/ref/heads/main')['object']['sha']
if api(repo + '/git/commits/' + dev)['tree']['sha'] == api(repo + '/git/commits/' + main)['tree']['sha']:
    print('No content difference; do not create a promotion/synchronization loop.')
    raise SystemExit(0)
runs = api(repo + '/actions/workflows/ci.yml/runs?branch=dev&status=success&per_page=50')['workflow_runs']
run = next((r for r in runs if r['head_sha'] == dev and r['event'] in ('push', 'workflow_dispatch')), None)
if run is None:
    raise ValueError('current dev has no successful upstream CI')
delivery.source_run(run, 'nightly')
jobs = api(repo + f'/actions/runs/{run["id"]}/jobs?per_page=100')['jobs']
if not any(j['name'] == 'CI Gate' and j['conclusion'] == 'success' for j in jobs):
    raise ValueError('CI Gate missing')
existing = api(repo + '/pulls?state=open&base=main&head=Energy-Quant:dev')
if existing:
    print('Existing promotion PR:', existing[0]['html_url'])
else:
    subprocess.run(['gh', 'pr', 'create', '--repo', 'Energy-Quant/Iskra', '--base', 'main', '--head', 'dev',
                    '--title', 'Promote green dev to main', '--body', 'Exact dev CI: ' + run['html_url']], check=True)
