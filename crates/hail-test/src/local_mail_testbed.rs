//! Local mail testbed helpers.
//!
//! These helpers are intentionally test-only support code. They know how to
//! select the checked-in synthetic `.eml` corpus and, once a real Stalwart user
//! exists, import those raw messages through JMAP `Email/import`.

use hail_jmap::jmap_client::mailbox::{Role, query::Filter};
use secrecy::SecretString;

use crate::{MailFixture, mail_fixture};

/// Synthetic messages imported by the local mail testbed smoke path.
pub const LOCAL_TESTBED_FIXTURE_NAMES: &[&str] = &[
    "personal-simple.eml",
    "newsletter-tracking-pixel.eml",
    "receipt-papertrail.eml",
];

/// One fixture selected for local-mail-testbed injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFixture {
    /// Stable fixture filename.
    pub name: &'static str,
    /// Raw fixture payload size in bytes.
    pub bytes: usize,
    /// Product view hinted by the fixture, when present.
    pub intended_view: Option<String>,
}

/// Result of importing one fixture through JMAP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedEmail {
    /// Fixture filename.
    pub fixture_name: &'static str,
    /// JMAP Email id returned by Stalwart.
    pub email_id: String,
    /// JMAP Thread id returned by Stalwart, when available.
    pub thread_id: Option<String>,
}

/// Return the local-mail-testbed fixture set in import order.
pub fn local_testbed_fixtures() -> Result<Vec<MailFixture>, LocalMailTestbedError> {
    LOCAL_TESTBED_FIXTURE_NAMES
        .iter()
        .map(|name| mail_fixture(name).ok_or(LocalMailTestbedError::MissingFixture { name }))
        .collect()
}

/// Return a dry-run plan for the local-mail-testbed fixture imports.
pub fn local_testbed_plan() -> Result<Vec<PlannedFixture>, LocalMailTestbedError> {
    local_testbed_fixtures()?
        .into_iter()
        .map(|fixture| {
            let headers = fixture.headers()?;
            Ok(PlannedFixture {
                name: fixture.name,
                bytes: fixture.bytes().len(),
                intended_view: headers.get("x-hail-intended-view").map(str::to_owned),
            })
        })
        .collect()
}

/// Import the default local-mail-testbed fixture set into a JMAP account.
///
/// This requires that the target Stalwart domain/user already exists. Current
/// Stalwart bootstrap automation is intentionally not faked; callers should use
/// this after manual provisioning or after `seed_user_domain()` is implemented.
pub async fn import_local_testbed_fixtures_via_jmap(
    jmap_url: &str,
    email: &str,
    password: SecretString,
) -> Result<Vec<ImportedEmail>, LocalMailTestbedError> {
    let session = hail_jmap::login_basic(jmap_url, email, password)
        .await
        .map_err(LocalMailTestbedError::Login)?;
    import_fixtures_via_jmap(&session, &local_testbed_fixtures()?).await
}

/// Import arbitrary raw RFC822 bytes into a logged-in JMAP account's Inbox.
pub async fn import_raw_message_via_jmap(
    session: &hail_jmap::Session,
    fixture_name: &'static str,
    raw_message: Vec<u8>,
) -> Result<ImportedEmail, LocalMailTestbedError> {
    let inbox_id = inbox_id(session).await?;
    let email = session
        .client()
        .email_import_account(
            session.account_id(),
            raw_message,
            [inbox_id],
            None::<Vec<String>>,
            None,
        )
        .await
        .map_err(jmap_error)?;

    let email_id = email
        .id()
        .map(str::to_owned)
        .ok_or(LocalMailTestbedError::MissingImportedEmailId { fixture_name })?;

    Ok(ImportedEmail {
        fixture_name,
        email_id,
        thread_id: email.thread_id().map(str::to_owned),
    })
}

/// Import arbitrary fixtures into a logged-in JMAP account's Inbox.
pub async fn import_fixtures_via_jmap(
    session: &hail_jmap::Session,
    fixtures: &[MailFixture],
) -> Result<Vec<ImportedEmail>, LocalMailTestbedError> {
    let inbox_id = inbox_id(session).await?;

    let mut imported = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        let email = session
            .client()
            .email_import_account(
                session.account_id(),
                fixture.bytes().to_vec(),
                [inbox_id.clone()],
                None::<Vec<String>>,
                None,
            )
            .await
            .map_err(jmap_error)?;

        let email_id =
            email
                .id()
                .map(str::to_owned)
                .ok_or(LocalMailTestbedError::MissingImportedEmailId {
                    fixture_name: fixture.name,
                })?;
        imported.push(ImportedEmail {
            fixture_name: fixture.name,
            email_id,
            thread_id: email.thread_id().map(str::to_owned),
        });
    }

    Ok(imported)
}

async fn inbox_id(session: &hail_jmap::Session) -> Result<String, LocalMailTestbedError> {
    let mut query = session
        .client()
        .mailbox_query(Some(Filter::role(Role::Inbox)), None::<Vec<_>>)
        .await
        .map_err(jmap_error)?;
    query
        .take_ids()
        .into_iter()
        .next()
        .ok_or(LocalMailTestbedError::MissingInbox)
}

fn jmap_error(error: hail_jmap::jmap_client::Error) -> LocalMailTestbedError {
    LocalMailTestbedError::Jmap(error.to_string())
}

/// Errors returned by local mail testbed helpers.
#[derive(Debug, thiserror::Error)]
pub enum LocalMailTestbedError {
    /// A required checked-in fixture is absent from the compiled corpus.
    #[error("required local-mail-testbed fixture is missing: {name}")]
    MissingFixture {
        /// Missing fixture filename.
        name: &'static str,
    },
    /// Fixture headers failed to parse.
    #[error(transparent)]
    Fixture(#[from] crate::FixtureError),
    /// Login to Stalwart/JMAP failed.
    #[error("failed to login to JMAP for local mail testbed: {0}")]
    Login(hail_jmap::Error),
    /// JMAP import/query failed.
    #[error("JMAP local mail testbed import failed: {0}")]
    Jmap(String),
    /// The account has no Inbox mailbox to receive imported mail.
    #[error("JMAP account has no Inbox mailbox for local mail testbed imports")]
    MissingInbox,
    /// Stalwart accepted the import but did not return an Email id.
    #[error("JMAP Email/import for {fixture_name} did not return an Email id")]
    MissingImportedEmailId {
        /// Fixture being imported.
        fixture_name: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_testbed_plan_contains_required_smoke_messages() {
        let plan = local_testbed_plan().expect("plan builds");
        let names = plan.iter().map(|fixture| fixture.name).collect::<Vec<_>>();
        assert_eq!(names, LOCAL_TESTBED_FIXTURE_NAMES);
        assert!(plan.iter().all(|fixture| fixture.bytes > 0));
        assert_eq!(plan[0].intended_view.as_deref(), Some("Imbox"));
        assert_eq!(plan[1].intended_view.as_deref(), Some("Feed"));
        assert_eq!(plan[2].intended_view.as_deref(), Some("Paper Trail"));
    }
}
