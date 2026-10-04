> This is a historical documentation archive. The current audited contract, installation limitations, executable examples and release gates are maintained in the [framework developer documentation](../../lex-framework-v1.0.0/lex-framework-v1.0.0/docs/README.md). Historical snippets do not establish release readiness.

# Lex Framework Documentation Website — v1.0.0

Static documentation website for Lex Framework v1.0.0, built with React and Vite.

## Run locally

```bash
npm install
npm run dev
```

## Production build

```bash
npm run build
```

The generated `dist/` directory is static and can be deployed to GitHub Pages, Cloudflare Pages, Netlify, S3-compatible object storage, or a standard web server. The app uses hash routing so static hosts do not require SPA rewrite rules.

## Documentation versioning

Versioned content lives under `src/content/v1.0.0/`. Future releases can add new version directories while retaining v1.0.0.
