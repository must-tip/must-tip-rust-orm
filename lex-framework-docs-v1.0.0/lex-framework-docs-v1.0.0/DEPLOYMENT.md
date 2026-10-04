# Static deployment

1. Run `npm install`.
2. Run `npm run build`.
3. Upload the contents of `dist/` to your static host.

Because the site uses `HashRouter` and Vite `base: "./"`, it works from a subdirectory and does not require URL rewrite rules.

Recommended production headers:

- `Content-Security-Policy` appropriate to your hosting setup.
- `Strict-Transport-Security` when HTTPS is enforced.
- `X-Content-Type-Options: nosniff`.
- `Referrer-Policy: strict-origin-when-cross-origin`.

The site does not require a backend service.
