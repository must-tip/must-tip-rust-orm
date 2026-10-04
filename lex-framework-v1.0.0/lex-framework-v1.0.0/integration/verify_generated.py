#!/usr/bin/env python3
"""Generate an independent application, compile it, migrate SQLite, and test public workflows."""
import argparse
import os
import json
import pathlib
import shutil
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request

root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--postgres', action='store_true')
args = parser.parse_args()

def run(command, cwd, environment=None):
    subprocess.run(command, cwd=cwd, env=environment, check=True)

run(['cargo', 'build', '-p', 'lex-cli'], root)
target = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', str(root / 'target')))
lex = target / 'debug/lex'
with tempfile.TemporaryDirectory(prefix='lex-generated-') as directory:
    directory = pathlib.Path(directory)
    run([str(lex), '--root', str(directory), 'create', 'project', 'acceptance', '--framework-path', str(root)], directory)
    project = directory / 'acceptance'
    run([str(lex), 'create', 'crate', 'projects'], project)
    app = project / 'apps/projects'
    models = (root / 'integration/consumer/src/lib.rs').read_text()
    models += '\npub fn register_models() { Project::register(); Budget::register(); Task::register(); }\n'
    (app / 'src/models.rs').write_text(models)
    manifest = app / 'Cargo.toml'
    content = manifest.read_text().replace('lex_framework.workspace = true', 'lex_framework = { workspace = true, features = ["axum"] }')
    content += '\n[dev-dependencies]\ntokio.workspace = true\naxum.workspace = true\ntower = { version = "0.5", features = ["util"] }\ntempfile = "3"\n\n[features]\npostgres-integration = []\n'
    manifest.write_text(content)
    (app / 'tests').mkdir()
    for source in (root / 'integration/consumer/tests').glob('*.rs'):
        (app / 'tests' / source.name).write_text(source.read_text().replace('lex_developer_acceptance', 'lex_app_projects::models'))
    run([str(lex), 'refresh'], project)
    run([str(lex), 'check'], project)
    run(['cargo', 'check', '--workspace', '--all-targets'], project)
    run(['cargo', 'test', '-p', 'lex_app_projects'] + (['--features', 'postgres-integration'] if args.postgres else []), project)
    database = project / 'migration-acceptance.sqlite3'
    sqlite3.connect(database).close()
    migrations = project / 'migrations'
    migrations.mkdir(exist_ok=True)
    (migrations / '0001_ledger_up.sql').write_text('CREATE TABLE acceptance_ledger(id INTEGER PRIMARY KEY, note TEXT NOT NULL);')
    (migrations / '0001_ledger_down.sql').write_text('DROP TABLE acceptance_ledger;')
    environment = dict(os.environ, DATABASE_URL=f'sqlite://{database}')
    for _ in range(2):
        run([str(lex), 'migrate', '--', 'migrate'], project, environment)
    with sqlite3.connect(database) as connection:
        connection.execute("INSERT INTO acceptance_ledger(note) VALUES (?)", ("developer's persisted data",))
        assert connection.execute('SELECT note FROM acceptance_ledger').fetchone()[0] == "developer's persisted data"
    run([str(lex), 'migrate', '--', 'rollback'], project, environment)
    with sqlite3.connect(database) as connection:
        assert connection.execute("SELECT name FROM sqlite_master WHERE name='acceptance_ledger'").fetchone() is None
    run(['cargo', 'build', '-p', 'acceptance_server', '--bin', 'acceptance_server'], project)
    with socket.socket() as reservation:
        reservation.bind(('127.0.0.1', 0))
        port = reservation.getsockname()[1]
    config = project / 'Config.toml'
    config.write_text(config.read_text().replace('[database]\n', f'[database]\nurl = "sqlite://{database}"\n').replace('port = 8080', f'port = {port}'))
    server_environment = dict(environment, APP_PROFILE='development')
    with (project / 'server.log').open('w+') as output:
        server = subprocess.Popen([str(target / 'debug/acceptance_server')], cwd=project, env=server_environment, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 20
            while True:
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}/projects', timeout=1) as response:
                        assert response.status == 200
                        assert json.load(response) == {'app': 'projects', 'status': 'ready'}
                        assert response.headers.get('Strict-Transport-Security') is None
                    break
                except (OSError, urllib.error.URLError):
                    if server.poll() is not None or time.monotonic() >= deadline:
                        output.seek(0)
                        raise RuntimeError('Generated server failed: ' + output.read())
                    time.sleep(0.1)
        finally:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
    print('Generated standalone project, model registration, public ORM/HTTP workflows, migration CLI, and live TCP server passed.')
