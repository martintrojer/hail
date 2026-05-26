//! Screener primitives shared by API request handling and worker routing.
//!
//! The sidecar database stores one rule per normalized sender address. These
//! helpers keep normalization and DB decision parsing consistent everywhere
//! rules are written or consumed.

use crate::MailClassification;

/// Normalize a user- or JMAP-provided sender into the sidecar rule key.
///
/// Handles bare addresses and simple display-name forms like
/// `Jane <JANE@Example.COM>`; validation remains caller-specific.
#[must_use]
pub fn normalize_sender(sender: &str) -> String {
    let trimmed = sender.trim();
    let email = match (trimmed.rfind('<'), trimmed.rfind('>')) {
        (Some(start), Some(end)) if start < end => &trimmed[start + 1..end],
        _ => trimmed,
    };
    email.trim().to_ascii_lowercase()
}

/// Alias for backward compatibility.
pub type Classification = MailClassification;

/// Parsed DB-level screener decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenerDecision {
    Allow(Classification),
    Deny,
    Pending,
}

/// One parsed screener rule for lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenerRule {
    pub sender_address: String,
    pub decision: ScreenerDecision,
}

impl ScreenerRule {
    /// Parse a row from the sidecar `screener_rules` table.
    ///
    /// `allow` rows must include a valid `classify_as`; `deny` and `pending`
    /// rows ignore it.
    pub fn from_db(
        sender_address: impl Into<String>,
        decision: &str,
        classify_as: Option<&str>,
    ) -> Result<Self, ScreenerRuleParseError> {
        let decision = match decision {
            "allow" => {
                let classify_as =
                    classify_as.ok_or(ScreenerRuleParseError::MissingClassification)?;
                let classification = Classification::parse(classify_as).ok_or_else(|| {
                    ScreenerRuleParseError::InvalidClassification(classify_as.to_string())
                })?;
                ScreenerDecision::Allow(classification)
            }
            "deny" => ScreenerDecision::Deny,
            "pending" => ScreenerDecision::Pending,
            other => return Err(ScreenerRuleParseError::InvalidDecision(other.to_string())),
        };

        Ok(Self {
            sender_address: normalize_sender(&sender_address.into()),
            decision,
        })
    }
}

/// Errors while parsing a screener rule from its DB representation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScreenerRuleParseError {
    #[error("allow rule missing classify_as")]
    MissingClassification,
    #[error("invalid classification: {0}")]
    InvalidClassification(String),
    #[error("invalid screener decision: {0}")]
    InvalidDecision(String),
}

/// Find the decision for `sender` in an in-memory rule set.
///
/// Both sides are normalized, so callers can pass raw JMAP `From` values or
/// stored rule addresses safely.
#[must_use]
pub fn lookup_rule(rules: &[ScreenerRule], sender: &str) -> Option<ScreenerDecision> {
    let sender = normalize_sender(sender);
    rules
        .iter()
        .find(|rule| normalize_sender(&rule.sender_address) == sender)
        .map(|rule| rule.decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_sender_cases() {
        let cases = [
            ("John <john@FOO.com>", "john@foo.com"),
            ("  bob@bar.org ", "bob@bar.org"),
            ("alice@example.com", "alice@example.com"),
            ("Jane Q. Public < JANE@Example.COM >", "jane@example.com"),
            ("UPPER@EXAMPLE.ORG", "upper@example.org"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_sender(input), expected);
        }
    }

    #[test]
    fn lookup_rule_normalizes_sender_and_rules() {
        let rules =
            [
                ScreenerRule::from_db(" Sender Name <SENDER@Example.COM> ", "allow", Some("feed"))
                    .expect("parse rule"),
            ];

        assert_eq!(
            lookup_rule(&rules, "sender@example.com"),
            Some(ScreenerDecision::Allow(Classification::Feed))
        );
        assert_eq!(lookup_rule(&rules, "other@example.com"), None);
    }

    #[test]
    fn allow_rule_requires_valid_classification() {
        assert_eq!(
            ScreenerRule::from_db("sender@example.com", "allow", None).unwrap_err(),
            ScreenerRuleParseError::MissingClassification
        );
        assert_eq!(
            ScreenerRule::from_db("sender@example.com", "allow", Some("later")).unwrap_err(),
            ScreenerRuleParseError::InvalidClassification("later".to_string())
        );
    }
}
