#!/usr/bin/env python3
"""Copy canonical guide sources into the documentation site; avoid independent contracts."""
import pathlib
import shutil
root = pathlib.Path(__file__).resolve().parents[1]
content = root / 'docs-site/src/content/v1.0.0'
pages = {
    'introduction': 'docs/README.md',
    'installation': 'docs/getting-started/installation.md',
    'quick-start': 'docs/getting-started/quickstart.md',
    'architecture': 'docs/concepts/architecture.md',
    'project-structure': 'docs/concepts/architecture.md',
    'database': 'docs/guides/databases.md',
    'sqlite': 'docs/guides/databases.md',
    'models-querysets': 'docs/concepts/models-and-queries.md',
    'orm': 'docs/concepts/models-and-queries.md',
    'transactions': 'docs/concepts/transactions.md',
    'migrations': 'docs/concepts/migrations.md',
    'settings': 'docs/reference/configuration.md',
    'cli': 'docs/reference/cli.md',
    'authentication': 'docs/guides/authentication.md',
    'sessions-tokens': 'docs/guides/authentication.md',
    'permissions': 'docs/guides/rbac.md',
    'middleware-security': 'docs/guides/security.md',
    'security': 'docs/guides/security.md',
    'api-contracts': 'docs/guides/security.md',
    'views-and-urls': 'docs/guides/rest-api.md',
    'rest-framework': 'docs/guides/rest-api.md',
    'media': 'docs/guides/application-boundaries.md',
    'secure-input': 'docs/guides/application-boundaries.md',
    'idempotency-throttling': 'docs/guides/application-boundaries.md',
    'websockets': 'docs/guides/application-boundaries.md',
    'production-checklist': 'docs/operations/release.md',
    'packaging': 'docs/operations/release.md',
    'operations': 'docs/operations/deployment.md',
    'versioning': 'docs/operations/release.md',
    'release-notes': 'CHANGELOG.md',
    'testing': 'audit/TEST_MATRIX.md',
    'full-developer-guide': 'docs/README.md',
}
for slug, source in pages.items():
    (content / f'{slug}.md').write_text((root / source).read_text())
for source in (root / 'docs').rglob('*.md'):
    destination = root / 'docs-site/framework-source-docs' / source.relative_to(root / 'docs')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
print(f'Synchronized {len(pages)} canonical documentation pages.')
