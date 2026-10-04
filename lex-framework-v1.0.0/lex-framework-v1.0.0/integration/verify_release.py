#!/usr/bin/env python3
"""Execute release gates and record evidence; never publish or supply deployment secrets."""
import argparse
import datetime
import json
import os
import pathlib
import subprocess

root = pathlib.Path(__file__).resolve().parents[1]
assert os.environ.get('CONFIRM_DISPOSABLE_DATABASE') == 'yes', 'explicit disposable PostgreSQL confirmation required'
assert os.environ.get('TEST_DATABASE_URL'), 'dedicated PostgreSQL URL required'
assert os.environ.get('LEX_TEST_COOKIE_SECRET'), 'disposable cookie test secret required'
checks = [
    ('toolchain', ['rustup', 'show']),
    ('rust-version', ['rustc', '--version']),
    ('cargo-version', ['cargo', '--version']),
    ('format', ['cargo', 'fmt', '--all', '--', '--check']),
    ('consumer-format', ['cargo', 'fmt', '--manifest-path', 'integration/consumer/Cargo.toml', '--', '--check']),
    ('check', ['cargo', 'check', '--workspace', '--all-targets', '--all-features', '--locked']),
    ('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--', '-D', 'warnings']),
    ('workspace-tests', ['cargo', 'test', '--workspace', '--all-targets', '--all-features', '--locked']),
    ('doctests', ['cargo', 'test', '--workspace', '--all-features', '--doc', '--locked']),
    ('postgres-orm', ['cargo', 'test', '-p', 'lex-db-controller', '--test', 'postgres_orm', '--', '--ignored', '--test-threads=1']),
    ('postgres-permissions', ['cargo', 'test', '-p', 'lex-rest', '--test', 'postgres_permissions', '--', '--ignored', '--test-threads=1']),
    ('postgres-mutations', ['cargo', 'test', '-p', 'lex-rest', '--test', 'postgres_mutation', '--', '--ignored', '--test-threads=1']),
    ('rustdoc', ['cargo', 'doc', '--workspace', '--all-features', '--no-deps', '--locked']),
    ('features-default', ['cargo', 'check', '-p', 'lex-framework', '--no-default-features', '--locked']),
    ('features-axum', ['cargo', 'check', '-p', 'lex-framework', '--features', 'axum', '--locked']),
    ('features-rustls', ['cargo', 'check', '-p', 'lex-framework', '--features', 'axum-rustls', '--locked']),
    ('default-tests', ['cargo', 'test', '--workspace', '--all-targets', '--locked']),
    ('consumer', ['python3', 'integration/verify_consumer.py', '--postgres']),
    ('macro-consumers', ['python3', 'integration/verify_macro_consumers.py']),
    ('quickstart', ['python3', 'integration/verify_examples.py']),
    ('generated', ['python3', 'integration/verify_generated.py', '--postgres']),
    ('git-consumer', ['python3', 'integration/verify_consumer.py', '--local-git']),
    ('package', ['cargo', 'package', '--workspace', '--exclude', 'lex_framework_server', '--all-features', '--locked']),
    ('packaged-consumer', ['python3', 'integration/verify_packages.py']),
    ('publish-dry-run', ['cargo', 'publish', '--workspace', '--exclude', 'lex_framework_server', '--all-features', '--locked', '--dry-run']),
    ('stable', ['cargo', '+stable', 'check', '--workspace', '--all-targets', '--all-features', '--locked']),
    ('audit', ['cargo', 'audit', '--json']),
]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--only', nargs='+', choices=[name for name, _ in checks], help='Repeat selected gates while retaining other recorded results')
args = parser.parse_args()
results = json.loads((root / 'audit/release-results.json').read_text()) if args.only and (root / 'audit/release-results.json').exists() else []
if args.only:
    checks = [(name, command) for name, command in checks if name in args.only]
for name, command in checks:
    print('RUN', name, ' '.join(command), flush=True)
    env = dict(os.environ)
    if name == 'rustdoc': env['RUSTDOCFLAGS'] = '-D warnings'
    if name == 'stable': env['CARGO_TARGET_DIR'] = '/tmp/lex-stable-target'
    start = datetime.datetime.now(datetime.timezone.utc)
    logfile = root / 'audit' / f'release-{name}.log'
    with logfile.open('w') as output:
        result = subprocess.run(command, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT)
    results = [record for record in results if record['name'] != name]
    results.append({'name': name, 'command': command, 'started_utc': start.isoformat(), 'exit_code': result.returncode, 'log': str(logfile.relative_to(root))})
    (root / 'audit/release-results.json').write_text(json.dumps(results, indent=2) + '\n')
    print('RESULT', name, result.returncode, flush=True)
print('Recorded all gates. Nonzero results remain failures or blockers; inspect logs.', flush=True)
raise SystemExit(0 if all(result['exit_code'] == 0 for result in results) else 1)
