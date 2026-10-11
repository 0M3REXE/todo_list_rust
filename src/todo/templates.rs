//! # Template structs — 3, one per template
//!
//! Each field becomes a variable in the matching `.html`. Askama compiles the
//! template at build time, so a wrong field is a build error — and editing a
//! template requires a rebuild before the change appears.

use askama::Template;

use super::Todo;

#[derive(Template)]
#[template(path = "index.html")]
pub(crate) struct IndexTemplate {
    pub(crate) todos: Vec<Todo>,
    pub(crate) open_count: i64,
}

#[derive(Template)]
#[template(path = "todo_panel.html")]
pub(crate) struct PanelTemplate {
    pub(crate) todos: Vec<Todo>,
    pub(crate) open_count: i64,
}

#[derive(Template)]
#[template(path = "todo_list.html")]
pub(crate) struct ListTemplate {
    pub(crate) todos: Vec<Todo>,
}
