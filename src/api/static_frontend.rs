use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use tower_http::services::{ServeDir, ServeFile};

const STATIC_ROOT: &str = "/app/frontend/dist";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub(super) fn router() -> Router {
    Router::new()
        .nest_service("/assets", ServeDir::new(format!("{STATIC_ROOT}/assets")))
        .fallback_service(
            ServeDir::new(STATIC_ROOT)
                .fallback(ServeFile::new(format!("{STATIC_ROOT}/index.html"))),
        )
        .layer(middleware::from_fn(static_policy))
}

async fn static_policy(request: Request, next: Next) -> Response {
    let Some(path) = safe_path(request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if reserved_path(&path)
        || ((path == "/assets" || path.starts_with("/assets/"))
            && !(request.uri().path() == "/assets" || request.uri().path().starts_with("/assets/")))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = next.run(request).await;
    let cache = cache_control(&path, response.status());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn safe_path(path: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            let value = (hi * 16 + lo) as u8;
            if value == b'/' || value == b'\\' {
                return None;
            }
            decoded.push(value);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.contains(['\\', '\0'])
        || decoded
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        return None;
    }
    Some(decoded)
}

fn reserved_path(path: &str) -> bool {
    path == "/v1"
        || path.starts_with("/v1/")
        || ["/openapi.json", "/healthz", "/readyz", "/livez"]
            .iter()
            .any(|prefix| {
                path == *prefix
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
}

fn cache_control(path: &str, status: StatusCode) -> &'static str {
    if (status.is_success() || status == StatusCode::NOT_MODIFIED) && path.starts_with("/assets/") {
        IMMUTABLE
    } else {
        "no-cache"
    }
}

#[cfg(test)]
mod tests {
    use super::{IMMUTABLE, cache_control, reserved_path, safe_path};
    use axum::http::StatusCode;

    #[test]
    fn reserved_api_and_health_paths_never_fall_back_to_the_spa() {
        for path in [
            "/v1",
            "/v1/unknown",
            "/v1/auth/login",
            "/openapi.json",
            "/healthz",
            "/readyz/extra",
            "/livez",
        ] {
            assert!(reserved_path(path), "{path}");
        }
        for path in ["/v10", "/healthzone", "/settings"] {
            assert!(!reserved_path(path), "{path}");
        }
        assert!(reserved_path(&safe_path("/%76%31/unknown").unwrap()));
    }

    #[test]
    fn paths_reject_traversal_and_invalid_escapes() {
        for path in [
            "/../index.html",
            "/%2e%2e/secret",
            "/assets/%2fsecret",
            "/assets/%5csecret",
            "/assets/%00",
            "/assets/%xx",
        ] {
            assert!(safe_path(path).is_none(), "{path}");
        }
        assert_eq!(
            safe_path("/assets/app.js").as_deref(),
            Some("/assets/app.js")
        );
    }

    #[test]
    fn assets_are_immutable_only_when_served_successfully() {
        assert_eq!(cache_control("/assets/app.js", StatusCode::OK), IMMUTABLE);
        assert_eq!(
            cache_control("/assets/app.js", StatusCode::NOT_MODIFIED),
            IMMUTABLE
        );
        assert_eq!(
            cache_control("/assets/missing.js", StatusCode::NOT_FOUND),
            "no-cache"
        );
        assert_eq!(cache_control("/index.html", StatusCode::OK), "no-cache");
        assert_eq!(cache_control("/assets-other", StatusCode::OK), "no-cache");
    }
}
