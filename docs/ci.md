# Push validation and documentation hosting

The workflow lives at the Git repository root. GitHub does not discover workflows nested inside the Rust workspace. Only the current framework `docs-site` is built; the sibling documentation archive is retained as historical material.

## Validation contract

Every push, pull request and manual run validates workflow syntax, Rust formatting/check/Clippy/rustdoc and feature combinations; default and all-feature tests with dedicated PostgreSQL 16; SQLite and PostgreSQL independent consumers; CLI-generated applications; procedural macro consumers; live HTTP quickstarts; local branch/revision/tag Git installation; all ten crate packages and publication dry runs; stable Rust; dependency advisories; and the production website in Chromium. Push runs additionally compile a consumer of the actual GitHub commit. Pull requests use the checked-out source and local Git snapshot so untrusted or unpushed commits do not need remote access or secrets.

Dependabot opens reviewed weekly dependency updates for GitHub Actions, Cargo and the active website. Jobs use explicit workspace directories, Cargo and npm caches, immutable lockfile installs, timeouts and cancellation of superseded runs. Test databases and cookie secrets are disposable fixtures. CI uploads independent evidence logs, crate archives and browser failure traces. Historical audit results cannot make a fresh CI job pass. No job uploads crates to crates.io.

Configure branch protection to require **release-contract**. That aggregation job fails if any required job fails, is cancelled or is skipped. Ordinary validation uses read-only repository permissions and no deployment secrets. GitHub Actions must be enabled for the repository. Actual hosted runs cannot be certified until the changes are pushed.

## Enable documentation deployment

1. In repository **Settings → Pages**, choose **GitHub Actions** as the source.
2. In **Settings → Secrets and variables → Actions → Variables**, create `LEX_DEPLOY_DOCS` with value `true`.
3. Push to the repository's default branch. Pages deploys only after every validation job succeeds. PRs, tags and manual validation runs never deploy.

The deployment job alone receives `pages: write` and `id-token: write` and uses the `github-pages` environment. Optional environment protection rules can require maintainer review. Disabling the variable leaves validation and downloadable website artifacts available without deployment. No GitHub setting is changed by these source edits.

The expected project-site address is `https://must-tip.github.io/must-tip-rust-orm/` once GitHub Pages is enabled and successfully deployed; availability has not been verified. Relative assets and hash routing support project subpaths and direct documentation reloads without server rewrite rules.

## Reproduce checks locally

From `lex-framework-v1.0.0/lex-framework-v1.0.0`:

```bash
python3 integration/verify_release.py --output-dir /tmp/lex-ci-evidence --only format consumer-format check clippy rustdoc
# For DB acceptance, start compose.test.yml and set the disposable variables
# documented in CONTRIBUTING.md, then select the database gates.
cd docs-site
npm ci
npm run build
npx playwright install --with-deps chromium
npm run test:browser
npm audit --audit-level=high
```

Browser tests exercise the built bundle, not the development server. They cover all advertised navigation pages, deep-link reloads, search, theme persistence, mobile navigation and section scrolling. The static test server mounts the bundle under `/must-tip-rust-orm/`, binds loopback with a strict port and never reuses an arbitrary running server.

After changing canonical guides, run `python3 integration/sync_docs.py` from the Rust workspace and include its copied website files in the same change. CI rejects stale copies. After changing dependencies, commit the corresponding lockfile. Keep archive paths out of workflow working directories.
