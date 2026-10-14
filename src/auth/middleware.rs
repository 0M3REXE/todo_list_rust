//! # Two middlewares: "you must be logged in", and "this POST is not a forgery"

use axum::{
    Extension,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

use super::AuthSession;

/// htmx sets this on every request it makes, and reads `HX-Redirect` back to
/// tell it to do a full browser navigation instead of a swap.
const HX_REQUEST: &str = "hx-request";
const HX_REDIRECT: &str = "hx-redirect";

/// Sends anyone without a session to the login page.
///
/// An htmx request gets `HX-Redirect` instead of a 303, because a 303 would
/// make htmx swap a whole login page into `#todo-list` — leaving the visitor
/// staring at a login form in the middle of their list, with no way out.
pub async fn require_auth(req: Request, next: Next) -> Response {
    let logged_in = req
        .extensions()
        .get::<AuthSession>()
        .is_some_and(|session| session.user.is_some());

    if logged_in {
        return next.run(req).await;
    }

    if req.headers().contains_key(HX_REQUEST) {
        return (StatusCode::OK, [(HX_REDIRECT, "/login")]).into_response();
    }

    // Remember where they were headed so the login page can send them back.
    let target = format!("/login?next={}", encode_query_value(req.uri().path()));
    Redirect::to(&target).into_response()
}

/// Rejects a request from an account whose address is not confirmed, when the
/// app is configured to require that.
///
/// Placed on the todo routes so an unverified account can still log in, read the
/// banner and ask for another link — but cannot touch the list. Blocking at the
/// door instead would make the email the only key to the account, so a forgotten
/// password would become a permanent lockout.
///
/// Only applies to state-changing requests. Rendering the page that explains
/// the problem is the whole point of letting them in at all.
pub async fn require_verified(
    axum::Extension(config): axum::Extension<std::sync::Arc<crate::app::AppConfig>>,
    Extension(session): Extension<AuthSession>,
    req: Request,
    next: Next,
) -> Response {
    if !config.require_email_verification || !is_state_changing(req.method()) {
        return next.run(req).await;
    }

    let verified = session.user.as_ref().is_some_and(|user| user.is_verified());

    if verified {
        return next.run(req).await;
    }

    tracing::warn!("blocked an unverified account from changing data");
    (
        StatusCode::FORBIDDEN,
        "Confirm your email address before using your list.",
    )
        .into_response()
}

/// Rejects state-changing requests that arrive from another site.
///
/// The session cookie is `SameSite=Lax`, which already stops a cross-site form
/// post from carrying credentials, so this is the second lock rather than the
/// only one. It catches a browser or extension that has been talked into sending
/// one anyway.
///
/// A request with neither header is allowed through. Browsers always send
/// `Origin` on a POST, so their absence means a non-browser client (curl, this
/// project's own test suite) rather than a forged browser request.
pub async fn verify_origin(
    State(allowed_host): State<String>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    if !is_state_changing(req.method()) {
        return next.run(req).await;
    }

    let Some(presented) = presented_host(&headers) else {
        return next.run(req).await;
    };

    if presented == allowed_host {
        return next.run(req).await;
    }

    tracing::warn!(host = %presented, %allowed_host, "rejected a cross-origin request");
    (StatusCode::FORBIDDEN, "cross-origin request rejected").into_response()
}

/// Only the methods that change something need the check. A GET cannot alter
/// state, so it is not a CSRF vector.
fn is_state_changing(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// The `host[:port]` the request claims to come from, preferring `Origin` and
/// falling back to `Referer`.
///
/// Only the authority is compared, not the scheme: the check is "did this come
/// from us", and `http://localhost:3000` and `https://localhost:3000` are the
/// same place.
fn presented_host(headers: &HeaderMap) -> Option<String> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        return authority_of(origin.to_str().ok()?);
    }
    authority_of(headers.get(header::REFERER)?.to_str().ok()?)
}

fn authority_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split('/').next().unwrap_or_default();
    (!authority.is_empty()).then(|| authority.to_owned())
}

/// Percent-encodes the characters that would otherwise break a query string,
/// so a path containing `&` or `#` cannot smuggle in extra parameters.
fn encode_query_value(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Turns the `?next=` value into a path we are willing to send someone to.
///
/// Only a path on this site qualifies. An absolute URL hands the visitor to
/// another site wearing our redirect; `//evil.com` and `/\evil.com` are the
/// same trick written so a naive "does it start with a slash" check misses it.
/// Anything that is not a local path falls back to the home page.
pub fn safe_next(candidate: &str) -> String {
    let trimmed = candidate.trim();
    let is_local =
        trimmed.starts_with('/') && !trimmed.starts_with("//") && !trimmed.contains('\\');

    if is_local {
        trimmed.to_owned()
    } else {
        "/".to_owned()
    }
}
