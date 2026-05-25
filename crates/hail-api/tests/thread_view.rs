use http_body_util::BodyExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{TimeZone, Utc};
use hail_api::middleware::auth::require_auth;
use hail_api::routes::threads_view::{
    AssembledMessage, AssembledThread, Participant, ThreadAssembleError, ThreadAssembler,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, seed_session};
use secrecy::SecretString;
use tower::ServiceExt;

fn app(state: AppState, assembler: Arc<FakeAssembler>) -> Router {
    let protected = Router::from(hail_api::routes::threads_view::router_with_assembler(
        assembler,
    ))
    .layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone)]
struct FakeAssembler {
    result: Result<Option<AssembledThread>, String>,
}

impl ThreadAssembler for FakeAssembler {
    fn assemble<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<AssembledThread>, ThreadAssembleError>> + Send + 'a>,
    > {
        Box::pin(async move { self.result.clone().map_err(ThreadAssembleError) })
    }
}

fn sample_thread(messages: Vec<AssembledMessage>) -> AssembledThread {
    AssembledThread {
        thread_id: "thread-123".to_string(),
        subject: "Status".to_string(),
        messages,
    }
}

fn sample_message(email_id: &str, html: &str) -> AssembledMessage {
    AssembledMessage {
        email_id: email_id.to_string(),
        from: vec![Participant {
            name: Some("Alice".to_string()),
            email: "alice@example.org".to_string(),
        }],
        to: vec![Participant {
            name: Some("Bob".to_string()),
            email: "bob@example.org".to_string(),
        }],
        received_at: Some(Utc.with_ymd_and_hms(2026, 5, 23, 10, 0, 0).unwrap()),
        subject: "Status".to_string(),
        html: html.to_string(),
        text: String::new(),
        preview: "preview text".to_string(),
    }
}

fn sample_text_message(email_id: &str, text: &str) -> AssembledMessage {
    AssembledMessage {
        text: text.to_string(),
        ..sample_message(email_id, "")
    }
}

async fn get_json(
    state: AppState,
    sid: &str,
    assembler: Arc<FakeAssembler>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/threads/thread-123")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();

    let resp = app(state, assembler).oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn auth_required_returns_401() {
    let (state, _key) = fixture_state().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/threads/thread-123")
        .body(Body::empty())
        .unwrap();

    let resp = app(
        state,
        Arc::new(FakeAssembler {
            result: Ok(Some(sample_thread(Vec::new()))),
        }),
    )
    .oneshot(req)
    .await
    .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn not_found_returns_404() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let (status, json) = get_json(state, &sid, Arc::new(FakeAssembler { result: Ok(None) })).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn html_is_sanitized_and_script_removed() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let thread = sample_thread(vec![sample_message(
        "email-a",
        "<p>Hello</p><script>alert('xss')</script>",
    )]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let html = json["messages"][0]["html"].as_str().unwrap();
    assert!(html.contains("<p>Hello</p>"));
    assert!(!html.contains("script"));
    assert!(!html.contains("alert"));
}

#[tokio::test]
async fn quoted_history_is_stripped() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let thread = sample_thread(vec![sample_message(
        "email-a",
        r#"<p>New reply</p><div class="gmail_quote"><p>Old quoted text</p></div>"#,
    )]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let html = json["messages"][0]["html"].as_str().unwrap();
    assert!(html.contains("New reply"));
    assert!(!html.contains("Old quoted text"));
    assert!(!html.contains("gmail_quote"));
}

#[tokio::test]
async fn plaintext_only_body_renders_full_escaped_body() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let thread = sample_thread(vec![sample_text_message(
        "email-a",
        "Hello <Alice> & Bob\nSecond line\n\nFinal line",
    )]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let html = json["messages"][0]["html"].as_str().unwrap();
    assert!(html.contains("Hello &lt;Alice&gt; &amp; Bob"));
    assert!(html.contains("Second line"));
    assert!(html.contains("Final line"));
    assert!(html.contains("<br>"));
    assert!(!html.contains("Hello <Alice> & Bob"));
    assert_ne!(html, "preview text");
}

#[tokio::test]
async fn plaintext_quote_history_is_stripped() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let thread = sample_thread(vec![sample_text_message(
        "email-a",
        "Fresh answer\n\n> old quoted line\n> another old line",
    )]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let html = json["messages"][0]["html"].as_str().unwrap();
    assert!(html.contains("Fresh answer"));
    assert!(!html.contains("old quoted line"));
    assert!(!html.contains("another old line"));
}

#[tokio::test]
async fn html_body_wins_over_plaintext_fallback() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let mut message = sample_message("email-a", "<p>HTML body</p>");
    message.text = "Plaintext body".to_string();
    let thread = sample_thread(vec![message]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let html = json["messages"][0]["html"].as_str().unwrap();
    assert!(html.contains("<p>HTML body</p>"));
    assert!(!html.contains("Plaintext body"));
}

#[tokio::test]
async fn tracking_pixel_is_counted_and_removed() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let thread = sample_thread(vec![sample_message(
        "email-a",
        r#"<p>Hi</p><img src="https://tracker.example/open.gif" width="1" height="1">"#,
    )]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let html = json["messages"][0]["html"].as_str().unwrap();
    assert!(!html.contains("tracker.example"));
    let trackers = json["messages"][0]["blocked_trackers"].as_array().unwrap();
    assert_eq!(trackers.len(), 1);
    assert_eq!(trackers[0]["src"], "https://tracker.example/open.gif");
}

#[tokio::test]
async fn messages_preserve_assembler_order() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let thread = sample_thread(vec![
        sample_message("email-c", "<p>third first</p>"),
        sample_message("email-a", "<p>first second</p>"),
        sample_message("email-b", "<p>second third</p>"),
    ]);

    let (status, json) = get_json(
        state,
        &sid,
        Arc::new(FakeAssembler {
            result: Ok(Some(thread)),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let ids = json["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["email_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["email-c", "email-a", "email-b"]);
}
