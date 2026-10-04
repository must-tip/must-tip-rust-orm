#!/usr/bin/env python3
"""Run the copyable quickstart outside the workspace, including real TCP HTTP requests."""
import json
import os
import pathlib
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

root=pathlib.Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='lex-quickstart-') as temporary:
    consumer=pathlib.Path(temporary)/'application'
    shutil.copytree(root/'integration/consumer',consumer,ignore=shutil.ignore_patterns('target','Cargo.lock'))
    manifest=consumer/'Cargo.toml'
    manifest.write_text(manifest.read_text().replace('path = "../../crate/lex_framework"',f'path = {json.dumps(str(root/"crate/lex_framework"))}'))
    for command in [['cargo','run','--example','first_model'],['cargo','build','--example','first_api']]:
        subprocess.run(command,cwd=consumer,check=True)
    target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',str(consumer/'target')))
    with socket.socket() as reservation:
        reservation.bind(('127.0.0.1',0)); port=reservation.getsockname()[1]
    environment=dict(os.environ,LEX_EXAMPLE_PORT=str(port),APP_PROFILE='development')
    with (consumer/'server.log').open('w+') as output:
        server=subprocess.Popen([str(target/'debug/examples/first_api')],cwd=consumer,env=environment,stdout=output,stderr=subprocess.STDOUT)
        try:
            address=f'http://127.0.0.1:{port}/notes'; deadline=time.monotonic()+20
            while True:
                try:
                    with urllib.request.urlopen(address,timeout=1) as response:
                        assert response.status==200 and json.load(response)==[]
                    break
                except (OSError,urllib.error.URLError):
                    if server.poll() is not None or time.monotonic()>=deadline:
                        output.seek(0); raise RuntimeError(output.read())
                    time.sleep(0.1)
            payload=json.dumps({'title':'Hello 世界'}).encode()
            request=urllib.request.Request(address,data=payload,headers={'Content-Type':'application/json'},method='POST')
            with urllib.request.urlopen(request,timeout=5) as response:
                assert response.status==201
                saved=json.load(response)
                assert saved['title']=='Hello 世界' and saved['id']
            with urllib.request.urlopen(address,timeout=5) as response:
                assert json.load(response)==[saved]
                assert response.headers['X-Content-Type-Options']=='nosniff'
            forged=urllib.request.Request(address,data=json.dumps({'id':saved['id'],'title':'forged'}).encode(),headers={'Content-Type':'application/json'},method='POST')
            try:
                urllib.request.urlopen(forged,timeout=5)
                raise AssertionError('client-controlled primary key was accepted')
            except urllib.error.HTTPError as error:
                assert error.code==400
        finally:
            server.terminate()
            try: server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill(); server.wait()
    print('Independent model and live HTTP quickstart passed.')
