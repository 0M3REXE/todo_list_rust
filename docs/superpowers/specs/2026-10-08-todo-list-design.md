# Todo List Design: Rust + Axum + Askama + htmx 4

Date: 2026-10-08
Status: approved
Deliverable: a single-screen todo list in the existing `practice` stack. No auth, in-memory
storage, HTML-over-the-wire.

---

## 1. Context

`practice/` is a 97-line server-rendered greeting app. This spec replaces it in place with a todo
list. No git commits yet.

| File | Current role | After |
|---|---|---|
| `src/main.rs` | everything: error type, templates, query struct, 3 handlers, router, listener | wiring only — tracing init, listener, serve |
| `src/todo.rs` | does not exist | model, state, handlers, template structs, `build_router` |
| `templates/index.html` | full page + htmx wiring | page shell, includes `todo_panel.html` |
| `templates/hello.html` | 1-line fragment | **deleted** |
| `templates/goodbye.html` | 1-line fragment | **deleted** |
| `templates/todo_panel.html` | does not exist | `#todo-panel` — add form + includes `todo_list.html` |
| `templates/todo_list.html` | does not exist | `#todo-list` — the rows |
| `static/style.css` | 187 lines, purple/amber greeting styles | rewritten — Hallmark austere tokens |
| `static/tokens.css` | does not exist | all design tokens |

Dependency stack (`Cargo.toml`) — unchanged except one dev-dependency:

| Layer | Crate | Version | Role |
|---|---|---|---|
| Runtime | tokio | 1.52.4 | async executor, TCP listener |
| HTTP | axum | 0.8.9 | router, extractors, handlers, `IntoResponse` |
| Middleware | tower-http | 0.7.0 | `ServeDir` static files, `TraceLayer` |
| Templating | askama | 0.16.0 | compile-time-checked Jinja-like templates |
| Deserialization | serde | 1.0.228 | form body / query -> struct |
| Logging | tracing / tracing-subscriber | 0.1.44 / 0.3.23 | structured logs |
| Frontend | htmx | 4.0.0 (CDN) | HTML-over-the-wire swaps |
| Testing | tower | 0.5.3 (**dev-only, new**) | `ServiceExt::oneshot` |

`tower 0.5.3` is already in `Cargo.lock` as a transitive dependency of axum, so adding it as a
dev-dependency introduces no new download.

Architecture: **HTML-over-the-wire / hypermedia**. The browser posts a form; the server returns an
HTML fragment; htmx swaps it in. No SPA, no JSON API, no client-side framework, no client-side JS.

---

## 2. Decisions

Settled by interview, recorded so implementation does not relitigate them:

| Decision | Choice | Rejected |
|---|---|---|
| Placement | Replace the greeting app in this crate | new crate alongside; same crate at `/todos` |
| Persistence | In-memory `Arc<RwLock<TodoList>>` | JSON file on disk; SQLite |
| Item fields | `text` + `done` only | due date; priority; in-place edit |
| Views | Single list, completed sorted to bottom | All/Active/Done filter tabs |
| Mutation transport | htmx fragment swaps (Approach A) | full-page POST + redirect (B); JSON API (C) |
| Auth | None | — |
| Target audience | The user, learning this stack | — |
| Primary job | Capture a todo fast | scan-outstanding; both equally |
| Tone | Austere | — |

**On Approach B (rejected):** full-page POST + redirect is genuinely simpler — zero JS involvement.
It was rejected because htmx is already a dependency and the existing handlers are already
fragment-serving, so A keeps the stack coherent. The only extra cost over B is one partial
template. It also avoids a full page flash on every checkbox tick.

**On SQLite (rejected):** adds a dependency, migrations, and a connection pool. Well beyond
"simple basic". In-memory is also what the project's own curriculum spec reaches for at Stage 5.

---

## 3. Data model

```rust
struct Todo {
    id: u64,
    text: String,
    done: bool,
}

#[derive(Default)]
struct TodoList {
    items: Vec<Todo>,
    next_id: u64,
}
```

One `TodoList` holds both the vec and the id counter so a single `RwLock` guards everything — the
counter cannot drift from the data it counts. `next_id` starts at 0; ids are assigned by
pre-increment, so the first todo is id 1.

Ids are never reused. A deleted todo's id is never reissued, so a stale in-flight request against a
deleted id cannot mutate a different todo.

**Ordering is not maintained on write.** Render sorts:

```rust
list.items.sort_by_key(|t| t.done);
```

`false` sorts before `true`, and `sort_by_key` is stable, so active todos keep insertion order and
completed todos keep theirs among the done group. Toggling and deleting never touch ordering
explicitly.

---

## 4. State and routing

**State.** `Arc<RwLock<TodoList>>`, injected with `.with_state()`. Handlers take
`State<Arc<RwLock<TodoList>>>`. Mutation handlers take a write lock; rendering takes a read lock.

```rust
pub type TodoStore = Arc<RwLock<TodoList>>;
```

**Routes.**

| Method | Path | Does | Returns |
|---|---|---|---|
| `GET` | `/` | full page | `index.html` |
| `POST` | `/todos` | push a new todo from form field `text` | `todo_panel.html` |
| `POST` | `/todos/{id}/toggle` | flip `done` | `todo_list.html` |
| `POST` | `/todos/{id}/delete` | remove the item | `todo_list.html` |

`/hello` and `/goodbye` are removed.

`build_router(store) -> Router` is a separate function so tests can construct an isolated app per
test case with no port bound.

**Shared mutation path.** Toggle and delete share a `Path<u64>` extractor and a private helper that
locks, mutates, and renders, so the lock-and-render step is written once:

```rust
fn with_list<F>(store: &TodoStore, mutate: F) -> Result<Response, AppError>
where
    F: FnOnce(&mut TodoList) -> bool,
```

The helper takes a closure over `&mut TodoList`, renders `todo_list.html`, and returns
`(StatusCode, Html<String>)` — `404` when the closure reports it found nothing, `200` otherwise.

**`sorted()` returns owned `Vec<Todo>`, not `Vec<&Todo>`.** A borrowed return would tie the
borrow's lifetime to the `RwLock` guard, which makes "drop the lock before rendering" impossible to
express — rendering does I/O and must not hold a write lock. All three template structs therefore
take `Vec<Todo>`, and `sorted()` clones once per render. The alternative, rendering inside the lock
scope, was rejected because Task 5's `with_list` would inherit the same constraint and rendering
under a write lock serialises every request behind it.

**Create flow:**

```
browser form submit -> htmx POST /todos (text=...)
  -> Form<NewTodo> extractor (parses + deserializes before the handler runs)
  -> handler: write lock, trim, push Todo, release lock
  -> read lock, sort_by_key(done), render todo_panel.html
  -> text/html -> htmx outerHTML-swaps #todo-panel
```

---

## 5. Templates and htmx

Three templates, composed by `{% include %}`. Askama includes have full access to the including
template's context, so `todo_list.html` reads `todos` without being passed anything.

```
templates/index.html        page shell   -> {% include "todo_panel.html" %}
templates/todo_panel.html   #todo-panel  -> add form + {% include "todo_list.html" %}
templates/todo_list.html    #todo-list   -> the rows
```

Each is also a standalone `#[derive(Template)]` struct, because `index.html` and
`todo_panel.html` are rendered by handlers directly:

| Template | Rendered by | Target element |
|---|---|---|
| `index.html` | `GET /` | full page |
| `todo_panel.html` | `POST /todos` | `#todo-panel` |
| `todo_list.html` | toggle, delete | `#todo-list` |

**The two-target split is load-bearing.** It is what lets the add input clear itself without JS:

- Create posts with `hx-target="#todo-panel" hx-swap="outerHTML"`. The whole panel re-renders
  server-side, so the input comes back with no `value` attribute and the box is empty. No JS reads
  or writes the input.
- Toggle and delete post with `hx-target="#todo-list" hx-swap="outerHTML"`. Only the list
  re-renders, so text typed into the add input but not yet submitted survives a checkbox tick.

Using one target for all three would either leave stale text in the input after adding, or wipe
unsubmitted text on every toggle.

**htmx 4 attributes.** Every triggering element carries its own `hx-post`, `hx-target`, and
`hx-swap`. htmx 4 made attribute inheritance opt-in via the `:inherited` modifier, so nothing
depends on a parent element supplying them.

`hx-post` sits on the `<form>` itself for create. htmx automatically includes a form's own values
in the request body, so the removed `hx-include` attribute is not needed and is not used.

`hx-boost` is not used. It is unnecessary when each element states its own target.

**Toggle is a native checkbox.** `<input type="checkbox" hx-post="/todos/{{ id }}/toggle" ...>`
gets keyboard and screen-reader support for free, and the server-rendered `checked` attribute is the
source of truth after each swap. The click optimistically toggles the DOM checkbox; the swap then
replaces it with the server's answer, so the two agree.

**Delete is a button**, `type="button"` (it is not inside the add form, but the attribute is set
defensively and costs nothing).

**The submit button** carries `hx-disabled-elt` pointing at itself, so a double-click cannot submit
twice while a request is in flight.

**Empty state.** When `todos` is empty, `todo_list.html` renders an empty-state line in place of the
rows — inside `#todo-list` itself, not wrapped in a second element. The line names what is missing
and names the one action that fills it. There is no bare "No results" string.

---

## 6. Error handling

| Case | Response | Rationale |
|---|---|---|
| Askama render fails | `500`, logged, plain-text body | existing `AppError`, unchanged |
| `text` blank or whitespace-only | no-op — `200`, current panel, state untouched | see below |
| `id` not found on toggle/delete | `404` + current list fragment | status stays honest |
| Malformed body / non-numeric id | axum's own `400`/`422` rejection | extractor rejects before the handler runs; no handler code |

**Blank text is a silent no-op.** `<input required>` already blocks it in the browser, so the server
path is defense-in-depth against a hand-crafted request. The rejected alternative was `422` plus an
error slot in the panel template — a real message at the cost of a template field and a second
status path. Chosen because there is currently no UI to display the error in, and a `422` whose
body says nothing useful is worse than a no-op.

**Unknown id returns `404` with the list as the body.** htmx 4 swaps 4xx responses by default, so
the list renders unchanged and nothing visibly breaks, while the status code stays honest rather
than masking a bad request as success.

**`RwLock` poisoning** propagates as a `500` via axum. Deliberately not handled: no auth, single
user, nothing worth recovering. `.expect()` on the lock is acceptable here.

**CSRF.** No auth, no cookies, no ambient credentials, so there is no CSRF surface to protect
against. Adding tokens would be cargo-culting from a different threat model.

---

## 7. CSS — Hallmark austere

Hallmark was consulted for the visual layer. The finding that shaped this section: the current
`static/style.css` trips several Hallmark critical tells, so preserving the existing look was not
an option.

| Tell | Current location |
|---|---|
| Inter as both display and body | `style.css:1,20` |
| Purple gradient buttons | `style.css:126,137` |
| Full-viewport centred everything | `style.css:34-38` |
| `:focus` instead of `:focus-visible` | `style.css:93` |
| Browser-default `ease` (unspecified in every `transition`) | `style.css:107,129,139` |
| Hover lift + shadow change + colour shift together | `style.css:130-133,141-144` |

**Scope deviation, stated plainly.** All 21 Hallmark macrostructures are landing-page shapes —
hero, sections, nav, footer. A single-screen todo app has none of that furniture, and Hallmark
names no single-screen case. So the page-scope disciplines are applied (tokens, typography, colour,
motion, interaction states, responsive, the 58 slop-test gates) and these are explicitly skipped:
macrostructure, nav archetype, footer archetype, hero enrichment, reveal script, `.hallmark/log.json`
diversification rotation (first run in this project, nothing to rotate from).

**Theme.** Genre modern-minimal (dev tool). Catalog themes are all full landing-page systems, so
rather than bolt one onto a todo list, Cobalt's *discipline* is taken — cool engineered paper,
hairlines instead of shadows, one signal accent, tight radii — without its code-hero-and-⌘K
furniture. Palette built by hand to Hallmark's four-layer rule, cool anchor hue 250.

| Token | Value | Role |
|---|---|---|
| `--color-paper` | `oklch(98.5% 0.004 250)` | ground — never `#fff` |
| `--color-ink` | `oklch(24% 0.02 258)` | primary text — never `#000` |
| `--color-body` | `oklch(34% 0.018 257)` | list text |
| `--color-muted` | `oklch(46% 0.014 255)` | completed text, meta |
| `--color-rule` | `oklch(88% 0.006 252)` | hairlines — all structure |
| `--color-accent` | `oklch(54% 0.20 256)` | focus ring + active count only |

Contrast, WCAG 2.1 against `--color-paper` (`oklch(98.5% 0.004 250)` = `rgb(248, 250, 253)`),
computed by converting each OKLCH value to sRGB and applying the WCAG relative-luminance formula:

| Pair | Ratio | Requirement | Pass |
|---|---|---|---|
| `--color-ink` on paper | 15.76:1 | 4.5:1 body | yes |
| `--color-body` on paper | 11.26:1 | 4.5:1 body | yes |
| `--color-muted` on paper | 6.82:1 | 4.5:1 body | yes |
| `--color-accent` on paper | 4.93:1 | 4.5:1 (it renders the count, which is text) | yes |
| `--color-rule` on paper | 1.37:1 | structural hairline, non-informational | n/a |

**Accent lightness was chosen against the arithmetic, not by eye.** Cobalt's canonical accent is
`oklch(58% 0.20 256)`, which measures 4.19:1 on this paper — fine for a focus ring (3:1) but short of
4.5:1. Because the accent also renders the active count, and that count is text, it drops to
`oklch(54% 0.20 256)` at 4.93:1. One token, both uses, no second accent.

**The accent never fills a surface.** The primary action is a bordered ink button, not a filled one.
This sidesteps the accent-ink contrast trap (white-on-light-accent) entirely and is more austere
anyway. Enter submits; the button is the fallback path.

**Type.** Two families. **Geist** serves display and body (Hallmark's austere row permits this
collapse); **Geist Mono** is the outlier, used only for the active count and the "clear" affordance.
This resolves the Inter-as-everything tell. Scale is major third 1.25, five sizes maximum, weights
400 and 600 only.

**Done state is not a colour.** Struck-through text at `--color-muted`. No green. A colour-only
"done" signal fails WCAG 1.4.1, and the app's own semantics should not rest on hue.

**Layout.** Left-biased, capped at `34rem`, top-aligned. Not `100vh` centred. The input carries the
top visual weight; the list is a stack of hairline-separated rows with no card wrapped around it
(kills card-in-card).

**States.** All eight on the input, submit, toggle, and delete: default, hover, `:focus-visible`,
active, disabled, loading, error, success. Hover rules live inside `@media (hover: hover)`.

The loading state is free: htmx adds `.htmx-request` during flight, so `.htmx-request` rules give
pending styling with no JS, and `hx-disabled-elt` blocks double-submits.

**Motion.** Three primitives maximum, transform and opacity only. Durations from the Hallmark
canon: 100ms press, 180ms hover. Named easings only — `--ease-out`, `--ease-in`, `--ease-in-out`.
Never browser-default `ease`. `prefers-reduced-motion: reduce` collapses all motion to a ≤150ms
opacity crossfade.

**Input borders never change width across states** — constant 1px, with a reserved transparent
2px `outline` so the focus ring appears without layout shift.

**Files.** `static/tokens.css` is new; `static/style.css` is rewritten to `@import` it. Deviation
from Hallmark's "tokens.css at project root": this server serves `static/`, so that is where a
served asset must live.

---

## 8. Testing

`#[cfg(test)] mod tests` in `src/todo.rs`. `build_router(store)` per test, driven with
`tower::ServiceExt::oneshot` — no port bound. Response bodies read via
`axum::body::to_bytes(body, usize::MAX)`, confirmed present in axum 0.8.9, so `http-body-util` is
not needed as a dev-dependency.

| Test | Asserts |
|---|---|
| `index_lists_seeded_todo` | `GET /` is `200`, body contains the seeded todo's text |
| `create_appends_todo` | `POST /todos` is `200`, response body contains the new text, item count grew by 1 |
| `blank_text_is_a_noop` | `POST /todos` with `text=%20%20` leaves the count unchanged, text absent from body |
| `toggle_marks_done` | toggle then `GET /` — the row carries the done class and `checked` |
| `delete_removes_todo` | delete then `GET /` — the text is gone |
| `unknown_id_is_404` | `POST /todos/9999/toggle` is `404`, body is the list |
| `text_is_html_escaped` | `text=<script>alert(1)</script>` comes back `&lt;script&gt;`, raw tag absent |
| `completed_sorts_below_active` | create three, toggle the first — render order is 2, 3, 1 |

Each test builds a fresh store, so no test can observe another's writes.

`cargo test` and `cargo build` are the verification commands. The server also needs a manual pass:
`cargo run`, then add / tick / delete through the browser at 320 / 375 / 414 / 768 px.

---

## 9. Out of scope

- Authentication, sessions, cookies, CSRF tokens
- Persistence beyond process lifetime
- Editing a todo after creation
- Due dates, priorities, tags, multiple lists
- Filter views (All / Active / Done)
- Drag-to-reorder
- Markdown or rich text in todo text
- Database, migrations, connection pooling
- Deployment, TLS, reverse proxies

Each becomes available once the app is actually used and a real need appears.

---

## 10. Version note

This spec pins crate versions as of 2026-10-08. `Cargo.toml` uses caret ranges, so `cargo update`
may pull newer patch releases at any time — expected, and not drift from this document.

htmx is pinned to exactly 4.0.0. htmx 4 is intentionally not npm's `latest` dist-tag until early
2027; `latest` resolves to 2.x.