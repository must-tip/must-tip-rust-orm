#!/usr/bin/env python3
"""Compile and execute Model derives with default and renamed public dependencies."""
import json
import pathlib
import subprocess
import tempfile
root = pathlib.Path(__file__).resolve().parents[1]
cases = [
    ('direct-default', 'lex-db-controller', 'lex_db_controller', 'lex-orm', 'lex_orm'),
    ('direct-alias', 'storage', 'storage', 'derive_model', 'derive_model'),
    ('facade-alias', None, 'lex::db_controller', None, None),
]
for name, key, runtime, macro_key, macro_import in cases:
    with tempfile.TemporaryDirectory(prefix=f'lex-macro-{name}-') as directory:
        directory = pathlib.Path(directory)
        (directory / 'src').mkdir()
        if key:
            dependencies = f'{key} = {{ package = "lex-db-controller", path = {json.dumps(str(root / "crate/db_controller"))} }}\n{macro_key} = {{ package = "lex-orm", path = {json.dumps(str(root / "crate/orm"))} }}\n'
        else:
            dependencies = f'lex = {{ package = "lex-framework", path = {json.dumps(str(root / "crate/lex_framework"))} }}\n'
        (directory / 'Cargo.toml').write_text('[package]\nname = "macro_consumer"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'+dependencies+'serde = { version = "1", features = ["derive"] }\nserde_json = "1"\ntokio = { version = "1", features = ["macros", "rt"] }\n')
        source = (root / 'integration/consumer/src/lib.rs').read_text()
        if key:
            source = source.replace('use lex_framework::prelude::*;',f'use {macro_import}::Model;').replace('lex_framework::db_controller',runtime)
        else:
            source = source.replace('lex_framework','lex')
        source += f'''
#[tokio::test]
async fn aliased_models_round_trip_and_expose_relation_and_constraint_metadata() {{
    use {runtime}::Entity;
    assert_eq!(ProjectProfile::relations().len(), 1);
    assert_eq!(ProjectProfile::unique_constraints().len(), 1);
    let pool = {runtime}::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    let manager = Budget::objects_sqlite_auto(pool).await.unwrap();
    let saved = manager.queryset().create(&Budget {{ id: UUIDField::new(), amount: {runtime}::DecimalField::from_str("9007199254740993.125").unwrap(), adjustment: None }}).await.unwrap();
    let loaded = manager.queryset().filter(Budget::ID.eq(saved.id)).get().await.unwrap();
    assert_eq!(loaded.amount.0.to_string(), "9007199254740993.125");
    assert!(loaded.adjustment.is_none());
}}
'''
        (directory / 'src/lib.rs').write_text(source)
        print('MACRO CONSUMER', name, dependencies, flush=True)
        subprocess.run(['cargo','test','--manifest-path',str(directory / 'Cargo.toml')], cwd=directory, check=True)
print('Default and aliased direct/facade macro consumers passed.', flush=True)
