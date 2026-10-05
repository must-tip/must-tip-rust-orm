# CI and website hardening — 2026-10-05

The real Git repository is `must-tip/must-tip-rust-orm`. The original workflow was nested beneath the Rust workspace and therefore undiscoverable by GitHub. The active workflow now lives at repository-root `.github/workflows/release-contract.yml`. Root README and `docs/ci.md` distinguish the current workspace/site from the retained documentation archive without breaking existing consumer paths.

## Changes

- Separate workflow-lint, quality, database/HTTP, developer acceptance, packaging, stable, dependency audit and documentation jobs. A required `release-contract` aggregation fails on failed, skipped or cancelled dependencies.
- Explicit working directories, target directories, Cargo/npm caching, strict lockfile installs, job/gate timeouts, superseded-run cancellation, read-only validation permissions, disposable database/cookie fixtures, independent evidence artifacts and no crate uploads.
- Push-time consumer of the actual GitHub SHA, plus the existing local branch/revision/tag, generated application, macro and packaged-source workflows.
- Chromium tests for all 34 advertised documentation pages and five user journeys, served beneath `/must-tip-rust-orm/` without a development-server SPA fallback.
- Dedicated section-navigation component preserves HashRouter routes, scrolls to the heading and moves keyboard focus. A browser regression first reproduced the lost-route bug and then passed after the fix.
- The runner can emit fresh CI evidence outside tracked historical reports, validates environment requirements only for DB gates, records missing commands/timeouts and prints failed-gate log tails.
- Packaged-source verification resolves Cargo's target directory through metadata, fixing failure with a shared target outside a temporary checkout.
- All ten crate manifests inherit the publicly verified repository URL. Weekly Dependabot updates cover Actions, Cargo and the active website.
- Optional default-branch Pages deployment requires all checks to pass and `LEX_DEPLOY_DOCS=true`; repository Pages settings must select GitHub Actions. PRs, tags and manual validation do not deploy. Deployment permissions are limited to its job.

## Executed evidence

See `ci-2026-10-05-results.json` for exact runner commands, exits, original failures, remediation contexts and log paths. All 27 final runner gates passed across the modified checkout and clean packaging snapshot. The workspace reported 223 passing tests and three legacy opt-in PostgreSQL tests, each subsequently run explicitly and passed. PostgreSQL/SQLite independent and generated consumers, strict formatting/Clippy/rustdoc, stable build, macro consumers, all ten packages, publication dry runs and Rust advisory checks passed.

Website `npm ci`, production build and npm audit passed; npm reported zero known vulnerabilities. The final production bundle passed 39 Chromium tests under the project prefix; the strengthened section-focus assertion also passed separately. Actionlint 1.7.7 and `git diff --check` passed. An independent consumer of public GitHub revision `0a3db27f743a9dc2fb9ca3f85692822525d8f702` passed 13 tests and the first-model example.

Two initial local packaging gates refused uncommitted source changes. Verification from a clean temporary Git snapshot passed without `--allow-dirty`, matching CI's committed-source policy. That snapshot exposed the package-consumer target-path bug; after correcting it, the consumer of rebuilt snapshot packages passed. Original nonzero results are retained in the evidence JSON rather than rewritten as successes.

## Practical limits

These are local executions of the workflow's commands and browser tests, not a completed GitHub Actions run. The newly modified source has not been pushed by this task. No GitHub settings, repository variables, branch protection or live deployment were changed. Pages requires the documented one-time maintainer setup. The production bundle still emits Vite's approximately 694 KB chunk warning. Earlier framework coverage and schema-upgrade limitations remain in REPORT.md; this CI work does not certify the entire original production-release definition of done.
