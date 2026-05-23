//! Per-user JMAP push supervisor.
//!
//! See design.md §8.1 / §8.2. One task per active user owns the
//! EventSource stream and drives `handle_changes` on each push event
//! plus once at startup (the catch-up shape requested by the task
//! contract).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use hail_jmap::jmap_client;
use hail_jmap::jmap_client::DataType;
use hail_jmap::jmap_client::email::Property;
use hail_jmap::jmap_client::event_source::PushNotification;
use secrecy::SecretString;
use sqlx::SqlitePool;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

use crate::backoff::Backoff;
use crate::catchup::catchup_user;
use crate::changes::{EmailChanges, EmailEnvelope, JmapChangeFetcher, handle_changes};
use crate::crypto::TokenDecryptor;
use crate::screener::JmapOpsLive;
use crate::state::AppState;

/// EventSource keep-alive ping interval (seconds) — the task contract
/// fixes this at 60s.
const EVENT_SOURCE_PING_SECS: u32 = 60;

/// Reasons a per-user supervisor stops itself for good rather than
/// retrying. FATAL outcomes are logged at ERROR; the supervisor exits
/// `Ok(())` so the top-level JoinSet doesn't treat it as a crash.
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

/// Per-user supervisor entry point. Owns the JMAP EventSource stream
/// for `user_id`, calls `handle_changes` on startup (catch-up shape)
/// and on every push event, and reconnects with exponential backoff
/// + full jitter on transient errors.
#[instrument(skip(state, cancel), fields(user_id = user_id))]
pub async fn run_user_supervisor(
    user_id: i64,
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> Result<()> {
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
            Ok(())
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
) -> Result<()> {
    info!(user_id, "per-user supervisor: starting");

    // Bring up the JMAP session once. Errors from this phase classify
    // into FATAL (user gone, token bad, auth revoked) vs TRANSIENT
    // (Stalwart unreachable, 5xx) below.
    let session = match bring_up_session(user_id, &state, decryptor.as_ref()).await {
        Ok(s) => s,
        Err(SupervisorBringupError::Fatal(e)) => {
            error!(user_id, error = %e, "per-user supervisor: FATAL during bring-up, exiting");
            return Ok(());
        }
        Err(SupervisorBringupError::Transient(e)) => {
            // We don't loop here — the top-level supervisor will
            // respawn us on its next tick if the user is still
            // active. That keeps backoff state machine in one place
            // (the event-source loop below).
            warn!(user_id, error = %e, "per-user supervisor: transient bring-up failure, exiting for top-level retry");
            return Ok(());
        }
    };

    let fetcher: Arc<dyn JmapChangeFetcher> = Arc::new(LiveJmapFetcher {
        session: session.session.clone(),
        account_id: session.account_id.clone(),
    });
    let jmap_ops = Arc::new(JmapOpsLive {
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
    jmap_ops: Arc<JmapOpsLive>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut backoff = Backoff::new();
    let types = vec![
        DataType::Email,
        DataType::EmailDelivery,
        DataType::Mailbox,
        DataType::EmailSubmission,
    ];

    loop {
        if cancel.is_cancelled() {
            info!(user_id, "per-user supervisor: cancelled");
            return Ok(());
        }

        let catchup_res = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!(user_id, "per-user supervisor: cancel during catchup");
                return Ok(());
            }
            result = catchup_user(&state.db, user_id, fetcher.as_ref(), jmap_ops.as_ref(), cancel.clone()) => result,
        };
        if let Err(e) = catchup_res {
            if is_auth_anyhow(&e) {
                error!(user_id, error = %e, "catchup: auth revoked (FATAL)");
                return Ok(());
            }
            let delay = backoff.next_delay();
            info!(
                user_id,
                delay_ms = delay.as_millis() as u64,
                error = %e,
                "catchup: transient failure; backing off"
            );
            if cancel_aware_sleep(delay, &cancel).await {
                return Ok(());
            }
            continue;
        }

        let stream_res = session
            .client()
            .event_source(Some(types.clone()), false, Some(EVENT_SOURCE_PING_SECS), None)
            .await;
        let mut stream = match stream_res {
            Ok(s) => {
                info!(user_id, "event_source: connected");
                backoff.reset();
                s
            }
            Err(e) => {
                if is_auth_error(&e) {
                    error!(user_id, error = %e, "event_source: auth revoked (FATAL)");
                    return Ok(());
                }
                let delay = backoff.next_delay();
                info!(
                    user_id,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "event_source: connect failed; backing off"
                );
                if cancel_aware_sleep(delay, &cancel).await {
                    return Ok(());
                }
                continue;
            }
        };

        // Inner loop: drain pushes until the stream ends or cancel.
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    info!(user_id, "per-user supervisor: cancel during stream");
                    return Ok(());
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
                                return Ok(());
                            }
                            warn!(user_id, error = %e, "event_source: stream error; reconnecting");
                            break;
                        }
                        Some(Ok(notification)) => {
                            handle_notification(
                                &state.db,
                                user_id,
                                &session.account_id,
                                fetcher.as_ref(),
                                jmap_ops.as_ref(),
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
        if cancel_aware_sleep(delay, &cancel).await {
            return Ok(());
        }
    }
}

/// Translate one PushNotification into a `handle_changes` round.
async fn handle_notification(
    db: &SqlitePool,
    user_id: i64,
    account_id: &str,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &JmapOpsLive,
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
    let changed_types: BTreeSet<String> = account_changes
        .keys()
        .map(|k| k.to_string())
        .collect();
    if let Err(e) = handle_changes(db, user_id, fetcher, jmap_ops, &changed_types).await {
        warn!(user_id, error = %e, "handle_changes failed for push event");
    }
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

impl JmapSession {
    fn client(&self) -> &jmap_client::client::Client {
        self.session.client()
    }
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
        return Err(SupervisorBringupError::Fatal(FatalError::UserMissing(user_id)));
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
            return Err(SupervisorBringupError::Fatal(
                FatalError::NoActiveSession(user_id),
            ));
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

