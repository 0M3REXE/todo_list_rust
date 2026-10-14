//! # One-time links — email verification and password reset
//!
//! Both flows are the same shape: mint a random token, store only its SHA-256,
//! mail the raw value, and consume it with a single `UPDATE` when it comes back.
//! They live together because they share all of that.
//!
//! **What is never stored, logged or emailed twice:** the raw token. It exists
//! only in the message that was sent. A leaked `email_tokens` table yields
//! nothing usable, because SHA-256 cannot be walked backwards to find the token
//! whose hash it is.

use askama::Template;
use axum::{
    Form, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use time::OffsetDateTime;

use crate::AppError;
use crate::app::AppConfig;
use crate::auth::{CurrentUser, Purpose, User, password, store};
use crate::mailer::{Email, Mailer, random_token};

// --- tokens ---

/// Creates a token, stores its hash, and mails the raw value.
///
/// Returns `Err` with the reason when nothing was sent, so the caller can
/// explain itself. The resend cooldown is checked *before* a token is minted,
/// so a refused resend does not invalidate the link the user already has.
async fn issue(
    pool: &PgPool,
    config: &AppConfig,
    mailer: &Arc<dyn Mailer>,
    user: &User,
    purpose: Purpose,
    ttl: time::Duration,
) -> Result<(), IssueError> {
    if cooldown_active(pool, config, user.id).await.is_err() {
        return Err(IssueError::CoolingDown);
    }

    let raw = random_token();
    let expires_at = OffsetDateTime::now_utc() + ttl;

    store::put_token(pool, user.id, purpose, &store::hash_token(&raw), expires_at)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not store the token");
            IssueError::Failed
        })?;

    let link = match purpose {
        Purpose::Verify => format!("{}/verify/{}", config.base_url, raw),
        Purpose::Reset => format!("{}/reset/{}", config.base_url, raw),
    };

    mailer
        .send(&message_for(user, purpose, &link))
        .await
        .map_err(|err| {
            tracing::error!(%err, "mail delivery failed");
            IssueError::Failed
        })?;

    Ok(())
}

/// Why a link was not sent. Kept as its own type rather than a `String` so the
/// cooldown cannot be confused with a delivery failure in a log.
///
/// The underlying cause is already logged with full detail at the point it
/// happens, so it is not carried here — `AppError` deliberately withholds detail
/// from whoever ends up holding it.
#[derive(Debug)]
enum IssueError {
    /// The cooldown is still running.
    CoolingDown,
    /// The mailer refused, or the database said no.
    Failed,
}

impl IssueError {
    fn message(&self) -> &'static str {
        match self {
            Self::CoolingDown => "A link was sent recently. Please wait before asking for another.",
            Self::Failed => "Could not send the email. Please try again.",
        }
    }
}

/// Whether the cooldown still applies.
///
/// Measured across *all* purposes for the account, not just the one being asked
/// for — otherwise a visitor could ask for a reset and then immediately a
/// verification to get round the wait.
async fn cooldown_active(
    pool: &PgPool,
    config: &AppConfig,
    user_id: i64,
) -> Result<bool, AppError> {
    let Some(last) = store::last_token_sent_at(pool, user_id).await? else {
        return Ok(false);
    };
    Ok(OffsetDateTime::now_utc() < last + config.resend_cooldown)
}

fn message_for(user: &User, purpose: Purpose, link: &str) -> Email {
    let (subject, body) = match purpose {
        Purpose::Verify => (
            "Confirm your address",
            format!(
                "Hello,\n\nConfirm your address to finish setting up your account:\n\n{link}\n\n\
                 The link works once, and expires in 24 hours.\n\n\
                 If you did not ask for this, you can ignore this message.\n"
            ),
        ),
        Purpose::Reset => (
            "Reset your password",
            format!(
                "Hello,\n\nSomebody asked to reset the password on this account. \
                 If that was you, use this link:\n\n{link}\n\n\
                 The link works once, and expires in one hour.\n\n\
                 If it was not you, nothing has changed and you can ignore this message.\n"
            ),
        ),
    };

    Email {
        to: user.email.clone(),
        subject: subject.to_owned(),
        body,
    }
}

// --- verification ---

/// Consumes a verification token.
///
/// Every failure — unknown, expired, already used, or issued for the other
/// purpose — produces the same page. Telling them apart would let someone probe
/// whether a token they guessed ever existed.
async fn verify_link(
    State(pool): State<PgPool>,
    Path(token): Path<String>,
) -> Result<Response, AppError> {
    let Some(user_id) = store::take_token(&pool, &token, Purpose::Verify).await? else {
        return Ok(render_result(
            false,
            "That link is not valid.",
            "It may have expired, already been used, or never been issued.",
        ));
    };

    store::mark_email_verified(&pool, user_id).await?;

    Ok(render_result(
        true,
        "Address confirmed.",
        "You can close this page and go back to your list.",
    ))
}

/// Re-sends the verification link.
///
/// Requires a session, so the button only works for someone who can prove the
/// address is theirs — which is the entire point of verification.
async fn resend_verification(
    State(pool): State<PgPool>,
    axum::Extension(config): axum::Extension<Arc<AppConfig>>,
    axum::Extension(mailer): axum::Extension<Arc<dyn Mailer>>,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    if user.is_verified() {
        return Ok(Redirect::to("/").into_response());
    }

    match issue(
        &pool,
        &config,
        &mailer,
        &user,
        Purpose::Verify,
        config.verify_ttl,
    )
    .await
    {
        Ok(()) => Ok(render_result(
            true,
            "Check your email.",
            "A fresh confirmation link is on its way.",
        )),
        Err(problem) => Ok(render_result(
            false,
            "Could not send another link.",
            problem.message(),
        )),
    }
}

// --- reset ---

/// The form behind a reset link. The token is in the URL, so it is carried in a
/// hidden field rather than a query parameter on the POST.
async fn reset_form(Path(token): Path<String>) -> Result<Html<String>, AppError> {
    Ok(Html(ResetTemplate { token, error: None }.render()?))
}

#[derive(Deserialize)]
struct ResetForm {
    token: String,
    new_password: String,
    new_password_confirm: String,
}

async fn reset(
    State(pool): State<PgPool>,
    Form(form): Form<ResetForm>,
) -> Result<Response, AppError> {
    // Same one-size-fits-all failure page as verification, for the same reason.
    let reject = |reason: &'static str| render_result(false, "That link is not valid.", reason);

    if form.new_password != form.new_password_confirm {
        return Ok(render_reset_form(
            &form.token,
            "New passwords do not match.",
        ));
    }
    if let Err(message) = password::validate(&form.new_password) {
        return Ok(render_reset_form(&form.token, message));
    }

    let Some(user_id) = store::take_token(&pool, &form.token, Purpose::Reset).await? else {
        return Ok(reject(
            "It may have expired, already been used, or never been issued.",
        ));
    };

    let hash = password::hash(form.new_password).await?;
    if !store::set_password(&pool, user_id, &hash).await? {
        return Ok(reject("The account no longer exists."));
    }

    // Any other reset link for this account stops working, and changing the
    // password hash invalidates every session that was made under the old one —
    // which is what makes "somebody reset my password" actually lock them out.
    store::delete_tokens(&pool, user_id).await?;
    store::delete_sessions_for_user(&pool, user_id).await?;

    tracing::info!(user.id = user_id, "password reset");
    Ok(render_result(
        true,
        "Password updated.",
        "Every session has been signed out. You can log in with the new password.",
    ))
}

/// Always the same answer, whether or not the address is registered.
///
/// A different reply for "no such account" turns this form into a way to
/// enumerate every address on the site.
async fn forgot(
    State(pool): State<PgPool>,
    axum::Extension(config): axum::Extension<Arc<AppConfig>>,
    axum::Extension(mailer): axum::Extension<Arc<dyn Mailer>>,
    Form(form): Form<ForgotForm>,
) -> Result<Response, AppError> {
    const SAME_ANSWER: &str = "If that address has an account, a reset link is on its way.";

    let Some(user_id) = store::user_id_by_email(&pool, form.email.trim()).await? else {
        return Ok(render_result(true, "Check your email.", SAME_ANSWER));
    };

    let Ok(Some(user)) = store::find_user(&pool, user_id).await else {
        return Ok(render_result(true, "Check your email.", SAME_ANSWER));
    };

    // A failure here is deliberately swallowed: telling the visitor that
    // sending failed would reveal that the account exists.
    if let Err(problem) = issue(
        &pool,
        &config,
        &mailer,
        &user,
        Purpose::Reset,
        config.reset_ttl,
    )
    .await
    {
        tracing::warn!(?problem, "reset mail not sent");
    }

    Ok(render_result(true, "Check your email.", SAME_ANSWER))
}

#[derive(Deserialize)]
struct ForgotForm {
    email: String,
}

async fn forgot_form() -> Result<Html<String>, AppError> {
    Ok(Html(ForgotTemplate { error: None }.render()?))
}

// --- rendering ---

/// The shared "here is the outcome" page. Success and failure use the same
/// shape so the two are not trivially distinguishable at a glance.
fn render_result(ok: bool, title: &str, detail: &str) -> Response {
    let template = NoticeTemplate {
        ok,
        title: title.to_owned(),
        detail: detail.to_owned(),
    };

    match template.render() {
        Ok(body) => Html(body).into_response(),
        Err(err) => {
            tracing::error!(?err, "template render failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn render_reset_form(token: &str, message: &str) -> Response {
    let template = ResetTemplate {
        token: token.to_owned(),
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

// --- router ---

/// Verification and reset links. Public, because the token in the URL is the
/// only credential and the visitor may not have a session.
pub fn routes(pool: PgPool) -> Router {
    Router::new()
        .route("/verify/{token}", get(verify_link))
        .route("/forgot", get(forgot_form).post(forgot))
        .route("/reset/{token}", get(reset_form).post(reset))
        .with_state(pool)
}

/// Needs a session: only somebody who is signed in can ask for another link,
/// which is what stops this being an open mail relay for anyone with an address.
pub fn session_routes(pool: PgPool) -> Router {
    Router::new()
        .route("/verify-email", post(resend_verification))
        .route_layer(axum::middleware::from_fn(crate::auth::require_auth))
        .with_state(pool)
}

#[derive(Template)]
#[template(path = "notice.html")]
struct NoticeTemplate {
    ok: bool,
    title: String,
    detail: String,
}

#[derive(Template)]
#[template(path = "reset.html")]
struct ResetTemplate {
    token: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "forgot.html")]
struct ForgotTemplate {
    error: Option<String>,
}
