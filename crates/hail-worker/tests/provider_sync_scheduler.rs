use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use hail_db::provider_sync_audit::list_provider_sync_audit_logs;
use hail_test::{TempDb, fresh_db_url};
use hail_worker::provider_sync_scheduler::{
    ProviderSyncAccount, ProviderSyncRunError, ProviderSyncRunOutcome, ProviderSyncRunner,
    ProviderSyncSchedulerOptions, process_provider_sync_tick,
};
use sqlx::SqlitePool;
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;

fn hostile_leak_error() -> String {
    "sync failed Authorization: Bearer ya29.scheduler-secret access_token=ya29.scheduler-secret refresh_token=1//scheduler-refresh\n\nSubject: Scheduler Private\n\nScheduler body text must not be stored".to_owned()
}

fn assert_no_hostile_leak(surface: &str) {
    for forbidden in [
        "Bearer",
        "ya29.scheduler-secret",
        "1//scheduler-refresh",
        "Subject: Scheduler Private",
        "Scheduler body text must not be stored",
    ] {
        assert!(
            !surface.contains(forbidden),
            "surface leaked {forbidden:?}: {surface}"
        );
    }
}

#[derive(Debug, Default)]
struct FakeRunner {
    calls: Mutex<Vec<(i64, &'static str)>>,
    results: Mutex<HashMap<i64, Result<ProviderSyncRunOutcome, ProviderSyncRunError>>>,
}

impl FakeRunner {
    fn set_result(&self, id: i64, result: Result<ProviderSyncRunOutcome, ProviderSyncRunError>) {
        self.results.lock().expect("results").insert(id, result);
    }

    fn calls(&self) -> Vec<(i64, &'static str)> {
        self.calls.lock().expect("calls").clone()
    }
}

#[async_trait]
impl ProviderSyncRunner for FakeRunner {
    async fn run_initial_sync(
        &self,
        account: ProviderSyncAccount,
        _cancel: &CancellationToken,
    ) -> Result<ProviderSyncRunOutcome, ProviderSyncRunError> {
        self.calls
            .lock()
            .expect("calls")
            .push((account.id, "initial"));
        self.results
            .lock()
            .expect("results")
            .remove(&account.id)
            .unwrap_or_else(|| Ok(ProviderSyncRunOutcome::completed_active()))
    }

    async fn run_incremental_sync(
        &self,
        account: ProviderSyncAccount,
        _cancel: &CancellationToken,
    ) -> Result<ProviderSyncRunOutcome, ProviderSyncRunError> {
        self.calls
            .lock()
            .expect("calls")
            .push((account.id, "incremental"));
        self.results
            .lock()
            .expect("results")
            .remove(&account.id)
            .unwrap_or_else(|| Ok(ProviderSyncRunOutcome::completed_active()))
    }
}

#[derive(Debug)]
struct BlockingRunner {
    entered: Barrier,
}

#[async_trait]
impl ProviderSyncRunner for BlockingRunner {
    async fn run_initial_sync(
        &self,
        _account: ProviderSyncAccount,
        cancel: &CancellationToken,
    ) -> Result<ProviderSyncRunOutcome, ProviderSyncRunError> {
        self.entered.wait().await;
        cancel.cancelled().await;
        Ok(ProviderSyncRunOutcome::completed_active())
    }

    async fn run_incremental_sync(
        &self,
        account: ProviderSyncAccount,
        cancel: &CancellationToken,
    ) -> Result<ProviderSyncRunOutcome, ProviderSyncRunError> {
        self.run_initial_sync(account, cancel).await
    }
}

async fn setup() -> (SqlitePool, TempDb, i64) {
    let (url, guard) = fresh_db_url("hail-worker-provider-sync-scheduler-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("provider@example.com")
        .bind("acct-provider")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("insert user");
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("provider@example.com")
        .fetch_one(&pool)
        .await
        .expect("user id");
    (pool, guard, user_id)
}

async fn insert_provider(
    pool: &SqlitePool,
    user_id: i64,
    status: &str,
    history_id: Option<&str>,
    last_attempt: Option<String>,
    next_sync_after: Option<String>,
) -> i64 {
    sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, refresh_token_enc, \
          last_profile_history_id, initial_sync_completed_at, sync_status, last_sync_attempted_at, next_sync_after, created_at, updated_at) \
         VALUES (?, 'acct-provider', 'gmail', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(format!("gmail-{status}-{history_id:?}-{next_sync_after:?}"))
    .bind(format!("{status}@gmail.example"))
    .bind(vec![1_u8; 29])
    .bind(history_id)
    .bind((status == "active").then_some("2026-01-01T00:10:00Z"))
    .bind(status)
    .bind(last_attempt)
    .bind(next_sync_after)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("provider insert")
    .last_insert_rowid()
}

async fn provider_state(
    pool: &SqlitePool,
    id: i64,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
) {
    sqlx::query_as(
        "SELECT sync_status, last_sync_succeeded_at, last_error_class, next_sync_after, sync_backoff_secs \
         FROM provider_accounts WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("provider state")
}

#[tokio::test]
async fn tick_runs_initial_and_incremental_accounts_and_persists_success() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let initial = insert_provider(&pool, user_id, "initial_sync", None, None, None).await;
    let incremental = insert_provider(&pool, user_id, "active", Some("100"), None, None).await;
    let runner = FakeRunner::default();

    let summary = process_provider_sync_tick(
        &pool,
        &runner,
        now,
        ProviderSyncSchedulerOptions::default(),
        &CancellationToken::new(),
    )
    .await
    .expect("provider sync tick");

    assert_eq!(summary.considered, 2);
    assert_eq!(summary.initial_runs, 1);
    assert_eq!(summary.incremental_runs, 1);
    assert_eq!(summary.succeeded, 2);
    assert_eq!(
        runner.calls(),
        vec![(initial, "initial"), (incremental, "incremental")]
    );
    assert_eq!(provider_state(&pool, initial).await.0, "active");
    assert_eq!(provider_state(&pool, incremental).await.0, "active");
    assert!(provider_state(&pool, initial).await.1.is_some());

    let audit = list_provider_sync_audit_logs(&pool, user_id, initial, 10)
        .await
        .expect("audit");
    assert!(audit.iter().any(|row| row.event_type == "sync_completed"));
}

#[tokio::test]
async fn scheduler_redacts_hostile_runner_error_in_account_and_audit_status() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let id = insert_provider(&pool, user_id, "active", Some("100"), None, None).await;
    let runner = FakeRunner::default();
    runner.set_result(
        id,
        Err(ProviderSyncRunError::retryable(
            "gmail_hostile_error",
            hostile_leak_error(),
        )),
    );

    let summary = process_provider_sync_tick(
        &pool,
        &runner,
        now,
        ProviderSyncSchedulerOptions::default(),
        &CancellationToken::new(),
    )
    .await
    .expect("provider sync tick");

    assert_eq!(summary.failed, 1);
    let account_error: String =
        sqlx::query_scalar("SELECT last_error_message FROM provider_accounts WHERE id = ?1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("last error message");
    assert_no_hostile_leak(&account_error);
    assert!(account_error.contains("[redacted]"));

    let audit = list_provider_sync_audit_logs(&pool, user_id, id, 10)
        .await
        .expect("audit");
    let failed = audit
        .iter()
        .find(|row| row.event_type == "message_retry_scheduled")
        .expect("message_retry_scheduled audit");
    let audit_error = failed
        .safe_error_message
        .as_deref()
        .expect("safe error message");
    assert_no_hostile_leak(audit_error);
    assert!(audit_error.contains("[redacted]"));
    let metadata = failed.metadata_json.as_deref().expect("retry metadata");
    assert_no_hostile_leak(metadata);
}

#[tokio::test]
async fn retryable_failure_sets_retry_backoff_and_audit() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let id = insert_provider(&pool, user_id, "active", Some("100"), None, None).await;
    let runner = FakeRunner::default();
    runner.set_result(
        id,
        Err(ProviderSyncRunError::retryable_after(
            "gmail_rate_limited",
            "rate limited",
            StdDuration::from_secs(120),
        )),
    );

    let summary = process_provider_sync_tick(
        &pool,
        &runner,
        now,
        ProviderSyncSchedulerOptions::default(),
        &CancellationToken::new(),
    )
    .await
    .expect("provider sync tick");

    assert_eq!(summary.failed, 1);
    let state = provider_state(&pool, id).await;
    assert_eq!(state.0, "error");
    assert_eq!(state.2.as_deref(), Some("gmail_rate_limited"));
    assert_eq!(state.4, Some(120));
    assert!(state.3.is_some_and(|next| next > now.to_rfc3339()));

    let audit = list_provider_sync_audit_logs(&pool, user_id, id, 10)
        .await
        .expect("audit");
    assert!(audit.iter().any(|row| {
        row.event_type == "message_retry_scheduled" && row.result_status == "retrying"
    }));
}

#[tokio::test]
async fn next_sync_after_prevents_early_retry() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let due = insert_provider(
        &pool,
        user_id,
        "error",
        Some("100"),
        Some((now - Duration::minutes(10)).to_rfc3339()),
        Some((now - Duration::seconds(1)).to_rfc3339()),
    )
    .await;
    sqlx::query("UPDATE provider_accounts SET initial_sync_completed_at = ?1 WHERE id = ?2")
        .bind("2026-01-01T00:10:00Z")
        .bind(due)
        .execute(&pool)
        .await
        .expect("mark initial complete");
    let future = insert_provider(
        &pool,
        user_id,
        "error",
        Some("200"),
        Some((now - Duration::minutes(10)).to_rfc3339()),
        Some((now + Duration::minutes(10)).to_rfc3339()),
    )
    .await;
    sqlx::query("UPDATE provider_accounts SET initial_sync_completed_at = ?1 WHERE id = ?2")
        .bind("2026-01-01T00:10:00Z")
        .bind(future)
        .execute(&pool)
        .await
        .expect("mark initial complete");
    let runner = FakeRunner::default();

    let summary = process_provider_sync_tick(
        &pool,
        &runner,
        now,
        ProviderSyncSchedulerOptions::default(),
        &CancellationToken::new(),
    )
    .await
    .expect("provider sync tick");

    assert_eq!(summary.considered, 1);
    assert_eq!(runner.calls(), vec![(due, "incremental")]);
    assert_eq!(provider_state(&pool, future).await.0, "error");
}

#[tokio::test]
async fn tick_ignores_accounts_with_unresolved_external_token_refs() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let id = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, refresh_token_ref, \
          sync_status, created_at, updated_at) \
         VALUES (?, 'acct-provider', 'gmail', 'gmail-ref-disabled', 'ref-disabled@gmail.example', \
                 'kms://hail/provider-token/1', 'disabled', ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("disabled provider ref insert")
    .last_insert_rowid();
    sqlx::query("UPDATE provider_accounts SET sync_status = 'active' WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .expect_err("constraint must reject active unresolved token refs");

    let runner = FakeRunner::default();
    let summary = process_provider_sync_tick(
        &pool,
        &runner,
        now,
        ProviderSyncSchedulerOptions::default(),
        &CancellationToken::new(),
    )
    .await
    .expect("provider sync tick");

    assert_eq!(summary.considered, 0);
    assert!(runner.calls().is_empty());
    assert_eq!(provider_state(&pool, id).await.0, "disabled");
}

#[tokio::test]
async fn errored_incomplete_initial_with_profile_history_retries_initial() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let incomplete = insert_provider(
        &pool,
        user_id,
        "error",
        Some("profile-history-before-import-failed"),
        Some((now - Duration::minutes(10)).to_rfc3339()),
        Some((now - Duration::seconds(1)).to_rfc3339()),
    )
    .await;
    let completed_then_errored = insert_provider(
        &pool,
        user_id,
        "error",
        Some("incremental-history"),
        Some((now - Duration::minutes(10)).to_rfc3339()),
        Some((now - Duration::seconds(1)).to_rfc3339()),
    )
    .await;
    sqlx::query("UPDATE provider_accounts SET initial_sync_completed_at = ?1 WHERE id = ?2")
        .bind("2026-01-01T00:10:00Z")
        .bind(completed_then_errored)
        .execute(&pool)
        .await
        .expect("mark initial complete");
    let runner = FakeRunner::default();

    let summary = process_provider_sync_tick(
        &pool,
        &runner,
        now,
        ProviderSyncSchedulerOptions::default(),
        &CancellationToken::new(),
    )
    .await
    .expect("provider sync tick");

    assert_eq!(summary.initial_runs, 1);
    assert_eq!(summary.incremental_runs, 1);
    assert_eq!(
        runner.calls(),
        vec![
            (incomplete, "initial"),
            (completed_then_errored, "incremental")
        ]
    );
}

#[tokio::test]
async fn tick_cancels_blocked_runner_without_waiting_forever() {
    let (pool, _guard, user_id) = setup().await;
    let now = Utc::now();
    let id = insert_provider(&pool, user_id, "initial_sync", None, None, None).await;
    let cancel = CancellationToken::new();
    let runner = Arc::new(BlockingRunner {
        entered: Barrier::new(2),
    });
    let tick_pool = pool.clone();
    let tick_runner = runner.clone();
    let tick_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        process_provider_sync_tick(
            &tick_pool,
            tick_runner.as_ref(),
            now,
            ProviderSyncSchedulerOptions::default(),
            &tick_cancel,
        )
        .await
        .expect("provider sync tick")
    });

    runner.entered.wait().await;
    cancel.cancel();
    let summary = tokio::time::timeout(StdDuration::from_millis(200), handle)
        .await
        .expect("tick should stop promptly")
        .expect("join");

    assert!(summary.cancelled);
    assert_eq!(
        provider_state(&pool, id).await.2.as_deref(),
        Some("cancelled")
    );
}
