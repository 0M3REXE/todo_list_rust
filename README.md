# practice

A small todo app in Rust: Axum, cookie sessions in Postgres, Askama templates, email verification and password reset.

`cargo run` serves it; set `SESSION_SECURE=false` and `APP_ALLOWED_HOST=localhost:3000` to work over plain http locally.

The server-rendered templates use Askama, HTMX, and Tailwind CSS. Tailwind is compiled at build time; the generated `static/app.css` is embedded in the Rust binary and should be regenerated and committed with template or style changes:

```sh
npm ci
npm run css:build
cargo run
```

Use `npm run css:watch` while editing templates and styles. Vercel's Rust builder embeds the committed `static/app.css` directly.

Tests build the same router as the binary through `app::build_app_with`, which swaps in a mailer that captures messages instead of sending them.
