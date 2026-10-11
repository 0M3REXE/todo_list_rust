//! # Data access — every SQL statement in the app
//!
//! Five functions, one query each. Values are always `.bind()`-ed rather than
//! interpolated, so injection payloads arrive as literal text. Mutations return
//! `bool` from `rows_affected()`, which is how handlers pick 200 vs 404.

use sqlx::PgPool;

use super::{AppError, Todo};

// The table also carries a CHECK constraint on the trimmed length. Validating
// here first is what keeps a blank or over-long todo a 200 with an unchanged
// panel instead of a constraint violation surfacing as a 500.
pub const MAX_TEXT_LEN: usize = 280;

/// Returns the trimmed text, or `None` when it is blank or over-long. Mirrors
/// the `todos_text_length` CHECK constraint so the common cases never reach
/// the database.
fn validated(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_TEXT_LEN {
        return None;
    }
    Some(text)
}

/// Active todos first, completed after, insertion order within each group.
/// `ORDER BY done` puts false before true, which is the SQL equivalent of the
/// old in-memory `sort_by_key(done)`, and `id` is monotonic so it stands in for
/// insertion order.
pub async fn sorted(pool: &PgPool) -> Result<Vec<Todo>, AppError> {
    let todos =
        sqlx::query_as::<_, Todo>("SELECT id, text, done FROM todos ORDER BY done ASC, id ASC")
            .fetch_all(pool)
            .await?;
    Ok(todos)
}

pub async fn open_count(pool: &PgPool) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM todos WHERE NOT done")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

/// Inserts unless the text is rejected. Returns false when nothing was written.
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

/// Flips `done`. Returns false when no todo has this id.
pub async fn flip_done(pool: &PgPool, id: i64) -> Result<bool, AppError> {
    let result = sqlx::query("UPDATE todos SET done = NOT done WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Removes the item. Returns false when no todo has this id.
pub async fn remove(pool: &PgPool, id: i64) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM todos WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
