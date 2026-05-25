use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_test::{TempDb, fresh_db_url};
use sqlx::{Connection, SqliteConnection};

#[path = "../src/screener.rs"]
mod screener;

use screener::{
    Classification, EmailEnvelope, JmapOps, RouteError, RouteOutcome, normalize_sender, route_email,
};

#[derive(Debug, Default)]
struct FakeJmapOps {
    calls: Mutex<Vec<Call>>,
    screener_id: String,
    trash_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    GetOrCreateMailbox(String),
    GetMailboxByRole(String),
    ApplyKeyword {
        email_id: String,
        keyword: String,
    },
    MoveToMailbox {
        email_id: String,
        mailbox_id: String,
    },
}

impl FakeJmapOps {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            screener_id: "screener-id".to_string(),
            trash_id: "trash-id".to_string(),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }
}

#[async_trait]
impl JmapOps for FakeJmapOps {
    async fn get_or_create_mailbox(&self, name: &str) -> Result<String, RouteError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push(Call::GetOrCreateMailbox(name.to_string()));
        Ok(self.screener_id.clone())
    }

    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push(Call::GetMailboxByRole(role.to_string()));
        Ok(if role == "trash" {
            Some(self.trash_id.clone())
        } else {
            None
        })
    }

    async fn apply_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push(Call::ApplyKeyword {
                email_id: email_id.to_string(),
                keyword: keyword.to_string(),
            });
        Ok(())
    }

    async fn move_to_mailbox(&self, email_id: &str, mailbox_id: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push(Call::MoveToMailbox {
                email_id: email_id.to_string(),
                mailbox_id: mailbox_id.to_string(),
            });
        Ok(())
    }
}

async fn setup_db() -> (SqliteConnection, TempDb, i64, i64) {
    let (url, guard) = fresh_db_url("hail-worker-screener-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    for email in ["alice@example.com", "bob@example.com"] {
        sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
            .bind(email)
            .bind(format!("acct-{email}"))
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("insert user");
    }

    let alice_id = user_id(&pool, "alice@example.com").await;
    let bob_id = user_id(&pool, "bob@example.com").await;
    pool.close().await;
    let conn = SqliteConnection::connect(&url).await.expect("connect conn");
    (conn, guard, alice_id, bob_id)
}

async fn user_id(pool: &sqlx::SqlitePool, email: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("fetch user id")
}

fn envelope(from: &str) -> EmailEnvelope {
    envelope_with_id(from, "email-1", vec!["inbox-id".to_string()])
}

fn envelope_with_id(from: &str, id: &str, mailbox_ids: Vec<String>) -> EmailEnvelope {
    EmailEnvelope {
        id: id.to_string(),
        thread_id: "thread-1".to_string(),
        from: from.to_string(),
        subject: "subject must not be info-logged".to_string(),
        mailbox_ids,
        keywords: vec![],
        received_at: None,
        size: Some(123),
    }
}

async fn insert_rule(
    conn: &mut SqliteConnection,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO screener_rules \
         (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(sender)
    .bind(decision)
    .bind(classify_as)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(conn)
    .await
    .expect("insert rule");
}

#[test]
fn normalize_sender_cases() {
    let cases = [
        ("John <john@FOO.com>", "john@foo.com"),
        ("  bob@bar.org ", "bob@bar.org"),
        ("alice@example.com", "alice@example.com"),
        ("Jane Q. Public < JANE@Example.COM >", "jane@example.com"),
        ("UPPER@EXAMPLE.ORG", "upper@example.org"),
    ];
    for (input, expected) in cases {
        assert_eq!(normalize_sender(input), expected);
    }
}

#[tokio::test]
async fn allow_rule_classifies_and_applies_keyword() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_rule(
        &mut conn,
        alice_id,
        "sender@example.com",
        "allow",
        Some("imbox"),
    )
    .await;
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(
        &mut conn,
        jmap.as_ref(),
        alice_id,
        &envelope("sender@example.com"),
    )
    .await
    .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::Classified {
            classification: Classification::Imbox
        }
    );
    assert_eq!(
        jmap.calls(),
        vec![Call::ApplyKeyword {
            email_id: "email-1".to_string(),
            keyword: "$hail_imbox".to_string()
        }]
    );
}

#[tokio::test]
async fn deny_rule_moves_to_trash() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_rule(&mut conn, alice_id, "sender@example.com", "deny", None).await;
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(
        &mut conn,
        jmap.as_ref(),
        alice_id,
        &envelope("sender@example.com"),
    )
    .await
    .expect("route");

    assert_eq!(outcome, RouteOutcome::Trashed);
    assert_eq!(
        jmap.calls(),
        vec![
            Call::GetMailboxByRole("trash".to_string()),
            Call::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "trash-id".to_string()
            }
        ]
    );
}

#[tokio::test]
async fn no_rule_moves_to_screener_and_inserts_pending() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(
        &mut conn,
        jmap.as_ref(),
        alice_id,
        &envelope("new@example.com"),
    )
    .await
    .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::ScreenerPending {
            sender: "new@example.com".to_string()
        }
    );
    assert_eq!(
        jmap.calls(),
        vec![
            Call::GetOrCreateMailbox("Screener".to_string()),
            Call::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "screener-id".to_string()
            }
        ]
    );
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(alice_id)
    .bind("new@example.com")
    .fetch_one(&mut conn)
    .await
    .expect("pending row");
    assert_eq!(row, ("pending".to_string(), None));
}

#[tokio::test]
async fn pending_rule_moves_subsequent_message_to_screener() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_rule(&mut conn, alice_id, "sender@example.com", "pending", None).await;
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(
        &mut conn,
        jmap.as_ref(),
        alice_id,
        &envelope_with_id(
            "sender@example.com",
            "email-2",
            vec!["inbox-id".to_string()],
        ),
    )
    .await
    .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::ScreenerPending {
            sender: "sender@example.com".to_string()
        }
    );
    assert_eq!(
        jmap.calls(),
        vec![
            Call::GetOrCreateMailbox("Screener".to_string()),
            Call::MoveToMailbox {
                email_id: "email-2".to_string(),
                mailbox_id: "screener-id".to_string()
            }
        ]
    );
    let rule_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(alice_id)
    .bind("sender@example.com")
    .fetch_one(&mut conn)
    .await
    .expect("rule count");
    assert_eq!(rule_count, 1);
}

#[tokio::test]
async fn pending_rule_is_idempotent_when_message_already_in_screener() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_rule(&mut conn, alice_id, "sender@example.com", "pending", None).await;
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(
        &mut conn,
        jmap.as_ref(),
        alice_id,
        &envelope_with_id(
            "sender@example.com",
            "email-2",
            vec!["screener-id".to_string()],
        ),
    )
    .await
    .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::ScreenerPending {
            sender: "sender@example.com".to_string()
        }
    );
    assert_eq!(
        jmap.calls(),
        vec![Call::GetOrCreateMailbox("Screener".to_string())]
    );
}

#[tokio::test]
async fn existing_hail_keyword_is_idempotent_skip() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_rule(
        &mut conn,
        alice_id,
        "sender@example.com",
        "allow",
        Some("imbox"),
    )
    .await;
    let mut env = envelope("sender@example.com");
    env.keywords.push("$hail_imbox".to_string());
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(&mut conn, jmap.as_ref(), alice_id, &env)
        .await
        .expect("route");

    assert_eq!(outcome, RouteOutcome::AlreadyScreened);
    assert!(jmap.calls().is_empty(), "no JMAP calls expected");
}

#[tokio::test]
async fn wrong_user_rule_does_not_affect_other_user() {
    let (mut conn, _guard, alice_id, bob_id) = setup_db().await;
    insert_rule(
        &mut conn,
        alice_id,
        "sender@example.com",
        "allow",
        Some("feed"),
    )
    .await;
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(
        &mut conn,
        jmap.as_ref(),
        bob_id,
        &envelope("sender@example.com"),
    )
    .await
    .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::ScreenerPending {
            sender: "sender@example.com".to_string()
        }
    );
    assert_eq!(
        jmap.calls(),
        vec![
            Call::GetOrCreateMailbox("Screener".to_string()),
            Call::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "screener-id".to_string()
            }
        ]
    );

    let bob_decision: String = sqlx::query_scalar(
        "SELECT decision FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(bob_id)
    .bind("sender@example.com")
    .fetch_one(&mut conn)
    .await
    .expect("bob pending row");
    assert_eq!(bob_decision, "pending");
}
