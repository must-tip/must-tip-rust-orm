#!/usr/bin/env python3
"""Test normalized packaged sources, using local patches until registry publication."""
import json, pathlib, shutil, subprocess, tarfile, tempfile, tomllib
root=pathlib.Path(__file__).resolve().parents[1]
names=['lex-db-controller','lex-media','lex-secure-input','lex-cli','lex-orm','lex-models','lex-migration','lex-db','lex-rest','lex-framework']
with tempfile.TemporaryDirectory(prefix='lex-packages-') as temporary:
    temporary=pathlib.Path(temporary)
    paths={}
    for name in names:
        archive=root/'target/package'/f'{name}-1.0.0.crate'
        with tarfile.open(archive) as package:
            members=package.getnames()
            for required in ['Cargo.toml','README.md','LICENSE-MIT','LICENSE-APACHE']:
                assert f'{name}-1.0.0/{required}' in members,(name,required)
            package.extractall(temporary,filter='data')
        source=temporary/f'{name}-1.0.0'
        manifest=tomllib.loads((source/'Cargo.toml').read_text())
        assert manifest['package']['version']=='1.0.0'
        for kind in ['dependencies','dev-dependencies','build-dependencies']:
            for declaration in manifest.get(kind,{}).values():
                if isinstance(declaration,dict):
                    assert 'path' not in declaration and 'workspace' not in declaration,(name,declaration)
        paths[name]=source
    consumer=temporary/'consumer'
    shutil.copytree(root/'integration/consumer',consumer,ignore=shutil.ignore_patterns('Cargo.lock','target'))
    manifest=consumer/'Cargo.toml'
    content=manifest.read_text().replace('{ package = "lex-framework", path = "../../crate/lex_framework", features = ["axum"] }','{ package = "lex-framework", version = "=1.0.0", features = ["axum"] }')
    content+='\n[patch.crates-io]\n'+''.join(f'{name} = {{ path = {json.dumps(str(path))} }}\n' for name,path in paths.items())
    manifest.write_text(content)
    subprocess.run(['cargo','test','--manifest-path',str(manifest)],cwd=consumer,check=True)
    subprocess.run(['cargo','run','--manifest-path',str(manifest),'--example','first_model'],cwd=consumer,check=True)
    print('Packaged source consumer passed; local patches simulate unpublished registry entries. Real registry ownership/publication is a separate gate.')
