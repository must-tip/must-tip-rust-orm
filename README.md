# Lex Framework repository

The supported framework and documentation website share one release contract.

| Path | Purpose |
|---|---|
| [Framework workspace](lex-framework-v1.0.0/lex-framework-v1.0.0/) | Rust crates, CLI, integration applications and canonical guides |
| [Documentation website](lex-framework-v1.0.0/lex-framework-v1.0.0/docs-site/) | Current React/Vite website built from those guides |
| [.github/workflows](.github/workflows/) | Push/PR validation and optional gated GitHub Pages deployment |
| `lex-framework-docs-v1.0.0/` | Historical documentation archive; not the deployed website |

See [CI and hosting](docs/ci.md) for checks, local commands, branch protection and website setup. See the framework's [contributor guide](lex-framework-v1.0.0/lex-framework-v1.0.0/CONTRIBUTING.md) and [audit report](lex-framework-v1.0.0/lex-framework-v1.0.0/audit/REPORT.md) before treating the release candidate as production ready.

## Local development

```bash
cd lex-framework-v1.0.0/lex-framework-v1.0.0
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cd docs-site
npm ci
npm run dev
```

The existing versioned source paths are retained so local consumers remain compatible. Run Cargo from the framework workspace and npm from its `docs-site` directory.
