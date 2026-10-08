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
// One error type every handler returns. Converts any Askama render failure
// into a clean 500 instead of panicking the request.

pub struct AppError(askama::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = ?self.0, "template render failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong").into_response()
    }
}

impl From<askama::Error> for AppError {
    fn from(err: askama::Error) -> Self {
        AppError(err)
    }
}

// --- Model ---

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
    /// Trims the text and pushes it. Returns false and changes nothing
    /// when the text is blank — `<input required>` already blocks that in
    /// the browser, so this is defense-in-depth.
    pub fn add(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
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
    /// insertion order survives inside each group.
    pub fn sorted(&self) -> Vec<&Todo> {
        let mut items: Vec<&Todo> = self.items.iter().collect();
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
fn open_count(todos: &[&Todo]) -> usize {
    todos.iter().filter(|todo| !todo.done).count()
}

// --- Templates ---

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    todos: Vec<&'a Todo>,
}

#[derive(Template)]
#[template(path = "todo_panel.html")]
struct TodoPanelTemplate<'a> {
    todos: Vec<&'a Todo>,
}

#[derive(Template)]
#[template(path = "todo_list.html")]
struct TodoListTemplate<'a> {
    todos: Vec<&'a Todo>,
}

// --- Handlers ---

async fn index(State(store): State<TodoStore>) -> Result<Html<String>, AppError> {
    let list = store.read().expect("lock poisoned");
    let todos = list.sorted();
    Ok(Html(IndexTemplate { todos }.render()?))
}

pub fn build_router(store: TodoStore) -> Router {
    Router::new().route("/", get(index)).with_state(store)
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

        let order: Vec<&str> = list.sorted().iter().map(|t| t.text.as_str()).collect();
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
}
