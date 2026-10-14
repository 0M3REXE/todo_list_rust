//! # One router, shared by the server and the tests
//!
//! `main.rs` used to build the router itself, which meant the auth and session
//! layers would have been added there and therefore absent from every test.
//! Building it here instead means `cargo test` drives the same session cookies,
//! the same login guard and the same CSRF check that `cargo run` does.
//!
//! ## Where the session layer sits
//!
//! Within this function, `.layer()` calls are applied last-outermost, the same
//! rule as everywhere else in axum:
//!
//! ```text
//!   require_auth   (only the todo routes)
//!   verify_origin  (everywhere)
//!   auth layer     = cookie manager → session manager → AuthManager
//! ```
//!
//! The auth layer must be outermost of the three because it is what puts
//! `AuthSession` into the request extensions that `require_auth` reads. It sits
//! *inside* `CatchPanic`, which `main.rs` still applies on top, so a panic in
//! a handler is still caught and turned into a 500.

use std::time::Duration as StdDuration;

use axum::{
    Router,
    http::HeaderValue,
    middleware,
    routing::{any_service, get},
};
use axum_login::{
    AuthManagerLayerBuilder,
    tower_sessions::{ExpiredDeletion, Expiry, SessionManagerLayer, cookie::SameSite},
};
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

use crate::auth::{self, AuthBackend, PostgresStore};
use crate::mailer;

/// How long a session may sit untouched before it stops working, and whether
/// the cookie is allowed to leave a plain-http connection.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// Must be `true` anywhere that is not localhost. A `Secure` cookie is
    /// never sent over plain http, so setting this wrongly on a real
    /// deployment silently logs everyone out.
    pub session_secure: bool,
    /// The `host[:port]` that state-changing requests are allowed to come
    /// from. See `auth::middleware::verify_origin`.
    pub allowed_host: String,
    /// The public origin, used to build the links in verification and reset
    /// emails. Getting this wrong sends working links to a host nobody can
    /// reach, so it must match where the app is actually served.
    pub base_url: String,
    /// When true, an account with an unconfirmed address may sign in and sees a
    /// banner, but may not touch the todo list.
    ///
    /// Off by default: on a local database nobody can receive the mail, and
    /// turning it on would lock the developer out of their own app.
    pub require_email_verification: bool,
    /// How long a verification or reset link stays valid.
    pub verify_ttl: time::Duration,
    pub reset_ttl: time::Duration,
    /// Shortest gap between two emails to the same address, so the resend
    /// button cannot be used to flood someone's inbox.
    pub resend_cooldown: time::Duration,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            // Secure by default: forgetting to set it should fail closed.
            session_secure: true,
            allowed_host: "localhost:3000".to_owned(),
            base_url: "http://localhost:3000".to_owned(),
            require_email_verification: false,
            verify_ttl: time::Duration::hours(24),
            reset_ttl: time::Duration::hours(1),
            resend_cooldown: time::Duration::minutes(5),
        }
    }
}

impl AppConfig {
    /// Reads the settings from the environment, falling back to the defaults.
    pub fn from_env() -> Self {
        let defaults = Self::default();

        let session_secure = std::env::var("SESSION_SECURE")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(defaults.session_secure);

        let allowed_host =
            std::env::var("APP_ALLOWED_HOST").unwrap_or(defaults.allowed_host.clone());

        // The base URL defaults to whatever the allowed host implies, so the
        // usual local setup needs only APP_ALLOWED_HOST.
        let base_url =
            std::env::var("APP_BASE_URL").unwrap_or_else(|_| format!("http://{allowed_host}"));

        let require_email_verification = std::env::var("REQUIRE_EMAIL_VERIFICATION")
            .map(|raw| raw.eq_ignore_ascii_case("true") || raw == "1")
            .unwrap_or(defaults.require_email_verification);

        Self {
            session_secure,
            allowed_host,
            base_url,
            require_email_verification,
            ..defaults
        }
    }
}

/// A session lives a week of inactivity. Long enough that a returning visitor
/// is not asked to log in daily, short enough that a forgotten laptop does not
/// keep a live session for months.
const SESSION_IDLE_LIFETIME: time::Duration = time::Duration::days(7);

/// How often expired session rows are swept up. Hourly is plenty — an expired
/// row is already unusable, this is only about not letting the table grow
/// forever.
const CLEANUP_PERIOD: StdDuration = StdDuration::from_secs(60 * 60);

/// Builds the whole application: routes, session layer, auth layer, CSRF check.
///
/// Uses the development mailer. Tests and anything that needs to read the
/// messages that were "sent" use [`build_app_with`] instead.
pub fn build_app(pool: PgPool, config: AppConfig) -> Router {
    build_app_with(pool, config, std::sync::Arc::new(mailer::LogMailer::new()))
}

/// [`build_app`], with the mailer supplied.
///
/// The mailer is a request extension rather than part of the router state so
/// that `State` can stay `PgPool` everywhere — otherwise every handler in the
/// app would take a struct it does not otherwise need.
pub fn build_app_with(
    pool: PgPool,
    config: AppConfig,
    mailer: std::sync::Arc<dyn mailer::Mailer>,
) -> Router {
    let session_layer = SessionManagerLayer::new(PostgresStore::new(pool.clone()))
        .with_name("session")
        // No JavaScript on this app reads the cookie, so nothing needs it.
        .with_http_only(true)
        // Lax, not Strict: Lax still blocks the cross-site POST that CSRF
        // depends on, while allowing a normal link from another site to arrive
        // logged in.
        .with_same_site(SameSite::Lax)
        .with_secure(config.session_secure)
        .with_expiry(Expiry::OnInactivity(SESSION_IDLE_LIFETIME));

    let auth_layer =
        AuthManagerLayerBuilder::new(AuthBackend { pool: pool.clone() }, session_layer)
            // Named rather than left to axum-login's default, because the
            // session store reads the account id back out from under this key.
            .with_data_key(crate::auth::AUTH_DATA_KEY)
            .build();

    // Scoped to ServeDir on purpose: on the whole app it would mark the HTML
    // immutable too.
    let static_files = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(ServeDir::new("static"));

    Router::new()
        // Public first, then the guarded todo routes. `route_layer` inside
        // `todo::build_router` is what keeps the guard off the auth pages.
        .merge(auth::routes(pool.clone()))
        .merge(auth::link::routes(pool.clone()))
        .merge(auth::link::session_routes(pool.clone()))
        .merge(auth::account_routes(pool.clone()))
        .merge(crate::todo::build_router(pool))
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/static", any_service(static_files))
        .layer(middleware::from_fn_with_state(
            config.allowed_host.clone(),
            auth::verify_origin,
        ))
        .layer(axum::Extension(mailer))
        .layer(axum::Extension(std::sync::Arc::new(config)))
        .layer(auth_layer)
}

/// Deletes expired session rows on a timer, until told to stop.
///
/// The handle is returned so `main.rs` can stop it during graceful shutdown
/// instead of leaving a task running after the listener has closed.
pub fn spawn_session_cleanup(store: PostgresStore) -> SessionCleanup {
    let (stop, mut stopped) = tokio::sync::watch::channel(false);

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CLEANUP_PERIOD);
        // The first tick fires straight away; skip it so startup does no work.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(err) = store.delete_expired().await {
                        // Not fatal: the rows are already unusable, they are
                        // only taking up space.
                        tracing::warn!(%err, "expired session cleanup failed");
                    }
                }
                _ = stopped.changed() => break,
            }
        }

        tracing::info!("session cleanup stopped");
    });

    SessionCleanup { handle, stop }
}

/// Handle for the cleanup task. Awaiting it finishes the task.
pub struct SessionCleanup {
    handle: tokio::task::JoinHandle<()>,
    stop: tokio::sync::watch::Sender<bool>,
}

impl SessionCleanup {
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        if let Err(err) = self.handle.await {
            tracing::error!(?err, "session cleanup task did not finish cleanly");
        }
    }
}
