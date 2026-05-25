use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::admin_domains::{ManagementError, StalwartManagement};
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session_with_admin};
use tower::ServiceExt;

fn app(state: AppState, management: Arc<FakeManagement>) -> Router {
    let protected = hail_api::routes::admin_domains::router_with_management(management).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone, Default)]
struct FakeManagement {
    domains: Arc<Mutex<Vec<String>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeManagement {
    fn with_domains(domains: &[&str]) -> Self {
        Self {
            domains: Arc::new(Mutex::new(
                domains.iter().map(ToString::to_string).collect(),
            )),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl StalwartManagement for FakeManagement {
    fn list_domains<'a>(
        &'a self,
        _state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("list".to_string());
            Ok(self.domains.lock().unwrap().clone())
        })
    }

    fn add_domain<'a>(
        &'a self,
        _state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("add:{domain}"));
            self.domains.lock().unwrap().push(domain.to_string());
            Ok(())
        })
    }

    fn delete_domain<'a>(
        &'a self,
        _state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("delete:{domain}"));
            self.domains.lock().unwrap().retain(|d| d != domain);
            Ok(())
        })
    }
}

async fn request(
    state: AppState,
    management: Arc<FakeManagement>,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    app(state, management)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn auth_required_for_list() {
    let (state, _key) = fixture_state().await;
    let management = Arc::new(FakeManagement::with_domains(&["example.org"]));

    let resp = request(
        state,
        management,
        Method::GET,
        "/api/admin/domains",
        None,
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_admin_returns_403() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session_with_admin(&state, &key, "alice@example.org", false).await;
    let management = Arc::new(FakeManagement::with_domains(&["example.org"]));

    let resp = request(
        state,
        management,
        Method::GET,
        "/api/admin/domains",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(resp).await["error"], "admin_required");
}

#[tokio::test]
async fn list_uses_fake_management() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session_with_admin(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::with_domains(&["z.example", "a.example"]));

    let resp = request(
        state,
        management.clone(),
        Method::GET,
        "/api/admin/domains",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(
        json["domains"],
        serde_json::json!(["a.example", "z.example"])
    );
    assert_eq!(management.calls(), vec!["list"]);
}

#[tokio::test]
async fn add_uses_fake_management_and_normalizes_domain() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session_with_admin(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::default());

    let resp = request(
        state.clone(),
        management.clone(),
        Method::POST,
        "/api/admin/domains",
        Some(&sid),
        true,
        Some(r#"{"domain":" Example.ORG. "}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = json_body(resp).await;
    assert_eq!(json["domain"], "example.org");
    assert_eq!(management.calls(), vec!["add:example.org"]);

    let audit: (i64, String, String) =
        sqlx::query_as("SELECT user_id, action, payload_json FROM audit_log")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(audit.0, 1);
    assert_eq!(audit.1, "admin.domain.add");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&audit.2).unwrap(),
        serde_json::json!({ "domain": "example.org" })
    );
}

#[tokio::test]
async fn delete_uses_fake_management_and_normalizes_domain() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session_with_admin(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::with_domains(&["example.org"]));

    let resp = request(
        state,
        management.clone(),
        Method::DELETE,
        "/api/admin/domains/Example.ORG.",
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(management.calls(), vec!["delete:example.org"]);
}

#[tokio::test]
async fn invalid_domain_returns_400_and_does_not_call_management() {
    let invalid_domains = ["-bad.example", "bad-.example", "example..org", "123.456"];

    for domain in invalid_domains {
        let (state, key) = fixture_state().await;
        let (_user_id, sid) =
            seed_session_with_admin(&state, &key, "admin@example.org", true).await;
        let management = Arc::new(FakeManagement::default());
        let body = serde_json::json!({ "domain": domain }).to_string();

        let resp = request(
            state,
            management.clone(),
            Method::POST,
            "/api/admin/domains",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "domain={domain:?}");
        assert_eq!(
            json_body(resp).await["error"],
            "invalid_domain",
            "domain={domain:?}"
        );
        assert!(management.calls().is_empty(), "domain={domain:?}");
    }
}
