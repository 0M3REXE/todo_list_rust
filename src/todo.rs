use axum::{routing::get, Router};

async fn ping() -> &'static str {
    "pong"
}

pub fn build_router() -> Router {
    Router::new().route("/ping", get(ping))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn harness_works() {
        let app = build_router();
        let res = app
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], b"pong");
    }
}
