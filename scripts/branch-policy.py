#!/usr/bin/env python3
"""Validate PR topology and branch names without executing contributor code."""
import json
import os
import re
import subprocess
from pathlib import Path


def validate(event, repository='Energy-Quant/Iskra'):
    pr = event.get('pull_request')
    if not pr:
        return
    base, head = pr['base'], pr['head']
    if base['ref'] == 'main':
        if head['ref'] != 'dev' or head['repo']['full_name'] != repository:
            raise ValueError('main accepts only the upstream dev branch')
    elif base['ref'] == 'dev':
        if not re.fullmatch(r'(feature|bugfix)-[a-z0-9][a-z0-9-]*', head['ref']):
            raise ValueError('use feature-name or bugfix-name based on dev')
        if head['repo']['full_name'] != repository:
            if head['repo']['owner']['login'].lower() != pr['user']['login'].lower():
                raise ValueError('a fork PR must come from its author-owned fork')
    else:
        raise ValueError('PR target must be dev or main')


if __name__ == '__main__':
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    validate(event, os.getenv('GITHUB_REPOSITORY', 'Energy-Quant/Iskra'))
    pr = event.get('pull_request')
    if pr and pr['base']['ref'] == 'dev':
        subprocess.run(['git', 'merge-base', '--is-ancestor', pr['base']['sha'], pr['head']['sha']], check=True)
    print('Branch policy PASS')
