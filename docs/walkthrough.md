# Line-by-line walkthrough

A companion to [`architecture.md`](./architecture.md). That document teaches
**why** — this one walks you through **what**, file by file, line by line, in
reading order.

Assume you know nothing about Rust, SQL, or web servers. Each section opens the
actual file, and every line that matters is explained in the order it appears.

---

## Reading order

Read in this order. Each file builds on the one before it.

```
 1. Cargo.toml          - what this project is, and what it depends on
 2. .env                - the one secret: the database URL
 3. migrations/0001.    - the todos table
 4. migrations/0002-3   - users, sessions, and giving todos an owner
 5. src/lib.rs          - the library root
 6. src/error.rs        - the single error type
 7. src/auth.rs         - the User type and the CurrentUser extractor
 8. src/auth/password.rs - Argon2id, and why it runs off-thread
 9. src/auth/store.rs   - user SQL + the hand-written session store
10. src/auth/backend.rs - the two traits axum-login requires
11. src/auth/middleware.rs - the login guard and the CSRF check
12. src/auth/handlers.rs - signup, login, logout
13. src/todo.rs         - the domain module: Todo struct + re-exports
14. src/todo/store.rs   - every SQL query
15. src/todo/templates.rs - the three Askama structs
16. src/todo/handlers.rs - the four HTTP handlers + guarded router
17. src/app.rs          - the one router, shared with the tests
18. src/main.rs         - wiring: pool, middleware, serve
19. templates/*.html    - the five HTML templates
20. static/app.css      - the theme layer over Bulma
21. tests/common/mod.rs - shared helpers + the cookie jar
22. tests/todo_model.rs - 14 data-layer tests
23. tests/todo_http.rs  - 29 HTTP-layer tests
24. tests/auth_model.rs - 10 auth data-layer tests
25. tests/auth_http.rs  - 26 auth HTTP-layer tests
```

**Part 2 (sections 16 and below) covers authentication, which arrived after
the todo app was finished. Read sections 1-15 first** — that is the original
app, unchanged apart from every store function having gained a `user_id`.

---

## 1. Cargo.toml — the project manifest

📄 `Cargo.toml`

This is Rust's equivalent of `package.json`. It tells the Rust compiler what
your project is called, what version of Rust to use, and what libraries
("crates") to download.

```toml
[package]
name = "practice"
version = "0.1.0"
edition = "2024"
```

- **`name`** — the name of your project. When you write `use practice::todo` in
  a test file, this is the name it looks up.
- **`version`** — a version number. Not deployed anywhere yet, so `0.1.0` is
  fine.
- **`edition`** — which Rust "edition" to use. Editions are backwards-compatible
  upgrades to Rust's syntax. `2024` is the latest.

```toml
[dependencies]
askama = "0.16.0"
axum = "0.8.9"
dotenvy = "0.15"
serde = { version = "1.0.228", features = ["derive"] }
libsql = { version = "0.10.0", default-features = false, features = ["core", "remote"] }
tokio = { version = "1.52.4", features = ["full"] }
tower = { version = "0.5", features = ["util"] }
tower-http = { version = "0.7.0", features = ["trace", "compression-gzip", "set-header", "timeout", "catch-panic"] }
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
vercel_runtime = { version = "2.4.1", features = ["axum"] }
```

Each line is a library your code uses:

| Crate | What it does |
|---|---|
| `askama` | HTML templates compiled into Rust at build time |
| `axum` | The web framework — handles routing and HTTP |
| `dotenvy` | Reads `.env` files into environment variables |
| `serde` | Converts form data → Rust structs. `features = ["derive"]` lets you write `#[derive(Deserialize)]` |
| `libsql` | Talks to Turso over HTTP (cloud) or a local SQLite file (dev). |
| `tokio` | The async runtime — the engine that runs all the `async` functions. `features = ["full"]` enables everything |
| `vercel_runtime` | Runs the same axum router as a Vercel serverless function. |
| `tower` | The middleware system that axum builds on |
| `tower-http` | Ready-made middleware: logging, gzip compression, security headers, timeouts, panic recovery, serving static files |
| `tracing` | Structured logging (like `console.log` but with structured fields) |
| `tracing-subscriber` | Configures where log output goes. `env-filter` lets `RUST_LOG=debug` control verbosity |

The `features = [...]` syntax is how Rust libraries ship optional functionality.
You only pay for what you enable.

```toml
[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
```

`[dev-dependencies]` are libraries used **only during testing**, not in the
compiled binary. `tower`'s `util` feature provides `.oneshot()`, which is how
tests drive one request through the entire router without starting a server.

---

## 2. .env — the database connection

📄 `.env`

```
DATABASE_URL=libsql://<your-db>.turso.io
DATABASE_AUTH_TOKEN=<your-token>
```

Two lines. The first tells libsql which Turso database to use; the second is
the auth token (`turso db tokens create`). Leave both unset and the app falls
back to `file:practice.db` — a local SQLite file for development without a
network.

This file is **gitignored** (check `.gitignore`). It contains a secret. On a
real project, each developer creates their own, and production uses a real
secret.

`dotenvy::dotenv()` in `main.rs` reads this file and makes `DATABASE_URL`
and `DATABASE_AUTH_TOKEN` available via `std::env::var(...)`.

---

## 3. The migration — the database table

📄 `migrations/0001_create_todos.sql`

```sql
create table todos (
    id   bigint generated always as identity primary key,
    text text not null,
    done boolean not null default false,

    constraint todos_text_length check (char_length(btrim(text)) between 1 and 280)
);

create index todos_done_id_idx on todos (done, id);
```

Line by line:

- **`id bigint generated always as identity primary key`** — an auto-incrementing
  number. Postgres hands them out from a sequence: 1, 2, 3, … Deleting id 2
  doesn't give that number to the next insert. `bigint` means 64-bit integer
  (matches Rust's `i64`). `primary key` means each id is unique and fast to look up.

- **`text text not null`** — the todo's text. `not null` means it can't be empty
  (SQL `NULL`, not empty string — that's what the `CHECK` handles).

- **`done boolean not null default false`** — whether it's ticked. New todos
  start as `false`.

- **`constraint todos_text_length check (...)`** — a database-level guardrail.
  `btrim(text)` trims whitespace, then `char_length` counts characters. The text
  must be between 1 and 280 characters. This is **defence in depth** — the Rust
  code also checks this, but if someone ever bypasses the Rust check (e.g. a
  future admin script), the database still rejects garbage.

- **`create index todos_done_id_idx on todos (done, id)`** — the app always
  fetches with `ORDER BY done ASC, id ASC`. Without this index, Postgres reads
  every row and sorts them. With it, the rows are already stored in that order,
  so Postgres just walks the index sequentially.

---

## 4. src/lib.rs — the library root

📄 `src/lib.rs`

```rust
//! # The library root
//!
//! Everything reusable lives here. `main.rs` runs it; `tests/*.rs` import it.
//! Integration tests are separate crates and cannot import a binary — without
//! this file the tests would have to live inside `src/`.
//!
//! Full walkthrough: `docs/architecture.md`.

pub mod todo;
```

That's the entire file. Here's what each piece means:

- **`//!`** — a "doc comment" that documents the *module itself* (not a function
  or struct). The `!` makes it about the container. You'll see these at the top
  of every file.

- **`pub mod todo;`** — two things happening:
  - `mod todo` tells Rust "there is a module called `todo`". Rust looks for
    either `src/todo.rs` or `src/todo/mod.rs`. It finds `src/todo.rs`.
  - `pub` makes it **public** — code outside this library (tests, `main.rs`) can
    use `practice::todo`.

### Why does lib.rs exist?

Rust has a rule: integration tests (files in `tests/`) can only import from a
**library** (`lib.rs`), never from a **binary** (`main.rs`). Without `lib.rs`,
all your code would live in `main.rs` and the test files couldn't reach any of
it. So the real logic goes in the library, and `main.rs` just wires it up and
starts the server.

---

## 5. src/todo.rs — the domain module

📄 `src/todo.rs`

```rust
mod error;
mod handlers;
mod store;
mod templates;
```

Four private sub-modules. `mod` without `pub` means **only code inside
`todo.rs`** (and its children) can see them. Outsiders can't directly reach
`error.rs` or `store.rs` — they go through the re-exports below.

```rust
pub use error::AppError;
pub use handlers::build_router;
pub use store::{MAX_TEXT_LEN, add, flip_done, open_count, remove, sorted};
```

`pub use` takes something from a private module and re-exports it under the
current module's name. So instead of writing
`practice::todo::handlers::build_router`, callers write
`practice::todo::build_router`. This is the module's **public API** — a
deliberate choice of what to expose.

```rust
use sqlx::PgPool;
```

`use` brings a name into scope so you can write `PgPool` instead of
`sqlx::PgPool`. This import is private (no `pub`) — just for use within this
file.

```rust
pub type TodoStore = PgPool;
```

A **type alias**. `TodoStore` and `PgPool` are the exact same type — the alias
exists for readability. Handler signatures say `State<TodoStore>` instead of
`State<PgPool>`, which makes it clear that the pool *is* the store. If the
storage ever changed (e.g. to a trait), this is the one place to update.

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Todo {
    pub id: i64,
    pub text: String,
    pub done: bool,
}
```

The `Todo` struct — a row from the `todos` table, mapped into Rust.

- **`#[derive(...)]`** — auto-generates code. Like adding `toString()` to a Java
  class, but at compile time:
  - `Debug` — lets you print a `Todo` with `{:?}` (like `console.log` for
    debugging)
  - `Clone` — lets you copy a `Todo` (needed because Rust tracks ownership)
  - `sqlx::FromRow` — lets sqlx automatically convert a database row into this
    struct. The field names (`id`, `text`, `done`) must match the column names
    in the SQL query.

- **`pub`** on the struct and each field means they're visible from outside the
  module.

- **`i64`** — a 64-bit integer. Matches Postgres's `bigint`.
- **`String`** — a heap-allocated text string. Matches Postgres's `text`.
- **`bool`** — true or false. Matches Postgres's `boolean`.

---

## 6. src/todo/error.rs — the single error type

📄 `src/todo/error.rs`

This file defines the **one error type for the entire app**. Every function that
can fail returns `Result<_, AppError>`.

```rust
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
```

Imports from axum. `StatusCode` is things like `200 OK` or `500 Internal Server
Error`. `IntoResponse` is the trait that turns things into HTTP responses.
`Response` is the actual response object.

```rust
#[derive(Debug)]
pub enum AppError {
    Render(askama::Error),
    Database(sqlx::Error),
}
```

An **enum** — Rust's version of a "one of these things". An `AppError` is either:
- `Render(askama::Error)` — a template failed to render, carrying the original
  askama error inside
- `Database(sqlx::Error)` — a SQL query failed, carrying the original sqlx error
  inside

The parentheses hold **data**. This isn't like a Java enum that's just a label —
each variant carries the original error so you can inspect it later.

```rust
impl AppError {
    fn into_response_inner(self) -> Response {
        match &self {
            Self::Render(err) => tracing::error!(?err, "template render failed"),
            Self::Database(err) => tracing::error!(?err, "database query failed"),
        }
        (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong").into_response()
    }
}
```

`impl AppError` means "I'm adding methods to `AppError`". This one:

1. **`match &self`** — pattern matching. Like a `switch` statement, but the
   compiler forces you to handle every variant. `&self` borrows `self` so it's
   not consumed yet.
2. Logs the real error to the server console with `tracing::error!`. The `?err`
   is a formatting directive that uses Rust's `Debug` trait to print the error.
3. Returns a `500 Internal Server Error` with the body `"something went wrong"`.
   **Never the real error** — that stays in the logs. This matters for security:
   you don't want to leak table names or SQL to the browser.

```rust
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(err) => write!(f, "template render failed: {err}"),
            Self::Database(err) => write!(f, "database query failed: {err}"),
        }
    }
}
```

Implements the `Display` trait — how the error is formatted when you print it
with `{}` (as opposed to `{:?}` which uses `Debug`). `write!` writes into the
formatter `f`. `{err}` inside the format string automatically calls `Display` on
the inner error.

```rust
impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Render(err) => Some(err),
            Self::Database(err) => Some(err),
        }
    }
}
```

Makes `AppError` a proper Rust error. `source()` returns the **inner error** so
error reporters can walk the chain ("this failed because *that* failed"). `dyn
std::error::Error` means "anything implementing the Error trait" — a trait
object, like an interface reference in Java.

```rust
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        self.into_response_inner()
    }
}
```

**This is the keystone.** axum says: "whatever a handler returns, I'll call
`into_response()` on it." By implementing `IntoResponse`, you can return
`Err(AppError::Database(...))` from a handler and axum automatically turns it
into a 500 response. No `match` in every handler. One implementation, used
everywhere.

```rust
impl From<askama::Error> for AppError {
    fn from(err: askama::Error) -> Self {
        Self::Render(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}
```

**The `From` impls are what make `?` work.** When you write:

```rust
let todos = sorted(&pool).await?;
```

That `?` means "if this is an `Err`, convert it and return it." The conversion
uses `From`. `sorted()` returns `Result<_, sqlx::Error>`. The `?` sees the
`From<sqlx::Error> for AppError` impl and automatically wraps it in
`AppError::Database(...)`. Without these two impls, you'd need to manually
convert every error.

---

## 7. src/todo/store.rs — every SQL query

📄 `src/todo/store.rs`

Every database interaction in the entire app. Five functions, one query each.

```rust
use sqlx::PgPool;
use super::{AppError, Todo};
```

`super::` means "the parent module" — `todo.rs`. It grabs `AppError` and `Todo`
from there.

```rust
pub const MAX_TEXT_LEN: usize = 280;
```

A **constant**. `const` means this value is baked in at compile time — it
doesn't live in memory at runtime, it's copy-pasted wherever it's used. `usize`
is Rust's "size" type — an unsigned integer matching the platform's pointer size
(64-bit on modern machines). This constant is used both in `validated()` and in
tests to ensure the template's `maxlength` attribute matches.

```rust
fn validated(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_TEXT_LEN {
        return None;
    }
    Some(text)
}
```

A **private helper** (no `pub`). Takes a `&str` (a borrowed string slice — the
caller keeps ownership, this function just reads it). Returns `Option<&str>`:
either `Some("trimmed text")` or `None`.

- `text.trim()` removes whitespace from both ends. The result **shadows** the
  parameter — same name, new value. This is idiomatic Rust.
- If blank or too long, return `None` immediately. The caller will see `None`
  and skip the insert.
- Otherwise, return `Some(text)` — the trimmed version.

This mirrors the database `CHECK` constraint. The Rust check must happen first,
because a CHECK violation produces a database error → a 500 response. But a
blank todo should be a silent no-op, not a server error.

```rust
pub async fn sorted(pool: &PgPool) -> Result<Vec<Todo>, AppError> {
    let todos =
        sqlx::query_as::<_, Todo>("SELECT id, text, done FROM todos ORDER BY done ASC, id ASC")
            .fetch_all(pool)
            .await?;
    Ok(todos)
}
```

Fetch all todos, sorted: active first, done last, insertion order within each
group.

- **`pub async fn`** — public, asynchronous function. `async` means it can
  `await` things (like database calls) without blocking the thread.
- **`pool: &PgPool`** — borrows the connection pool. `&` means "I'm reading it,
  not taking it."
- **`-> Result<Vec<Todo>, AppError>`** — returns either a `Vec<Todo>` (a
  growable list of `Todo` structs) or an `AppError`.
- **`sqlx::query_as::<_, Todo>(...)`** — run this SQL and map each row into a
  `Todo` struct. The `::<_, Todo>` is the "turbofish" syntax — it tells the
  compiler which type to use. `_` lets the compiler infer the first type
  parameter (the database type).
- **`.fetch_all(pool)`** — run the query and collect all rows.
- **`.await?`** — wait for the database to respond. `?` handles errors: if the
  query fails, the error is converted to `AppError` (via the `From` impl) and
  returned immediately.
- **`Ok(todos)`** — wrap the result in `Ok` to signal success.

```rust
pub async fn open_count(pool: &PgPool) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM todos WHERE NOT done")
        .fetch_one(pool)
        .await?;
    Ok(count)
}
```

Counts undone todos. `query_scalar` returns a single value (not a row).
`fetch_one` expects exactly one row — `count(*)` always returns one.

```rust
pub async fn add(pool: &PgPool, text: &str) -> Result<bool, AppError> {
    let Some(text) = validated(text) else {
        return Ok(false);
    };
    sqlx::query("INSERT INTO todos (text) VALUES ($1)")
        .bind(text)
        .execute(pool)
        .await?;
    Ok(true)
}
```

Add a new todo:

- **`let Some(text) = validated(text) else { return Ok(false); }`** — a
  "let-else" pattern. If `validated()` returns `None` (blank or too long), skip
  the insert and return `false`. If it returns `Some(trimmed_text)`, bind that
  to `text` and continue.
- **`$1`** — a query parameter placeholder. The actual value comes from
  `.bind(text)`. This is **parameterized SQL** — the reason SQL injection is
  impossible. The text `'; DROP TABLE todos; --` is sent as a literal value,
  never as SQL syntax.
- **`.execute(pool)`** — run the query without expecting rows back.
- Returns `true` on success.

```rust
pub async fn flip_done(pool: &PgPool, id: i64) -> Result<bool, AppError> {
    let result = sqlx::query("UPDATE todos SET done = NOT done WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
```

Toggle done/undone:

- **`SET done = NOT done`** — flips the boolean in one statement. No need to read
  the current value first.
- **`rows_affected()`** — the database tells you how many rows were actually
  updated. If the id doesn't exist, this returns 0 → the function returns
  `false` → the handler returns 404.

```rust
pub async fn remove(pool: &PgPool, id: i64) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM todos WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
```

Delete a todo. Same pattern as `flip_done` — returns `false` if no row existed.

---

## 8. src/todo/templates.rs — the three template structs

📄 `src/todo/templates.rs`

```rust
use askama::Template;
use super::Todo;
```

Imports Askama's `Template` trait and the `Todo` struct.

```rust
#[derive(Template)]
#[template(path = "index.html")]
pub(crate) struct IndexTemplate {
    pub(crate) todos: Vec<Todo>,
    pub(crate) open_count: i64,
}
```

This struct **is** the template. Here's how Askama works:

1. `#[derive(Template)]` tells Askama to generate a `.render()` method.
2. `#[template(path = "index.html")]` tells it which HTML file to use.
3. Every field of the struct becomes a variable in the HTML template. So
   `{{ open_count }}` in `index.html` reads `self.open_count`.

**`pub(crate)`** means "visible within this crate (this project) but not to
outside code." The templates are implementation details — tests and `main.rs`
don't create templates directly, handlers do.

There are three templates because different handlers return different amounts of
HTML:

| Struct | Template | When used | What it returns |
|---|---|---|---|
| `IndexTemplate` | `index.html` | `GET /` | The entire page |
| `PanelTemplate` | `todo_panel.html` | `POST /todos` | The panel (form + list + count) |
| `ListTemplate` | `todo_list.html` | toggle, delete | Just the `<ul>` of todos |

The key insight: `POST /todos` returns the **panel** (which includes the form),
while toggle/delete return just the **list**. If toggle returned the panel, it
would wipe out whatever the user was typing in the input box.

---

## 9. src/todo/handlers.rs — the HTTP handlers

📄 `src/todo/handlers.rs`

This is where HTTP requests become HTML responses.

```rust
use askama::Template as _;
```

Imports the `Template` trait but renames it to `_`, meaning "I need this trait in
scope so I can call `.render()`, but I don't need to refer to the trait by
name." Without this import, `.render()` would not compile — Rust requires traits
to be in scope to call their methods.

```rust
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;
```

axum building blocks:
- `Router` — the route table
- `Path` — extracts values from the URL (e.g. `/todos/3/toggle` → `3`)
- `State` — extracts shared application state (the database pool)
- `Html` — wraps a string as an HTML response with the right content-type
- `get` — creates a GET route

```rust
use super::{
    AppError, TodoStore,
    store::{add, flip_done, open_count, remove, sorted},
    templates::{IndexTemplate, ListTemplate, PanelTemplate},
};
```

Imports from sibling modules via the parent (`super`).

### Handler 1: `GET /` — show the page

```rust
async fn index(State(pool): State<TodoStore>) -> Result<Html<String>, AppError> {
    let (todos, open_count) = (sorted(&pool).await?, open_count(&pool).await?);
    Ok(Html(IndexTemplate { todos, open_count }.render()?))
}
```

Let's unpack the signature:

- **`State(pool): State<TodoStore>`** — this is an axum **extractor**. When a
  request comes in, axum sees this parameter and automatically pulls the
  `TodoStore` (= `PgPool`) from the app's shared state. `State(pool)` is
  destructuring — it extracts the inner `pool` from the `State` wrapper.
- **`-> Result<Html<String>, AppError>`** — returns either an HTML response or
  an error.

The body:
1. Fetch todos and the open count. Both can fail, so both use `?`.
2. Create an `IndexTemplate` struct, call `.render()` on it (which runs the
   compiled template and produces a `String` of HTML), wrap it in `Html`, wrap
   that in `Ok`.

### Handler 2: `POST /todos` — create a todo

```rust
#[derive(serde::Deserialize)]
struct NewTodo {
    text: String,
}
```

A tiny struct to receive form data. `#[derive(serde::Deserialize)]` auto-
generates code to parse URL-encoded form data (`text=buy+milk`) into this
struct. The field name `text` must match the form field name.

```rust
async fn create(
    State(pool): State<TodoStore>,
    axum::extract::Form(form): axum::extract::Form<NewTodo>,
) -> Result<Html<String>, AppError> {
    add(&pool, &form.text).await?;
    let (todos, open_count) = (sorted(&pool).await?, open_count(&pool).await?);
    Ok(Html(PanelTemplate { todos, open_count }.render()?))
}
```

- **`Form(form)`** — a second extractor. axum reads the request body as
  URL-encoded form data and deserializes it into `NewTodo`. If the body can't be
  parsed, axum returns 422 automatically.
- Calls `add()` — if the text is blank, `add` returns `false` and nothing is
  inserted. The handler doesn't care about the return value here.
- Returns a `PanelTemplate` — the **entire panel** including the empty input
  box, which clears whatever the user typed.

### The shared mutation helper

```rust
async fn with_list<F>(pool: &PgPool, mutate: F) -> Result<Response, AppError>
where
    F: std::future::Future<Output = Result<bool, AppError>>,
{
    let found = mutate.await?;
    let body = ListTemplate {
        todos: sorted(pool).await?,
    }
    .render()?;
    let status = if found {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    Ok((status, Html(body)).into_response())
}
```

This is a **generic function** — the most complex piece of Rust in the codebase.

- **`<F>`** — a generic type parameter. `F` is "some type we don't know yet."
- **`where F: std::future::Future<Output = Result<bool, AppError>>`** — a
  constraint. `F` must be a future that eventually produces a `Result<bool,
  AppError>`. In practice, `F` is the return value of `flip_done()` or
  `remove()` before you `.await` it.
- **`mutate.await?`** — runs the mutation and checks for errors.
- Returns the list fragment with either 200 (the id existed) or 404 (it didn't).

### Handlers 3 & 4: toggle and delete

```rust
async fn toggle(State(pool): State<TodoStore>, Path(id): Path<i64>) -> Result<Response, AppError> {
    with_list(&pool, flip_done(&pool, id)).await
}

async fn delete(State(pool): State<TodoStore>, Path(id): Path<i64>) -> Result<Response, AppError> {
    with_list(&pool, remove(&pool, id)).await
}
```

- **`Path(id): Path<i64>`** — extracts the `{id}` from the URL and parses it as
  an `i64`. If someone sends `/todos/abc/toggle`, the parse fails and axum
  returns 400 automatically. This is the injection defence for URL parameters.
- Both delegate to `with_list`, passing their specific mutation function.

### The router

```rust
pub fn build_router(pool: TodoStore) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/todos", axum::routing::post(create))
        .route("/todos/{id}/toggle", axum::routing::post(toggle))
        .route("/todos/{id}/delete", axum::routing::post(delete))
        .with_state(pool)
}
```

Builds the route table:

| Method | Path | Handler |
|---|---|---|
| GET | `/` | `index` |
| POST | `/todos` | `create` |
| POST | `/todos/{id}/toggle` | `toggle` |
| POST | `/todos/{id}/delete` | `delete` |

**`.with_state(pool)`** — injects the pool into the router so every handler can
extract it with `State(pool)`. The pool is cloned for each request (but `PgPool`
is reference-counted, so "clone" just bumps a counter — it's cheap).

---

## 10. src/main.rs — wiring it all together

📄 `src/main.rs`

This is the executable. It reads config, connects to the database, builds the
router with middleware, and starts the server.

```rust
use practice::todo;
```

Imports the library. `practice` is the crate name from `Cargo.toml`.

```rust
#[tokio::main]
async fn main() {
```

`#[tokio::main]` is a macro that transforms `async fn main()` into a regular
`fn main()` that starts the tokio runtime. Without it, `async` functions can't
run — they need a runtime to poll them.

```rust
    dotenvy::dotenv().ok();
```

Reads `.env` into environment variables. `.ok()` swallows any error (the file
might not exist in production). The `.ok()` converts `Result` to `Option` and
discards it.

```rust
    let addr = std::env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
```

Read the listen address from the environment, defaulting to `127.0.0.1:3000`.
`unwrap_or_else` provides a fallback value if the variable isn't set. `.to_owned()`
converts a `&str` (borrowed) to a `String` (owned).

```rust
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
```

Sets up logging:
- **`EnvFilter`** reads `RUST_LOG` to control verbosity. Default is `info`.
- **`fmt::layer()`** formats log entries as human-readable text to stdout.
- **`.init()`** makes this the global logger.

```rust
    let pool = connect_pool().await;
```

Calls the helper function below to create a database pool and run migrations.

### Static files with caching

```rust
    let static_files = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(ServeDir::new("static"));
```

Serves files from the `static/` directory with an aggressive cache header.
`max-age=31536000` = 1 year. `immutable` tells the browser "don't even
revalidate." This is safe because CSS and JS files at fixed URLs won't change
without a deploy.

**This header is scoped to static files only** — if you put it on the whole app,
HTML pages would be cached too, and users would see stale content.

### Building the app with middleware

```rust
    let app = todo::build_router(pool)
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/static", any_service(static_files))
```

- Starts with the todo router
- Adds a health check endpoint at `/healthz` (a closure as a handler)
- Mounts the static file server at `/static`

```rust
        .layer(CompressionLayer::new())
```

Gzip compresses responses. The architecture doc says this shrinks 678KB → 64KB.

```rust
        .layer(SetResponseHeaderLayer::overriding(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; \
                 script-src 'self'; \
                 style-src 'self' https://fonts.googleapis.com; \
                 font-src 'self' https://fonts.gstatic.com; \
                 img-src 'self' data:; \
                 frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
            ),
        ))
```

The **Content Security Policy** header. This is a second line of XSS defence:
even if somehow a `<script>` tag got into the page, the browser would refuse to
run it because the policy says only scripts from `'self'` (this server) are
allowed.

```rust
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
```

More security headers:
- `nosniff` stops the browser from guessing content types
- `no-referrer` stops the browser from sending the URL to third parties

```rust
        .layer(TraceLayer::new_for_http())
```

Logs every request with method, path, status code, and duration.

```rust
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ))
```

If a request takes more than 15 seconds, abort it and return 408 Request
Timeout.

```rust
        .layer(CatchPanicLayer::new());
```

If a handler panics (crashes), catch the panic and return a clean 500 instead of
dropping the connection. Without this, a panicking handler would just silently
kill the request.

> **Middleware order matters.** The last `.layer()` is outermost. So the request
> flows: CatchPanic → Timeout → Trace → Headers → Compression → Handler →
> back out. CatchPanic is innermost (closest to the handler) because it needs to
> catch a handler panic first.

### Starting the server

```rust
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(%err, %addr, "failed to bind");
            std::process::exit(1);
        }
    };
```

Bind to the address. If it fails (port in use, permission denied), log the real
error and exit. The `match` is like a `try/catch` — `Ok` is success, `Err` is
failure.

```rust
    if let Err(err) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!(?err, "server error");
        std::process::exit(1);
    }
```

Start serving. `with_graceful_shutdown` means when you press Ctrl+C, the server
finishes in-flight requests before stopping instead of killing them.

### connect_pool — database setup

```rust
async fn connect_pool() -> sqlx::PgPool {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            tracing::error!("DATABASE_URL is not set");
            std::process::exit(1);
        }
    };

    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(?err, "could not connect to the database");
            std::process::exit(1);
        }
    };

    if let Err(err) = sqlx::migrate!().run(&pool).await {
        tracing::error!(?err, "migrations failed");
        std::process::exit(1);
    }

    tracing::info!("database ready");
    pool
}
```

Step by step:
1. Read `DATABASE_URL` from the environment. Fatal if missing.
2. Create a connection pool with max 5 connections and a 5-second timeout.
3. **`sqlx::migrate!()`** — this macro embeds the `migrations/` folder into the
   binary at compile time. `.run(&pool)` applies any unapplied migrations. This
   means deploying is just "run the binary" — no separate migration step.
4. Return the pool.

### shutdown_signal — graceful exit

```rust
#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(?err, "failed to install ctrl-c handler");
        std::future::pending::<()>().await;
    }
    tracing::info!("shutdown signal received, draining");
}
```

- **`#[cfg(not(unix))]`** — conditional compilation. This version compiles on
  Windows. The `#[cfg(unix)]` version below compiles on Linux/Mac and also
  handles `SIGTERM` (what Docker sends to stop a container).
- `std::future::pending::<()>().await` is a future that never resolves — used as
  a "hang forever" fallback if the handler install fails.

---

## 11. templates/*.html — the HTML

### index.html — the full page

📄 `templates/index.html`

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover">
    <title>Todos</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link rel="stylesheet"
        href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&display=swap">
    <link rel="stylesheet" href="/static/vendor/bulma.min.css">
    <link rel="stylesheet" href="/static/app.css">
    <script src="/static/vendor/htmx.min.js" defer></script>
</head>
```

Standard HTML head. Loads the Geist font from Google Fonts, then Bulma CSS, then
the app's own CSS, then htmx. `defer` means htmx loads in the background and
runs after the page is parsed.

```html
<body class="has-gradient-page">
    <main class="section">
        <div class="container is-max-desktop">
            <header class="page__head has-gradient-sheen">
                <h1 class="title is-3 page__title">Todos</h1>
                <p class="subtitle is-6 page__sub">…</p>
            </header>
{% include "todo_panel.html" %}
        </div>
    </main>
</body>
</html>
```

- `has-gradient-page`, `has-gradient-sheen` — custom CSS classes from `app.css`
- `is-3`, `is-6`, `section`, `container`, `is-max-desktop` — Bulma classes
- **`{% include "todo_panel.html" %}`** — Askama includes the panel template
  inline. At compile time, the panel's HTML is inserted here. This is how
  templates compose.

### todo_panel.html — the form + list + count

📄 `templates/todo_panel.html`

```html
<div id="todo-panel" class="todo-panel box">
    <form class="todo-add field has-addons"
          hx-post="/todos"
          hx-target="#todo-panel"
          hx-swap="outerHTML">
```

The **add form**. The htmx attributes:

- `hx-post="/todos"` — when the form submits, POST to `/todos` via JavaScript
  (no page reload)
- `hx-target="#todo-panel"` — put the response here (replace the entire panel)
- `hx-swap="outerHTML"` — replace the target element itself, not just its
  contents

The form targets `#todo-panel` because the response from `POST /todos` is a
full panel (form + list + count). This clears the input box.

```html
        <input
            id="todo-text"
            class="todo-input input"
            type="text"
            name="text"
            placeholder="What needs doing?"
            maxlength="280"
            autocomplete="off"
            required
        >
```

- `name="text"` — this is the field name that serde deserializes into
  `NewTodo.text`
- `maxlength="280"` — matches `MAX_TEXT_LEN` in Rust. A test verifies this.
- `required` — the browser won't submit an empty form

```html
        <button type="submit" class="todo-submit button has-gradient-accent has-gradient-sheen"
                hx-disabled-elt="this">
            Add
        </button>
```

- `hx-disabled-elt="this"` — htmx disables the button while the request is
  in-flight, preventing double-submits.

```html
{% include "todo_list.html" %}
    <p class="todo-count has-text-grey is-size-7" aria-live="polite">{{ open_count }} open</p>
</div>
```

- Includes the list template
- **`{{ open_count }}`** — Askama expression. Inserts the value of `open_count`
  from the struct, escaped for HTML safety.
- `aria-live="polite"` — screen readers announce changes to this element

### todo_list.html — the list of todos

📄 `templates/todo_list.html`

```html
<ul id="todo-list" class="todo-list">
{% for todo in todos %}
```

**`{% for todo in todos %}`** — Askama loop. Iterates over the `todos` Vec from
the template struct.

```html
    <li class="todo-row{% if todo.done %} todo-row--done{% endif %}">
```

Conditionally adds the `todo-row--done` class (which adds strikethrough styling)
when the todo is done.

```html
        <label class="checkbox">
            <input
                class="todo-toggle"
                type="checkbox"
                {% if todo.done %}checked{% endif %}
                hx-post="/todos/{{ todo.id }}/toggle"
                hx-target="#todo-list"
                hx-swap="outerHTML"
            >
```

The checkbox:

- `{% if todo.done %}checked{% endif %}` — conditionally adds the `checked`
  attribute
- `hx-post="/todos/{{ todo.id }}/toggle"` — when clicked, POST to the toggle
  endpoint for this specific todo. `{{ todo.id }}` inserts the id, HTML-escaped.
- **`hx-target="#todo-list"`** — targets the list, NOT the panel. This is the
  critical design decision: toggling replaces only the list, preserving whatever
  the user was typing in the add form.

```html
        <span class="todo-text">{{ todo.text }}</span>
```

**`{{ todo.text }}`** — Askama escapes this automatically. `<script>` becomes
`&#60;script&#62;`. This single fact is the app's XSS defence.

```html
        <button
            class="todo-delete delete"
            hx-post="/todos/{{ todo.id }}/delete"
            hx-target="#todo-list"
            hx-swap="outerHTML"
            hx-confirm="Delete this todo?"
            aria-label="Delete {{ todo.text }}"
        ></button>
```

The delete button:
- `hx-confirm` pops a browser confirm dialog before sending the request
- `aria-label` provides an accessible name for screen readers. It includes the
  todo text so a blind user knows *which* todo the button deletes. The text is
  escaped here too — a test verifies quotes can't break out of the attribute.

```html
{% else %}
    <li class="todo-empty …">Nothing on the list. Type above and press Enter.</li>
{% endfor %}
```

**`{% else %}`** in a `for` loop is Askama's empty-state — it runs when the
collection is empty.

---

## 12. static/app.css — the theme

📄 `static/app.css`

Not line-by-line (it's 234 lines of CSS), but the three things it does:

1. **Re-hues Bulma** — overrides `--bulma-primary-h/s/l` to change the accent
   from turquoise to violet. Because Bulma derives all its colours from HSL
   variables, changing three values recolours every button, link, and focus ring.

2. **Adds gradient utilities** — `.has-gradient-accent`, `.has-gradient-sheen`,
   `.has-gradient-page`. Bulma has no gradient primitive.

3. **Styles the todo list** — `.todo-row`, `.todo-text`, `.todo-delete`, etc.
   Bulma has no component for a todo list, so these are custom.

Notable details:
- Mobile touch targets: on small screens, the checkbox label grows to fill the
  row (so tapping anywhere toggles) and the delete button grows to 2.5rem
  (above the 44px touch target minimum).
- `@media (prefers-reduced-motion: reduce)` disables transitions for users who
  request it.

---

## 13. tests/common/mod.rs — shared test helpers

📄 `tests/common/mod.rs`

```rust
#![allow(dead_code)]
```

Suppresses "unused function" warnings. Some helpers are used only in one test
file, and the other test binary would warn about the unused ones. This silences
that.

```rust
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use practice::todo::{add, build_router};
use sqlx::PgPool;
use tower::ServiceExt;
```

`ServiceExt` brings `.oneshot()` into scope — the method that drives a single
request through the router without starting a server.

### Key helpers

```rust
pub async fn seed(pool: &PgPool, texts: &[&str]) {
    for text in texts {
        add(pool, text).await.expect("seed insert");
    }
}
```

Seeds the test database with todos. `&[&str]` is a slice of string slices — like
an array of strings. `.expect("seed insert")` unwraps the `Result` and panics
with that message if it's an `Err`.

```rust
pub async fn get_to(pool: &PgPool, uri: &str) -> (StatusCode, String) {
    let app = build_router(pool.clone());
    let res = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let text = String::from_utf8(
        to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec(),
    ).unwrap();
    (status, text)
}
```

Sends a GET request through the router:
1. Build the router with the pool
2. **`.oneshot(request)`** — sends exactly one request through the entire
   `Router` — real routing, real extractors, real middleware — but no socket, no
   port, no background task. This is why tests are fast and parallel-safe.
3. Read the response body into a string
4. Return the status code and body text as a tuple

`post_to` and `post_todo` do the same for POST requests, with `post_todo` also
setting the `content-type` header for form data.

---

## 14. tests/todo_model.rs — data-layer tests

📄 `tests/todo_model.rs`

14 tests that call the store functions directly (no HTTP).

Tests use an in-memory libSQL database — `src/db.rs::open_in_memory()` — so
there's no external database to run. Each test gets its own private database,
migrations are applied to it, and it's dropped when the test finishes. That's
why tests run in parallel without interfering, and why `cargo test` works with
nothing but a Rust toolchain.

### What the tests prove

| Test name | What it checks |
|---|---|
| `add_assigns_incrementing_ids…` | IDs start at 1 and increment |
| `add_trims_whitespace` | `"  buy milk  "` is stored as `"buy milk"` |
| `add_rejects_blank_text…` | Empty, spaces, tabs → nothing inserted |
| `add_rejects_text_longer_than…` | 280 chars OK, 281 rejected |
| `the_length_limit_is_measured_after_trimming` | Trailing spaces don't count |
| `the_database_rejects_what_the_handler_rejects` | CHECK constraint agrees with Rust check |
| `the_template_advertises_the_same_limit…` | `maxlength="280"` matches the constant |
| `a_stale_request_against_a_deleted_id…` | Can't act on a deleted id; adding doesn't reuse it |
| `toggle_flips_state_and_reports…` | Toggle works; unknown ids return false |
| `delete_removes_and_reports…` | Delete works; unknown ids return false |
| `ids_are_never_reused_after_delete` | Identity columns never reuse values |
| `sorted_puts_completed_below_active…` | Sort order: active first, done last |
| `sorted_with_nothing_is_empty` | No todos → empty vec |
| `sql_injection_payloads_are_stored_as_text` | Injection stored verbatim; table survives |

---

## 15. tests/todo_http.rs — HTTP-layer tests

📄 `tests/todo_http.rs`

29 tests that drive the router via `oneshot`. A failure here means a handler,
template, or route is wrong — not the SQL.

### What the tests prove

**Read path:**
- The page renders with title, panel, and list
- Seeded todos appear in the HTML
- XSS payloads are escaped (no raw `<` in the text node)
- Empty state shows the placeholder message
- Open count displays correctly

**Create path:**
- Creating a todo returns 200 and the todo appears
- Returns the panel fragment, not a full page
- The input box is empty after submit (no stale `value=`)
- Blank text doesn't create a todo
- Unparseable body → 422
- XSS payloads are escaped in the create response too

**Toggle path:**
- Toggle targets `#todo-list`, never `#todo-panel` (the critical test)
- Toggling adds `todo-row--done` and `checked`
- Returns only the list fragment (no form, no DOCTYPE)
- Toggle twice returns to undone
- Completed sorts below active in the rendered fragment

**Delete path:**
- Delete removes the todo and shows the empty state
- The row has a delete button with correct htmx attributes and `hx-confirm`
- The button has an `aria-label` for accessibility
- The delete fragment has no form

**Edge cases:**
- Unknown id → 404 with the current list as body
- Non-numeric id → 400 (caught by `Path<i64>`)
- SQL injection in the URL path → 400
- Unknown route → 404
- Wrong method on known route → 405

**Security:**
- 8 XSS payloads seeded, all escaped on the read path
- XSS escaping works through the create round-trip
- Quote payloads can't break out of attributes (`aria-label`)

---

## The whole data flow, one more time

Now that you've seen every file, here's the complete picture for **adding a
todo**:

```
 1.  You type "buy milk" and press Enter
 2.  Browser → htmx intercepts the form submit
 3.  htmx sends: POST /todos with body text=buy+milk
 4.  axum matches the route → calls create()
 5.  create() extracts Form(NewTodo { text: "buy milk" })
 6.  create() calls add(&pool, "buy milk")
 7.  add() calls validated("buy milk") → Some("buy milk")
 8.  add() runs: INSERT INTO todos (text) VALUES ('buy milk')
 9.  Postgres assigns id=1 from the identity sequence
10.  create() calls sorted(&pool) → Vec<Todo>
11.  create() calls open_count(&pool) → 1
12.  create() renders PanelTemplate { todos, open_count: 1 }
13.  Askama compiles the template → HTML string
14.  axum wraps it in Html, returns 200
15.  htmx receives the HTML
16.  htmx replaces #todo-panel with the new HTML
17.  You see "buy milk" in the list, input box is cleared
```

Every other operation follows the same shape: browser → htmx → handler →
store → template → htmx swap.

---

# Part 2 - authentication

Everything above describes the app as it was: one list, no accounts. Part 2
covers what arrived next. The short version is that **every store function
gained a `user_id`**, and a stack of middleware now sits in front of the todo
routes.

Start here, because it is the part every other file refers to.

## 16. src/app.rs - the one router

`main.rs` used to build the router. That is a problem the moment authentication
arrives: anything added in `main.rs` is **absent from the test suite**, because
the tests call `todo::build_router` directly. So the router moved into its own
function that both callers use.

```rust
pub fn build_app(db: Db, config: AppConfig) -> Router {
    let session_layer = SessionManagerLayer::new(DbStore::new(db.clone()))
        .with_name("session")
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_secure(config.session_secure)
        .with_expiry(Expiry::OnInactivity(SESSION_IDLE_LIFETIME));

    let auth_layer = AuthManagerLayerBuilder::new(AuthBackend { db: db.clone() }, session_layer)
        .build();

    Router::new()
        .merge(auth::routes(pool.clone()))      // /login, /signup, /logout - public
        .merge(crate::todo::build_router(pool)) // / and the todos - guarded
        .route("/healthz", get(|| async { "ok" }))
        .layer(middleware::from_fn_with_state(
            config.allowed_host,
            auth::verify_origin,
        ))
        .layer(auth_layer)
}
```

- **`SessionManagerLayer`** is the thing that reads and writes the session
  cookie and calls the store. The options are the cookie's settings, and they are
  read once at startup rather than per request.
- **`AuthManagerLayerBuilder`** wraps that with axum-login's own layer, whose job
  is to put an `AuthSession` into the request extensions.
- **The last `.layer()` is the outermost**, so `auth_layer` wraps the CSRF check
  wraps the guard wraps the handler. That order is required: the guard reads the
  `AuthSession` that `auth_layer` put there.

### The cookie settings, and why each one

| Setting | Value | Why |
|---|---|---|
| `with_http_only` | `true` | No JavaScript on this app reads the cookie, so an XSS bug cannot steal it |
| `with_same_site` | `Lax` | Blocks the cross-site POST that CSRF relies on, while still allowing a link from elsewhere to arrive logged in |
| `with_secure` | from `SESSION_SECURE` | Never sent over plain http, so it cannot be sniffed. Default `true` — forgetting to set it should fail closed |
| `with_expiry` | `OnInactivity(7 days)` | Long enough not to nag, short enough that a forgotten laptop is not a live session for months |

The cookie carries **only an opaque 128-bit random id**. No email, no user id,
nothing to read. Everything else is a row in `sessions`.

## 17. src/auth.rs - User, and the CurrentUser extractor

Two things live in the module front door.

**`User`** is the account behind the session. It deliberately does *not* keep
the password hash:

```rust
pub struct User {
    pub id: i64,
    pub email: String,
    pub email_verified_at: Option<OffsetDateTime>,
    auth_hash: Vec<u8>,   // SHA-256 of the stored Argon2 hash
}
```

axum-login copies whatever `session_auth_hash()` returns **into the session**.
Returning the Argon2 hash directly would put a crackable password hash in the
`sessions` table. Hashing it again with SHA-256 costs nothing and keeps the
Argon2 string inside the `users` table.

It still changes whenever the password changes, which is the part that matters:
that is how changing a password logs every other device out.

**`CurrentUser`** is how a handler asks who is signed in:

```rust
pub struct CurrentUser(pub User);

impl<S> FromRequestParts<S> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthSession>()
            .and_then(|session| session.user.clone())
            .map(CurrentUser)
            .ok_or(AppError::Unauthenticated)
    }
}
```

Read that as: "look in the request extensions for the session axum-login put
there, take its user, and fail if there isn't one." It returns a `Result` rather
than unwrapping, so a route mounted outside the guard gets a 401 and a log line
instead of a panic.

## 18. src/auth/password.rs - hashing, off the async thread

```rust
pub async fn hash(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || password_auth::generate_hash(password))
        .await
        .map_err(AppError::Hashing)
}
```

Argon2 is slow **on purpose** - roughly 50ms of CPU at the default cost. That is
what makes a stolen database expensive to crack. But tokio's worker threads are
for waiting on I/O, not burning CPU: 50ms on a worker thread stalls every other
request that thread is serving. `spawn_blocking` hands the work to a dedicated
thread and returns the worker immediately.

`password-auth` cannot report a hashing failure - its signature has no `Result` -
so it panics instead. Because the panic happens on the blocking thread,
`spawn_blocking` catches it and hands it back as a `JoinError`, which is why
this function can still return a `Result`.

### The dummy hash

```rust
pub async fn dummy_hash() -> &'static str { /* built once, kept forever */ }
```

When someone logs in with an address that does not exist, there is no hash to
verify against. Skipping the check would make "no such account" return in
microseconds while a real account takes 50ms - and that difference alone is
enough to list every address on the site. So a throwaway hash is verified
instead. It is built once because generating it costs the same 50ms it protects.

## 19. src/auth/store.rs - user SQL and the session store

User queries are ordinary. The interesting function is this one:

```rust
pub async fn create_user(pool: &PgPool, email: &str, password_hash: &str)
    -> Result<Option<User>, sqlx::Error>
{
    let inserted = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2)
         RETURNING id, email, password_hash, email_verified_at",
    )
    .bind(email)
    .bind(password_hash)
    .fetch_optional(pool)
    .await;

    match inserted {
        Ok(row) => Ok(row.map(UserRow::into_user)),
        Err(err) if is_unique_violation(&err) => Ok(None),
        Err(err) => Err(err),
    }
}
```

A duplicate email is Postgres error `23505`. It comes back as `None` rather than
an `Err` because **it is not a server fault** - it is something the visitor can
fix, so it belongs in the form as a message, not in the log as a 500.

### Why the session store is hand-written

`tower-sessions-sqlx-store` exists and would have saved ~90 lines. It depends on
**sqlx 0.8** while this app is on **sqlx 0.9**. Two sqlx versions in one binary
means two incompatible `PgPool` types, and the pool this app already has cannot
be handed to it. Four methods beat that:

```rust
#[async_trait]
impl SessionStore for PostgresStore {
    async fn create(&self, record: &mut Record) -> SessionResult<()> { /* INSERT */ }
    async fn save(&self, record: &Record) -> SessionResult<()> { /* INSERT ... ON CONFLICT */ }
    async fn load(&self, id: &Id) -> SessionResult<Option<Record>> { /* SELECT */ }
    async fn delete(&self, id: &Id) -> SessionResult<()> { /* DELETE */ }
}
```

Two details worth knowing:

- **`create` uses a plain INSERT, `save` uses an upsert.** If `create` upserted,
  a colliding session id would silently overwrite somebody else's session. On a
  collision it asks for a fresh id and tries again.
- **`load` filters on `expiry_date > now()` in SQL**, so an expired row is never
  handed back and never has to be parsed.

## 20. src/auth/backend.rs - the two traits

axum-login asks the app exactly two questions.

```rust
async fn authenticate(&self, creds: Credentials) -> Result<Option<User>, sqlx::Error> {
    let found = store::find_by_email(&self.pool, creds.email.trim()).await?;

    let stored_hash = match &found {
        Some(row) => row.password_hash.clone(),
        None => password::dummy_hash().await.to_owned(),
    };

    if !password::verify(creds.password, stored_hash).await {
        return Ok(None);
    }
    Ok(found.map(UserRow::into_user))
}
```

Note the shape: the hash is verified **before** the row becomes a `User`. That
is why `User` never needs to hold the password hash.

`Credentials` deliberately does not implement `Debug`. Anything that derives
`Debug` can end up inside a tracing span or a panic message, and this struct
holds a plaintext password.

## 21. src/auth/middleware.rs - the guard and the CSRF check

### require_auth

```rust
pub async fn require_auth(req: Request, next: Next) -> Response {
    let logged_in = req
        .extensions()
        .get::<AuthSession>()
        .is_some_and(|session| session.user.is_some());

    if logged_in {
        return next.run(req).await;
    }

    if req.headers().contains_key("hx-request") {
        return (StatusCode::OK, [(HX_REDIRECT, "/login")]).into_response();
    }

    Redirect::to(&format!("/login?next={}", encode_query_value(req.uri().path()))).into_response()
}
```

The htmx branch is the interesting one. A `303` is correct for a browser and
catastrophic for htmx: htmx would swap a whole login page into `#todo-list`,
leaving someone stuck in the middle of their list. `HX-Redirect` tells htmx to
do a real navigation instead.

### verify_origin

`SameSite=Lax` already stops a cross-site form post from carrying credentials,
so this is the second lock rather than the only one. It compares the **authority**
(`host:port`) of `Origin`, falling back to `Referer`, and only on methods that
change something.

A request with neither header is **allowed through**. Browsers always send
`Origin` on a POST, so its absence means a non-browser client - curl, or this
project's own tests - rather than a forgery.

### safe_next

`?next=` is attacker-supplied, so it is filtered:

```rust
let is_local = trimmed.starts_with('/')
    && !trimmed.starts_with("//")
    && !trimmed.contains('\\');
```

`//evil.example` and `/\evil.example` are the same open redirect written so a
naive "does it start with a slash" check misses it. Anything that fails falls
back to `/`.

## 22. src/auth/handlers.rs - signup, login, logout

The auth pages are **plain forms**. No htmx: a swapped login page is a confusing
thing to hand someone who is not logged in.

### One message for every login failure

```rust
const BAD_CREDENTIALS: &str = "Invalid email or password.";
```

Saying "no such email" versus "wrong password" is enough to enumerate every
account on the site. Both failures return byte-identical responses, and there is
a test that asserts exactly that by comparing the two bodies.

### The session id is always rotated

```rust
session.session.cycle_id().await?;
session.login(&user).await?;
```

This line looks redundant - axum-login's own `login` does cycle the id. It does
not always. Its `login` only calls `cycle_id` when the session carries no auth
hash, and its `logout` deletes the stored record **without clearing the
in-memory hash**. So switching accounts without this line kept the previous
session id, which is textbook session fixation. The test
`switching_accounts_changes_the_session_id` caught it.

## 23. tests/common/mod.rs - the cookie jar

Almost every request needs a session now, so the test helpers grew one:

```rust
pub async fn signed_up(pool: &PgPool, email: &str) -> Self {
    let mut client = Self::anonymous();
    let (status, body) = client.post_form(&pool, "/signup", &[...]).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "signup failed: {body}");
    client
}
```

It goes through the **real signup handler** rather than inserting a row, so
signing up is itself under test. After each request the jar stores whatever
`Set-Cookie` arrived and sends it back next time.

One detail that is easy to get wrong and is handled here: a logout sends an
*expired* cookie under the same name, and that must **replace** the stored one.
Otherwise a signed-out client would still be carrying the old cookie and the
tests after it would pass for the wrong reason.

## The whole data flow, one more time

Creating a todo, now that there is a login:

```
 1.  You are signed in; your browser sends Cookie: session=<opaque id>
 2.  AuthManager reads that id, loads the session row, looks up the user
 3.  require_auth sees session.user is Some - passes through
 4.  verify_origin checks Origin - not a POST to /todos, so skipped
 5.  htmx sends: POST /todos with body text=buy+milk
 6.  CurrentUser extracts User from the extensions
 7.  add(&pool, user.id, "buy milk") runs
 8.  INSERT INTO todos (text, user_id) VALUES ($1, $2)   <- both bound
 9.  Postgres assigns id=1 from the identity sequence
10.  sorted(&pool, user.id) returns only YOUR todos
11.  PanelTemplate renders the new list
12.  htmx replaces #todo-panel
13.  You see "buy milk", input box is cleared
```

Step 10 is the one that matters. The `user_id` filter is not a check in the
handler - it is part of the query, so another account's todo is simply not in the
result set. And because `rows_affected()` is 0 when you try to *change* somebody
else's todo, the answer is **404**, not 403. A 403 would confirm the id exists.
