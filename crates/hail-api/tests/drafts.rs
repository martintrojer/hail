use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::drafts::{
    DraftCreate, DraftDetails, DraftStore, DraftStoreError, DraftUpdate,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use secrecy::SecretString;
use tower::ServiceExt;

fn app(state: AppState, store: Arc<FakeDraftStore>) -> Router {
    let protected = hail_api::routes::drafts::router_with_store(store).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn request(
    state: AppState,
    store: Arc<FakeDraftStore>,
    method: Method,
    path: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }

    app(state, store)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Create {
        from: String,
        to: Vec<String>,
        cc: Vec<String>,
        bcc: Vec<String>,
        subject: String,
        body_markdown: String,
    },
    Get {
        draft_id: String,
    },
    Update {
        draft_id: String,
        to: Option<Vec<String>>,
        cc: Option<Vec<String>>,
        bcc: Option<Vec<String>>,
        subject: Option<String>,
        body_markdown: Option<String>,
    },
    Delete {
        draft_id: String,
    },
}

#[derive(Default)]
struct FakeDraftStore {
    calls: Mutex<Vec<Call>>,
    fail: Mutex<Option<DraftStoreError>>,
}

impl FakeDraftStore {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }

    fn fail_next(&self, error: DraftStoreError) {
        *self.fail.lock().expect("fail mutex") = Some(error);
    }

    fn fail_next_provider(&self) {
        self.fail_next(DraftStoreError::Provider("boom".to_string()));
    }

    fn fail_next_sender_identity(&self) {
        self.fail_next(DraftStoreError::SenderIdentityUnavailable);
    }

    fn should_fail(&self) -> Option<DraftStoreError> {
        self.fail.lock().expect("fail mutex").take()
    }
}

impl DraftStore for FakeDraftStore {
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Create {
                from: from.to_string(),
                to: draft.to,
                cc: draft.cc,
                bcc: draft.bcc,
                subject: draft.subject,
                body_markdown: draft.body_markdown,
            });
            Ok("draft-1".to_string())
        })
    }

    fn get<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DraftDetails>, DraftStoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Get {
                draft_id: draft_id.to_string(),
            });
            Ok(Some(DraftDetails {
                draft_id: draft_id.to_string(),
                to: vec!["bob@example.org".to_string()],
                cc: vec!["carol@example.org".to_string()],
                bcc: vec!["dana@example.org".to_string()],
                subject: "Saved draft".to_string(),
                body_markdown: "Saved body".to_string(),
            }))
        })
    }

    fn update<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Update {
                draft_id: draft_id.to_string(),
                to: draft.to,
                cc: draft.cc,
                bcc: draft.bcc,
                subject: draft.subject,
                body_markdown: draft.body_markdown,
            });
            Ok(())
        })
    }

    fn delete<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Delete {
                draft_id: draft_id.to_string(),
            });
            Ok(())
        })
    }
}

fn create_body() -> &'static str {
    r#"{
        "to":["bob@example.org"],
        "cc":["carol@example.org"],
        "subject":"Hello",
        "body_markdown":"Hi Bob"
    }"#
}

fn update_recipients_body(field: &str, recipients: Vec<String>) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert(
        field.to_string(),
        Value::Array(recipients.into_iter().map(Value::String).collect()),
    );
    Value::Object(payload).to_string()
}

fn recipient_list(count: usize) -> Vec<String> {
    (0..count)
        .map(|idx| format!("user{idx}@example.org"))
        .collect()
}

#[tokio::test]
async fn create_requires_auth() {
    let (state, _key) = fixture_state().await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store,
        Method::POST,
        "/api/drafts",
        None,
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_requires_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store,
        Method::POST,
        "/api/drafts",
        Some(&sid),
        false,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_requires_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store,
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        Some(r#"{"subject":"Revised"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_draft_calls_store_and_returns_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert!(body["updated_at"].as_str().is_some());
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "alice@example.org".to_string(),
            to: vec!["bob@example.org".to_string()],
            cc: vec!["carol@example.org".to_string()],
            bcc: vec![],
            subject: "Hello".to_string(),
            body_markdown: "Hi Bob".to_string(),
        }]
    );
}

#[tokio::test]
async fn create_draft_accepts_missing_send_fields() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "alice@example.org".to_string(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: String::new(),
            body_markdown: String::new(),
        }]
    );
}

#[tokio::test]
async fn create_draft_accepts_empty_send_fields() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"to":[],"cc":[],"bcc":[],"subject":"","body_markdown":"","attachments":[]}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "alice@example.org".to_string(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: String::new(),
            body_markdown: String::new(),
        }]
    );
}

#[tokio::test]
async fn get_draft_returns_saved_composer_fields() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::GET,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert_eq!(body["to"], serde_json::json!(["bob@example.org"]));
    assert_eq!(body["cc"], serde_json::json!(["carol@example.org"]));
    assert_eq!(body["bcc"], serde_json::json!(["dana@example.org"]));
    assert_eq!(body["subject"], "Saved draft");
    assert_eq!(body["body_markdown"], "Saved body");
    assert_eq!(
        store.calls(),
        vec![Call::Get {
            draft_id: "draft-1".to_string(),
        }]
    );
}

#[tokio::test]
async fn update_draft_calls_store_and_returns_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"to":["erin@example.org"],"cc":["carol@example.org"],"bcc":["blind@example.org"],"subject":"Revised","body_markdown":"new body"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert!(body["updated_at"].as_str().is_some());
    assert_eq!(
        store.calls(),
        vec![Call::Update {
            draft_id: "draft-1".to_string(),
            to: Some(vec!["erin@example.org".to_string()]),
            cc: Some(vec!["carol@example.org".to_string()]),
            bcc: Some(vec!["blind@example.org".to_string()]),
            subject: Some("Revised".to_string()),
            body_markdown: Some("new body".to_string()),
        }]
    );
}

#[tokio::test]
async fn invalid_recipient_returns_400_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"to":["not-an-email"],"subject":"x","body_markdown":"y"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid_to");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_attachments_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(
            r#"{"to":["bob@example.org"],"subject":"x","body_markdown":"y","attachments":[{"name":"a.txt"}]}"#,
        ),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "attachments_not_supported");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_attachments_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"subject":"x","attachments":[{"name":"a.txt"}]}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "attachments_not_supported");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_invalid_cc_bcc_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [("cc", "invalid_cc"), ("bcc", "invalid_bcc")] {
        let body = serde_json::json!({
            "to": ["bob@example.org"],
            field: ["not-an-email"],
            "subject": "x",
            "body_markdown": "y",
        })
        .to_string();
        let resp = request(
            state.clone(),
            store.clone(),
            Method::POST,
            "/api/drafts",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_too_many_recipients_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [
        ("to", "too_many_to"),
        ("cc", "too_many_cc"),
        ("bcc", "too_many_bcc"),
    ] {
        let mut body = serde_json::json!({
            "to": ["bob@example.org"],
            "subject": "x",
            "body_markdown": "y",
        });
        body[field] = serde_json::json!(recipient_list(201));
        let body = body.to_string();
        let resp = request(
            state.clone(),
            store.clone(),
            Method::POST,
            "/api/drafts",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_subject_crlf_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "to": ["bob@example.org"],
        "subject": "Hi\r\nBcc: attacker@example.org",
        "body_markdown": "y",
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid_subject");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_subject_crlf_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "subject": "Hi\nInjected: header",
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid_subject");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_body_too_large_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "to": ["bob@example.org"],
        "subject": "x",
        "body_markdown": "x".repeat(1024 * 1024 + 1),
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "body_too_large");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_body_too_large_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "body_markdown": "x".repeat(1024 * 1024 + 1),
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "body_too_large");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_empty_patch_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "empty_patch");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_invalid_cc_bcc_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [("cc", "invalid_cc"), ("bcc", "invalid_bcc")] {
        let body = update_recipients_body(field, vec!["not-an-email".to_string()]);
        let resp = request(
            state.clone(),
            store.clone(),
            Method::PATCH,
            "/api/drafts/draft-1",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_too_many_cc_bcc_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [("cc", "too_many_cc"), ("bcc", "too_many_bcc")] {
        let body = update_recipients_body(field, recipient_list(201));
        let resp = request(
            state.clone(),
            store.clone(),
            Method::PATCH,
            "/api/drafts/draft-1",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn provider_error_returns_500() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next_provider();

    let resp = request(
        state,
        store,
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "internal");
}

#[tokio::test]
async fn update_not_found_returns_404() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next(DraftStoreError::NotFound);

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/missing-draft",
        Some(&sid),
        true,
        Some(r#"{"subject":"Revised"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        store.calls(),
        Vec::<Call>::new(),
        "fake store returns before recording the mutation"
    );
}

#[tokio::test]
async fn delete_not_found_returns_404() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next(DraftStoreError::NotFound);

    let resp = request(
        state,
        store.clone(),
        Method::DELETE,
        "/api/drafts/missing-draft",
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        store.calls(),
        Vec::<Call>::new(),
        "fake store returns before recording the mutation"
    );
}

#[tokio::test]
async fn delete_draft_requires_csrf_and_deletes_by_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "delete-draft@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state.clone(),
        store.clone(),
        Method::DELETE,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = request(
        state,
        store.clone(),
        Method::DELETE,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        store.calls(),
        vec![Call::Delete {
            draft_id: "draft-1".to_string()
        }]
    );
}

#[tokio::test]
async fn create_returns_clear_400_when_sender_identity_missing() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "missing-draft-identity@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next_sender_identity();

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "sender_identity_unavailable");
    assert!(store.calls().is_empty());
}
