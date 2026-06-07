use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend, Mailbox,
    MailboxRole, Page, PageRequest, Principal, Query, RawMessage, SubmissionId, SyncCursor,
};
use hail_core::{Config, MailBackend as ConfigMailBackend};
use hail_gmail::gmail_client::{CachedGmailTokenSource, GmailClient};
use hail_gmail::gmail_outbound_smtp::{GmailOutboundSmtpClient, LettreGmailSmtpSender};
use secrecy::ExposeSecret;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const JMAP_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: true,
    supports_principals_admin: true,
    supports_send: true,
    native_threading: true,
    max_attachment_size: 25 * 1024 * 1024,
    label_path_separator: '/',
};

const GMAIL_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: true,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: true,
    max_attachment_size: 25 * 1024 * 1024,
    label_path_separator: '/',
};

#[derive(Clone)]
pub struct ApiBackendFactory {
    config: Arc<Config>,
    db: SqlitePool,
    server_key: [u8; hail_core::KEY_LEN],
    http: reqwest::Client,
}

impl ApiBackendFactory {
    #[must_use]
    pub fn new(config: Arc<Config>, db: SqlitePool, server_key: [u8; hail_core::KEY_LEN]) -> Self {
        Self {
            config,
            db,
            server_key,
            http: reqwest::Client::new(),
        }
    }

    pub async fn backend_for_current_account(
        &self,
        cancel: CancellationToken,
    ) -> Result<Option<Box<dyn MailBackend + Send + Sync>>> {
        match self.config.mail.backend {
            ConfigMailBackend::Jmap => Ok(Some(self.jmap_backend().await?)),
            ConfigMailBackend::Gmail => self.gmail_backend(cancel).await,
        }
    }

    async fn jmap_backend(&self) -> Result<Box<dyn MailBackend + Send + Sync>> {
        let session = hail_jmap::login_bearer(
            &self.config.mail.jmap.jmap_url,
            self.config.secrets.server_key.clone(),
        )
        .await
        .context("connecting JMAP backend for cache")?;
        Ok(Box::new(hail_jmap::JmapBackend::new(session)))
    }

    async fn gmail_backend(
        &self,
        cancel: CancellationToken,
    ) -> Result<Option<Box<dyn MailBackend + Send + Sync>>> {
        let Some(account) = self.current_gmail_account().await? else {
            return Ok(None);
        };
        let token_source = DbGmailTokenSource::load(self, account).await?;
        let token_source = CachedGmailTokenSource::new(token_source);
        let gmail_base_url = self
            .config
            .provider_import
            .gmail
            .api_base_url
            .clone()
            .unwrap_or_else(|| "https://gmail.googleapis.com/gmail/v1/".to_string());
        let gmail =
            GmailClient::with_base_url(self.http.clone(), token_source.clone(), &gmail_base_url)
                .map_err(|err| anyhow!("build Gmail client: {err}"))?;
        let smtp = GmailOutboundSmtpClient::new(token_source, LettreGmailSmtpSender);
        Ok(Some(Box::new(
            hail_gmail::GmailBackend::from_parts(gmail, smtp).with_cancel_token(cancel),
        )))
    }

    async fn current_gmail_account(&self) -> Result<Option<(i64, i64, String)>> {
        sqlx::query_as::<_, (i64, i64, String)>(
            "SELECT id, user_id, provider_account_id FROM mail_accounts \
             WHERE backend_kind = 'gmail' AND sync_status != 'disconnected' AND refresh_token_enc IS NOT NULL \
             ORDER BY id LIMIT 1",
        )
        .fetch_optional(&self.db)
        .await
        .context("loading gmail account for cache backend")
    }
}

impl Drop for ApiBackendFactory {
    fn drop(&mut self) {
        self.server_key.fill(0);
    }
}

pub struct LazyMailBackend {
    factory: ApiBackendFactory,
    capabilities: &'static Capabilities,
    cancel: CancellationToken,
}

impl LazyMailBackend {
    #[must_use]
    pub fn new(factory: ApiBackendFactory) -> Self {
        let capabilities = match factory.config.mail.backend {
            ConfigMailBackend::Jmap => &JMAP_CAPABILITIES,
            ConfigMailBackend::Gmail => &GMAIL_CAPABILITIES,
        };
        Self {
            factory,
            capabilities,
            cancel: CancellationToken::new(),
        }
    }

    async fn require_backend(&self) -> hail_backend::Result<Box<dyn MailBackend + Send + Sync>> {
        self.factory
            .backend_for_current_account(self.cancel.child_token())
            .await
            .map_err(|err| hail_backend::Error::Other(err.to_string()))?
            .ok_or_else(not_connected)
    }
}

impl Drop for LazyMailBackend {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[async_trait]
impl MailBackend for LazyMailBackend {
    fn capabilities(&self) -> &'static Capabilities {
        self.capabilities
    }

    async fn list_message_ids(
        &self,
        query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        self.require_backend()
            .await?
            .list_message_ids(query, page)
            .await
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.require_backend().await?.get_message(id).await
    }

    async fn fetch_blob(&self, id: &BlobRef) -> hail_backend::Result<Bytes> {
        self.require_backend().await?.fetch_blob(id).await
    }

    async fn set_keywords(
        &self,
        id: &BackendMsgId,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> hail_backend::Result<()> {
        self.require_backend()
            .await?
            .set_keywords(id, add, remove)
            .await
    }

    async fn move_to_role(&self, id: &BackendMsgId, role: MailboxRole) -> hail_backend::Result<()> {
        self.require_backend().await?.move_to_role(id, role).await
    }

    async fn delete_permanently(&self, id: &BackendMsgId) -> hail_backend::Result<()> {
        self.require_backend().await?.delete_permanently(id).await
    }

    async fn send(&self, rfc822: &[u8], envelope: &Envelope) -> hail_backend::Result<SubmissionId> {
        self.require_backend().await?.send(rfc822, envelope).await
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        self.require_backend().await?.poll_changes(cursor).await
    }

    async fn watch_changes(&self) -> futures_core::stream::BoxStream<'static, Change> {
        match self.require_backend().await {
            Ok(backend) => backend.watch_changes().await,
            Err(_) => Box::pin(stream::empty()),
        }
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        self.require_backend().await?.list_mailboxes().await
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        self.require_backend().await?.list_principals().await
    }
}

fn not_connected() -> hail_backend::Error {
    hail_backend::Error::NotConnected
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
        factory: &ApiBackendFactory,
        (account_id, user_id, provider_account_id): (i64, i64, String),
    ) -> Result<Self> {
        let ciphertext: Vec<u8> = sqlx::query_scalar(
            "SELECT refresh_token_enc FROM mail_accounts WHERE id = ?1 AND user_id = ?2",
        )
        .bind(account_id)
        .bind(user_id)
        .fetch_one(&factory.db)
        .await
        .context("loading encrypted gmail refresh token")?;
        let context = hail_core::ProviderTokenContext::new(
            user_id,
            account_id,
            "gmail",
            provider_account_id,
            hail_core::ProviderOAuthTokenKind::Refresh,
        );
        let token =
            hail_core::open_provider_oauth_token(&ciphertext, &factory.server_key, &context)
                .map_err(|err| anyhow!(err))?;
        Ok(Self {
            http: factory.http.clone(),
            client_id: factory
                .config
                .mail
                .gmail
                .oauth_client_id
                .clone()
                .or_else(|| factory.config.provider_import.gmail.oauth_client_id.clone()),
            client_secret: factory
                .config
                .mail
                .gmail
                .oauth_client_secret
                .clone()
                .or_else(|| {
                    factory
                        .config
                        .provider_import
                        .gmail
                        .oauth_client_secret
                        .clone()
                }),
            token_url: factory
                .config
                .mail
                .gmail
                .oauth_token_url
                .clone()
                .or_else(|| factory.config.provider_import.gmail.oauth_token_url.clone())
                .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()),
            refresh_token: secrecy::SecretString::from(token.expose_secret().to_string()),
        })
    }
}

#[async_trait]
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
