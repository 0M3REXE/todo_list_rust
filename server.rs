//! # The serverless entrypoint — what Vercel actually runs
//!
//! `src/main.rs` is the local development server: a real TCP listener with
//! graceful shutdown. A serverless function wants neither, so this file hosts
//! the *same* router (`app::build_app`, the one the tests use) through
//! `vercel_runtime`'s adapter, which bends Vercel's request/response types
//! around axum.
//!
//! The whole app is one lambda behind a catch-all: routing — sessions, login
//! guards, htmx fragments — stays exactly what `app::build_app` declares, and a
//! Vercel routing table sends every path here. See `vercel.json` for the
//! mapping.
//!
//! Build note: this file is a second `[[bin]]` target of the `practice`
//! package, so it can import the library directly — no workspace changes, and
//! `cargo test` keeps testing the same code that gets deployed.

use tower::Layer as _;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use vercel_runtime::run;
use vercel_runtime::axum::VercelLayer;

use practice::{app, db};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Cold-start fail-fast: a database that will not open would fail every
    // request anyway, so it is clearer to fail the boot and let Vercel report
    // the startup error than to serve a half-initialised lambda.
    let database = match db::open_from_env().await {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(?err, "could not open the database on cold start");
            std::process::exit(1);
        }
    };

    let router = app::build_app(database, app::AppConfig::from_env());

    if let Err(err) = run(VercelLayer.layer(router)).await {
        tracing::error!(?err, "vercel runtime error");
        std::process::exit(1);
    }
}
