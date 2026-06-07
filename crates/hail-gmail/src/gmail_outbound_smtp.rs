//! Gmail outbound SMTP delivery via SASL XOAUTH2.
//!
//! Gmail's SMTP XOAUTH2 flow uses the OAuth access token as the SMTP
//! credential. Keep this module token-conscious: do not log credentials or raw
//! message bodies, and classify authentication failures so callers can surface a
//! reconnect action without breaking read-only import.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use lettre::address::{Address, Envelope as LettreEnvelope};
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::response::Response;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::gmail_client::{GmailClientError, GmailTokenSource};

pub const GMAIL_SMTP_HOST: &str = "smtp.gmail.com";
pub const GMAIL_SMTP_PORT: u16 = 465;
pub const GMAIL_SMTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailOutboundMessage {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub plain_text: String,
    pub html: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailRawOutboundMessage {
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
    pub rfc822: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailSmtpSubmission {
    pub id: String,
}

#[derive(Debug, Error)]
pub enum GmailOutboundSmtpError {
    #[error("gmail outbound token refresh failed: {0}")]
    Token(#[source] GmailClientError),
    #[error("gmail outbound SMTP authentication failed")]
    Authentication,
    #[error("gmail outbound SMTP send timed out")]
    Timeout,
    #[error("gmail outbound SMTP message build failed: {0}")]
    Message(String),
    #[error("gmail outbound SMTP transport failed: {0}")]
    Transport(String),
}

impl GmailOutboundSmtpError {
    #[must_use]
    pub fn error_class(&self) -> &'static str {
        match self {
            Self::Token(_) | Self::Authentication => "provider_token",
            Self::Timeout => "provider_smtp_timeout",
            Self::Message(_) => "provider_message",
            Self::Transport(_) => "provider_smtp",
        }
    }
}

#[async_trait]
pub trait GmailSmtpSender: Send + Sync {
    async fn send_message(
        &self,
        access_token: SecretString,
        message: &GmailOutboundMessage,
    ) -> Result<(), GmailOutboundSmtpError>;

    async fn send_raw_message(
        &self,
        access_token: SecretString,
        message: &GmailRawOutboundMessage,
    ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError>;
}

pub struct LettreGmailSmtpSender;

#[async_trait]
impl GmailSmtpSender for LettreGmailSmtpSender {
    async fn send_message(
        &self,
        access_token: SecretString,
        message: &GmailOutboundMessage,
    ) -> Result<(), GmailOutboundSmtpError> {
        let email = build_lettre_message(message)?;
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(GMAIL_SMTP_HOST)
            .map_err(|err| GmailOutboundSmtpError::Transport(err.to_string()))?
            .port(GMAIL_SMTP_PORT)
            .credentials(Credentials::new(
                message.from.clone(),
                access_token.expose_secret().to_owned(),
            ))
            .authentication(vec![Mechanism::Xoauth2])
            .build();

        let send = mailer.send(email);
        match tokio::time::timeout(GMAIL_SMTP_TIMEOUT, send).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) if is_smtp_auth_error(&err) => Err(GmailOutboundSmtpError::Authentication),
            Ok(Err(err)) => Err(GmailOutboundSmtpError::Transport(err.to_string())),
            Err(_) => Err(GmailOutboundSmtpError::Timeout),
        }
    }

    async fn send_raw_message(
        &self,
        access_token: SecretString,
        message: &GmailRawOutboundMessage,
    ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError> {
        let envelope = build_lettre_envelope(message)?;
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(GMAIL_SMTP_HOST)
            .map_err(|err| GmailOutboundSmtpError::Transport(err.to_string()))?
            .port(GMAIL_SMTP_PORT)
            .credentials(Credentials::new(
                message.mail_from.clone(),
                access_token.expose_secret().to_owned(),
            ))
            .authentication(vec![Mechanism::Xoauth2])
            .build();

        let send = mailer.send_raw(&envelope, &message.rfc822);
        match tokio::time::timeout(GMAIL_SMTP_TIMEOUT, send).await {
            Ok(Ok(response)) => Ok(GmailSmtpSubmission {
                id: smtp_submission_id(&response),
            }),
            Ok(Err(err)) if is_smtp_auth_error(&err) => Err(GmailOutboundSmtpError::Authentication),
            Ok(Err(err)) => Err(GmailOutboundSmtpError::Transport(err.to_string())),
            Err(_) => Err(GmailOutboundSmtpError::Timeout),
        }
    }
}

pub struct GmailOutboundSmtpClient<T, S> {
    token_source: T,
    sender: S,
}

impl<T, S> GmailOutboundSmtpClient<T, S> {
    #[must_use]
    pub fn new(token_source: T, sender: S) -> Self {
        Self {
            token_source,
            sender,
        }
    }
}

impl<T, S> GmailOutboundSmtpClient<T, S>
where
    T: GmailTokenSource,
    S: GmailSmtpSender,
{
    pub async fn send(&self, message: &GmailOutboundMessage) -> Result<(), GmailOutboundSmtpError> {
        let first_token = self
            .token_source
            .bearer_token()
            .await
            .map_err(GmailOutboundSmtpError::Token)?;
        match self.sender.send_message(first_token, message).await {
            Ok(()) => Ok(()),
            Err(GmailOutboundSmtpError::Authentication) => {
                let retry_token = self
                    .token_source
                    .bearer_token()
                    .await
                    .map_err(GmailOutboundSmtpError::Token)?;
                self.sender.send_message(retry_token, message).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn send_raw(
        &self,
        message: &GmailRawOutboundMessage,
    ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError> {
        let first_token = self
            .token_source
            .bearer_token()
            .await
            .map_err(GmailOutboundSmtpError::Token)?;
        match self.sender.send_raw_message(first_token, message).await {
            Ok(submission) => Ok(submission),
            Err(GmailOutboundSmtpError::Authentication) => {
                let retry_token = self
                    .token_source
                    .bearer_token()
                    .await
                    .map_err(GmailOutboundSmtpError::Token)?;
                self.sender.send_raw_message(retry_token, message).await
            }
            Err(error) => Err(error),
        }
    }
}

#[must_use]
pub fn xoauth2_initial_response(user: &str, access_token: &str) -> String {
    format!("user={user}\x01auth=Bearer {access_token}\x01\x01")
}

#[must_use]
pub fn xoauth2_initial_response_b64(user: &str, access_token: &str) -> String {
    BASE64_STANDARD.encode(xoauth2_initial_response(user, access_token))
}

pub type BoxSendFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), GmailOutboundSmtpError>> + Send + 'a>>;

pub trait GmailOutboundSmtp: Send + Sync + 'static {
    fn send_gmail<'a>(&'a self, message: &'a GmailOutboundMessage) -> BoxSendFuture<'a>;
}

impl<T, S> GmailOutboundSmtp for GmailOutboundSmtpClient<T, S>
where
    T: GmailTokenSource + Send + Sync + 'static,
    S: GmailSmtpSender + Send + Sync + 'static,
{
    fn send_gmail<'a>(&'a self, message: &'a GmailOutboundMessage) -> BoxSendFuture<'a> {
        Box::pin(async move { self.send(message).await })
    }
}

fn build_lettre_envelope(
    message: &GmailRawOutboundMessage,
) -> Result<LettreEnvelope, GmailOutboundSmtpError> {
    let from = parse_reverse_path(&message.mail_from)?;
    let recipients = message
        .rcpt_to
        .iter()
        .map(|recipient| parse_address(recipient))
        .collect::<Result<Vec<_>, _>>()?;
    LettreEnvelope::new(from, recipients)
        .map_err(|err| GmailOutboundSmtpError::Message(err.to_string()))
}

fn build_lettre_message(message: &GmailOutboundMessage) -> Result<Message, GmailOutboundSmtpError> {
    let mut builder = Message::builder()
        .from(parse_mailbox(&message.from)?)
        .subject(message.subject.clone());
    for to in &message.to {
        builder = builder.to(parse_mailbox(to)?);
    }
    for cc in &message.cc {
        builder = builder.cc(parse_mailbox(cc)?);
    }
    for bcc in &message.bcc {
        builder = builder.bcc(parse_mailbox(bcc)?);
    }
    builder
        .header(ContentType::TEXT_HTML)
        .body(message.html.clone())
        .map_err(|err| GmailOutboundSmtpError::Message(err.to_string()))
}

fn parse_mailbox(address: &str) -> Result<Mailbox, GmailOutboundSmtpError> {
    address
        .parse()
        .map_err(|err| GmailOutboundSmtpError::Message(format!("invalid mailbox: {err}")))
}

fn parse_reverse_path(address: &str) -> Result<Option<Address>, GmailOutboundSmtpError> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_address(trimmed).map(Some)
}

fn parse_address(address: &str) -> Result<Address, GmailOutboundSmtpError> {
    address
        .parse()
        .map_err(|err| GmailOutboundSmtpError::Message(format!("invalid SMTP address: {err}")))
}

fn smtp_submission_id(response: &Response) -> String {
    let code = u16::from(response.code());
    let message = response.message().collect::<Vec<_>>().join(" | ");
    if message.is_empty() {
        code.to_string()
    } else {
        format!("{code} {message}")
    }
}

fn is_smtp_auth_error(err: &lettre::transport::smtp::Error) -> bool {
    err.to_string().contains("535") || err.to_string().to_ascii_lowercase().contains("auth")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use secrecy::SecretString;

    use super::*;
    use crate::gmail_client::{GmailClientError, GmailTokenSource};

    #[test]
    fn xoauth2_string_matches_google_sasl_format() {
        let raw = xoauth2_initial_response("someuser@example.com", "ya29.token");
        assert_eq!(
            raw,
            "user=someuser@example.com\x01auth=Bearer ya29.token\x01\x01"
        );
        assert_eq!(
            xoauth2_initial_response_b64("someuser@example.com", "ya29.token"),
            BASE64_STANDARD.encode(raw)
        );
    }

    #[derive(Clone)]
    struct SequenceTokenSource {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GmailTokenSource for SequenceTokenSource {
        async fn bearer_token(&self) -> Result<SecretString, GmailClientError> {
            let next = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(SecretString::from(format!("token-{next}")))
        }
    }

    struct FailingOnceSender {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GmailSmtpSender for FailingOnceSender {
        async fn send_message(
            &self,
            access_token: SecretString,
            _message: &GmailOutboundMessage,
        ) -> Result<(), GmailOutboundSmtpError> {
            let next = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if next == 1 {
                assert_eq!(access_token.expose_secret(), "token-1");
                return Err(GmailOutboundSmtpError::Authentication);
            }
            assert_eq!(access_token.expose_secret(), "token-2");
            Ok(())
        }

        async fn send_raw_message(
            &self,
            access_token: SecretString,
            _message: &GmailRawOutboundMessage,
        ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError> {
            let next = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if next == 1 {
                assert_eq!(access_token.expose_secret(), "token-1");
                return Err(GmailOutboundSmtpError::Authentication);
            }
            assert_eq!(access_token.expose_secret(), "token-2");
            Ok(GmailSmtpSubmission {
                id: "250 2.0.0 accepted".to_string(),
            })
        }
    }

    struct AlwaysAuthFailSender {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GmailSmtpSender for AlwaysAuthFailSender {
        async fn send_message(
            &self,
            _access_token: SecretString,
            _message: &GmailOutboundMessage,
        ) -> Result<(), GmailOutboundSmtpError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(GmailOutboundSmtpError::Authentication)
        }

        async fn send_raw_message(
            &self,
            _access_token: SecretString,
            _message: &GmailRawOutboundMessage,
        ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(GmailOutboundSmtpError::Authentication)
        }
    }

    #[tokio::test]
    async fn smtp_535_refreshes_and_retries_once() {
        let token_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let client = GmailOutboundSmtpClient::new(
            SequenceTokenSource {
                calls: token_calls.clone(),
            },
            FailingOnceSender {
                calls: send_calls.clone(),
            },
        );
        client.send(&sample_message()).await.unwrap();
        assert_eq!(token_calls.load(Ordering::SeqCst), 2);
        assert_eq!(send_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn second_535_classifies_as_provider_token() {
        let token_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let client = GmailOutboundSmtpClient::new(
            SequenceTokenSource { calls: token_calls },
            AlwaysAuthFailSender {
                calls: send_calls.clone(),
            },
        );
        let err = client.send_raw(&sample_raw_message()).await.unwrap_err();
        assert_eq!(err.error_class(), "provider_token");
        assert_eq!(send_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn smtp_envelope_uses_supplied_envelope_not_message_headers() {
        let message = GmailRawOutboundMessage {
            mail_from: "bounce@example.org".to_string(),
            rcpt_to: vec![
                "actual-to@example.org".to_string(),
                "actual-bcc@example.org".to_string(),
            ],
            rfc822: b"From: Header From <header-from@example.org>\r\nTo: Header To <header-to@example.org>\r\nSubject: Keep bytes\r\n\r\nBody".to_vec(),
        };

        let envelope = build_lettre_envelope(&message).unwrap();
        assert_eq!(
            envelope.from().map(ToString::to_string),
            Some("bounce@example.org".to_string())
        );
        assert_eq!(
            envelope
                .to()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["actual-to@example.org", "actual-bcc@example.org"]
        );
    }

    #[test]
    fn smtp_envelope_rejects_empty_recipient_list() {
        let mut message = sample_raw_message();
        message.rcpt_to.clear();

        let err = build_lettre_envelope(&message).unwrap_err();
        assert!(matches!(err, GmailOutboundSmtpError::Message(_)));
    }

    fn sample_message() -> GmailOutboundMessage {
        GmailOutboundMessage {
            from: "alice@example.org".to_string(),
            to: vec!["bob@example.org".to_string()],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Hello".to_string(),
            plain_text: "Hello Bob".to_string(),
            html: "<p>Hello Bob</p>".to_string(),
        }
    }

    fn sample_raw_message() -> GmailRawOutboundMessage {
        GmailRawOutboundMessage {
            mail_from: "alice@example.org".to_string(),
            rcpt_to: vec!["bob@example.org".to_string()],
            rfc822:
                b"From: alice@example.org\r\nTo: bob@example.org\r\nSubject: Hello\r\n\r\nHello Bob"
                    .to_vec(),
        }
    }
}
