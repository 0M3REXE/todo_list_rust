use std::sync::{Arc, RwLock};

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
