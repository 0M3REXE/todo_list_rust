//! # Two middlewares: "you must be logged in", and "this POST is not a forgery"

use axum::{
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

    // Modern browsers report this even when privacy settings omit Referer.
    // Reject cross-site writes independently of Origin so same-site sibling
    // subdomains cannot skip the origin check.
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        == Some("cross-site")
    {
        return (StatusCode::FORBIDDEN, "cross-origin request rejected").into_response();
    }

    // A supplied but malformed Origin/Referer must fail closed. Requests with
    // neither header remain supported for non-browser clients; browser writes
    // are still covered by Sec-Fetch-Site above and SameSite cookies.
    let supplied_origin = headers.contains_key(header::ORIGIN)
        || headers.contains_key(header::REFERER);
    let Some(presented) = presented_host(&headers) else {
        return if supplied_origin {
            (StatusCode::FORBIDDEN, "cross-origin request rejected").into_response()
        } else {
            next.run(req).await
        };
    };

    let request_host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| req.uri().authority().map(|authority| authority.as_str()))
        .and_then(normalize_host);
    let matches_request_host = request_host.as_deref() == Some(presented.as_str());
    let matches_configured_host = allowed_host
        .split(',')
        .filter_map(normalize_host)
        .any(|allowed| allowed == presented);

    if matches_request_host || matches_configured_host {
        return next.run(req).await;
    }

    tracing::warn!(host = %presented, request_host = ?request_host, %allowed_host, "rejected a cross-origin request");
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
        return normalize_host(origin.to_str().ok()?);
    }
    normalize_host(headers.get(header::REFERER)?.to_str().ok()?)
}

/// Accepts either a URL or a host[:port], and normalizes DNS case and trailing
/// slashes before exact comparison. Userinfo and whitespace are never valid.
fn normalize_host(value: &str) -> Option<String> {
    let value = value.trim();
    let authority = match value.split_once("://") {
        Some((_, rest)) => rest.split('/').next().unwrap_or_default(),
        None => value.split('/').next().unwrap_or_default(),
    };
    if authority.is_empty()
        || authority.contains('@')
        || authority.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(authority.trim_end_matches('.').to_ascii_lowercase())
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
