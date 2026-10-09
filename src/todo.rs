use std::sync::{Arc, RwLock};

use askama::Template;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};

// --- Error handling ---
// One error type every handler returns. Renders every failure as a plain 500
// with a fixed body: the cause goes to the log, never to the client.

#[derive(Debug)]
pub enum AppError {
    Render(askama::Error),
    LockPoisoned,
}

impl AppError {
    fn into_response_inner(self) -> Response {
        match &self {
            Self::Render(err) => tracing::error!(?err, "template render failed"),
            Self::LockPoisoned => tracing::error!("store lock poisoned"),
        }
        (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong").into_response()
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(err) => write!(f, "template render failed: {err}"),
            Self::LockPoisoned => f.write_str("store lock poisoned"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Render(err) => Some(err),
            Self::LockPoisoned => None,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        self.into_response_inner()
    }
}

impl From<askama::Error> for AppError {
    fn from(err: askama::Error) -> Self {
        Self::Render(err)
    }
}

/// A poisoned lock means a previous request panicked mid-write, so the data
/// inside may be inconsistent. Refusing to serve beats serving garbage, and a
/// 500 beats taking the whole process down with `.expect`.
fn read_lock(store: &TodoStore) -> Result<std::sync::RwLockReadGuard<'_, TodoList>, AppError> {
    store.read().map_err(|_| AppError::LockPoisoned)
}

fn write_lock(store: &TodoStore) -> Result<std::sync::RwLockWriteGuard<'_, TodoList>, AppError> {
    store.write().map_err(|_| AppError::LockPoisoned)
}

// --- Limits ---

pub const MAX_TODOS: usize = 1_000;
pub const MAX_TEXT_LEN: usize = 280;

// --- Model ---

#[derive(Clone)]
pub struct Todo {
    pub id: u64,
    pub text: String,
    pub done: bool,
}

/// The vec and the id counter share one lock, so the counter can never
/// drift from the data it counts.
#[derive(Default)]
pub struct TodoList {
    items: Vec<Todo>,
    next_id: u64,
}

impl TodoList {
    /// Trims the text and pushes it. Returns false and changes nothing when
    /// the text is blank, over `MAX_TEXT_LEN`, or the list is full.
    pub fn add(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_TEXT_LEN {
            return false;
        }
        if self.items.len() >= MAX_TODOS {
            return false;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Todo {
            id,
            text: text.to_owned(),
            done: false,
        });
        true
    }

    /// Flips `done`. Returns false if no todo has this id.
    pub fn toggle(&mut self, id: u64) -> bool {
        match self.items.iter_mut().find(|t| t.id == id) {
            Some(todo) => {
                todo.done = !todo.done;
                true
            }
            None => false,
        }
    }

    /// Removes the item. Returns false if no todo has this id.
    pub fn delete(&mut self, id: u64) -> bool {
        match self.items.iter().position(|t| t.id == id) {
            Some(index) => {
                self.items.remove(index);
                true
            }
            None => false,
        }
    }

    /// Active todos first, completed after. `sort_by_key` is stable, so
    /// insertion order survives inside each group. Returns owned todos so
    /// callers can drop the lock guard before rendering.
    pub fn sorted(&self) -> Vec<Todo> {
        let mut items = self.items.clone();
        items.sort_by_key(|t| t.done);
        items
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.items.len()
    }
}

pub type TodoStore = Arc<RwLock<TodoList>>;

pub fn new_store() -> TodoStore {
    Arc::new(RwLock::new(TodoList::default()))
}

/// Counts the unfinished todos for the panel's "N open" line. Askama's
/// parser cannot handle a Rust closure in an expression, so the count is a
/// module function the template calls as `{{ self::open_count(&todos) }}`.
fn open_count(todos: &[Todo]) -> usize {
    todos.iter().filter(|todo| !todo.done).count()
}

// --- Templates ---

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    todos: Vec<Todo>,
}

#[derive(Template)]
#[template(path = "todo_panel.html")]
struct TodoPanelTemplate {
    todos: Vec<Todo>,
}

#[derive(Template)]
#[template(path = "todo_list.html")]
struct TodoListTemplate {
    todos: Vec<Todo>,
}

// --- Handlers ---

async fn index(State(store): State<TodoStore>) -> Result<Html<String>, AppError> {
    let todos = read_lock(&store)?.sorted();
    Ok(Html(IndexTemplate { todos }.render()?))
}

#[derive(serde::Deserialize)]
struct NewTodo {
    text: String,
}

async fn create(
    State(store): State<TodoStore>,
    axum::extract::Form(form): axum::extract::Form<NewTodo>,
) -> Result<Html<String>, AppError> {
    let todos = {
        let mut list = write_lock(&store)?;
        list.add(&form.text);
        list.sorted()
    };
    Ok(Html(TodoPanelTemplate { todos }.render()?))
}

/// Locks the store, applies `mutate`, and renders the list fragment.
/// `mutate` returns false when the id was not found, which becomes a 404
/// whose body is still the current list.
fn with_list<F>(store: &TodoStore, mutate: F) -> Result<Response, AppError>
where
    F: FnOnce(&mut TodoList) -> bool,
{
    let (found, todos) = {
        let mut list = write_lock(store)?;
        let found = mutate(&mut list);
        (found, list.sorted())
    };

    let body = TodoListTemplate { todos }.render()?;
    let status = if found {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    Ok((status, Html(body)).into_response())
}

async fn toggle(
    State(store): State<TodoStore>,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<Response, AppError> {
    with_list(&store, |list| list.toggle(id))
}

async fn delete(
    State(store): State<TodoStore>,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<Response, AppError> {
    with_list(&store, |list| list.delete(id))
}

pub fn build_router(store: TodoStore) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/todos", axum::routing::post(create))
        .route("/todos/{id}/toggle", axum::routing::post(toggle))
        .route("/todos/{id}/delete", axum::routing::post(delete))
        .with_state(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[test]
    fn add_assigns_incrementing_ids_starting_at_one() {
        let mut list = TodoList::default();
        assert!(list.add("first"));
        assert!(list.add("second"));

        let ids: Vec<u64> = list.items.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn add_trims_whitespace() {
        let mut list = TodoList::default();
        assert!(list.add("  buy milk  "));

        assert_eq!(list.items[0].text, "buy milk");
    }

    #[test]
    fn add_rejects_blank_text_without_consuming_an_id() {
        let mut list = TodoList::default();

        assert!(!list.add(""));
        assert!(!list.add("   "));
        assert!(!list.add("\t\n "));
        assert_eq!(list.len(), 0);
        assert_eq!(list.next_id, 0, "a rejected add must not burn an id");
    }

    #[test]
    fn add_refuses_to_exceed_the_max_todo_count() {
        let mut list = TodoList::default();
        for i in 0..MAX_TODOS {
            assert!(list.add(&format!("todo {i}")), "add {i} should fit");
        }

        assert!(
            !list.add("one too many"),
            "the cap must reject the overflow"
        );
        assert_eq!(list.len(), MAX_TODOS);

        // A rejected add must not consume an id, same as a blank one.
        assert_eq!(list.next_id, MAX_TODOS as u64);
    }
    #[test]
    fn deleting_frees_a_slot_at_the_cap() {
        let mut list = TodoList::default();
        for i in 0..MAX_TODOS {
            list.add(&format!("todo {i}"));
        }
        assert!(!list.add("blocked"));

        assert!(list.delete(1));
        assert!(list.add("now it fits"));
        assert_eq!(list.len(), MAX_TODOS);
    }
    #[test]
    fn add_rejects_text_longer_than_the_limit() {
        let mut list = TodoList::default();

        let at_limit = "a".repeat(MAX_TEXT_LEN);
        assert!(list.add(&at_limit), "exactly at the limit must be allowed");

        let over_limit = "a".repeat(MAX_TEXT_LEN + 1);
        assert!(!list.add(&over_limit), "one char over must be rejected");
        assert_eq!(list.len(), 1);
    }
    #[test]
    fn the_length_limit_is_measured_after_trimming() {
        let mut list = TodoList::default();

        let padded = format!("{}   ", "a".repeat(MAX_TEXT_LEN));
        assert!(list.add(&padded));
        assert_eq!(list.items[0].text.len(), MAX_TEXT_LEN);
    }
    #[test]
    fn the_template_advertises_the_same_limit_it_enforces() {
        let panel = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/todo_panel.html"
        ))
        .expect("panel template readable");

        assert!(
            panel.contains(&format!("maxlength=\"{MAX_TEXT_LEN}\"")),
            "input must carry maxlength={MAX_TEXT_LEN} so the browser agrees \
             with the server"
        );
    }

    #[test]
    fn a_stale_request_against_a_deleted_id_does_not_touch_another_todo() {
        let mut list = TodoList::default();
        list.add("a");
        list.add("b");
        list.delete(2);

        // The id is gone and must never be reissued.
        assert!(!list.toggle(2));
        assert!(!list.delete(2));

        list.add("c");
        let ids: Vec<u64> = list.items.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 3]);
        assert!(list.items.iter().all(|t| !t.done));
    }

    #[test]
    fn toggle_flips_state_and_reports_unknown_ids() {
        let mut list = TodoList::default();
        list.add("a");

        assert!(list.toggle(1));
        assert!(list.items[0].done);

        assert!(list.toggle(1));
        assert!(!list.items[0].done);

        assert!(!list.toggle(9999));
    }

    #[test]
    fn delete_removes_and_reports_unknown_ids() {
        let mut list = TodoList::default();
        list.add("a");
        list.add("b");

        assert!(list.delete(1));
        assert_eq!(list.len(), 1);
        assert_eq!(list.items[0].text, "b");

        assert!(!list.delete(1));
    }

    #[test]
    fn ids_are_never_reused_after_delete() {
        let mut list = TodoList::default();
        list.add("a");
        list.add("b");
        list.delete(2);

        list.add("c");
        let ids: Vec<u64> = list.items.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn sorted_puts_completed_below_active_within_order() {
        let mut list = TodoList::default();
        list.add("one");
        list.add("two");
        list.add("three");
        list.add("four");

        // Two completed, so the completed group's own order is exercised too.
        list.toggle(1);
        list.toggle(2);

        let sorted = list.sorted();
        let order: Vec<&str> = sorted.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(order, vec!["three", "four", "one", "two"]);
    }

    #[test]
    fn sorted_does_not_mutate_the_stored_order() {
        let mut list = TodoList::default();
        list.add("one");
        list.add("two");
        list.toggle(1);

        list.sorted();

        // Ordering is a render-time concern; storage order is untouched.
        let stored: Vec<&str> = list.items.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(stored, vec!["one", "two"]);
    }

    #[test]
    fn sorted_with_nothing_is_empty() {
        let list = TodoList::default();
        assert!(list.sorted().is_empty());
    }

    async fn get_index(store: &TodoStore) -> String {
        let app = build_router(store.clone());
        let res = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        String::from_utf8(
            to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn index_renders_the_page() {
        let store = new_store();
        let body = get_index(&store).await;

        assert!(body.contains("<title>Todos</title>"));
        assert!(body.contains("todo-panel"));
        assert!(body.contains("todo-list"));
    }

    #[tokio::test]
    async fn index_lists_a_seeded_todo() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let body = get_index(&store).await;

        assert!(body.contains("buy milk"));
    }

    /// The toggle checkbox must swap `#todo-list`, never `#todo-panel`.
    /// Panel-wide swaps would re-render the add input and discard text the
    /// user typed but had not yet submitted.
    #[tokio::test]
    async fn toggle_targets_the_list_not_the_panel() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let body = get_index(&store).await;

        // The attributes sit on separate lines in the markup, so slice the
        // whole rendered `<input>` element rather than one line of it. Match
        // on `todo-toggle`, not `<input`: the add form's input comes first.
        let checkbox = body
            .split_once("todo-toggle")
            .expect("checkbox rendered")
            .1
            .split_once('>')
            .expect("input tag closed")
            .0;

        assert!(checkbox.contains("hx-target=\"#todo-list\""));
        assert!(!checkbox.contains("#todo-panel"));
    }

    #[tokio::test]
    async fn index_escapes_html_in_todo_text() {
        let store = new_store();
        store.write().unwrap().add("<script>alert(1)</script>");

        let body = get_index(&store).await;

        // Askama escapes angle brackets as numeric references (&#60; &#62;),
        // not as the named entities &lt; &gt;. Either encoding is safe; what
        // matters is that no raw tag reaches the browser.
        let text_row = body
            .lines()
            .find(|line| line.contains("class=\"todo-text\""))
            .expect("todo text rendered");

        assert!(text_row.contains("&#60;script&#62;"));
        assert!(!text_row.contains("<script>"));
    }

    #[tokio::test]
    async fn index_shows_the_empty_state_when_there_are_no_todos() {
        let store = new_store();
        let body = get_index(&store).await;

        assert!(body.contains("todo-empty"));
        assert!(body.contains("Nothing on the list"));
    }

    #[tokio::test]
    async fn index_shows_the_open_count() {
        let store = new_store();
        store.write().unwrap().add("a");
        store.write().unwrap().add("b");
        store.write().unwrap().toggle(1);

        let body = get_index(&store).await;

        assert!(body.contains("1 open"));
    }

    async fn post_todo(store: &TodoStore, body: &'static str) -> (StatusCode, String) {
        let app = build_router(store.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/todos")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let text = String::from_utf8(
            to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        (status, text)
    }

    async fn get_to(store: &TodoStore, uri: &str) -> (StatusCode, String) {
        let app = build_router(store.clone());
        let res = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let text = String::from_utf8(
            to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        (status, text)
    }

    #[tokio::test]
    async fn create_appends_a_todo() {
        let store = new_store();
        let (status, body) = post_todo(&store, "text=buy+milk").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("buy milk"));
        assert_eq!(store.read().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn create_returns_the_panel_not_a_full_page() {
        let store = new_store();
        let (_, body) = post_todo(&store, "text=buy+milk").await;

        // A fragment, not a document — no <html>, no <head>.
        assert!(!body.contains("<!DOCTYPE html>"));
        assert!(body.contains("todo-panel"));
    }

    #[tokio::test]
    async fn create_rerenders_the_input_empty_so_the_box_clears() {
        let store = new_store();
        let (_, body) = post_todo(&store, "text=buy+milk").await;

        // The input carries no value attribute, so the swapped-in form is
        // blank. Anchor on the tag itself rather than the class, so a
        // `value` written before `class="todo-input"` is caught too.
        let input = body
            .split_once("<input")
            .expect("input rendered")
            .1
            .split_once('>')
            .expect("input tag closed")
            .0;
        assert!(!input.contains("value="));
    }

    #[tokio::test]
    async fn create_rejects_blank_text_without_adding() {
        let store = new_store();
        let (status, body) = post_todo(&store, "text=%20%20%20").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(store.read().unwrap().len(), 0);
        assert!(!body.contains("%20"));
    }

    #[tokio::test]
    async fn create_returns_a_422_when_the_body_cannot_be_parsed() {
        let store = new_store();
        let app = build_router(store.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/todos")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("no_text_field=1"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_escapes_html_in_the_todo_text() {
        let store = new_store();
        let (_, body) = post_todo(&store, "text=%3Cscript%3Ealert(1)%3C%2Fscript%3E").await;

        let text_row = body
            .lines()
            .find(|line| line.contains("class=\"todo-text\""))
            .expect("todo text rendered");

        assert!(text_row.contains("&#60;script&#62;"));
        assert!(!text_row.contains("<script>"));
    }

    async fn post_to(store: &TodoStore, uri: &str) -> (StatusCode, String) {
        let app = build_router(store.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let text = String::from_utf8(
            to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        (status, text)
    }

    #[tokio::test]
    async fn toggle_marks_a_todo_done() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let (status, body) = post_to(&store, "/todos/1/toggle").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("todo-row--done"));
        assert!(body.contains("checked"));
    }

    #[tokio::test]
    async fn toggle_returns_only_the_list_fragment() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let (_, body) = post_to(&store, "/todos/1/toggle").await;

        // No add form — that is the point of the second swap target.
        assert!(!body.contains("<!DOCTYPE html>"));
        assert!(!body.contains("todo-input"));
        assert!(body.contains("todo-list"));
    }

    /// A poisoned lock is a server fault, not a crash. The response must be a
    /// 500 whose body says nothing about why, so the cause lands in the log
    /// where it belongs.
    #[tokio::test]
    async fn a_poisoned_lock_yields_a_500_not_a_panic() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        // Poison it: panic while the guard is alive.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.write().unwrap();
            panic!("poison the lock");
        }));

        let (status, body) = get_to(&store, "/").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            !body.contains("poison") && !body.contains("panicked"),
            "the response must not leak internals: {body}"
        );
    }

    /// Every handler must survive a poisoned lock, not just the read path.
    #[tokio::test]
    async fn a_poisoned_lock_yields_a_500_on_every_route() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.write().unwrap();
            panic!("poison the lock");
        }));

        let (read_status, _) = get_to(&store, "/").await;
        assert_eq!(
            read_status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "GET / must not panic on a poisoned lock"
        );

        for uri in ["/todos/1/toggle", "/todos/1/delete"] {
            let (status, _) = post_to(&store, uri).await;
            assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "{uri} must not panic on a poisoned lock"
            );
        }
    }

    /// An unmatched path is a client error, and gets a real 404 — not axum's
    /// bare empty response.
    #[tokio::test]
    async fn an_unknown_path_returns_404() {
        let store = new_store();
        let (status, _) = get_to(&store, "/no-such-route").await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// `GET /todos/1/toggle` is a real route with the wrong verb, so it is a
    /// 405 rather than a 404 — and must not fall through to the 404 handler.
    #[tokio::test]
    async fn a_wrong_method_on_a_known_path_returns_405() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let (status, _) = get_to(&store, "/todos/1/toggle").await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn toggle_twice_returns_to_undone() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        post_to(&store, "/todos/1/toggle").await;
        let (status, body) = post_to(&store, "/todos/1/toggle").await;

        // The status matters: without it the negative assertion below would
        // also hold against a 404 from an unregistered route.
        assert_eq!(status, StatusCode::OK);
        assert!(!body.contains("todo-row--done"));
    }

    #[tokio::test]
    async fn delete_removes_a_todo() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let (status, body) = post_to(&store, "/todos/1/delete").await;

        assert_eq!(status, StatusCode::OK);
        assert!(!body.contains("buy milk"));
        assert!(body.contains("todo-empty"));
        assert_eq!(store.read().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn unknown_id_is_404_with_the_current_list_as_the_body() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let (status, body) = post_to(&store, "/todos/9999/toggle").await;

        // Honest status. htmx 4 swaps 4xx by default, so the list renders unchanged.
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("buy milk"));
    }

    #[tokio::test]
    async fn unknown_id_delete_is_404_and_leaves_state_alone() {
        let store = new_store();
        store.write().unwrap().add("buy milk");

        let (status, _) = post_to(&store, "/todos/9999/delete").await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(store.read().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn non_numeric_id_is_rejected_by_the_path_extractor() {
        let store = new_store();
        let app = build_router(store);
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/todos/abc/toggle")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn completed_sorts_below_active_in_the_rendered_fragment() {
        let store = new_store();
        store.write().unwrap().add("one");
        store.write().unwrap().add("two");
        store.write().unwrap().add("three");

        let (_, body) = post_to(&store, "/todos/1/toggle").await;

        let one = body.find(">one<").expect("one rendered");
        let two = body.find(">two<").expect("two rendered");
        let three = body.find(">three<").expect("three rendered");
        assert!(two < one, "two should precede the completed one");
        assert!(three < one, "the completed one should sort last");
    }
}
