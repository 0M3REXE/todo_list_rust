//! # Error handling — 1 type, 2 variants
//!
//! `AppError` wraps `askama::Error` and `sqlx::Error`. Its `IntoResponse` impl
//! is what turns a failure into a 500 without matching in every handler; the
//! two `From` impls are what make `?` work. The cause is logged, never sent to
//! the client.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug)]
pub enum AppError {
    Render(askama::Error),
    Database(sqlx::Error),
}

impl AppError {
    fn into_response_inner(self) -> Response {
        match &self {
            Self::Render(err) => tracing::error!(?err, "template render failed"),
            Self::Database(err) => tracing::error!(?err, "database query failed"),
        }
        (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong").into_response()
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(err) => write!(f, "template render failed: {err}"),
            Self::Database(err) => write!(f, "database query failed: {err}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Render(err) => Some(err),
            Self::Database(err) => Some(err),
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

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}
