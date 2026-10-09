# Design: Row Delete Button + Paginated Todo List

Date: 2026-10-09
Status: approved in dialogue
Supersedes nothing. Extends `2026-10-08-todo-list-design.md`.

---

## 1. Context

The todo list renders every todo in one `<ul id="todo-list">`. Two gaps:

1. `POST /todos/{id}/delete` exists (`src/todo.rs:251`) and is fully tested, but no
   template calls it. The capability is unreachable from the UI.
2. `MAX_TODOS` is 1,000 (`src/todo.rs`). The list renders all of them, so the DOM
   grows without bound relative to what a human will actually scan.

Scope is deliberately narrow: a delete control per row, and pagination over the
existing sorted list. No database, no auth, no new routes beyond one fragment
route, no change to the store's sort order.

---

## 2. Decisions

| Question | Decision | Rationale |
|---|---|---|
| Page size | **20** | Balances scan depth against page-click count at a 1,000-item ceiling. |
| Ticking a todo on page 2 that sorts it to the completed pages | **Let it move** | Matches how sorted lists behave everywhere (Gmail, Linear). Preserves the active-first sort that `completed_sorts_below_active_within_order` pins. |
| Deleting the only todo on the last page | **Step back a page** | Landing on an empty shell reads as breakage. |
| Adding a todo while on page 3 | **Stay on page 3** | Preserves the user's place. The new todo is on page 1, where it sorts. |

---

## 3. Swap target

The binding constraint is `toggle_targets_the_list_not_the_panel`: swapping
`#todo-panel` would discard text the user typed but had not submitted. Pagination
controls and the "N open" count both change on every mutation but currently sit
outside `#todo-list`, so a list-only swap leaves them stale.

**Chosen: one swap region.** A new `#todo-page` div wraps the list, the count, and
the pagination controls. It sits inside `#todo-panel` but below the add form.

```
#todo-panel
├── form (add)          ← never swapped
└── #todo-page          ← the single swap target
    ├── ul#todo-list
    ├── count
    └── pagination
```

Every mutation returns `#todo-page`. The form is structurally out of reach, the
count cannot go stale, and one target keeps the htmx markup plain.

**Rejected: out-of-band swaps.** `hx-swap-oob` would let `#todo-list` stay the
primary target and update the controls from the same response. More flexible, but
it adds OOB syntax and split response bodies for a single-screen page.

### Consequence for an existing pinned test

`toggle_targets_the_list_not_the_panel` asserts the toggle carries
`hx-target="#todo-list"`. That target becomes `#todo-page`, so **the test must be
updated**. Its intent — never swap the panel, never eat the typed input — is
preserved; only the target id changes. This is the one intentional break of a
currently-green assertion.

---

## 4. Model

A `Page<T>` value carries everything the templates need:

```rust
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: usize,        // 1-based, always clamped into range
    pub per_page: usize,
    pub total_items: usize,
    pub total_pages: usize, // at least 1, so an empty list is page 1 of 1
}
```

`TodoList::page(&self, page: usize, per_page: usize) -> Page<Todo>` calls
`sorted()` first, then slices. Sorting stays where it is, so every existing
ordering test keeps passing untouched.

Clamping: `total_pages = max(1, ceil(total_items / per_page))`, then
`page = page.clamp(1, total_pages)`. Out-of-range requests land on the last page
rather than 404ing. This gives "step back a page" on delete for free — when the
last item leaves page 5, the requested page 5 now clamps to 4.

---

## 5. Routes

```
GET  /                     index, ?page=N (defaults to 1)
GET  /todos?page=N         returns only the #todo-page fragment   [new]
POST /todos?page=N         create; re-renders the panel on that page
POST /todos/{id}/toggle?page=N
POST /todos/{id}/delete?page=N
```

`GET /todos` is a fragment route for htmx: requested directly in a browser it
returns a bare fragment, not a document. That is consistent with the app already
being htmx-first — toggle and delete are also unusable without JavaScript.

**No routing conflict.** `/todos/page/...` is not used, and `/todos/{id}/toggle`
cannot match `/todos` because the two-segment paths differ. `{id}` is a `u64`, so
the literal segment `page` never parses as an id.

Every mutating route reads `?page=` and re-renders at that page. That is what
keeps the view stable across a toggle or delete instead of snapping to page 1.

---

## 6. Templates

| File | Change |
|---|---|
| `todo_page.html` | **New.** `#todo-page` region: includes the list, the count, and pagination. |
| `todo_pagination.html` | **New.** Bulma `pagination`, `hx-get`-driven, omitted entirely when there is one page. |
| `todo_list.html` | Delete button per row. |
| `todo_panel.html` | Includes `todo_page.html` instead of the list and count. |
| `index.html` | Unchanged. |

The "N open" count stays a global total across all pages. It was global before
pagination and remains so — only the rows around it become a slice.

### Delete button

Placed after `todo-text` in the row, Bulma's `delete` class:

```html
<button class="delete"
        hx-post="/todos/{{ todo.id }}/delete?page={{ page }}"
        hx-target="#todo-page"
        hx-swap="outerHTML"
        hx-confirm="Delete this todo?"></button>
```

`hx-confirm` guards against a stray click destroying work.

Position matters: the toggle's `todo-toggle` → next `>` slice that
`toggle_targets_the_list_not_the_panel` parses must still terminate at the input
tag, so the button goes after the text, never before the checkbox.

The button must not carry `todo-input`; `toggle_returns_only_the_list_fragment`
asserts that class is absent from the list fragment.

### Pagination controls

Bulma's `pagination` component, with `hx-get="/todos?page=N"`,
`hx-target="#todo-page"`, `hx-swap="outerHTML"`. `is-current` and
`aria-current="page"` mark the active page.

---

## 7. Error handling

Unchanged from the current conventions. An out-of-range page clamps rather than
erroring. A stale id returns 404 with the page fragment as the body, matching
`unknown_id_is_404_with_the_current_list_as_the_body` — htmx 4 swaps 4xx, so the
list renders unchanged rather than blanking.

A non-numeric or absent `?page=` (`?page=abc`, `?page=-1`, no param at all)
defaults to page 1 rather than erroring. A malformed page number is a stale
bookmark, not a client bug worth a 400.

`AppError` needs no new variants; pagination adds no new failure mode.

---

## 8. Testing

Model:

- `page_slices_after_sorting` — page 2 of 25 items returns 21-25, order preserved
- `page_clamps_above_the_last_page` — `page(99, 20)` lands on the last page
- `page_clamps_below_one` — `page(0, 20)` lands on page 1
- `an_empty_list_is_page_one_of_one` — no divide-by-zero, `total_pages == 1`
- `total_pages_rounds_up` — 21 items at 20 per page is 2 pages, not 1

Handlers:

- `a_mutation_keeps_the_requested_page` — toggle with `?page=2` re-renders page 2
- `create_keeps_the_current_page` — adding from page 2 stays on page 2
- `deleting_the_last_item_on_the_last_page_steps_back`
- `the_page_fragment_is_not_a_full_page` — mirrors the existing panel assertion
- `the_row_carries_a_delete_control` — button present, correct target
- `the_page_region_carries_list_count_and_controls`

Updated:

- `toggle_targets_the_list_not_the_panel` — retargeted to `#todo-page`, intent
  (never swap the form) unchanged

Preserved unchanged: every existing ordering, escaping, id-reuse, limit, and
poisoned-lock test.

---

## 9. Out of scope

Deleting many at once, filtering or searching, changing page size at runtime,
remembering the current page across reloads, and a database. The delete endpoint
already exists, so no persistence work is implied here.