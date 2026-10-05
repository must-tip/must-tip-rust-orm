#!/usr/bin/env python3
"""Execute release gates and record evidence; never publish or supply deployment secrets."""
import argparse
import datetime
import json
import os
import pathlib
import subprocess

root = pathlib.Path(__file__).resolve().parents[1]
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
parser.add_argument('--only', nargs='+', choices=[name for name, _ in checks], help='Run selected gates; retain historical results only without --output-dir')
parser.add_argument('--output-dir', type=pathlib.Path, help='Write fresh evidence outside the tracked audit directory (CI)')
args = parser.parse_args()
output_dir = args.output_dir.resolve() if args.output_dir else root / 'audit'
output_dir.mkdir(parents=True, exist_ok=True)
results_file = output_dir / 'release-results.json'
results = json.loads(results_file.read_text()) if args.only and not args.output_dir and results_file.exists() else []
if args.only:
    checks = [(name, command) for name, command in checks if name in args.only]
database_checks = {'workspace-tests', 'default-tests', 'postgres-orm', 'postgres-permissions', 'postgres-mutations', 'consumer', 'generated'}
if any(name in database_checks for name, _ in checks):
    for setting in ('TEST_DATABASE_URL', 'LEX_TEST_COOKIE_SECRET'):
        if not os.environ.get(setting): parser.error(f'{setting} is required for database acceptance gates')
    if os.environ.get('CONFIRM_DISPOSABLE_DATABASE') != 'yes':
        parser.error('CONFIRM_DISPOSABLE_DATABASE=yes is required for database acceptance gates')
for name, command in checks:
    print('RUN', name, ' '.join(command), flush=True)
    env = dict(os.environ)
    if name == 'rustdoc': env['RUSTDOCFLAGS'] = '-D warnings'
    if name == 'stable': env['CARGO_TARGET_DIR'] = '/tmp/lex-stable-target'
    start = datetime.datetime.now(datetime.timezone.utc)
    logfile = output_dir / f'release-{name}.log'
    with logfile.open('w') as output:
        try:
            result = subprocess.run(command, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT, timeout=1800)
            exit_code = result.returncode
        except (OSError, subprocess.TimeoutExpired) as error:
            output.write(f'\nGate could not complete: {error}\n')
            exit_code = 124 if isinstance(error, subprocess.TimeoutExpired) else 127
    results = [record for record in results if record['name'] != name]
    results.append({'name': name, 'command': command, 'started_utc': start.isoformat(), 'exit_code': exit_code, 'log': str(logfile) if args.output_dir else str(logfile.relative_to(root))})
    results_file.write_text(json.dumps(results, indent=2) + '\n')
    print('RESULT', name, exit_code, flush=True)
    if exit_code:
        print(logfile.read_text()[-12000:], flush=True)
print('Recorded all gates. Nonzero results remain failures or blockers; inspect logs.', flush=True)
raise SystemExit(0 if all(result['exit_code'] == 0 for result in results) else 1)
