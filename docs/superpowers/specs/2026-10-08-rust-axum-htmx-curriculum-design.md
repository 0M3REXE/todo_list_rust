# Curriculum Design: Rust + Axum + Askama + htmx 4

Date: 2026-10-08
Status: approved
Deliverable: an interactive, milestone-based curriculum — not code. The student writes all code.

---

## 1. Context

`practice/` is a 97-line server-rendered web app. No git commits yet.

| File | Role |
|---|---|
| `src/main.rs` | everything: error type, templates, query struct, 3 handlers, router, listener |
| `templates/index.html` | full page + htmx wiring |
| `templates/hello.html` | 1-line fragment |
| `templates/goodbye.html` | 1-line fragment |
| `static/style.css` | styling |

Dependency stack (`Cargo.toml`):

| Layer | Crate | Version present | Role |
|---|---|---|---|
| Runtime | tokio | 1.52.4 | async executor, TCP listener |
| HTTP | axum | 0.8.9 | router, extractors, handlers, `IntoResponse` |
| Middleware | tower-http | 0.7.0 | `ServeDir` static files, `TraceLayer` |
| Templating | askama | 0.16.0 | compile-time-checked Jinja-like templates |
| Deserialization | serde | 1.0.228 | query param -> struct |
| Logging | tracing / tracing-subscriber | 0.1.44 / 0.3.23 | structured logs |
| Frontend | htmx | 1.9.10 (CDN) | HTML-over-the-wire swaps |

Architecture: **HTML-over-the-wire / hypermedia**. htmx fires `GET /hello?name=Omer`; Askama
renders an HTML fragment; axum returns `text/html`; htmx injects it into `#result`. No SPA, no
JSON API, no client-side framework.

Request flow today:

```
browser -> TCP -> axum Router -> [TraceLayer] -> extractors (Query<NameQuery>)
       -> handler -> Askama render -> Html<String> -> TCP -> htmx swaps into #result
```

---

## 2. Learner profile (established by interview)

- Rust: **knows the language, new to async**. Ownership, borrowing, traits, generics,
  `Result`/`Option` are all assumed known.
- Goal: **build their own web app in this stack.** App type deliberately undecided, so the
  curriculum teaches the stack generically rather than driving toward one product.
- Session shape: **one concept at a time**. Confirm between lessons.
- Plan size: **~8-10 milestones.**

---

## 3. Approach: Spiral (approved)

Rejected alternatives, with reasons:

- **Bottom-up (language-first).** Learn `Future`, `poll`, `Waker`, `Pin`, `Send`/`Sync` as
  language topics before touching axum. Rejected: 1-2 weeks before anything runs, and the
  `Waker`/`Pin` internals rabbit hole exceeds what production work requires.
- **Top-down (app-first).** Trace the existing app, learn async as axum forces it on you.
  Rejected: `Send` bounds, cancellation and concurrency stay magic precisely when a real app
  needs them.
- **Spiral (chosen).** One fast top-down pass of the request lifecycle for a mental model, then
  bottom-up theory drilled into each real feature milestone. Costs some revisiting; prevents
  cargo-culting `.await` and prevents mystery-typing axum on day one.

Deliberate omissions: `Pin`/`Waker` internals (explained only when a compiler error demands it),
SQL and auth (out of scope until the app type is chosen).

---

## 4. Roadmap

Stage 0 is the top-down pass. Stages 1-8 each add a real feature to the student's own app and
drill only the async theory that feature exposes.

| # | Stage | Feature built | Async theory drilled |
|---|---|---|---|
| 0.5 | Version migration | bump crates to latest, htmx 1.9.10 -> 4.0.0 | n/a (real migration, observed) |
| 0 | Orientation | none — trace `GET /hello?name=Omer` end to end | `Future` trait, lazy polling, Tower `Service` |
| 1 | Async foundations | experiments in `src/bin/` | `async fn` as state machine, `.await` is free, `Send` bounds, cancellation |
| 2 | Routing & extractors | `GET /greet/{lang}` path param | `FromRequestParts` vs `FromRequest`, extractors run before the handler |
| 3 | Askama deeper | page rendering `Vec<Greeting>` with loop + conditional | templates are sync — that is the point |
| 4 | htmx properly | one handler serving fragment vs full page | `HX-Request-Type`; why hypermedia avoids JSON |
| 5 | Forms & shared state | POST form + validation + `AppState` behind `Arc<RwLock<..>>` | `Send + Sync` on state, handlers must be `'static` |
| 6 | Errors & middleware | error enum -> HTTP response, timeout layer, 404 fallback | layers are futures wrapping the inner service |
| 7 | Concurrency | fan out 5 requests with `JoinSet`, one slow, SSE progress stream | `tokio::select!`, cancellation, dropping an unfinished future, timeouts |
| 8 | Testing | handler tests with `oneshot`, no port bound | `#[tokio::test]` — a test is itself a future |
| 9 | Structure & next steps | split monolith into modules; where sqlx/auth would slot in | `with_graceful_shutdown` |

### Amendments from the htmx 4 migration (Section 5)

- **Stage 4 teaches `HX-Request-Type`, not `HX-Request`.** htmx 4 sends
  `HX-Request-Type: partial` for targeted swaps and `full` for body-level / `hx-select`
  requests. This is a reliable, self-describing signal for the fragment-vs-full-page pattern,
  replacing the old `HX-Request: true` heuristic.
- **Stage 6 teaches swap-ready error responses.** htmx 4 swaps 4xx/5xx by default (only 204/304
  are exempt). The current `AppError` returns `(500, "something went wrong")` — a bare text body
  that htmx 4 will inject into `#result`. Error handling becomes: errors must render as HTML
  fragments, and `hx-status` routes 422 vs 5xx to different targets.
- **Stage 4 uses `<hx-partial>`** (new in htmx 4) instead of teaching OOB swaps.
- **Stage 7's SSE stream requires the `hx-sse` extension**, which htmx 4 ships as a separate
  script (`hx-sse.js`) rather than an in-core feature. Stage 7 includes vendoring it.

---

## 5. Version pinning

Verified against `cargo search` and four.htmx.org on 2026-10-08.

| Crate | Present | Latest | Action |
|---|---|---|---|
| axum | 0.8.9 | 0.8.9 | current |
| tracing | 0.1.44 | 0.1.44 | current |
| tracing-subscriber | 0.3.23 | 0.3.23 | current |
| askama | 0.16.0 | 0.16.1 | bump |
| tokio | 1.52.4 | 1.53.2 | bump |
| tower-http | 0.7.0 | 0.7.1 | bump |
| serde | 1.0.228 | 1.0.229 | bump |
| htmx | 1.9.10 | 4.0.0 | major jump, exact-pin |

### htmx 4.0.0 (released 2026-08-28)

```html
<script src="https://cdn.jsdelivr.net/npm/htmx.org@4.0.0"
        integrity="sha384-BvJpBiO8Kh31EqtJe5DRIeWrHWnCGkwytKs9NKFi86Hhw96dEqdEMzZDeK9iEGTc"
        crossorigin="anonymous"></script>
```

**Must pin the exact version.** htmx 4 is intentionally *not* npm's `latest` dist-tag until early
2027, to protect users on unversioned CDN URLs. `latest` resolves to 2.x. A new project should
start on 4.

Changes that matter to this curriculum:

- `fetch()` replaces `XMLHttpRequest`. XHR events (`htmx:xhr:*`) removed.
- Attribute inheritance is now **explicit** via the `:inherited` modifier. `hx-inherit` and
  `hx-disinherit` are removed. Opt back into 2.x behavior with
  `htmx.config.implicitInheritance = true`.
- 4xx/5xx responses **swap by default**. Restore old behavior with
  `htmx.config.noSwap = [204, 304, '4xx', '5xx']`.
- History no longer snapshots to `localStorage`; the back button re-fetches from the server.
  `hx-history` and `historyCacheSize` removed. `hx-history-cache` extension restores snapshots.
- Events renamed to `htmx:phase:action[:sub-action]` — `htmx:afterRequest` ->
  `htmx:after:request`, `htmx:beforeSwap` -> `htmx:before:swap`.
- Attribute renames, order matters: `hx-disable` -> `hx-ignore` **first**, then
  `hx-disabled-elt` -> `hx-disable`.
- Removed: `hx-ext`, `hx-vars` (use `hx-vals` with `js:` prefix), `hx-params`,
  `hx-history`, `hx-request`, `hx-prompt`.
- New: `HX-Request-Type`, `HX-Source` (`tag#id`), `HX-Target` (`tag#id`), `hx-partial`,
  built-in `innerMorph`/`outerMorph` swaps, `hx-status` routing, 60s `defaultTimeout`,
  `hx-sync`, SSE/WebSocket/multipart streaming extensions.

`templates/index.html` places every `hx-*` attribute on the `<button>` itself, so explicit
inheritance breaks nothing. The script tag swap is the whole migration.

Upgrade checker (for reference, not needed here):

```
npx htmx.org@4.0.0 upgrade-check -- ./path/to/project/root
```

---

## 6. Lesson protocol

Every lesson has identical shape:

```
CONCEPT        one idea, named
WHY IT EXISTS  the problem it solves
SNIPPET        minimal, self-contained
WHERE IT GOES  exact file + anchor (line, or "next to X")
WHY THAT SPOT  why it goes there and not elsewhere
CHECKPOINT     a command to run, or a question to answer
                ↓
         student edits, says "next"
```

Rules the instructor holds to:

1. **Never write to the student's files.** Snippets only. The student pastes.
2. **One concept per lesson.** Two ideas means two lessons.
3. **Every snippet has a home.** New files get a full path and an explicit "create this".
4. **Explain placement, not just syntax.** "Why here and not in `main`" is often the real lesson.
5. **Compiler errors are lesson content.** If a snippet fails to compile, the error gets explained
   rather than patched.

### Target file layout

```
src/
  main.rs            wiring only — shrinks as stages progress
  app.rs             build_router(): routes, layers, state   (Stage 6+)
  state.rs           AppState                                (Stage 5)
  routes/
    mod.rs
    greetings.rs     handlers                                 (Stage 2+)
  error.rs           AppError -> IntoResponse                (Stage 6)
templates/
  layout.html        shared shell                             (Stage 3)
  greeting.html      fragment                                 (Stage 4)
static/
  style.css
```

Stages 0-2 live in `main.rs` as-is. Extraction into these files happens from Stage 6 onward, so
every file move is motivated rather than ceremonial.

`templates/hello.html` is renamed to `greeting.html` in Stage 4, at the point where fragment
rendering becomes the lesson and the singular name starts to lie. The rename is a
`git mv`, not a delete-and-recreate, so the diff shows as a pure move.

---

## 7. Definition of done

A stage is done when the student can do the following unprompted — not when the code runs.

- **0.5** — state, from memory, which crate versions are current and explain *why* the htmx 4 CDN
  URL must pin an exact version rather than using `latest`
- **0** — narrate `GET /hello?name=Omer` from accept to htmx swap, naming every layer
- **1** — predict which of four `async fn`s run to completion and in what order; explain why
  `.await` does not block the thread
- **2** — add a route with a path param *and* a query param without looking up extractor names
- **3** — render a list with a conditional and an `Option` fallback without touching the Rust side
- **4** — serve a fragment vs. a full page from one handler, keyed off `HX-Request-Type`
- **5** — write a POST handler with validation that re-renders the form with errors on failure
- **6** — add a custom error type that maps to a status code; explain how a layer wraps the
  inner service
- **7** — fan out concurrent work with a timeout and state what happens to the abandoned futures
- **8** — write a test that hits a handler with no port bound
- **9** — explain where a database would live in this structure

Progress is reviewed at the end of each stage; the roadmap adjusts if pacing is off.

---

## 8. Out of scope

- SQL / sqlx / migrations
- Authentication, sessions, cookies
- Deployment, TLS, reverse proxies
- `Pin` and `Waker` internals beyond what an error message requires
- Client-side JS beyond what htmx requires

Each becomes available once the student picks an app direction.

---

## 9. Version note

This spec pins crate versions as of 2026-10-08. `Cargo.toml` uses caret ranges, so
`cargo update` may pull newer patch releases at any time — expected, and not drift from this
document.