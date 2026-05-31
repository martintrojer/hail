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
use secrecy::SecretString;
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
        _bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("list".to_string());
            Ok(self.domains.lock().unwrap().clone())
        })
    }

    fn add_domain<'a>(
        &'a self,
        _state: &'a AppState,
        _bearer: SecretString,
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
        _bearer: SecretString,
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

#[derive(Debug, Clone)]
struct JmapCall {
    path: String,
    body: serde_json::Value,
}

async fn start_fake_management() -> (
    String,
    Arc<Mutex<Vec<JmapCall>>>,
    tokio::task::JoinHandle<()>,
) {
    async fn handler(
        axum::extract::State(calls): axum::extract::State<Arc<Mutex<Vec<JmapCall>>>>,
        headers: axum::http::HeaderMap,
        uri: axum::http::Uri,
        body: String,
    ) -> impl axum::response::IntoResponse {
        let path = uri.path().to_string();
        let body_json = if body.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&body).unwrap()
        };
        calls.lock().unwrap().push(JmapCall {
            path: path.clone(),
            body: body_json.clone(),
        });

        if path == "/.well-known/jmap" {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok()),
                Some("Bearer dummy-token")
            );
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!({"primaryAccounts": {"urn:stalwart:jmap": "mgmt"}})),
            );
        }
        if path == "/jmap/" {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok()),
                Some("Bearer dummy-token")
            );
            let method = body_json["methodCalls"][0][0].as_str().unwrap();
            return match method {
                "Principal/query" => (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({"methodResponses": [
                        ["Principal/query", {"accountId": "mgmt", "ids": ["domain-id"]}, "query-0"],
                        ["Principal/get", {"accountId": "mgmt", "list": [{"id": "domain-id", "name": "example.org", "type": "domain"}], "notFound": []}, "get-0"]
                    ]})),
                ),
                "Principal/set" => {
                    if body_json.pointer("/methodCalls/0/1/create/new-0").is_some() {
                        (
                            StatusCode::OK,
                            axum::Json(
                                serde_json::json!({"methodResponses": [["Principal/set", {"accountId": "mgmt", "notCreated": {"new-0": {"type": "primaryKeyViolation", "description": "already exists"}}}, "set-0"]]}),
                            ),
                        )
                    } else {
                        (
                            StatusCode::OK,
                            axum::Json(
                                serde_json::json!({"methodResponses": [["Principal/set", {"accountId": "mgmt", "destroyed": ["domain-id"]}, "set-0"]]}),
                            ),
                        )
                    }
                }
                other => panic!("unexpected method {other}"),
            };
        }
        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"detail": "missing"})),
        )
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .fallback(axum::routing::any(handler))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    (format!("http://{addr}"), calls, handle)
}

fn production_domain_app(state: AppState) -> Router {
    let protected = hail_api::routes::admin_domains::router().layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[tokio::test]
async fn production_domain_routes_use_principal_jmap_shapes_and_are_idempotent() {
    let (url, calls, _server) = start_fake_management().await;
    let (mut state, key) = fixture_state().await;
    state.config.stalwart.management_url = Some(url);
    let (_user_id, sid) = seed_session_with_admin(&state, &key, "admin@example.org", true).await;

    let list = production_domain_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/admin/domains")
                .header(header::COOKIE, format!("hail_session={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);

    let add = production_domain_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/admin/domains")
                .header(header::COOKIE, format!("hail_session={sid}"))
                .header(CSRF_HEADER, "1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"domain":"Example.ORG"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);

    let delete = production_domain_app(state)
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/admin/domains/example.org")
                .header(header::COOKIE, format!("hail_session={sid}"))
                .header(CSRF_HEADER, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let observed = calls.lock().unwrap().clone();
    assert!(observed.iter().any(|call| call.path == "/jmap/"
        && call.body.pointer("/methodCalls/0/0") == Some(&serde_json::json!("Principal/query"))
        && call.body.pointer("/methodCalls/0/1/filter/type")
            == Some(&serde_json::json!("domain"))));
    assert!(
        observed
            .iter()
            .any(|call| call.body.pointer("/methodCalls/0/0")
                == Some(&serde_json::json!("Principal/set"))
                && call.body.pointer("/methodCalls/0/1/create/new-0/type")
                    == Some(&serde_json::json!("domain")))
    );
    assert!(
        observed
            .iter()
            .any(|call| call.body.pointer("/methodCalls/0/1/destroy/0")
                == Some(&serde_json::json!("domain-id")))
    );
}
