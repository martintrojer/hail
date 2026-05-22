use secrecy::{ExposeSecret, SecretString};
use tracing::instrument;

use crate::Error;
use jmap_client::client::Credentials;

const JMAP_MAIL_CAPABILITY: &str = "urn:ietf:params:jmap:mail";

/// Authenticated JMAP session state used by hail services.
pub struct Session {
    client: jmap_client::client::Client,
    account_id: String,
}

/// Connect to a JMAP server using Basic authentication.
#[instrument(skip(password), fields(base_url = %base_url, email = %email))]
pub async fn login_basic(
    base_url: &str,
    email: &str,
    password: SecretString,
) -> Result<Session, Error> {
    let client = jmap_client::client::Client::new()
        .credentials((email, password.expose_secret()))
        .connect(base_url)
        .await
        .map_err(classify_connect_error)?;

    session_from_client(client)
}

/// Connect to a JMAP server using Bearer token authentication.
#[instrument(skip(token), fields(base_url = %base_url))]
pub async fn login_bearer(base_url: &str, token: SecretString) -> Result<Session, Error> {
    let client = jmap_client::client::Client::new()
        .credentials(Credentials::bearer(token.expose_secret()))
        .connect(base_url)
        .await
        .map_err(classify_connect_error)?;

    session_from_client(client)
}

impl Session {
    /// Primary JMAP Mail account id selected for this session.
    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Access the underlying `jmap-client` client for raw JMAP calls.
    #[must_use]
    pub fn client(&self) -> &jmap_client::client::Client {
        &self.client
    }
}

fn session_from_client(client: jmap_client::client::Client) -> Result<Session, Error> {
    let account_id = client
        .session()
        .primary_accounts()
        .find_map(|(capability, account_id)| {
            (capability == JMAP_MAIL_CAPABILITY).then(|| account_id.clone())
        })
        .ok_or(Error::MissingPrimaryAccount)?;

    Ok(Session { client, account_id })
}

fn classify_connect_error(error: jmap_client::Error) -> Error {
    if is_auth_error(&error) {
        Error::Auth(error)
    } else {
        Error::Connect(error)
    }
}

fn is_auth_error(error: &jmap_client::Error) -> bool {
    match error {
        jmap_client::Error::Problem(problem) => matches!(problem.status, Some(401 | 403)),
        jmap_client::Error::Server(status) => status.starts_with("401") || status.starts_with("403"),
        _ => false,
    }
}
