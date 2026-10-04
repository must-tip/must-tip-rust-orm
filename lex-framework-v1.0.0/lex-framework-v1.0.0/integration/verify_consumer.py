#!/usr/bin/env python3
"""Build outside the workspace. --local-git tests transport, not a hosted GitHub release."""
import argparse, json, os, pathlib, shutil, subprocess, tempfile
root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--local-git', action='store_true')
parser.add_argument('--postgres', action='store_true')
parser.add_argument('--git')
parser.add_argument('--tag')
parser.add_argument('--rev')
args = parser.parse_args()
def run(command, cwd):
    subprocess.run(command, cwd=cwd, check=True)
with tempfile.TemporaryDirectory(prefix='lex-consumer-') as temporary:
    temporary = pathlib.Path(temporary)
    cases = []
    if args.local_git:
        repository = temporary / 'repository'
        repository.mkdir()
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']:
            shutil.copy2(root / name, repository / name)
        for name in ['crate', 'project', 'apps', 'migrations']:
            shutil.copytree(root / name, repository / name)
        run(['git', 'init', '-b', 'main'], repository)
        run(['git', 'add', '.'], repository)
        run(['git', '-c', 'user.name=Lex acceptance', '-c', 'user.email=acceptance@localhost', 'commit', '-m', 'Local acceptance snapshot'], repository)
        run(['git', 'tag', 'v1.0.0'], repository)
        revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip()
        url = repository.as_uri()
        cases = [('branch', {'git': url, 'branch': 'main'}), ('revision', {'git': url, 'rev': revision}), ('tag', {'git': url, 'tag': 'v1.0.0'})]
    elif args.git:
        if bool(args.tag) == bool(args.rev):
            parser.error('--git requires exactly one of --tag or --rev')
        cases = [('hosted-git', {'git': args.git, 'tag' if args.tag else 'rev': args.tag or args.rev})]
    else:
        cases = [('path', {'path': str(root / 'crate/lex_framework')})]
    for name, source in cases:
        consumer = temporary / name
        shutil.copytree(root / 'integration/consumer', consumer, ignore=shutil.ignore_patterns('target', 'Cargo.lock'))
        declaration = '{ package = "lex-framework", ' + ', '.join(f'{key} = {json.dumps(value)}' for key, value in source.items()) + ', features = ["axum"] }'
        manifest = consumer / 'Cargo.toml'
        manifest.write_text(manifest.read_text().replace('{ package = "lex-framework", path = "../../crate/lex_framework", features = ["axum"] }', declaration))
        print('TESTED DEPENDENCY:', declaration, flush=True)
        run(['cargo', 'test', '--manifest-path', str(manifest)] + (['--features','postgres-integration'] if args.postgres else []), consumer)
        run(['cargo', 'run', '--manifest-path', str(manifest), '--example', 'first_model'], consumer)
