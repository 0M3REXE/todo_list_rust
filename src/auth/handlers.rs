//! # Auth handlers — signup, login, logout
//!
//! These are plain HTML forms: POST, then either a 303 to a new page or the
//! same page re-rendered with a message. No htmx, because a swapped login page
//! is a confusing thing to hand someone who is not logged in.
//!
//! Every handler returns `Result<_, AppError>`; `AppError` turns a failure into
//! a 500 with the real cause logged and nothing useful leaked to the client.

use askama::Template as _;
use axum::{
    Form, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;

use super::{
    AuthBackend, AuthSession, CurrentUser, User, middleware::safe_next, password, store, templates,
};
use crate::AppError;

/// Every way a login can fail says the same thing. Naming the half that was
/// wrong — "no such email" — is enough to enumerate every account on the site.
const BAD_CREDENTIALS: &str = "Invalid email or password.";

/// How many `@` checks the email gets before we believe it.
///
/// A full RFC 5322 parser would reject addresses some mail servers accept and
/// accept some they reject, which buys nothing here. The database's length
/// constraint is the real limit.
fn validate_email(email: &str) -> Result<(), &'static str> {
    if email.chars().count() > 254 {
        return Err("Email address is too long.");
    }

    match email.split_once('@') {
        Some((local, domain))
            if !local.is_empty() && !domain.is_empty() && !domain.contains('@') =>
        {
            Ok(())
        }
        _ => Err("Enter a valid email address."),
    }
}

// --- signup ------------------------------------------------------------------

async fn signup_form() -> Result<Html<String>, AppError> {
    Ok(Html(templates::SignupTemplate::default().render()?))
}

#[derive(Deserialize)]
struct SignupForm {
    email: String,
    password: String,
    password_confirm: String,
}

async fn signup(
    State(pool): State<PgPool>,
    mut session: AuthSession,
    Form(form): Form<SignupForm>,
) -> Result<Response, AppError> {
    let email = form.email.trim().to_lowercase();

    let rejection = validate_email(&email)
        .and_then(|()| check_confirmation(&form))
        .and_then(|()| password::validate(&form.password))
        .err();

    if let Some(message) = rejection {
        return Ok(render_signup_error(&email, message));
    }

    let hash = password::hash(form.password).await?;

    let Some(user) = store::create_user(&pool, &email, &hash).await? else {
        return Ok(render_signup_error(
            &email,
            "That email is already registered.",
        ));
    };

    session
        .login(&user)
        .await
        .map_err(|err| AppError::Session(err.to_string()))?;

    tracing::info!(user.id = user.id, "account created");
    Ok(Redirect::to("/").into_response())
}

fn check_confirmation(form: &SignupForm) -> Result<(), &'static str> {
    match form.password == form.password_confirm {
        true => Ok(()),
        false => Err("Passwords do not match."),
    }
}

/// Re-renders the form with a message. The status stays 200, matching how the
/// todo panel handles a rejected todo: the page worked, the input did not.
///
/// The submitted password is dropped on the floor — a browser would have to
/// send it again, and it must not end up in the HTML we send back.
fn render_signup_error(email: &str, message: &str) -> Response {
    let template = templates::SignupTemplate {
        email: email.to_owned(),
        error: Some(message.to_owned()),
    };

    match template.render() {
        Ok(body) => Html(body).into_response(),
        Err(err) => {
            tracing::error!(?err, "template render failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// --- login -------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginQuery {
    next: Option<String>,
}

async fn login_form(Query(query): Query<LoginQuery>) -> Result<Html<String>, AppError> {
    Ok(Html(
        templates::LoginTemplate {
            error: None,
            next: query.next.map(|raw| safe_next(&raw)).unwrap_or_default(),
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
struct LoginForm {
    email: String,
    password: String,
    next: String,
}

async fn login(
    mut session: AuthSession,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let outcome = session
        .authenticate(super::Credentials {
            email: form.email,
            password: form.password,
        })
        .await
        .map_err(|err| AppError::Session(err.to_string()));

    let user = match outcome {
        Ok(Some(user)) => user,
        Ok(None) => {
            tracing::warn!("login refused");
            return Ok(render_login_error(&form.next, BAD_CREDENTIALS));
        }
        Err(err) => return Err(err),
    };

    // Always rotate the session id, never reuse it.
    //
    // axum-login's own `login` only calls `cycle_id` when the session carries
    // no auth hash, and its `logout` deletes the stored record without clearing
    // the in-memory hash — so switching accounts without this line would keep
    // the previous session id, which is textbook session fixation. Rotating
    // unconditionally is cheap and has no downside.
    session
        .session
        .cycle_id()
        .await
        .map_err(|err| AppError::Session(err.to_string()))?;

    session
        .login(&user)
        .await
        .map_err(|err| AppError::Session(err.to_string()))?;

    tracing::info!(user.id = user.id, "login succeeded");
    Ok(Redirect::to(&safe_next(&form.next)).into_response())
}

fn render_login_error(next: &str, message: &str) -> Response {
    let template = templates::LoginTemplate {
        error: Some(message.to_owned()),
        next: safe_next(next),
    };

    match template.render() {
        Ok(body) => Html(body).into_response(),
        Err(err) => {
            tracing::error!(?err, "template render failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// --- logout ------------------------------------------------------------------

async fn logout(mut session: AuthSession) -> Result<Response, AppError> {
    let who = session
        .logout()
        .await
        .map_err(|err| AppError::Session(err.to_string()))?;

    if let Some(User { id, .. }) = who {
        tracing::info!(user.id = id, "logout");
    }

    Ok(Redirect::to("/login").into_response())
}

// --- account: change password, log out everywhere --------------------------

async fn account_form(
    State(pool): State<PgPool>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>, AppError> {
    let sessions = store::count_sessions_for_user(&pool, user.id).await?;
    Ok(Html(
        templates::AccountTemplate {
            email: user.email,
            sessions,
            error: None,
            notice: None,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
struct PasswordForm {
    current_password: String,
    new_password: String,
    new_password_confirm: String,
}

async fn change_password(
    State(pool): State<PgPool>,
    mut session: AuthSession,
    CurrentUser(user): CurrentUser,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    // The cheap checks come first. Telling someone their two new passwords
    // differ should not cost a 50ms Argon2 verification.
    let rejection = match check_new_password(&form) {
        Err(message) => Err(message),
        Ok(()) => check_current_password(&pool, &user, &form.current_password).await,
    };

    if let Err(message) = rejection {
        return Ok(render_account_error(&user, &message));
    }

    let hash = password::hash(form.new_password).await?;
    if !store::set_password(&pool, user.id, &hash).await? {
        return Ok(render_account_error(
            &user,
            "That account no longer exists.",
        ));
    }

    // The stored hash is what `session_auth_hash` is derived from, so from here
    // on every session made under the old password fails its check. This one is
    // re-issued so the person who just changed it stays signed in on this
    // device - and only this device.
    //
    // `refreshed` matters. The `user` we were handed still carries the auth
    // hash of the *old* password, and issuing a session from it would store a
    // value that no longer matches what the next request computes - which signs
    // the person straight back out again.
    let refreshed = User::new(user.id, user.email.clone(), user.email_verified_at, &hash);

    session
        .session
        .cycle_id()
        .await
        .map_err(raw_session_error)?;
    session.login(&refreshed).await.map_err(session_error)?;

    tracing::info!(user.id = user.id, "password changed");
    Ok(Redirect::to("/account/password?changed=1").into_response())
}

/// Checks the two new-password fields. Kept apart from the current-password
/// check so the visitor is told about a mismatch before we spend 50ms of Argon2.
fn check_new_password(form: &PasswordForm) -> Result<(), String> {
    if form.new_password != form.new_password_confirm {
        return Err("New passwords do not match.".to_owned());
    }
    password::validate(&form.new_password).map_err(str::to_owned)
}

/// Verifies the password currently on the account.
///
/// This is the step that stops a walk-up attacker using an unlocked browser to
/// take over the account. The email alone is not enough.
async fn check_current_password(pool: &PgPool, user: &User, current: &str) -> Result<(), String> {
    let stored = sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(pool)
        .await
        .map_err(|err| err.to_string())?;

    match password::verify(current.to_owned(), stored).await {
        true => Ok(()),
        false => Err("Current password is incorrect.".to_owned()),
    }
}

/// Ends every session for this account, including the one making the request.
///
/// The current session is re-issued first so the visitor keeps this device;
/// everything else is deleted, which is the entire point of the button.
async fn logout_everywhere(
    State(pool): State<PgPool>,
    mut session: AuthSession,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    session
        .session
        .cycle_id()
        .await
        .map_err(raw_session_error)?;
    session.login(&user).await.map_err(session_error)?;

    let removed = store::delete_sessions_for_user(&pool, user.id).await?;
    tracing::info!(user.id = user.id, removed, "logged out everywhere");

    Ok(Redirect::to("/account/password?logged_out=1").into_response())
}

fn render_account_error(user: &User, message: &str) -> Response {
    // The session count is not re-read here on purpose: an error page should
    // not need the database a second time, and the number is only decoration.
    let template = templates::AccountTemplate {
        email: user.email.clone(),
        sessions: 0,
        error: Some(message.to_owned()),
        notice: None,
    };

    match template.render() {
        Ok(body) => Html(body).into_response(),
        Err(err) => {
            tracing::error!(?err, "template render failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn session_error(err: axum_login::Error<AuthBackend>) -> AppError {
    AppError::Session(err.to_string())
}

/// `cycle_id` comes from tower-sessions itself rather than axum-login, so it
/// has its own error type.
fn raw_session_error(err: axum_login::tower_sessions::session::Error) -> AppError {
    AppError::Session(err.to_string())
}

// --- router ------------------------------------------------------------------

/// The unauthenticated half of the app, mounted without any guard.
///
/// Takes the pool and applies it with `with_state`, so what comes back is a
/// plain `Router` that `src/app.rs` can merge with the todo routes. The merge
/// itself is what places the session layer above them.
///
/// Callers merge this in and add the session layer on top; `src/app.rs` shows
/// the whole arrangement.
pub fn routes(pool: PgPool) -> Router {
    Router::new()
        .route("/signup", get(signup_form).post(signup))
        .route("/login", get(login_form).post(login))
        // POST only. A GET logout would let a third-party page end someone's
        // session with an `<img src>`.
        .route("/logout", post(logout))
        .with_state(pool)
}

/// The pages that need a session but are about the account rather than the
/// list, so `todo::build_router` does not own them.
///
/// They are guarded here rather than there because that router is about todos.
/// The guard is the same middleware either way — there is one login check in
/// the app, not two that could drift apart.
pub fn account_routes(pool: PgPool) -> Router {
    Router::new()
        .route("/account/password", get(account_form).post(change_password))
        .route("/account/logout-everywhere", post(logout_everywhere))
        .route_layer(axum::middleware::from_fn(super::require_auth))
        .with_state(pool)
}
