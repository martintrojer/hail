use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_test::{TempDb, fresh_db_url};
use sqlx::{Connection, SqliteConnection};

#[path = "../src/screener.rs"]
mod screener;

use screener::{
    Classification, EmailEnvelope, JmapOps, RouteError, RouteOutcome, is_spam_flagged,
    normalize_sender, route_email,
};

#[derive(Debug, Default)]
struct FakeJmapOps {
    calls: Mutex<Vec<Call>>,
    screener_id: String,
    trash_id: String,
    junk_id: String,
    junk_role_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    GetOrCreateMailbox(String),
    GetMailboxByRole(String),
    ApplyKeyword {
        email_id: String,
        keyword: String,
    },
    RemoveKeyword {
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
            junk_id: "junk-id".to_string(),
            junk_role_exists: true,
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
        Ok(if name == "Junk" {
            self.junk_id.clone()
        } else {
            self.screener_id.clone()
        })
    }

    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push(Call::GetMailboxByRole(role.to_string()));
        Ok(match role {
            "trash" => Some(self.trash_id.clone()),
            "junk" if self.junk_role_exists => Some(self.junk_id.clone()),
            _ => None,
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

    async fn remove_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push(Call::RemoveKeyword {
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
        preview: None,
        raw_rfc822: None,
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

async fn insert_speakeasy(conn: &mut SqliteConnection, user_id: i64, passphrase: &str) {
    sqlx::query(
        "INSERT INTO speakeasy_passphrases \
         (user_id, passphrase, period, rotates_at, generated_at, updated_at) \
         VALUES (?, ?, '2026-05', ?, ?, ?)",
    )
    .bind(user_id)
    .bind(passphrase)
    .bind("2026-06-01T00:00:00Z")
    .bind("2026-05-27T12:00:00Z")
    .bind("2026-05-27T12:00:00Z")
    .execute(conn)
    .await
    .expect("insert speakeasy");
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

#[test]
fn spam_flagged_keywords_detect_stalwart_junk_verdict() {
    assert!(is_spam_flagged(&["$Junk".to_string()]));
    assert!(is_spam_flagged(&["Junk".to_string()]));
    assert!(is_spam_flagged(&["$seen".to_string(), "$Junk".to_string()]));
    assert!(!is_spam_flagged(&["$NotJunk".to_string()]));
    assert!(!is_spam_flagged(&[]));
}

#[tokio::test]
async fn stalwart_spam_flag_moves_to_junk_and_bypasses_screener() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    let jmap = Arc::new(FakeJmapOps::new());
    let mut env = envelope("new@example.com");
    env.keywords.push("$Junk".to_string());

    let outcome = route_email(&mut conn, jmap.as_ref(), alice_id, &env)
        .await
        .expect("route spam");

    assert_eq!(outcome, RouteOutcome::Spam);
    assert_eq!(
        jmap.calls(),
        vec![
            Call::GetMailboxByRole("junk".to_string()),
            Call::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "junk-id".to_string()
            },
            Call::ApplyKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_spam".to_string()
            }
        ]
    );
    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(alice_id)
    .bind("new@example.com")
    .fetch_one(&mut conn)
    .await
    .expect("pending count");
    assert_eq!(pending_count, 0, "spam must not create a pending rule");
}

#[tokio::test]
async fn stalwart_spam_flag_falls_back_to_junk_mailbox_by_name() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    let mut fake = FakeJmapOps::new();
    fake.junk_role_exists = false;
    let jmap = Arc::new(fake);
    let mut env = envelope("new@example.com");
    env.keywords.push("Junk".to_string());

    let outcome = route_email(&mut conn, jmap.as_ref(), alice_id, &env)
        .await
        .expect("route spam");

    assert_eq!(outcome, RouteOutcome::Spam);
    assert_eq!(
        jmap.calls(),
        vec![
            Call::GetMailboxByRole("junk".to_string()),
            Call::GetOrCreateMailbox("Junk".to_string()),
            Call::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "junk-id".to_string()
            },
            Call::ApplyKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_spam".to_string()
            }
        ]
    );
}

#[tokio::test]
async fn speakeasy_subject_match_bypasses_screener_without_creating_rule() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_speakeasy(&mut conn, alice_id, "amber-basil-coral-delta").await;
    let mut env = envelope("new@example.com");
    env.subject = "Please let amber-basil-coral-delta through".to_string();
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(&mut conn, jmap.as_ref(), alice_id, &env)
        .await
        .expect("route");

    assert_eq!(outcome, RouteOutcome::SpeakeasyBypass);
    assert_eq!(
        jmap.calls(),
        vec![Call::ApplyKeyword {
            email_id: "email-1".to_string(),
            keyword: "$hail_imbox".to_string(),
        }]
    );
    let rule_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(alice_id)
    .bind("new@example.com")
    .fetch_one(&mut conn)
    .await
    .expect("rule count");
    assert_eq!(rule_count, 0, "Speakeasy must not approve or pend sender");
}

#[tokio::test]
async fn speakeasy_body_match_bypasses_pending_sender_for_message_only() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
    insert_speakeasy(&mut conn, alice_id, "amber-basil-coral-delta").await;
    insert_rule(&mut conn, alice_id, "sender@example.com", "pending", None).await;
    let mut env = envelope("sender@example.com");
    env.raw_rfc822 = Some(
        b"From: sender@example.com\r\nSubject: hi\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nThe passphrase is amber-basil-coral-delta."
            .to_vec(),
    );
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(&mut conn, jmap.as_ref(), alice_id, &env)
        .await
        .expect("route");

    assert_eq!(outcome, RouteOutcome::SpeakeasyBypass);
    assert_eq!(
        jmap.calls(),
        vec![Call::ApplyKeyword {
            email_id: "email-1".to_string(),
            keyword: "$hail_imbox".to_string(),
        }]
    );
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(alice_id)
    .bind("sender@example.com")
    .fetch_one(&mut conn)
    .await
    .expect("pending row");
    assert_eq!(row, ("pending".to_string(), None));
}

#[tokio::test]
async fn speakeasy_match_is_scoped_to_user() {
    let (mut conn, _guard, alice_id, bob_id) = setup_db().await;
    insert_speakeasy(&mut conn, alice_id, "amber-basil-coral-delta").await;
    let mut env = envelope("new@example.com");
    env.subject = "amber-basil-coral-delta".to_string();
    let jmap = Arc::new(FakeJmapOps::new());

    let outcome = route_email(&mut conn, jmap.as_ref(), bob_id, &env)
        .await
        .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::ScreenerPending {
            sender: "new@example.com".to_string()
        }
    );
    assert!(
        jmap.calls()
            .iter()
            .any(|call| matches!(call, Call::MoveToMailbox { mailbox_id, .. } if mailbox_id == "screener-id")),
        "non-owner secret must not bypass screener"
    );
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
async fn rule_lookup_normalizes_envelope_sender_before_matching() {
    let (mut conn, _guard, alice_id, _) = setup_db().await;
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
        alice_id,
        &envelope(" Sender Name <SENDER@Example.COM> "),
    )
    .await
    .expect("route");

    assert_eq!(
        outcome,
        RouteOutcome::Classified {
            classification: Classification::Feed
        }
    );
    assert_eq!(
        jmap.calls(),
        vec![Call::ApplyKeyword {
            email_id: "email-1".to_string(),
            keyword: "$hail_feed".to_string()
        }]
    );

    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(alice_id)
    .bind("sender name <sender@example.com>")
    .fetch_one(&mut conn)
    .await
    .expect("pending count");
    assert_eq!(
        pending_count, 0,
        "normalized match must not create a pending rule"
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
            },
            Call::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
            Call::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_feed".to_string(),
            },
            Call::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_papertrail".to_string(),
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
            },
            Call::RemoveKeyword {
                email_id: "email-2".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
            Call::RemoveKeyword {
                email_id: "email-2".to_string(),
                keyword: "$hail_feed".to_string(),
            },
            Call::RemoveKeyword {
                email_id: "email-2".to_string(),
                keyword: "$hail_papertrail".to_string(),
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
            },
            Call::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
            Call::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_feed".to_string(),
            },
            Call::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_papertrail".to_string(),
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
