use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use hail_core::MailClassification;
use hail_test::{TempDb, fresh_db_url};
use sqlx::{SqliteConnection, SqlitePool};

use hail_worker::provider_import_routing::{Rfc822ImportRouter, ScreenerRfc822ImportRouter};
use hail_worker::rfc822_import::{ImportedRfc822Message, Rfc822ImportRequest};
use hail_worker::screener::{EmailEnvelope, JmapOps, RouteError, RouteOutcome, route_email};
use hail_worker::workflows::{WorkflowMessageContext, evaluate_workflows};

#[derive(Debug, Default)]
struct FakeJmapOps {
    keywords: Mutex<HashMap<String, HashSet<String>>>,
    moves: Mutex<Vec<(String, String)>>,
}

impl FakeJmapOps {
    fn keywords_for(&self, email_id: &str) -> HashSet<String> {
        self.keywords
            .lock()
            .expect("keywords lock")
            .get(email_id)
            .cloned()
            .unwrap_or_default()
    }

    fn moves(&self) -> Vec<(String, String)> {
        self.moves.lock().expect("moves lock").clone()
    }
}

#[async_trait]
impl JmapOps for FakeJmapOps {
    async fn get_or_create_mailbox(&self, name: &str) -> Result<String, RouteError> {
        Ok(format!("{name}-id"))
    }

    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError> {
        Ok(Some(format!("{role}-id")))
    }

    async fn apply_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.keywords
            .lock()
            .expect("keywords lock")
            .entry(email_id.to_string())
            .or_default()
            .insert(keyword.to_string());
        Ok(())
    }

    async fn remove_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        if let Some(keywords) = self
            .keywords
            .lock()
            .expect("keywords lock")
            .get_mut(email_id)
        {
            keywords.remove(keyword);
        }
        Ok(())
    }

    async fn move_to_mailbox(&self, email_id: &str, mailbox_id: &str) -> Result<(), RouteError> {
        self.moves
            .lock()
            .expect("moves lock")
            .push((email_id.to_string(), mailbox_id.to_string()));
        Ok(())
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64, i64) {
    let (url, guard) = fresh_db_url("hail-worker-workflows-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    for email in ["alice@example.com", "bob@example.com"] {
        sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?1, ?2, ?3)")
            .bind(email)
            .bind(format!("acct-{email}"))
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("insert user");
    }

    let alice_id = user_id(&pool, "alice@example.com").await;
    let bob_id = user_id(&pool, "bob@example.com").await;
    (pool, guard, alice_id, bob_id)
}

async fn user_id(pool: &SqlitePool, email: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("fetch user id")
}

async fn insert_screener_rule(
    conn: &mut SqliteConnection,
    user_id: i64,
    decision: &str,
    classify_as: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO screener_rules (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) VALUES (?1, 'sender@example.com', ?2, ?3, ?4, ?5)",
    )
    .bind(user_id)
    .bind(decision)
    .bind(classify_as)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(conn)
    .await
    .expect("insert screener rule");
}

async fn insert_workflow(
    conn: &mut SqliteConnection,
    user_id: i64,
    enabled: bool,
    conditions_json: &str,
    action_json: &str,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO workflow_rules (user_id, name, enabled, conditions_json, action_json, created_at, updated_at) VALUES (?1, 'rule', ?2, ?3, ?4, ?5, ?5) RETURNING id",
    )
    .bind(user_id)
    .bind(if enabled { 1 } else { 0 })
    .bind(conditions_json)
    .bind(action_json)
    .bind("2026-01-01T00:00:00Z")
    .fetch_one(conn)
    .await
    .expect("insert workflow")
}

fn envelope() -> EmailEnvelope {
    EmailEnvelope {
        id: "email-1".to_string(),
        thread_id: "thread-1".to_string(),
        from: "sender@example.com".to_string(),
        to: vec![
            "alice@example.com".to_string(),
            "team@example.com".to_string(),
        ],
        cc: vec!["audit@example.com".to_string()],
        subject: "Quarterly receipt ready".to_string(),
        preview: None,
        raw_rfc822: None,
        mailbox_ids: vec!["inbox-id".to_string()],
        keywords: Vec::new(),
        received_at: None,
        size: Some(123),
    }
}

fn context() -> WorkflowMessageContext {
    let env = envelope();
    WorkflowMessageContext {
        email_id: env.id,
        thread_id: env.thread_id,
        from: env.from,
        to: env.to,
        cc: env.cc,
        subject: env.subject,
    }
}

#[tokio::test]
async fn enabled_rules_match_but_disabled_rules_do_not() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_workflow(
        conn.as_mut(),
        alice_id,
        false,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"feed"}"#,
    )
    .await;
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"papertrail"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let evaluation = evaluate_workflows(conn.as_mut(), &jmap, alice_id, &context())
        .await
        .expect("evaluate");

    assert_eq!(
        evaluation.classification,
        Some(MailClassification::Papertrail)
    );
    assert!(jmap.keywords_for("email-1").contains("$hail_papertrail"));
    assert!(!jmap.keywords_for("email-1").contains("$hail_feed"));
}

#[tokio::test]
async fn conditions_match_from_to_cc_subject_contains_equals_and_and_together() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[
      {"field":"from","op":"equals","value":"sender@example.com"},
      {"field":"to","op":"contains","value":"team@"},
      {"field":"cc","op":"equals","value":"audit@example.com"},
      {"field":"subject","op":"contains","value":"receipt"}
    ]"#,
        r#"{"classify_as":"papertrail"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let evaluation = evaluate_workflows(conn.as_mut(), &jmap, alice_id, &context())
        .await
        .expect("evaluate");

    assert_eq!(
        evaluation.classification,
        Some(MailClassification::Papertrail)
    );
}

#[tokio::test]
async fn and_condition_mismatch_returns_no_match() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[
      {"field":"from","op":"equals","value":"sender@example.com"},
      {"field":"cc","op":"equals","value":"missing@example.com"}
    ]"#,
        r#"{"classify_as":"feed"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let evaluation = evaluate_workflows(conn.as_mut(), &jmap, alice_id, &context())
        .await
        .expect("evaluate");

    assert_eq!(evaluation.matched_rule_id, None);
    assert!(jmap.keywords_for("email-1").is_empty());
}

#[tokio::test]
async fn rules_are_scoped_to_user() {
    let (pool, _guard, alice_id, bob_id) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_workflow(
        conn.as_mut(),
        bob_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"feed"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let evaluation = evaluate_workflows(conn.as_mut(), &jmap, alice_id, &context())
        .await
        .expect("evaluate");

    assert_eq!(evaluation.matched_rule_id, None);
}

#[tokio::test]
async fn first_matching_rule_wins_by_id_order() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    let first_id = insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"feed"}"#,
    )
    .await;
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"papertrail"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let evaluation = evaluate_workflows(conn.as_mut(), &jmap, alice_id, &context())
        .await
        .expect("evaluate");

    assert_eq!(evaluation.matched_rule_id, Some(first_id));
    assert_eq!(evaluation.classification, Some(MailClassification::Feed));
    assert!(jmap.keywords_for("email-1").contains("$hail_feed"));
    assert!(!jmap.keywords_for("email-1").contains("$hail_papertrail"));
}

#[tokio::test]
async fn classify_as_overrides_allowed_screener_default_routing() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_screener_rule(conn.as_mut(), alice_id, "allow", Some("imbox")).await;
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"papertrail"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let outcome = route_email(conn.as_mut(), &jmap, alice_id, &envelope())
        .await
        .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::Classified {
            classification: MailClassification::Papertrail
        }
    );
    let keywords = jmap.keywords_for("email-1");
    assert!(keywords.contains("$hail_papertrail"));
    assert!(!keywords.contains("$hail_imbox"));
}

#[tokio::test]
async fn add_label_assigns_to_thread_and_is_idempotent() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"add_label":"Work/Receipts"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();
    let ctx = context();

    evaluate_workflows(conn.as_mut(), &jmap, alice_id, &ctx)
        .await
        .expect("first eval");
    evaluate_workflows(conn.as_mut(), &jmap, alice_id, &ctx)
        .await
        .expect("second eval");

    let label_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM labels WHERE user_id = ?1")
        .bind(alice_id)
        .fetch_one(conn.as_mut())
        .await
        .expect("label count");
    let assignment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1")
            .bind(alice_id)
            .fetch_one(conn.as_mut())
            .await
            .expect("assignment count");

    assert_eq!(label_count, 1);
    assert_eq!(assignment_count, 1);
}

#[tokio::test]
async fn provider_import_routing_runs_workflows_after_dedupe_import() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_screener_rule(conn.as_mut(), alice_id, "allow", Some("imbox")).await;
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"to","op":"contains","value":"team@example.com"}]"#,
        r#"{"classify_as":"feed","add_label":"Team"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();
    let router = ScreenerRfc822ImportRouter::new(&jmap);
    let imported = ImportedRfc822Message {
        jmap_email_id: "email-imported".to_string(),
        jmap_thread_id: Some("thread-imported".to_string()),
        jmap_mailbox_ids: vec!["inbox-id".to_string()],
        rfc822_message_ids: vec!["message-id".to_string()],
        duplicate: false,
    };
    let request = Rfc822ImportRequest::into_mailbox(
        b"From: Sender <sender@example.com>\r\nTo: Team <team@example.com>\r\nSubject: Import\r\n\r\nBody"
            .to_vec(),
        "inbox-id",
    );

    router
        .route_imported_rfc822(conn.as_mut(), alice_id, &imported, &request)
        .await
        .expect("route import");

    assert!(jmap.keywords_for("email-imported").contains("$hail_feed"));
    let assignment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1 AND thread_id = 'thread-imported'",
    )
    .bind(alice_id)
    .fetch_one(conn.as_mut())
    .await
    .expect("assignment count");
    assert_eq!(assignment_count, 1);
}

#[tokio::test]
async fn denied_sender_does_not_run_workflows_but_allowed_sender_does() {
    let (pool, _guard, alice_id, _) = setup_db().await;
    let mut conn = pool.acquire().await.expect("conn");
    insert_screener_rule(conn.as_mut(), alice_id, "deny", None).await;
    insert_workflow(
        conn.as_mut(),
        alice_id,
        true,
        r#"[{"field":"subject","op":"contains","value":"receipt"}]"#,
        r#"{"classify_as":"papertrail","add_label":"Receipts"}"#,
    )
    .await;
    let jmap = FakeJmapOps::default();

    let denied = route_email(conn.as_mut(), &jmap, alice_id, &envelope())
        .await
        .expect("denied route");
    assert_eq!(denied, RouteOutcome::Trashed);
    assert_eq!(
        jmap.moves(),
        vec![("email-1".to_string(), "trash-id".to_string())]
    );
    assert!(jmap.keywords_for("email-1").is_empty());
    let assignment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1")
            .bind(alice_id)
            .fetch_one(conn.as_mut())
            .await
            .expect("assignment count");
    assert_eq!(assignment_count, 0);

    sqlx::query("UPDATE screener_rules SET decision = 'allow', classify_as = 'imbox' WHERE user_id = ?1 AND sender_address = 'sender@example.com'").bind(alice_id).execute(conn.as_mut()).await.expect("allow sender");
    let allowed = route_email(conn.as_mut(), &jmap, alice_id, &envelope())
        .await
        .expect("allowed route");

    assert_eq!(
        allowed,
        RouteOutcome::Classified {
            classification: MailClassification::Papertrail
        }
    );
    assert!(jmap.keywords_for("email-1").contains("$hail_papertrail"));
    let assignment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1")
            .bind(alice_id)
            .fetch_one(conn.as_mut())
            .await
            .expect("assignment count");
    assert_eq!(assignment_count, 1);
}
