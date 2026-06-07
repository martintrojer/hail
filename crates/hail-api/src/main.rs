//! `hail-api` — Axum HTTP server.
//!
//! The router is built in `hail_api::build_router` so integration tests
//! can exercise the exact same stack without binding a TCP port; this
//! shell takes care of:
//!   1. tracing setup,
//!   2. loading the unified `hail-core` config + parsing the server key,
//!   3. opening the SQLite pool and running migrations,
//!   4. wiring `AppState` + the in-memory login rate limiter,
//!   5. serving with eager-installed SIGINT/SIGTERM shutdown handlers.
//!
//! Mirrors `hail-worker` deliberately so both binaries behave the same
//! under systemd / Docker.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_blob_store::FilesystemBlobStore;
use hail_cache::{CachePolicy, CachedMail};
use hail_core::{Config, MailBackend};
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
use tokio::signal::unix::{Signal, SignalKind, signal as unix_signal};
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::load().context("loading hail config (TOML + env)")?;
    // NB: we deliberately do NOT log `config.secrets` or the parsed key.
    info!(
        database_url = %config.database_url,
        stalwart_url = %config.stalwart.jmap_url,
        bind = %config.server.bind,
        "hail-api starting"
    );

    // Parse the server key once at startup; failures here are fatal —
    // the API cannot encrypt/decrypt session tokens without it.
    let server_key = hail_core::parse_server_key(&config.secrets.server_key)
        .context("parsing secrets.server_key")?;

    let db = hail_db::connect(&config.database_url)
        .await
        .with_context(|| format!("opening sqlite pool at {}", config.database_url))?;
    hail_db::migrate(&db)
        .await
        .context("running hail-db migrations")?;
    info!("db ready");

    let mail = build_cached_mail(&config, &db, server_key).await?;

    let bind = config.server.bind.clone();
    let state = AppState {
        db,
        config,
        server_key: Arc::new(server_key),
        auth_rate_limiter: Arc::new(IpRateLimiter::default()),
        mail,
        events: hail_api::events::AppEventBus::default(),
    };

    let bridge_cancel = CancellationToken::new();
    let bridge_handle = hail_api::events::spawn_db_event_bridge(
        state.db.clone(),
        state.events.clone(),
        bridge_cancel.clone(),
    );

    let router = hail_api::build_router(state.clone(), false);

    // Install signal handlers EAGERLY — before we bind the listener or
    // enter the serve loop. `tokio::signal::unix::signal` registers a
    // per-SignalKind dispatcher; if we leave the install lazy (inside
    // the `select!` future) and a signal arrives before that future is
    // first polled, the default disposition runs and the process dies
    // hard (exit 143 for SIGTERM). Hoisting the install closes that
    // window.
    let mut sigint = unix_signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    let mut sigterm = unix_signal(SignalKind::terminate()).context("install SIGTERM handler")?;

    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding HTTP listener at {bind}"))?;
    let local = listener.local_addr().ok();
    info!(?local, "hail-api listening");

    // We use `into_make_service_with_connect_info` so the login handler
    // can read the peer's `SocketAddr` for the per-IP rate limiter.
    let make_svc = router.into_make_service_with_connect_info::<std::net::SocketAddr>();

    // Shutdown shape: race `axum::serve` against the signal streams in a
    // `tokio::select!`. We deliberately do NOT use
    // `with_graceful_shutdown` here — it waits for every open connection
    // (including idle HTTP/1.1 keep-alives) to close before resolving,
    // which wedges shutdown indefinitely under realistic clients.
    tokio::select! {
        res = axum::serve(listener, make_svc) => {
            res.context("axum serve loop")?;
        }
        _ = wait_for_shutdown(&mut sigint, &mut sigterm) => {
            info!("shutting down");
        }
    }

    bridge_cancel.cancel();
    let _ = bridge_handle.await;
    state.db.close().await;
    info!("hail-api stopped");
    Ok(())
}

async fn build_cached_mail(
    config: &Config,
    db: &sqlx::SqlitePool,
    server_key: [u8; hail_core::KEY_LEN],
) -> Result<Arc<CachedMail>> {
    let backend: Box<dyn hail_backend::MailBackend> = match config.mail.backend {
        MailBackend::Jmap => {
            let session = hail_jmap::login_bearer(
                &config.mail.jmap.jmap_url,
                config.secrets.server_key.clone(),
            )
            .await
            .context("connecting JMAP backend for cache")?;
            Box::new(hail_jmap::JmapBackend::new(session))
        }
        MailBackend::Gmail => {
            let account = sqlx::query_as::<_, (i64, i64, String)>(
                "SELECT id, user_id, provider_account_id FROM mail_accounts \
                 WHERE backend_kind = 'gmail' AND sync_status != 'disconnected' AND refresh_token_enc IS NOT NULL \
                 ORDER BY id LIMIT 1",
            )
            .fetch_optional(db)
            .await
            .context("loading gmail account for cache backend")?
            .ok_or_else(|| anyhow!("mail.backend=gmail requires a connected Gmail account"))?;
            let token_source = DbGmailTokenSource::load(config, db, server_key, account).await?;
            let token_source = hail_gmail::gmail_client::CachedGmailTokenSource::new(token_source);
            Box::new(
                hail_gmail::GmailBackend::new(reqwest::Client::new(), token_source)
                    .map_err(|err| anyhow!(err))?,
            )
        }
    };

    Ok(Arc::new(CachedMail::new(
        db.clone(),
        Arc::new(FilesystemBlobStore::new(
            config.mail.cache.blob_root.clone(),
        )),
        backend,
        CachePolicy::from(&config.mail.cache),
    )))
}

#[derive(Clone, Debug)]
struct DbGmailTokenSource {
    http: reqwest::Client,
    client_id: Option<String>,
    client_secret: Option<secrecy::SecretString>,
    token_url: String,
    refresh_token: secrecy::SecretString,
}

impl DbGmailTokenSource {
    async fn load(
        config: &Config,
        db: &sqlx::SqlitePool,
        server_key: [u8; hail_core::KEY_LEN],
        (account_id, user_id, provider_account_id): (i64, i64, String),
    ) -> Result<Self> {
        let ciphertext: Vec<u8> = sqlx::query_scalar(
            "SELECT refresh_token_enc FROM mail_accounts WHERE id = ?1 AND user_id = ?2",
        )
        .bind(account_id)
        .bind(user_id)
        .fetch_one(db)
        .await
        .context("loading encrypted gmail refresh token")?;
        let context = hail_core::ProviderTokenContext::new(
            user_id,
            account_id,
            "gmail",
            provider_account_id,
            hail_core::ProviderOAuthTokenKind::Refresh,
        );
        let token = hail_core::open_provider_oauth_token(&ciphertext, &server_key, &context)
            .map_err(|err| anyhow!(err))?;
        Ok(Self {
            http: reqwest::Client::new(),
            client_id: config
                .mail
                .gmail
                .oauth_client_id
                .clone()
                .or_else(|| config.provider_import.gmail.oauth_client_id.clone()),
            client_secret: config
                .mail
                .gmail
                .oauth_client_secret
                .clone()
                .or_else(|| config.provider_import.gmail.oauth_client_secret.clone()),
            token_url: config
                .mail
                .gmail
                .oauth_token_url
                .clone()
                .or_else(|| config.provider_import.gmail.oauth_token_url.clone())
                .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()),
            refresh_token: secrecy::SecretString::from(token.expose_secret().to_string()),
        })
    }
}

#[async_trait::async_trait]
impl hail_gmail::gmail_client::GmailAccessTokenProvider for DbGmailTokenSource {
    async fn refresh_access_token(
        &self,
    ) -> std::result::Result<
        hail_gmail::gmail_client::GmailAccessToken,
        hail_gmail::gmail_client::GmailClientError,
    > {
        let client_id = self.client_id.as_deref().ok_or_else(|| {
            hail_gmail::gmail_client::GmailClientError::token_error(std::io::Error::other(
                "gmail oauth client id is not configured",
            ))
        })?;
        let client_secret = self.client_secret.as_ref().ok_or_else(|| {
            hail_gmail::gmail_client::GmailClientError::token_error(std::io::Error::other(
                "gmail oauth client secret is not configured",
            ))
        })?;
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("client_id", client_id);
            form.append_pair("client_secret", client_secret.expose_secret());
            form.append_pair("refresh_token", self.refresh_token.expose_secret());
            form.append_pair("grant_type", "refresh_token");
            form.finish()
        };
        let token: GoogleRefreshTokenResponse = self
            .http
            .post(&self.token_url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(hail_gmail::gmail_client::GmailClientError::Request)?
            .error_for_status()
            .map_err(hail_gmail::gmail_client::GmailClientError::Request)?
            .json()
            .await
            .map_err(hail_gmail::gmail_client::GmailClientError::Request)?;
        Ok(hail_gmail::gmail_client::GmailAccessToken {
            token: token.access_token,
            expires_in: std::time::Duration::from_secs(token.expires_in.unwrap_or(3600)),
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct GoogleRefreshTokenResponse {
    #[serde(deserialize_with = "deserialize_secret")]
    access_token: secrecy::SecretString,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn deserialize_secret<'de, D>(
    deserializer: D,
) -> std::result::Result<secrecy::SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <String as serde::Deserialize>::deserialize(deserializer).map(secrecy::SecretString::from)
}

/// Initialise `tracing` with an env-driven filter and a compact text
/// formatter. Matches the shape used in `hail-worker` so both binaries
/// produce identically-structured logs under `RUST_LOG`.
fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("hail_api=info,info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

/// Block until SIGINT or SIGTERM arrives. Takes pre-installed streams
/// by `&mut` so the handler registration happens at startup.
async fn wait_for_shutdown(sigint: &mut Signal, sigterm: &mut Signal) {
    tokio::select! {
        _ = sigint.recv() => {
            info!("received SIGINT");
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM");
        }
    }
}
