use jmap_client::mailbox::{Mailbox, Property, Role, query::Filter};

use crate::Session;

/// Hail-owned mailbox where unknown senders wait for user approval.
pub const SCREENER_MAILBOX_NAME: &str = "Screener";

pub async fn mailbox_id_by_name(
    session: &Session,
    name: &str,
) -> jmap_client::Result<Option<String>> {
    let mut query = session
        .client()
        .mailbox_query(Some(Filter::name(name)), None::<Vec<_>>)
        .await?;
    Ok(query.take_ids().into_iter().next())
}

/// Return the id of the mailbox with the requested JMAP role.
///
/// Stalwart has been observed to return an unrelated mailbox for
/// `Mailbox/query` role filters in some accounts. Fetching all mailboxes and
/// matching the deserialized role client-side is more robust and avoids routing
/// drafts into user-created folders such as Screener.
pub async fn mailbox_id_by_role(
    session: &Session,
    role: Role,
) -> jmap_client::Result<Option<String>> {
    let mut request = session.client().build();
    request
        .get_mailbox()
        .properties([Property::Id, Property::Role]);
    let mut response = request.send_get_mailbox().await?;
    Ok(mailbox_id_with_role(response.take_list(), role))
}

fn mailbox_id_with_role(mailboxes: Vec<Mailbox>, role: Role) -> Option<String> {
    mailboxes
        .into_iter()
        .find(|mailbox| mailbox.role() == role)
        .map(|mut mailbox| mailbox.take_id())
        .filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mailbox_by_role_without_using_query_order() {
        let mailboxes = serde_json::from_str::<Vec<Mailbox>>(
            r#"[
                {"id":"h","name":"Screener","role":null},
                {"id":"d","name":"Drafts","role":"drafts"}
            ]"#,
        )
        .unwrap();

        assert_eq!(
            mailbox_id_with_role(mailboxes, Role::Drafts),
            Some("d".to_string())
        );
    }

    #[test]
    fn skips_empty_ids() {
        let mailboxes = serde_json::from_str::<Vec<Mailbox>>(
            r#"[
                {"id":"","name":"Drafts","role":"drafts"}
            ]"#,
        )
        .unwrap();

        assert_eq!(mailbox_id_with_role(mailboxes, Role::Drafts), None);
    }
}
