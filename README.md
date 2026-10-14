# practice

A small todo app in Rust: Axum, cookie sessions in Postgres, Askama templates, email verification and password reset.

`cargo run` serves it; set `SESSION_SECURE=false` and `APP_ALLOWED_HOST=localhost:3000` to work over plain http locally.

Tests build the same router as the binary through `app::build_app_with`, which swaps in a mailer that captures messages instead of sending them.