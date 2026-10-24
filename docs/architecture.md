# Current architecture

This guide describes the checked-in implementation. Older learning notes in this directory may refer to Postgres, SQLx, or htmx 2; those are historical and are not instructions for the current app.

## What the app does

Practice is a server-rendered todo list with account login. A user can create an account, sign in, add/toggle/delete their own tasks, change their password, and revoke other sessions. It does not send email; verification, password reset, account deletion, and task export are not available.

## Stack and request flow

- Axum routes HTTP requests and middleware.
- `axum-login` and `tower-sessions` manage cookie sessions.
- `libsql` talks to a local SQLite-compatible file or a remote Turso database.
- Askama renders templates into the HTML returned by handlers.
- htmx 4 requests and swaps HTML fragments; it is vendored at `static/vendor/htmx.min.js`.
- Tailwind compiles CSS to `static/app.css`, embedded in the binary.

For a todo mutation: browser → htmx request → auth/origin middleware → handler → owner-scoped SQL → Askama fragment → htmx swap. The query includes both the todo id and current user id, so another user's row behaves like a missing row.

## Code map

- `src/main.rs`: local HTTP server and graceful shutdown.
- `server.rs`: Vercel serverless entry point; uses the same app router.
- `src/app.rs`: routes, sessions, common headers, cache policy, and session cleanup.
- `src/auth/`: credentials, password hashing, session storage, auth/account handlers.
- `src/todo/`: todo validation, SQL, handlers, and fragment models.
- `src/db.rs`: libSQL opening, migrations, parameter/row helpers.
- `migrations/`: ordered schema changes embedded in the binary.
- `templates/`: Askama full pages and fragments.
- `static/`: CSS, JavaScript, PWA assets, and vendored htmx.
- `tests/`: integration tests drive the shared Axum router with in-memory databases.

## Local development

The default database is `file:practice.db`; migrations run on startup. For local plain HTTP, set `SESSION_SECURE=false` and allow the local host in `APP_ALLOWED_HOST`. See the root README for commands and the `.env.example` for settings. Never commit `.env` or database tokens.

After editing templates or CSS, run `npm ci` once and `npm run css:build`; rebuild the Rust binary because `static/app.css` is embedded at compile time. `npm run css:watch` keeps CSS current while editing.

## Security and operations

- Passwords are Argon2-hashed on Tokio's blocking pool.
- Session cookies are HttpOnly and SameSite=Lax; production requires Secure cookies.
- State-changing requests with browser Origin/Referer data are checked against the request/configured host. Cross-site Fetch Metadata is rejected.
- Private dynamic responses use `Cache-Control: private, no-store`; static assets have explicit caching.
- Production auth throttling uses atomic fixed-window counters in libSQL, keyed by a hash of Vercel's client IP header. No Redis or email provider is required. Local development skips IP throttling.
- Vercel startup requires an explicit database URL, secure session cookies, and a non-empty `CRON_SECRET` of at least 16 characters. The daily cron removes expired session and throttle rows.
- Errors sent to visitors are generic; the cause is logged server-side.

## Known product limits

The app is intentionally small: no email delivery, verification, password reset, task editing/ordering, offline task storage, export, or account deletion. The offline PWA screen contains no account data. Public copy and setup instructions should not promise those capabilities.
