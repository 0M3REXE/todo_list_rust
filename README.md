# Practice

A small, server-rendered todo app built with Rust, Axum, Askama, HTMX 4, and libSQL. Accounts can sign up, sign in, change their password, and end other sessions. Each todo is scoped to its owner.

## Run locally

Rust is required. The app defaults to a local `practice.db` file, `127.0.0.1:3000`, and secure cookies. For plain HTTP development, set:

```powershell
$env:SESSION_SECURE = "false"
$env:APP_ALLOWED_HOST = "127.0.0.1:3000,localhost:3000"
cargo run
```

The development database is created and migrated automatically. To use Turso instead, set `DATABASE_URL` to a `libsql://` URL and `DATABASE_AUTH_TOKEN` to its token. Never commit either secret.

## Frontend assets

Askama compiles templates into the Rust binary. Tailwind builds `static/app.css`, which the server embeds at compile time. After changing a template or style, rebuild the CSS and restart/rebuild the app:

```sh
npm ci
npm run css:build
cargo run
```

Use `npm run css:watch` while editing styles. The vendored `static/vendor/htmx.min.js` is pinned to 4.0.0.

## Deploy to Vercel

Configure `DATABASE_URL` and `DATABASE_AUTH_TOKEN` for the Turso database. HTTPS deployments automatically use secure session cookies. Add custom trusted domains to `APP_ALLOWED_HOST` only when needed; Vercel deployment hostnames are included automatically.

## Account limitations

The app does not send email. Email verification and password reset are not available, so users must keep their password safe. Account deletion and task export are not currently available.

## Project map

- `src/app.rs` — routes, session layer, shared middleware, static assets
- `src/auth/` — authentication, password handling, account pages, session storage
- `src/todo/` — todo handlers, queries, and templates
- `src/db.rs` and `migrations/` — libSQL connection and schema
- `templates/` and `static/` — server-rendered UI and browser assets
- `tests/` — router and data-layer coverage
- `AUDIT_REMEDIATION_PLAN.md` — audit findings and remaining roadmap
