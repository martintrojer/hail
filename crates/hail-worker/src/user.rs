//! Per-user JMAP push supervisor.
//!
//! See design.md §8.1 / §8.2. One task per active user owns the
//! EventSource stream and drives `handle_changes` on each push event
//! plus once at startup (the catch-up shape requested by the task
//! contract).

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::{Stream, StreamExt};
use hail_jmap::jmap_client;
use hail_jmap::jmap_client::DataType;
use hail_jmap::jmap_client::email::Property;
use hail_jmap::jmap_client::event_source::PushNotification;
use secrecy::SecretString;
use sqlx::SqlitePool;
use tokio::time::{MissedTickBehavior, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

use crate::backoff::Backoff;
use crate::catchup::catchup_user;
use crate::changes::{EmailChanges, EmailEnvelope, JmapChangeFetcher, handle_changes};
use crate::crypto::TokenDecryptor;
use crate::screener::{JmapOps, JmapOpsLive};
use crate::state::AppState;

/// EventSource keep-alive ping interval (seconds) — the task contract
/// fixes this at 60s.
const EVENT_SOURCE_PING_SECS: u32 = 60;

/// Default periodic catch-up cadence while EventSource is connected. This is a
/// safety net for JMAP servers/import paths that advance `Email/changes` state
/// without reliably delivering an EventSource notification.
const DEFAULT_LIVE_CATCHUP_SECS: u64 = 60;
const LIVE_CATCHUP_ENV: &str = "HAIL_IMPORT_CATCHUP_SECS";

/// Reasons a per-user supervisor stops itself for good rather than
/// retrying. FATAL outcomes are logged at ERROR; terminal exits are surfaced to
/// the top-level supervisor so it does not respawn the same user forever.
#[derive(Debug, thiserror::Error)]
enum FatalError {
    #[error("user {0} not found in hail.db")]
    UserMissing(i64),
    #[error("no active (non-expired) session for user {0}")]
    NoActiveSession(i64),
    #[error("failed to decrypt JMAP token: {0}")]
    TokenDecrypt(#[source] crate::crypto::DecryptError),
    #[error("JMAP server rejected our bearer token (401/403)")]
    AuthRevoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserSupervisorExit {
    Retryable,
    Terminal,
}

/// Per-user supervisor entry point. Owns the JMAP EventSource stream
/// for `user_id`, calls `handle_changes` on startup (catch-up shape)
/// and on every push event, and reconnects with exponential backoff
/// + full jitter on transient errors.
#[instrument(skip(state, cancel), fields(user_id = user_id))]
pub async fn run_user_supervisor(
    user_id: i64,
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> Result<UserSupervisorExit> {
    let decryptor = state.token_decryptor.clone();
    // Wrap the entire per-user pipeline in a top-level select! against
    // the cancel token. This is load-bearing: every `.await` inside
    // `run_user_supervisor_with` (DB queries, `login_bearer`'s TCP
    // connect, `handle_changes`'s `Email/get` round-trips,
    // `stream.next()`, etc.) gets dropped when cancel fires, which
    // tears down the in-flight `reqwest` connection via `Drop`. The
    // narrower inner `select!`s still preempt sleeps and stream waits
    // promptly, but this outer guard is what bounds shutdown time to
    // "however long Drop takes" — in practice <100ms even with no
    // reachable Stalwart.
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            info!(user_id, "per-user supervisor: cancelled (top-level)");
            Ok(UserSupervisorExit::Retryable)
        }
        res = run_user_supervisor_with(user_id, state, decryptor, cancel.clone()) => res,
    }
}

/// Inner form taking an explicit decryptor for test injection.
async fn run_user_supervisor_with(
    user_id: i64,
    state: Arc<AppState>,
    decryptor: Arc<dyn TokenDecryptor>,
    cancel: CancellationToken,
) -> Result<UserSupervisorExit> {
    info!(user_id, "per-user supervisor: starting");

    // Bring up the JMAP session once. Errors from this phase classify
    // into FATAL (user gone, token bad, auth revoked) vs TRANSIENT
    // (Stalwart unreachable, 5xx) below.
    let session = match bring_up_session(user_id, &state, decryptor.as_ref()).await {
        Ok(s) => s,
        Err(SupervisorBringupError::Fatal(e)) => {
            error!(user_id, error = %e, "per-user supervisor: FATAL during bring-up, exiting");
            return Ok(UserSupervisorExit::Terminal);
        }
        Err(SupervisorBringupError::Transient(e)) => {
            // We don't loop here — the top-level supervisor will
            // respawn us on its next tick if the user is still
            // active. That keeps backoff state machine in one place
            // (the event-source loop below).
            warn!(user_id, error = %e, "per-user supervisor: transient bring-up failure, exiting for top-level retry");
            return Ok(UserSupervisorExit::Retryable);
        }
    };

    let fetcher: Arc<dyn JmapChangeFetcher> = Arc::new(LiveJmapFetcher {
        session: session.session.clone(),
        account_id: session.account_id.clone(),
    });
    let jmap_ops: Arc<dyn JmapOps> = Arc::new(JmapOpsLive {
        session: session.session.clone(),
        account_id: session.account_id.clone(),
    });

    run_event_loop(user_id, state, session, fetcher, jmap_ops, cancel).await
}

/// EventSource consume-and-reconnect loop. Each iteration of the
/// outer loop opens a fresh stream; backoff is reset on a successful
/// open. The inner `select!` races stream messages against the
/// cancellation token so SIGINT preempts the wait.
async fn run_event_loop(
    user_id: i64,
    state: Arc<AppState>,
    session: JmapSession,
    fetcher: Arc<dyn JmapChangeFetcher>,
    jmap_ops: Arc<dyn JmapOps>,
    cancel: CancellationToken,
) -> Result<UserSupervisorExit> {
    let event_source = LiveEventSource {
        session: session.session.clone(),
    };
    let mut backoff = Backoff::new();
    let sleeper = TokioSleeper;
    run_event_loop_with(
        &state.db,
        user_id,
        &session.account_id,
        fetcher.as_ref(),
        jmap_ops.as_ref(),
        &event_source,
        &mut backoff,
        &sleeper,
        live_catchup_interval(),
        cancel,
    )
    .await
}

type EventStream =
    Pin<Box<dyn Stream<Item = std::result::Result<PushNotification, jmap_client::Error>> + Send>>;

#[async_trait]
trait EventSourceProvider: Send + Sync {
    async fn open(
        &self,
        types: Vec<DataType>,
    ) -> std::result::Result<EventStream, jmap_client::Error>;
}

struct LiveEventSource {
    session: Arc<hail_jmap::Session>,
}

#[async_trait]
impl EventSourceProvider for LiveEventSource {
    async fn open(
        &self,
        types: Vec<DataType>,
    ) -> std::result::Result<EventStream, jmap_client::Error> {
        let stream = self
            .session
            .client()
            .event_source(Some(types), false, Some(EVENT_SOURCE_PING_SECS), None)
            .await?;
        Ok(Box::pin(stream))
    }
}

trait ReconnectBackoff: Send {
    fn reset(&mut self);
    fn next_delay(&mut self) -> Duration;
}

impl ReconnectBackoff for Backoff {
    fn reset(&mut self) {
        Backoff::reset(self);
    }

    fn next_delay(&mut self) -> Duration {
        Backoff::next_delay(self)
    }
}

#[async_trait]
trait CancelSleeper: Send + Sync {
    async fn sleep(&self, delay: Duration, cancel: &CancellationToken) -> bool;
}

struct TokioSleeper;

#[async_trait]
impl CancelSleeper for TokioSleeper {
    async fn sleep(&self, delay: Duration, cancel: &CancellationToken) -> bool {
        cancel_aware_sleep(delay, cancel).await
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_event_loop_with(
    db: &SqlitePool,
    user_id: i64,
    account_id: &str,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &dyn JmapOps,
    event_source: &dyn EventSourceProvider,
    backoff: &mut dyn ReconnectBackoff,
    sleeper: &dyn CancelSleeper,
    live_catchup_interval: Duration,
    cancel: CancellationToken,
) -> Result<UserSupervisorExit> {
    let types = vec![
        DataType::Email,
        DataType::EmailDelivery,
        DataType::Mailbox,
        DataType::EmailSubmission,
    ];

    loop {
        if cancel.is_cancelled() {
            info!(user_id, "per-user supervisor: cancelled");
            return Ok(UserSupervisorExit::Retryable);
        }

        let catchup_res = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!(user_id, "per-user supervisor: cancel during catchup");
                return Ok(UserSupervisorExit::Retryable);
            }
            result = catchup_user(db, user_id, fetcher, jmap_ops, cancel.clone()) => result,
        };
        if let Err(e) = catchup_res {
            if is_auth_anyhow(&e) {
                error!(user_id, error = %e, "catchup: auth revoked (FATAL)");
                return Ok(UserSupervisorExit::Terminal);
            }
            let delay = backoff.next_delay();
            info!(
                user_id,
                delay_ms = delay.as_millis() as u64,
                error = %e,
                "catchup: transient failure; backing off"
            );
            if sleeper.sleep(delay, &cancel).await {
                return Ok(UserSupervisorExit::Retryable);
            }
            continue;
        }

        let stream_res = event_source.open(types.clone()).await;
        let mut stream = match stream_res {
            Ok(s) => {
                info!(user_id, "event_source: connected");
                backoff.reset();
                s
            }
            Err(e) => {
                if is_auth_error(&e) {
                    error!(user_id, error = %e, "event_source: auth revoked (FATAL)");
                    return Ok(UserSupervisorExit::Terminal);
                }
                let delay = backoff.next_delay();
                info!(
                    user_id,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "event_source: connect failed; backing off"
                );
                if sleeper.sleep(delay, &cancel).await {
                    return Ok(UserSupervisorExit::Retryable);
                }
                continue;
            }
        };

        // Inner loop: drain pushes until the stream ends or cancel. Also run a
        // periodic strict catch-up while the EventSource is connected: Stalwart
        // JMAP Email/import can advance Email/changes without a timely push, so
        // polling the persisted cursor keeps imported/inbound mail routed
        // without requiring a worker restart.
        let mut catchup_interval = tokio::time::interval(live_catchup_interval);
        catchup_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        catchup_interval.tick().await;
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    info!(user_id, "per-user supervisor: cancel during stream");
                    return Ok(UserSupervisorExit::Retryable);
                }
                _ = catchup_interval.tick() => {
                    let catchup_res = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            info!(user_id, "per-user supervisor: cancel during live catchup");
                            return Ok(UserSupervisorExit::Retryable);
                        }
                        result = catchup_user(db, user_id, fetcher, jmap_ops, cancel.clone()) => result,
                    };
                    if let Err(e) = catchup_res {
                        if is_auth_anyhow(&e) {
                            error!(user_id, error = %e, "live catchup: auth revoked (FATAL)");
                            return Ok(UserSupervisorExit::Terminal);
                        }
                        warn!(user_id, error = %e, "live catchup: failed; reconnecting");
                        break;
                    }
                }
                next = stream.next() => {
                    match next {
                        None => {
                            warn!(user_id, "event_source: stream ended; reconnecting");
                            break;
                        }
                        Some(Err(e)) => {
                            if is_auth_error(&e) {
                                error!(user_id, error = %e, "event_source: auth revoked mid-stream (FATAL)");
                                return Ok(UserSupervisorExit::Terminal);
                            }
                            warn!(user_id, error = %e, "event_source: stream error; reconnecting");
                            break;
                        }
                        Some(Ok(notification)) => {
                            handle_notification(
                                db,
                                user_id,
                                account_id,
                                fetcher,
                                jmap_ops,
                                notification,
                            )
                            .await;
                        }
                    }
                }
            }
        }

        // Reconnect path: bounded backoff with cancel sensitivity.
        let delay = backoff.next_delay();
        info!(
            user_id,
            delay_ms = delay.as_millis() as u64,
            "event_source: scheduling reconnect"
        );
        if sleeper.sleep(delay, &cancel).await {
            return Ok(UserSupervisorExit::Retryable);
        }
    }
}

/// Translate one PushNotification into a `handle_changes` round.
async fn handle_notification(
    db: &SqlitePool,
    user_id: i64,
    account_id: &str,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &dyn JmapOps,
    notification: PushNotification,
) {
    let mut changes = match notification {
        PushNotification::StateChange(c) => c,
        PushNotification::CalendarAlert(_) => return,
    };
    let account_changes = match changes.account_changes(account_id) {
        Some(c) => c,
        None => return,
    };
    let changed_types: BTreeSet<String> = account_changes.keys().map(|k| k.to_string()).collect();
    if let Err(e) = handle_changes(db, user_id, fetcher, jmap_ops, &changed_types).await {
        warn!(user_id, error = %e, "handle_changes failed for push event");
    }
}

fn live_catchup_interval() -> Duration {
    let secs = std::env::var(LIVE_CATCHUP_ENV)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_LIVE_CATCHUP_SECS);
    Duration::from_secs(secs)
}

/// Sleep for `delay`, returning early with `true` if cancellation
/// fires. The outer top-level `select!` already wraps every await
/// here, but having an explicit cancel branch lets the supervisor
/// reach the "cancelled" log line on the *normal* path (rather than
/// via future-drop) which makes the shutdown log readable.
async fn cancel_aware_sleep(delay: Duration, cancel: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => true,
        _ = sleep(delay) => false,
    }
}

/// Internal handle on a live session — the client + the primary
/// mail account id we resolved at login. We keep the whole
/// `hail_jmap::Session` behind an `Arc` so the fetcher can hold a
/// shared reference for `email_changes` / `email_get`.
struct JmapSession {
    session: Arc<hail_jmap::Session>,
    account_id: String,
}

enum SupervisorBringupError {
    Fatal(FatalError),
    Transient(anyhow::Error),
}

/// Load the user row + latest active session, decrypt the bearer
/// token, and connect to Stalwart. Classifies failures into FATAL
/// vs TRANSIENT for the caller.
async fn bring_up_session(
    user_id: i64,
    state: &AppState,
    decryptor: &dyn TokenDecryptor,
) -> Result<JmapSession, SupervisorBringupError> {
    let user_exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| SupervisorBringupError::Transient(anyhow!(e)))?;
    if user_exists.is_none() {
        return Err(SupervisorBringupError::Fatal(FatalError::UserMissing(
            user_id,
        )));
    }

    let now = Utc::now().to_rfc3339();
    let token_row: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT jmap_token_enc FROM sessions \
         WHERE user_id = ? AND expires_at > ? \
         ORDER BY last_used_at DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(&now)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| SupervisorBringupError::Transient(anyhow!(e)))?;

    let enc = match token_row {
        Some((blob,)) => blob,
        None => {
            return Err(SupervisorBringupError::Fatal(FatalError::NoActiveSession(
                user_id,
            )));
        }
    };

    let token: SecretString = decryptor
        .decrypt(&enc)
        .map_err(|e| SupervisorBringupError::Fatal(FatalError::TokenDecrypt(e)))?;

    let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
        .await
        .map_err(classify_login_error)?;
    let account_id = session.account_id().to_string();

    Ok(JmapSession {
        session: Arc::new(session),
        account_id,
    })
}

/// Classify a `hail_jmap::Error` from login into FATAL (auth) or
/// TRANSIENT (network / 5xx).
fn classify_login_error(e: hail_jmap::Error) -> SupervisorBringupError {
    match e {
        hail_jmap::Error::Auth(_) => SupervisorBringupError::Fatal(FatalError::AuthRevoked),
        other => SupervisorBringupError::Transient(anyhow!(other)),
    }
}

/// Heuristic: does this client error look like an auth-revoked
/// problem (401/403) we should treat as FATAL?
fn is_auth_error(e: &jmap_client::Error) -> bool {
    match e {
        jmap_client::Error::Problem(p) => matches!(p.status(), Some(401 | 403)),
        jmap_client::Error::Server(msg) => msg.starts_with("401") || msg.starts_with("403"),
        _ => false,
    }
}

fn is_auth_anyhow(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|cause| cause.downcast_ref::<jmap_client::Error>())
        .any(is_auth_error)
}

/// Production `JmapChangeFetcher`: drives `Email/changes` then
/// resolves created+updated ids via `Email/get` with the property
/// list named in the task contract.
struct LiveJmapFetcher {
    session: Arc<hail_jmap::Session>,
    account_id: String,
}

#[async_trait]
impl JmapChangeFetcher for LiveJmapFetcher {
    async fn current_state(&self, type_state: &str) -> Result<String> {
        let client = self.session.client();
        let state = match type_state {
            "Email" => {
                let mut request = client.build();
                request.get_email().ids(std::iter::empty::<String>());
                request
                    .send_get_email()
                    .await
                    .context("initial Email/get failed")?
                    .take_state()
            }
            "Mailbox" => {
                let mut request = client.build();
                request.get_mailbox().ids(std::iter::empty::<String>());
                request
                    .send_get_mailbox()
                    .await
                    .context("initial Mailbox/get failed")?
                    .take_state()
            }
            "EmailSubmission" => {
                let mut request = client.build();
                request
                    .get_email_submission()
                    .ids(std::iter::empty::<String>());
                request
                    .send_get_email_submission()
                    .await
                    .context("initial EmailSubmission/get failed")?
                    .take_state()
            }
            "EmailDelivery" => {
                // jmap-client 0.3 exposes EmailDelivery as an EventSource
                // TypeState but not as a first-class */get or */changes
                // helper. Use a cheap Email/get state token to seed the row
                // so first-run users do not replay history; future crate
                // support can replace this with EmailDelivery/get.
                let mut request = client.build();
                request.get_email().ids(std::iter::empty::<String>());
                request
                    .send_get_email()
                    .await
                    .context("initial EmailDelivery state fallback failed")?
                    .take_state()
            }
            other => anyhow::bail!("unsupported TypeState {other}"),
        };
        Ok(state)
    }

    async fn fetch(&self, type_state: &str, since_cursor: &str) -> Result<EmailChanges> {
        if type_state == "Mailbox" {
            let mut resp = self
                .session
                .client()
                .mailbox_changes(since_cursor.to_string(), 512)
                .await
                .context("Mailbox/changes failed")?;
            return Ok(EmailChanges {
                new_state: resp.take_new_state(),
                destroyed: resp.take_destroyed(),
                ..Default::default()
            });
        }
        if type_state == "EmailSubmission" {
            let mut resp = self
                .session
                .client()
                .email_submission_changes(since_cursor.to_string(), 512)
                .await
                .context("EmailSubmission/changes failed")?;
            return Ok(EmailChanges {
                new_state: resp.take_new_state(),
                destroyed: resp.take_destroyed(),
                ..Default::default()
            });
        }
        if type_state == "EmailDelivery" {
            // See `current_state`: jmap-client currently has no
            // EmailDelivery helpers, so there is no object diff to fetch.
            return Ok(EmailChanges {
                new_state: since_cursor.to_string(),
                ..Default::default()
            });
        }

        // First run: catchup_user seeds the cursor with */get state.
        // If a legacy empty cursor is encountered, seed it the same way.
        if since_cursor.is_empty() {
            return Ok(EmailChanges {
                new_state: self.current_state(type_state).await?,
                ..Default::default()
            });
        }

        let mut resp = self
            .session
            .client()
            .email_changes(since_cursor.to_string(), None)
            .await
            .context("Email/changes failed")?;
        let new_state = resp.take_new_state();
        let created_ids = resp.take_created();
        let updated_ids = resp.take_updated();
        let destroyed = resp.take_destroyed();

        let props = [
            Property::Id,
            Property::ThreadId,
            Property::ReceivedAt,
            Property::From,
            Property::Subject,
            Property::Preview,
            Property::Keywords,
            Property::MailboxIds,
            Property::Size,
        ];

        let mut created = Vec::with_capacity(created_ids.len());
        for id in created_ids {
            if let Some(em) = self
                .session
                .client()
                .email_get(&id, Some(props.iter().cloned()))
                .await
                .with_context(|| format!("Email/get failed for {id}"))?
            {
                created.push(envelope_from(em));
            }
        }
        let mut updated = Vec::with_capacity(updated_ids.len());
        for id in updated_ids {
            if let Some(em) = self
                .session
                .client()
                .email_get(&id, Some(props.iter().cloned()))
                .await
                .with_context(|| format!("Email/get failed for {id}"))?
            {
                updated.push(envelope_from(em));
            }
        }

        // account_id is captured for symmetry with the WS fanout in
        // screener-routing; suppress dead-code for now.
        let _ = &self.account_id;

        Ok(EmailChanges {
            new_state,
            created,
            updated,
            destroyed,
        })
    }
}

fn envelope_from(em: jmap_client::email::Email) -> EmailEnvelope {
    let from = em
        .from()
        .map(|addrs| {
            addrs
                .iter()
                .map(|a| (a.name().map(str::to_string), a.email().to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    EmailEnvelope {
        id: em.id().unwrap_or_default().to_string(),
        thread_id: em.thread_id().map(str::to_string),
        received_at: em.received_at(),
        from,
        subject: em.subject().map(str::to_string),
        preview: em.preview().map(str::to_string),
        keywords: em.keywords().into_iter().map(str::to_string).collect(),
        mailbox_ids: em.mailbox_ids().into_iter().map(str::to_string).collect(),
        size: em.size(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    use ahash::AHashMap;
    use futures_util::stream;
    use hail_jmap::jmap_client::CalendarAlert;
    use hail_test::{TempDb, fresh_db_url};
    use tokio::sync::oneshot;

    use crate::screener::RouteError;

    async fn setup_db() -> (SqlitePool, TempDb, i64) {
        let (url, guard) = fresh_db_url("hail-worker-user-test");
        let pool = hail_db::connect(&url).await.expect("connect");
        hail_db::migrate(&pool).await.expect("migrate");

        sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
            .bind("alice@example.com")
            .bind("acct-alice")
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("insert user");
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
            .bind("alice@example.com")
            .fetch_one(&pool)
            .await
            .expect("fetch user id");
        (pool, guard, user_id)
    }

    struct RecordingFetcher {
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingFetcher {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    #[async_trait]
    impl JmapChangeFetcher for RecordingFetcher {
        async fn current_state(&self, type_state: &str) -> Result<String> {
            Ok(format!("{type_state}-initial"))
        }

        async fn fetch(&self, type_state: &str, since_cursor: &str) -> Result<EmailChanges> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(type_state.to_string());
            Ok(EmailChanges {
                new_state: format!("{type_state}-after-{since_cursor}"),
                ..Default::default()
            })
        }
    }

    struct NoopJmapOps;

    #[async_trait]
    impl JmapOps for NoopJmapOps {
        async fn get_or_create_mailbox(
            &self,
            _name: &str,
        ) -> std::result::Result<String, RouteError> {
            Ok("screener-id".to_string())
        }

        async fn get_mailbox_by_role(
            &self,
            _role: &str,
        ) -> std::result::Result<Option<String>, RouteError> {
            Ok(Some("trash-id".to_string()))
        }

        async fn apply_keyword(
            &self,
            _email_id: &str,
            _keyword: &str,
        ) -> std::result::Result<(), RouteError> {
            Ok(())
        }

        async fn remove_keyword(
            &self,
            _email_id: &str,
            _keyword: &str,
        ) -> std::result::Result<(), RouteError> {
            Ok(())
        }

        async fn move_to_mailbox(
            &self,
            _email_id: &str,
            _mailbox_id: &str,
        ) -> std::result::Result<(), RouteError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingBackoff {
        next_calls: usize,
        reset_calls: usize,
    }

    impl ReconnectBackoff for RecordingBackoff {
        fn reset(&mut self) {
            self.reset_calls += 1;
        }

        fn next_delay(&mut self) -> Duration {
            self.next_calls += 1;
            Duration::from_millis(self.next_calls as u64)
        }
    }

    #[derive(Default)]
    struct RecordingSleeper {
        delays: std::sync::Mutex<Vec<Duration>>,
    }

    #[async_trait]
    impl CancelSleeper for RecordingSleeper {
        async fn sleep(&self, delay: Duration, cancel: &CancellationToken) -> bool {
            self.delays.lock().expect("delays lock").push(delay);
            cancel.is_cancelled()
        }
    }

    impl RecordingSleeper {
        fn delays(&self) -> Vec<Duration> {
            self.delays.lock().expect("delays lock").clone()
        }
    }

    struct ScriptedEventSource {
        opens: std::sync::Mutex<Vec<OpenResult>>,
        open_calls: AtomicUsize,
    }

    enum OpenResult {
        Stream(Vec<std::result::Result<PushNotification, jmap_client::Error>>),
        Error(jmap_client::Error),
    }

    #[async_trait]
    impl EventSourceProvider for ScriptedEventSource {
        async fn open(
            &self,
            _types: Vec<DataType>,
        ) -> std::result::Result<EventStream, jmap_client::Error> {
            self.open_calls.fetch_add(1, Ordering::SeqCst);
            let next = self.opens.lock().expect("opens lock").remove(0);
            match next {
                OpenResult::Stream(items) => Ok(Box::pin(stream::iter(items))),
                OpenResult::Error(error) => Err(error),
            }
        }
    }

    impl ScriptedEventSource {
        fn new(opens: Vec<OpenResult>) -> Self {
            Self {
                opens: std::sync::Mutex::new(opens),
                open_calls: AtomicUsize::new(0),
            }
        }

        fn open_calls(&self) -> usize {
            self.open_calls.load(Ordering::SeqCst)
        }
    }

    struct PendingStream {
        entered: Option<oneshot::Sender<()>>,
    }

    impl Stream for PendingStream {
        type Item = std::result::Result<PushNotification, jmap_client::Error>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
            }
            Poll::Pending
        }
    }

    struct PendingEventSource {
        entered: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl EventSourceProvider for PendingEventSource {
        async fn open(
            &self,
            _types: Vec<DataType>,
        ) -> std::result::Result<EventStream, jmap_client::Error> {
            Ok(Box::pin(PendingStream {
                entered: self.entered.lock().expect("entered lock").take(),
            }))
        }
    }

    struct BlockingSleeper {
        entered: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl CancelSleeper for BlockingSleeper {
        async fn sleep(&self, _delay: Duration, cancel: &CancellationToken) -> bool {
            if let Some(entered) = self.entered.lock().expect("entered lock").take() {
                let _ = entered.send(());
            }
            cancel.cancelled().await;
            true
        }
    }

    fn state_change(account_id: &str, types: &[DataType]) -> PushNotification {
        let mut account_changes = AHashMap::new();
        for data_type in types {
            account_changes.insert(data_type.clone(), format!("{data_type}-state"));
        }
        let mut changes = AHashMap::new();
        changes.insert(account_id.to_string(), account_changes);
        PushNotification::StateChange(jmap_client::event_source::Changes::new(None, changes))
    }

    fn calendar_alert(account_id: &str) -> PushNotification {
        PushNotification::CalendarAlert(CalendarAlert {
            account_id: account_id.to_string(),
            calendar_event_id: "event-1".to_string(),
            uid: "uid-1".to_string(),
            recurrence_id: None,
            alert_id: "alert-1".to_string(),
        })
    }

    async fn seed_cursors(db: &SqlitePool, user_id: i64) {
        for type_state in crate::changes::TRACKED_TYPE_STATES {
            crate::changes::upsert_cursor(db, user_id, type_state, &format!("{type_state}-before"))
                .await
                .expect("seed cursor");
        }
    }

    #[tokio::test]
    async fn event_loop_filters_state_changes_and_ignores_calendar_alerts() {
        let (db, _guard, user_id) = setup_db().await;
        seed_cursors(&db, user_id).await;
        let fetcher = RecordingFetcher::new();
        let jmap_ops = NoopJmapOps;
        let events = ScriptedEventSource::new(vec![
            OpenResult::Stream(vec![
                Ok(calendar_alert("acct-alice")),
                Ok(state_change("other-account", &[DataType::Email])),
                Ok(state_change(
                    "acct-alice",
                    &[DataType::Identity, DataType::Email, DataType::Mailbox],
                )),
            ]),
            OpenResult::Error(jmap_client::Error::Server("401 unauthorized".to_string())),
        ]);
        let mut backoff = RecordingBackoff::default();
        let sleeper = RecordingSleeper::default();

        run_event_loop_with(
            &db,
            user_id,
            "acct-alice",
            &fetcher,
            &jmap_ops,
            &events,
            &mut backoff,
            &sleeper,
            Duration::from_secs(60),
            CancellationToken::new(),
        )
        .await
        .expect("event loop exits cleanly on terminal auth");

        assert_eq!(
            fetcher.calls(),
            vec![
                "Email".to_string(),
                "EmailDelivery".to_string(),
                "Mailbox".to_string(),
                "EmailSubmission".to_string(),
                "Email".to_string(),
                "Mailbox".to_string(),
                "Email".to_string(),
                "EmailDelivery".to_string(),
                "Mailbox".to_string(),
                "EmailSubmission".to_string(),
            ]
        );
        assert_eq!(events.open_calls(), 2);
        assert_eq!(backoff.reset_calls, 1);
        assert_eq!(backoff.next_calls, 1);
        assert_eq!(sleeper.delays(), vec![Duration::from_millis(1)]);
    }

    #[tokio::test]
    async fn event_loop_resets_backoff_after_successful_reconnect() {
        let (db, _guard, user_id) = setup_db().await;
        seed_cursors(&db, user_id).await;
        let fetcher = RecordingFetcher::new();
        let jmap_ops = NoopJmapOps;
        let events = ScriptedEventSource::new(vec![
            OpenResult::Error(jmap_client::Error::Internal("connect failed".to_string())),
            OpenResult::Stream(vec![]),
            OpenResult::Error(jmap_client::Error::Server("401 unauthorized".to_string())),
        ]);
        let mut backoff = RecordingBackoff::default();
        let sleeper = RecordingSleeper::default();

        run_event_loop_with(
            &db,
            user_id,
            "acct-alice",
            &fetcher,
            &jmap_ops,
            &events,
            &mut backoff,
            &sleeper,
            Duration::from_secs(60),
            CancellationToken::new(),
        )
        .await
        .expect("event loop exits cleanly on terminal auth");

        assert_eq!(events.open_calls(), 3);
        assert_eq!(backoff.next_calls, 2);
        assert_eq!(backoff.reset_calls, 1);
        assert_eq!(
            sleeper.delays(),
            vec![Duration::from_millis(1), Duration::from_millis(2)]
        );
    }

    #[tokio::test]
    async fn auth_errors_mid_stream_are_terminal_without_reconnect_backoff() {
        let (db, _guard, user_id) = setup_db().await;
        seed_cursors(&db, user_id).await;
        let fetcher = RecordingFetcher::new();
        let jmap_ops = NoopJmapOps;
        let events = ScriptedEventSource::new(vec![OpenResult::Stream(vec![Err(
            jmap_client::Error::Server("403 forbidden".to_string()),
        )])]);
        let mut backoff = RecordingBackoff::default();
        let sleeper = RecordingSleeper::default();

        run_event_loop_with(
            &db,
            user_id,
            "acct-alice",
            &fetcher,
            &jmap_ops,
            &events,
            &mut backoff,
            &sleeper,
            Duration::from_secs(60),
            CancellationToken::new(),
        )
        .await
        .expect("event loop exits cleanly on terminal auth");

        assert_eq!(events.open_calls(), 1);
        assert_eq!(backoff.next_calls, 0);
        assert_eq!(sleeper.delays(), Vec::<Duration>::new());
    }

    #[tokio::test]
    async fn live_catchup_replays_while_stream_is_open() {
        let (db, _guard, user_id) = setup_db().await;
        seed_cursors(&db, user_id).await;
        let fetcher = RecordingFetcher::new();
        let jmap_ops = NoopJmapOps;
        let (entered_tx, entered_rx) = oneshot::channel();
        let events = PendingEventSource {
            entered: std::sync::Mutex::new(Some(entered_tx)),
        };
        let mut backoff = RecordingBackoff::default();
        let sleeper = RecordingSleeper::default();
        let cancel = CancellationToken::new();

        let run = run_event_loop_with(
            &db,
            user_id,
            "acct-alice",
            &fetcher,
            &jmap_ops,
            &events,
            &mut backoff,
            &sleeper,
            Duration::from_millis(10),
            cancel.clone(),
        );
        tokio::pin!(run);

        tokio::select! {
            res = &mut run => panic!("event loop completed before stream wait: {res:?}"),
            received = entered_rx => received.expect("stream.next polled"),
        }
        assert_eq!(
            fetcher.calls(),
            vec![
                "Email".to_string(),
                "EmailDelivery".to_string(),
                "Mailbox".to_string(),
                "EmailSubmission".to_string(),
            ],
            "startup catch-up should run before opening EventSource"
        );

        tokio::select! {
            res = &mut run => panic!("event loop completed before live catchup: {res:?}"),
            result = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if fetcher.calls().len() >= 8 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }) => result.expect("periodic live catch-up should run promptly"),
        }

        assert_eq!(
            fetcher.calls(),
            vec![
                "Email".to_string(),
                "EmailDelivery".to_string(),
                "Mailbox".to_string(),
                "EmailSubmission".to_string(),
                "Email".to_string(),
                "EmailDelivery".to_string(),
                "Mailbox".to_string(),
                "EmailSubmission".to_string(),
            ],
            "periodic catch-up should replay persisted cursors while EventSource stays open"
        );
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), &mut run)
            .await
            .expect("cancel should preempt stream.next")
            .expect("event loop result");
    }

    #[tokio::test]
    async fn cancellation_preempts_waiting_on_stream_next() {
        let (db, _guard, user_id) = setup_db().await;
        seed_cursors(&db, user_id).await;
        let fetcher = RecordingFetcher::new();
        let jmap_ops = NoopJmapOps;
        let (entered_tx, entered_rx) = oneshot::channel();
        let events = PendingEventSource {
            entered: std::sync::Mutex::new(Some(entered_tx)),
        };
        let mut backoff = RecordingBackoff::default();
        let sleeper = RecordingSleeper::default();
        let cancel = CancellationToken::new();

        let run = run_event_loop_with(
            &db,
            user_id,
            "acct-alice",
            &fetcher,
            &jmap_ops,
            &events,
            &mut backoff,
            &sleeper,
            Duration::from_secs(60),
            cancel.clone(),
        );
        tokio::pin!(run);

        tokio::select! {
            res = &mut run => panic!("event loop completed before stream wait: {res:?}"),
            received = entered_rx => received.expect("stream.next polled"),
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), &mut run)
            .await
            .expect("cancel should preempt stream.next")
            .expect("event loop result");
    }

    #[tokio::test]
    async fn cancellation_preempts_reconnect_backoff() {
        let (db, _guard, user_id) = setup_db().await;
        seed_cursors(&db, user_id).await;
        let fetcher = RecordingFetcher::new();
        let jmap_ops = NoopJmapOps;
        let events = ScriptedEventSource::new(vec![OpenResult::Stream(vec![])]);
        let mut backoff = RecordingBackoff::default();
        let (entered_tx, entered_rx) = oneshot::channel();
        let sleeper = BlockingSleeper {
            entered: std::sync::Mutex::new(Some(entered_tx)),
        };
        let cancel = CancellationToken::new();

        let run = run_event_loop_with(
            &db,
            user_id,
            "acct-alice",
            &fetcher,
            &jmap_ops,
            &events,
            &mut backoff,
            &sleeper,
            Duration::from_secs(60),
            cancel.clone(),
        );
        tokio::pin!(run);

        tokio::select! {
            res = &mut run => panic!("event loop completed before backoff wait: {res:?}"),
            received = entered_rx => received.expect("backoff sleep entered"),
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), &mut run)
            .await
            .expect("cancel should preempt backoff")
            .expect("event loop result");
    }
}
