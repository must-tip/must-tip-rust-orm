#!/usr/bin/env python3
"""Read-only namespace evidence. Existence does not establish Lex maintainer ownership."""
import datetime
import argparse
import json
import pathlib
import time
import urllib.error
import urllib.request

root = pathlib.Path(__file__).resolve().parents[1]
results = []
import tomllib
parser = argparse.ArgumentParser()
parser.add_argument('--names', nargs='+')
parser.add_argument('--output', default='registry.json')
args = parser.parse_args()
names = args.names or [tomllib.loads(manifest.read_text())['package']['name'] for manifest in sorted((root / 'crate').glob('*/Cargo.toml'))]
for name in names:
    request = urllib.request.Request(f'https://crates.io/api/v1/crates/{name}', headers={'User-Agent': 'Lex release audit (read-only namespace verification)'})
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            crate = json.load(response)['crate']
        results.append({'name': name, 'status': 'exists', 'repository': crate.get('repository'), 'latest': crate.get('max_version'), 'ownership': 'not established'})
    except urllib.error.HTTPError as error:
        results.append({'name': name, 'status': 'not found' if error.code == 404 else 'request failed', 'http_status': error.code})
    except Exception as error:
        results.append({'name': name, 'status': 'unverified', 'reason': str(error)})
    time.sleep(1)
report = {'checked_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'crates': results}
(root / 'audit' / args.output).write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
