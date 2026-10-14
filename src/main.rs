//! # The executable — wiring only
//!
//! Real work lives in the `practice` library. This loads `.env`, sets up
//! tracing, opens the pool, applies migrations, builds the router, and serves.
//! The router itself comes from `app::build_app` so that the test suite drives
//! the same routes, session layer and login guard that `cargo run` does.
//!
//! Middleware order is deliberate: the last `.layer()` call is outermost, so
//! CatchPanic sees a panic first and Compression wraps the final response.
//! The session and auth layers are already inside `build_app`, so they sit
//! *below* everything applied here — see the diagram in `src/app.rs`.

use practice::{app, auth::PostgresStore};
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
    // sqlx's macros read .env at compile time; the running process still needs
    // it, so load the same file here rather than requiring an exported variable.
    dotenvy::dotenv().ok();

    let addr = std::env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let pool = connect_pool().await;
    let config = app::AppConfig::from_env();

    // A misconfigured mailer is fatal here rather than at the first reset
    // request: SMTP_HOST set but MAIL_FROM missing must not silently fall back
    // to printing reset links into a production log.
    let mailer = match practice::mailer::from_env() {
        Ok(mailer) => mailer,
        Err(reason) => {
            tracing::error!(%reason, "mail configuration is unusable");
            std::process::exit(1);
        }
    };

    // Started before the listener so a boot-time database problem is caught
    // here rather than on the first request that needs a session.
    let cleanup = app::spawn_session_cleanup(PostgresStore::new(pool.clone()));

    let app = app::build_app_with(pool, config, mailer)
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

/// Opens the pool and applies any pending migrations.
///
/// `sqlx::migrate!` embeds `migrations/` at compile time, so a deploy needs no
/// separate migration step. A failure here is fatal: serving reads against a
/// schema the app does not recognise would fail later, less clearly.
async fn connect_pool() -> sqlx::PgPool {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            tracing::error!("DATABASE_URL is not set");
            std::process::exit(1);
        }
    };

    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(?err, "could not connect to the database");
            std::process::exit(1);
        }
    };

    if let Err(err) = sqlx::migrate!().run(&pool).await {
        tracing::error!(?err, "migrations failed");
        std::process::exit(1);
    }

    tracing::info!("database ready");
    pool
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
