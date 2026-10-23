//! # The executable — wiring only
//!
//! Real work lives in the `practice` library. This loads `.env`, sets up
//! tracing, opens the database, applies migrations, builds the router, and
//! serves. The router itself comes from `app::build_app` so that the test suite
//! drives the same routes, session layer and login guard that `cargo run` does.
//!
//! Middleware order is deliberate: the last `.layer()` call is outermost, so
//! CatchPanic sees a panic first and Compression wraps the final response.
//! The session and auth layers are already inside `build_app`, so they sit
//! *below* everything applied here — see the diagram in `src/app.rs`.

use practice::{app, db};
use std::time::Duration;

use axum::http::{
    HeaderValue, StatusCode,
    header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS},
};
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer,
    set_header::SetResponseHeaderLayer, timeout::TimeoutLayer, trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let addr = std::env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let pool = connect_db().await;
    let config = app::AppConfig::from_env();

    // Started before the listener so a boot-time database problem is caught
    // here rather than on the first request that needs a session.
    let cleanup = app::spawn_session_cleanup(practice::auth::DbStore::new(pool.clone()));

    let app = app::build_app(pool, config)
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; \
                 script-src 'self'; \
                 style-src 'self' https://fonts.googleapis.com; \
                 font-src 'self' https://fonts.gstatic.com; \
                 img-src 'self' data:; \
                 frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ))
        .layer(CatchPanicLayer::new());

    // Nothing below this point can recover, so bind and serve failures log the
    // cause and exit non-zero. A bare `.expect` would print the panic and bury
    // the real error in a backtrace; the io::Error carries the useful detail
    // (address in use, permission denied).
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(%err, %addr, "failed to bind");
            std::process::exit(1);
        }
    };

    match listener.local_addr() {
        Ok(local) => tracing::info!(%local, "listening"),
        Err(err) => tracing::warn!(?err, "listening, but local_addr failed"),
    }

    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;

    // The listener has closed; nothing else can arrive, so the cleanup task can
    // be told to finish instead of being killed with the process.
    cleanup.stop().await;

    if let Err(err) = served {
        tracing::error!(?err, "server error");
        std::process::exit(1);
    }
}

/// Opens the database named in the environment and applies any pending
/// migrations.
///
/// Migrations are embedded (`src/db.rs`), so a deploy needs no separate
/// migration step. A failure here is fatal: serving reads against a schema the
/// app does not recognise would fail later, less clearly.
async fn connect_db() -> practice::db::Db {
    match db::open_from_env().await {
        Ok(db) => {
            tracing::info!("database ready");
            db
        }
        Err(err) => {
            tracing::error!(?err, "could not open the database");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(?err, "failed to install ctrl-c handler");
        std::future::pending::<()>().await;
    }
    tracing::info!("shutdown signal received, draining");
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let terminate = async {
        match signal(SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!(?err, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate => {}
    }

    tracing::info!("shutdown signal received, draining");
}
