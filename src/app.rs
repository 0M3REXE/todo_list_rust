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
//! ## Embedded static assets
//!
//! The CSS and JS ship *inside* the binary rather than from disk. A serverless
//! (Vercel) function carries no asset directory with it, and a `ServeDir`
//! lookup would 404 there; embedding keeps local and deployed behaviour
//! identical. Templates are already compiled in by askama for the same reason.

use std::time::Duration as StdDuration;
use std::time::Duration;

use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_login::{
    AuthManagerLayerBuilder,
    tower_sessions::{Expiry, SessionManagerLayer, cookie::SameSite, ExpiredDeletion},
};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::auth::{self, AuthBackend, AuthSession, DbStore};
use crate::db::Db;
use crate::AppError;

use askama::Template as _;

/// The app stylesheet.
pub static APP_CSS: &str = include_str!("../static/app.css");
/// The small in-house script for the password fields.
pub static PASSWORD_TOGGLE_JS: &str = include_str!("../static/password-toggle.js");
/// Theme toggle: light/dark mode persistence.
pub static THEME_TOGGLE_JS: &str = include_str!("../static/theme-toggle.js");
/// Keyboard shortcuts + client-side search filter.
pub static KEYBOARD_JS: &str = include_str!("../static/keyboard.js");
/// Service worker for offline PWA support.
pub static SERVICE_WORKER_JS: &str = include_str!("../static/service-worker.js");
/// A safe, data-free page shown when an installed app opens without a network.
pub static OFFLINE_HTML: &str = include_str!("../static/offline.html");
/// Web app manifest for PWA installation.
pub static MANIFEST_JSON: &str = include_str!("../static/manifest.json");
/// Icons referenced by the PWA manifest.
pub static ICON_192_SVG: &str = include_str!("../static/icons/icon-192x192.svg");
pub static ICON_512_SVG: &str = include_str!("../static/icons/icon-512x512.svg");
/// Vendored htmx, unmodified.
pub static HTMX_JS: &str = include_str!("../static/vendor/htmx.min.js");

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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            // Secure by default: forgetting to set it should fail closed.
            session_secure: true,
            allowed_host: "localhost:3000".to_owned(),
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

        Self {
            session_secure,
            allowed_host,
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
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REQUEST_BODY: usize = 16 * 1024;

/// Serves an embedded asset with a long cache life. These paths are stable
/// filenames rather than content-hashed, but the content is fixed per build, so
/// "immutable" is still honest: a new build arrives as a new deployment.
fn asset(body: &'static str, content_type: &'static str) -> axum::response::Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response()
}

/// The stylesheet keeps a stable URL so new deployments must revalidate it.
fn stylesheet() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "public, max-age=0, must-revalidate"),
        ],
        APP_CSS,
    )
        .into_response()
}

/// Sets safe cache headers on dynamic responses.
///
/// Static assets already set their own long-lived immutable headers via
/// `asset()`, so this middleware only touches responses that haven't
/// specified a cache policy. User-specific HTML must never be stored by
/// shared caches (CDNs, proxies), so we mark it `private`. `no-cache` means
/// the browser can keep a copy but must revalidate with the server before
/// using it — which, for a todo list that changes constantly, is the right
/// tradeoff between speed and freshness.
async fn cache_control(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    if response.headers().get(header::CACHE_CONTROL).is_none() {
        let _ = response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, no-cache, max-age=0"),
        );
    }
    response
}

/// The landing page. Public — no session required. A logged-in visitor is
/// sent straight to their todo app; everyone else sees the marketing page
/// with login and signup buttons.
async fn landing(session: AuthSession) -> Result<Response, AppError> {
    if session.user.is_some() {
        return Ok(Redirect::to("/app").into_response());
    }
    Ok(Html(
        crate::auth::LandingTemplate::default()
            .render()
            .map_err(|err| AppError::Render(err))?,
    )
    .into_response())
}

/// Vercel invokes this daily. The bearer secret must be configured in the
/// deployment environment as `CRON_SECRET`.
async fn cleanup_expired_sessions(
    State(db): State<Db>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let Ok(secret) = std::env::var("CRON_SECRET") else {
        tracing::error!("CRON_SECRET is not configured");
        return Ok(StatusCode::SERVICE_UNAVAILABLE);
    };
    let expected = format!("Bearer {secret}");
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return Ok(StatusCode::UNAUTHORIZED);
    }

    DbStore::new(db)
        .delete_expired()
        .await
        .map_err(|err| AppError::Session(err.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Builds the whole application: routes, session layer, auth layer, CSRF check.
pub fn build_app(db: Db, config: AppConfig) -> Router {
    let session_layer = SessionManagerLayer::new(DbStore::new(db.clone()))
        .with_name("session")
        // No JavaScript on this app reads the cookie, so nothing needs it.
        .with_http_only(true)
        // Lax, not Strict: Lax still blocks the cross-site POST that CSRF
        // depends on, while allowing a normal link from another site to arrive
        // logged in.
        .with_same_site(SameSite::Lax)
        .with_secure(config.session_secure)
        .with_expiry(Expiry::OnInactivity(SESSION_IDLE_LIFETIME));

    let auth_layer = AuthManagerLayerBuilder::new(AuthBackend { db: db.clone() }, session_layer)
        // Named rather than left to axum-login's default, because the
        // session store reads the account id back out from under this key.
        .with_data_key(crate::auth::AUTH_DATA_KEY)
        .build();

    // The todo app lives under /app so the landing page can own /.
    let todo_app = crate::todo::build_router(db.clone());
    let maintenance = Router::new()
        .route("/api/maintenance/expired-sessions", get(cleanup_expired_sessions))
        .with_state(db.clone());

    Router::new()
        // Public first: landing page, then auth pages.
        .route("/", get(landing))
        .merge(auth::routes(db.clone()))
        .merge(auth::account_routes(db.clone()))
        .merge(maintenance)
        // The todo app is nested under /app so its internal routes (/,
        // /todos, /todos/{id}/toggle, /todos/{id}/delete) become
        // /app/, /app/todos, etc.
        .nest("/app", todo_app)
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/static/app.css",
            get(|| std::future::ready(stylesheet())),
        )
        .route(
            "/static/password-toggle.js",
            get(|| std::future::ready(asset(PASSWORD_TOGGLE_JS, "text/javascript"))),
        )
        .route(
            "/static/theme-toggle.js",
            get(|| std::future::ready(asset(THEME_TOGGLE_JS, "text/javascript"))),
        )
        .route(
            "/static/keyboard.js",
            get(|| std::future::ready(asset(KEYBOARD_JS, "text/javascript"))),
        )
        .route(
            "/static/service-worker.js",
            get(|| std::future::ready(asset(SERVICE_WORKER_JS, "application/javascript"))),
        )
        .route(
            "/service-worker.js",
            get(|| {
                std::future::ready((
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "application/javascript"),
                        (header::CACHE_CONTROL, "no-cache"),
                    ],
                    SERVICE_WORKER_JS,
                ))
            }),
        )
        .route(
            "/offline",
            get(|| std::future::ready(asset(OFFLINE_HTML, "text/html; charset=utf-8"))),
        )
        .route(
            "/static/manifest.json",
            get(|| std::future::ready(asset(MANIFEST_JSON, "application/json"))),
        )
        .route(
            "/static/icons/icon-192x192.svg",
            get(|| std::future::ready(asset(ICON_192_SVG, "image/svg+xml"))),
        )
        .route(
            "/static/icons/icon-512x512.svg",
            get(|| std::future::ready(asset(ICON_512_SVG, "image/svg+xml"))),
        )
        .route(
            "/static/vendor/htmx.min.js",
            get(|| std::future::ready(asset(HTMX_JS, "text/javascript"))),
        )
        // Cache control wraps every route handler. Static assets already set
        // their own headers via `asset()`, so this only touches dynamic HTML.
        .layer(middleware::from_fn(cache_control))
        .layer(middleware::from_fn_with_state(
            config.allowed_host.clone(),
            auth::verify_origin,
        ))
        .layer(auth_layer)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; \
                 base-uri 'self'; object-src 'none'; frame-ancestors 'none'; \
                 form-action 'self'; script-src 'self'; \
                 style-src 'self' https://fonts.googleapis.com; \
                 font-src 'self' https://fonts.gstatic.com; \
                 img-src 'self' data:; connect-src 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(CatchPanicLayer::new())
}

/// Deletes expired session rows on a timer, until told to stop.
///
/// The handle is returned so `main.rs` can stop it during graceful shutdown
/// instead of leaving a task running after the listener has closed. The
/// serverless entrypoint does not run this: an expired row is already unusable,
/// and a constantly-warm process is not guaranteed there.
pub fn spawn_session_cleanup(store: DbStore) -> SessionCleanup {
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
