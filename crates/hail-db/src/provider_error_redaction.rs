//! Redaction helpers for provider import/sync error surfaces.
//!
//! Provider integrations may receive error strings from OAuth, Gmail, JMAP, or
//! MIME/import code. Before those strings are persisted or returned through API
//! status responses they must be reduced to UI-safe diagnostics: no bearer,
//! access, refresh-token looking values and no raw RFC822/message body snippets.

const MAX_SAFE_ERROR_CHARS: usize = 240;
const REDACTED: &str = "[redacted]";

/// Return a bounded provider error string safe for DB/API status fields.
#[must_use]
pub fn safe_provider_error_message(error: &impl std::fmt::Display) -> String {
    sanitize_provider_error_text(&error.to_string(), MAX_SAFE_ERROR_CHARS)
}

/// Return a bounded provider metadata string safe for audit JSON fields.
#[must_use]
pub fn safe_provider_metadata_json(metadata_json: &str) -> String {
    sanitize_provider_error_text(metadata_json, MAX_SAFE_ERROR_CHARS)
}

fn sanitize_provider_error_text(input: &str, max_chars: usize) -> String {
    let input = strip_raw_rfc822_body(input);
    let redacted = redact_secret_tokens(&input);
    redacted.chars().take(max_chars).collect()
}

fn strip_raw_rfc822_body(input: &str) -> String {
    let markers = [
        "\r\n\r\n",
        "\n\n",
        "\\r\\n\\r\\n",
        "\\n\\n",
        "body=",
        "body:",
        r#""body""#,
        "raw_rfc822=",
        "raw rfc822:",
        "Content-Type:",
    ];
    let mut cut = input.len();
    let lower = input.to_ascii_lowercase();
    for marker in markers {
        if let Some(idx) = lower.find(&marker.to_ascii_lowercase()) {
            cut = cut.min(idx + marker.len());
        }
    }
    if cut < input.len() {
        format!("{} {}", input[..cut].trim_end(), REDACTED)
    } else {
        input.to_owned()
    }
}

fn redact_secret_tokens(input: &str) -> String {
    input
        .split_whitespace()
        .map(redact_tokenish_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_tokenish_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    let trimmed =
        word.trim_matches(|ch: char| matches!(ch, ',' | ';' | ')' | ']' | '}' | '\'' | '"'));
    let trimmed_lower = trimmed.to_ascii_lowercase();

    if lower == "bearer" {
        return REDACTED.to_owned();
    }

    if let Some((key, _value)) = trimmed.split_once('=') {
        let key_lower = key.to_ascii_lowercase();
        if key_lower.contains("access_token")
            || key_lower.contains("refresh_token")
            || key_lower == "token"
            || key_lower.ends_with("_token")
        {
            return format_preserving_assignment(word, key, '=');
        }
    }

    if let Some((key, _value)) = trimmed.split_once(':') {
        let key_lower = key.to_ascii_lowercase();
        if key_lower.contains("access_token")
            || key_lower.contains("refresh_token")
            || key_lower == "authorization"
            || key_lower == "token"
            || key_lower.ends_with("_token")
        {
            return format_preserving_assignment(word, key, ':');
        }
    }

    if looks_like_oauth_token(&trimmed_lower) {
        REDACTED.to_owned()
    } else {
        word.to_owned()
    }
}

fn format_preserving_assignment(original: &str, key: &str, sep: char) -> String {
    let suffix: String = original
        .chars()
        .rev()
        .take_while(|ch| matches!(ch, ',' | ';' | ')' | ']' | '}' | '\'' | '"'))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{key}{sep}{REDACTED}{suffix}")
}

fn looks_like_oauth_token(value: &str) -> bool {
    value.starts_with("ya29.")
        || value.starts_with("1//")
        || value.starts_with("oauth2:")
        || value.starts_with("access-token-")
        || value.starts_with("refresh-token-")
}

#[cfg(test)]
mod tests {
    use super::safe_provider_error_message;

    #[test]
    fn redacts_provider_tokens_and_body_snippets() {
        let safe = safe_provider_error_message(
            &"Authorization: Bearer ya29.access-secret refresh_token=1//refresh-secret\r\n\r\nSubject: private\r\n\r\nhello body",
        );
        assert!(!safe.contains("Bearer"));
        assert!(!safe.contains("ya29.access-secret"));
        assert!(!safe.contains("1//refresh-secret"));
        assert!(!safe.contains("Subject: private"));
        assert!(safe.contains("[redacted]"));
    }
}
