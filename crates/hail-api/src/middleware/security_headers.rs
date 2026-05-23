//! Browser hardening response headers for the API and bundled SPA.
//!
//! These headers are deliberately global: API JSON, WebSocket upgrade
//! failures, OpenAPI, health checks, and static SPA assets all get the same
//! baseline defense in depth. The CSP is tuned for the Vite-built SPA plus the
//! server-sanitized mail HTML rendered inside the app: same-origin scripts and
//! assets, inline styles for React/Tailwind/mail markup compatibility, and a
//! same-origin WebSocket endpoint for `/api/ws`.

use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::{extract::Request, http::Uri};

use crate::state::AppState;

const X_CONTENT_TYPE_OPTIONS: HeaderValue = HeaderValue::from_static("nosniff");
const REFERRER_POLICY: HeaderValue = HeaderValue::from_static("no-referrer");
const X_FRAME_OPTIONS: HeaderValue = HeaderValue::from_static("DENY");
const HSTS: HeaderValue = HeaderValue::from_static("max-age=31536000; includeSubDomains");

/// Attach browser hardening headers to every response.
pub async fn add_security_headers(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let security = SecurityHeaders::from_public_url(&state.config.server.public_url);
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers.insert(header::X_CONTENT_TYPE_OPTIONS, X_CONTENT_TYPE_OPTIONS);
    headers.insert(header::REFERRER_POLICY, REFERRER_POLICY);
    headers.insert(header::X_FRAME_OPTIONS, X_FRAME_OPTIONS);
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        security.content_security_policy,
    );
    if security.hsts {
        headers.insert(header::STRICT_TRANSPORT_SECURITY, HSTS);
    }

    response
}

struct SecurityHeaders {
    content_security_policy: HeaderValue,
    hsts: bool,
}

impl SecurityHeaders {
    fn from_public_url(public_url: &str) -> Self {
        let parsed = public_url.parse::<Uri>().ok();
        let scheme = parsed.as_ref().and_then(Uri::scheme_str);
        let authority = parsed
            .as_ref()
            .and_then(Uri::authority)
            .map(|authority| authority.as_str());
        let hsts = matches!(scheme, Some("https"));
        let websocket_source = match (scheme, authority) {
            (Some("https"), Some(authority)) => format!(" wss://{authority}"),
            (Some("http"), Some(authority)) => format!(" ws://{authority}"),
            _ => String::new(),
        };

        let csp = format!(
            "default-src 'self'; \
             base-uri 'self'; \
             object-src 'none'; \
             frame-ancestors 'none'; \
             script-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; \
             font-src 'self' data:; \
             connect-src 'self'{websocket_source}; \
             form-action 'self'"
        );

        Self {
            content_security_policy: HeaderValue::from_str(&csp)
                .expect("generated CSP contains only valid header characters"),
            hsts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SecurityHeaders;

    #[test]
    fn csp_includes_same_origin_websocket_source_for_https_public_url() {
        let headers = SecurityHeaders::from_public_url("https://mail.example.test");
        let csp = headers.content_security_policy.to_str().unwrap();

        assert!(headers.hsts);
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("connect-src 'self' wss://mail.example.test"));
    }

    #[test]
    fn csp_includes_same_origin_websocket_source_for_http_public_url() {
        let headers = SecurityHeaders::from_public_url("http://localhost:8080");
        let csp = headers.content_security_policy.to_str().unwrap();

        assert!(!headers.hsts);
        assert!(csp.contains("connect-src 'self' ws://localhost:8080"));
    }
}
