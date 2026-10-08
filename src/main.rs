use askama::Template;
use axum::{
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// --- Error handling ---
// A single error type every handler can return. Converts any Askama render
// failure into a clean 500 instead of panicking the request.

struct AppError(askama::Error);

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

// --- Templates ---
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

#[derive(Template)]
#[template(path = "hello.html")]
struct HelloTemplate<'a> {
    name: &'a str,
}

#[derive(Template)]
#[template(path = "goodbye.html")]
struct GoodByeTemplate<'a> {
    name: &'a str,
}

// --- Query params ---
#[derive(Deserialize)]
struct NameQuery {
    name: Option<String>,
}

// --- Handlers ---
async fn index() -> Result<Html<String>, AppError> {
    Ok(Html(IndexTemplate.render()?))
}

async fn hello(Query(params): Query<NameQuery>) -> Result<Html<String>, AppError> {
    let name = params.name.as_deref().unwrap_or("World");
    Ok(Html(HelloTemplate { name }.render()?))
}

async fn goodbye(Query(params): Query<NameQuery>) -> Result<Html<String>, AppError> {
    let name = params.name.as_deref().unwrap_or("World");
    Ok(Html(GoodByeTemplate { name }.render()?))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app = Router::new()
        .route("/", get(index))
        .route("/hello", get(hello))
        .route("/goodbye", get(goodbye))
        .nest_service("/static", ServeDir::new("static"))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind to port 3000");

    tracing::info!("listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app)
        .await
        .expect("server error");
}
