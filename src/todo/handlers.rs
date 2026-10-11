//! # HTTP layer — 4 handlers + router
//!
//! Handlers take axum extractors (`State`, `Path`, `Form`) and return
//! `Result<_, AppError>`. `Path(id): Path<i64>` rejects a non-numeric id before
//! any SQL runs. Three of the four return fragments, not pages — htmx swaps
//! them into the page.

use askama::Template as _;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;

use super::{
    AppError, TodoStore,
    store::{add, flip_done, open_count, remove, sorted},
    templates::{IndexTemplate, ListTemplate, PanelTemplate},
};

async fn index(State(pool): State<TodoStore>) -> Result<Html<String>, AppError> {
    let (todos, open_count) = (sorted(&pool).await?, open_count(&pool).await?);
    Ok(Html(IndexTemplate { todos, open_count }.render()?))
}

#[derive(serde::Deserialize)]
struct NewTodo {
    text: String,
}

async fn create(
    State(pool): State<TodoStore>,
    axum::extract::Form(form): axum::extract::Form<NewTodo>,
) -> Result<Html<String>, AppError> {
    add(&pool, &form.text).await?;
    let (todos, open_count) = (sorted(&pool).await?, open_count(&pool).await?);
    Ok(Html(PanelTemplate { todos, open_count }.render()?))
}

/// Runs `mutate`, then renders the list fragment. `mutate` returns false when
/// the id was not found, which becomes a 404 whose body is still the current
/// list.
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

async fn toggle(State(pool): State<TodoStore>, Path(id): Path<i64>) -> Result<Response, AppError> {
    with_list(&pool, flip_done(&pool, id)).await
}

async fn delete(State(pool): State<TodoStore>, Path(id): Path<i64>) -> Result<Response, AppError> {
    with_list(&pool, remove(&pool, id)).await
}

pub fn build_router(pool: TodoStore) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/todos", axum::routing::post(create))
        .route("/todos/{id}/toggle", axum::routing::post(toggle))
        .route("/todos/{id}/delete", axum::routing::post(delete))
        .with_state(pool)
}
