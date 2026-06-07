use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use hail_backend::MailBackend as MailBackendTrait;
use hail_core::{Config, MailBackend, ProviderOAuthTokenKind, ProviderTokenContext};
use hail_gmail::gmail_client::{
    CachedGmailTokenSource, GmailAccessToken, GmailAccessTokenProvider, GmailClient,
    GmailClientError,
};
use hail_gmail::gmail_outbound_smtp::{GmailOutboundSmtpClient, LettreGmailSmtpSender};
use secrecy::{ExposeSecret, SecretString};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use crate::crypto::TokenDecryptor;

#[derive(Clone)]
pub struct WorkerBackendFactory {
    config: Arc<Config>,
    db: SqlitePool,
    server_key: [u8; hail_core::KEY_LEN],
    token_decryptor: Arc<dyn TokenDecryptor>,
    http: reqwest::Client,
}

impl WorkerBackendFactory {
    pub fn new(
        config: Arc<Config>,
        db: SqlitePool,
        token_decryptor: Arc<dyn TokenDecryptor>,
    ) -> Result<Self> {
        Ok(Self {
            server_key: hail_core::parse_server_key(&config.secrets.server_key)
                .map_err(|err| anyhow!("parse server key for worker backend factory: {err}"))?,
            config,
            db,
            token_decryptor,
            http: reqwest::Client::new(),
        })
    }

    pub async fn backend_for_account(
        &self,
        account_id: i64,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MailBackendTrait + Send + Sync>> {
        match self.config.mail.backend {
            MailBackend::Jmap => self.jmap_backend_for_account(account_id, cancel).await,
            MailBackend::Gmail => self.gmail_backend_for_account(account_id, cancel).await,
        }
    }

    async fn jmap_backend_for_account(
        &self,
        account_id: i64,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MailBackendTrait + Send + Sync>> {
        let user_id: i64 = sqlx::query_scalar("SELECT user_id FROM mail_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&self.db)
            .await
            .with_context(|| format!("load mail account {account_id} user for JMAP backend"))?;
        let token =
            crate::jmap_helpers::latest_active_token(&self.db, &self.token_decryptor, user_id)
                .await?;
        let session = hail_jmap::login_bearer(&self.config.mail.jmap.jmap_url, token)
            .await
            .with_context(|| format!("connect JMAP backend for mail account {account_id}"))?;
        Ok(Box::new(hail_jmap::JmapBackend::with_cancel(
            session, cancel,
        )))
    }

    async fn gmail_backend_for_account(
        &self,
        account_id: i64,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MailBackendTrait + Send + Sync>> {
        let account = sqlx::query_as::<_, (i64, i64, String)>(
            "SELECT id, user_id, provider_account_id FROM mail_accounts \
             WHERE id = ?1 AND backend_kind = 'gmail' AND sync_status != 'disconnected'",
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await
        .with_context(|| format!("load Gmail account {account_id} for backend"))?
        .ok_or_else(|| anyhow!("mail account {account_id} is not an active Gmail account"))?;
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
        Ok(Box::new(
            hail_gmail::GmailBackend::from_parts(gmail, smtp).with_cancel_token(cancel),
        ))
    }
}

impl Drop for WorkerBackendFactory {
    fn drop(&mut self) {
        self.server_key.fill(0);
    }
}

#[derive(Clone, Debug)]
struct DbGmailTokenSource {
    http: reqwest::Client,
    client_id: Option<String>,
    client_secret: Option<SecretString>,
    token_url: String,
    refresh_token: SecretString,
}

impl DbGmailTokenSource {
    async fn load(
        factory: &WorkerBackendFactory,
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
        let context = ProviderTokenContext::new(
            user_id,
            account_id,
            "gmail",
            provider_account_id,
            ProviderOAuthTokenKind::Refresh,
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
            refresh_token: SecretString::from(token.expose_secret().to_string()),
        })
    }
}

#[async_trait::async_trait]
impl GmailAccessTokenProvider for DbGmailTokenSource {
    async fn refresh_access_token(
        &self,
    ) -> std::result::Result<GmailAccessToken, GmailClientError> {
        let client_id = self.client_id.as_deref().ok_or_else(|| {
            GmailClientError::token_error(std::io::Error::other(
                "gmail oauth client id is not configured",
            ))
        })?;
        let client_secret = self.client_secret.as_ref().ok_or_else(|| {
            GmailClientError::token_error(std::io::Error::other(
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
            .map_err(GmailClientError::Request)?
            .error_for_status()
            .map_err(GmailClientError::Request)?
            .json()
            .await
            .map_err(GmailClientError::Request)?;
        Ok(GmailAccessToken {
            token: token.access_token,
            expires_in: std::time::Duration::from_secs(token.expires_in.unwrap_or(3600)),
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct GoogleRefreshTokenResponse {
    #[serde(deserialize_with = "deserialize_secret")]
    access_token: SecretString,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn deserialize_secret<'de, D>(deserializer: D) -> std::result::Result<SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <String as serde::Deserialize>::deserialize(deserializer).map(SecretString::from)
}
