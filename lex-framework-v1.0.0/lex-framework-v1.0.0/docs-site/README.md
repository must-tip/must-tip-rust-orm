# Lex Framework Documentation Website — v1.0.0

Static documentation website for Lex Framework v1.0.0, built with React and Vite.

## Run locally

```bash
npm ci
npm run dev
```

## Production build

```bash
npm run build
```

The generated `dist/` directory is static and can be deployed to GitHub Pages, Cloudflare Pages, Netlify, S3-compatible object storage, or a standard web server. The app uses hash routing so static hosts do not require SPA rewrite rules.

## Documentation versioning

Versioned content lives under `src/content/v1.0.0/`. Future releases can add new version directories while retaining v1.0.0.

## Browser acceptance and hosting

Run `npx playwright install --with-deps chromium`, `npm run build`, then `npm run test:browser`. Tests use the production bundle and validate every navigation page, search, mobile navigation, deep links, theme persistence and section links. See the repository root `docs/ci.md` for opt-in GitHub Pages deployment after all framework and website checks pass. The sibling historical website is not deployed.
