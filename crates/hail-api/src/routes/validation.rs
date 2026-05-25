//! Shared validation helpers for user-facing identifiers.
//!
//! Setup and admin routes should reject the same malformed email/domain values
//! before calling Stalwart. Keep these helpers intentionally conservative:
//! hail only needs ordinary ASCII mailbox names and DNS host-style domains.

/// Return true when `email` is a conservative ASCII mailbox address whose
/// domain also satisfies [`valid_domain`].
pub(crate) fn valid_email(email: &str) -> bool {
    if email.is_empty()
        || email.len() > 254
        || email.contains(char::is_whitespace)
        || email.matches('@').count() != 1
    {
        return false;
    }

    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    valid_email_local_part(local) && valid_domain(domain)
}

fn valid_email_local_part(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(
                    b,
                    b'@' | b'<' | b'>' | b'(' | b')' | b'[' | b']' | b':' | b';' | b','
                )
        })
}

/// Return true when `domain` is a conservative DNS host-style name.
pub(crate) fn valid_domain(domain: &str) -> bool {
    if domain.is_empty()
        || domain.len() > 253
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
        || !domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return false;
    }

    domain.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().any(|b| b.is_ascii_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    use super::{valid_domain, valid_email};

    #[test]
    fn domain_validation_rejects_malformed_labels() {
        for domain in [
            "",
            "localhost",
            ".example.org",
            "example.org.",
            "example..org",
            "-bad.example",
            "bad-.example",
            "exa mple.org",
            "123.456",
        ] {
            assert!(!valid_domain(domain), "domain={domain:?}");
        }

        for domain in ["example.org", "mail.example.org", "xn--bcher-kva.example"] {
            assert!(valid_domain(domain), "domain={domain:?}");
        }
    }

    #[test]
    fn email_validation_rejects_malformed_addresses() {
        for email in [
            "",
            "not-an-email",
            "alice@@example.org",
            "@example.org",
            ".alice@example.org",
            "alice.@example.org",
            "ali..ce@example.org",
            "alice@-bad.example",
            "alice@example..org",
            "ali ce@example.org",
        ] {
            assert!(!valid_email(email), "email={email:?}");
        }

        for email in [
            "alice@example.org",
            "user+tag@example.org",
            "a.b-c_d@example.org",
        ] {
            assert!(valid_email(email), "email={email:?}");
        }
    }
}
