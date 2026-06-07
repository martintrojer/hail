//! Sanitizers for provider import/sync audit and account error surfaces.
//!
//! Provider integrations may receive error strings from OAuth, Gmail, JMAP, or
//! MIME/import code. Every string persisted to mail account, provider
//! mapping, and provider audit error fields should pass through this module so
//! the safe path is ergonomic: no bearer/access/refresh-token looking values
//! and no raw RFC822/message body snippets.

use serde_json::{Map, Value, json};

const MAX_SAFE_ERROR_CHARS: usize = 240;
const MAX_SAFE_ERROR_KEY_CHARS: usize = 96;
const MAX_SAFE_METADATA_JSON_CHARS: usize = 2048;
const REDACTED: &str = "[redacted]";
const PROVIDER_ERROR_FALLBACK: &str = "provider_error";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeProviderErrorFields {
    pub code: Option<String>,
    pub class: Option<String>,
    pub message: Option<String>,
}

impl SafeProviderErrorFields {
    #[must_use]
    pub fn new(
        code: Option<&str>,
        class: Option<&str>,
        message: Option<&impl std::fmt::Display>,
    ) -> Self {
        Self {
            code: code.map(safe_provider_error_key),
            class: class.map(safe_provider_error_key),
            message: message.map(safe_provider_error_message),
        }
    }
}

#[must_use]
pub fn safe_provider_error_message(error: &impl std::fmt::Display) -> String {
    sanitize_provider_error_text(&error.to_string(), MAX_SAFE_ERROR_CHARS)
}

#[must_use]
pub fn safe_provider_account_error_message(error: &impl std::fmt::Display) -> String {
    safe_provider_error_message(error)
}

#[must_use]
pub fn safe_provider_error_key(key: &str) -> String {
    let trimmed = key.trim();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.is_empty()
        || lower.contains("bearer")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("authorization")
        || lower.contains("ya29.")
        || lower.contains("1//")
        || contains_rfc822_or_body_marker(trimmed)
    {
        return PROVIDER_ERROR_FALLBACK.to_owned();
    }

    let sanitized = trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':') {
                ch
            } else {
                '_'
            }
        })
        .take(MAX_SAFE_ERROR_KEY_CHARS)
        .collect::<String>();

    if sanitized.is_empty() {
        PROVIDER_ERROR_FALLBACK.to_owned()
    } else {
        sanitized
    }
}

#[must_use]
pub fn safe_provider_metadata_json_value(value: &Value) -> String {
    let sanitized = sanitize_metadata_value(None, value);
    let rendered = sanitized.to_string();
    if rendered.chars().count() > MAX_SAFE_METADATA_JSON_CHARS {
        json!({"truncated": true}).to_string()
    } else {
        rendered
    }
}

#[must_use]
pub fn safe_provider_metadata_json(metadata_json: &str) -> String {
    match serde_json::from_str::<Value>(metadata_json) {
        Ok(value) => safe_provider_metadata_json_value(&value),
        Err(_) => sanitize_provider_error_text(metadata_json, MAX_SAFE_ERROR_CHARS),
    }
}

fn sanitize_metadata_value(key: Option<&str>, value: &Value) -> Value {
    match value {
        Value::String(text) => {
            if key.is_some_and(is_body_like_metadata_key) || contains_rfc822_or_body_marker(text) {
                Value::String(REDACTED.to_owned())
            } else {
                Value::String(sanitize_provider_error_text(text, MAX_SAFE_ERROR_CHARS))
            }
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| sanitize_metadata_value(key, value))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        sanitize_metadata_value(Some(key.as_str()), value),
                    )
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

fn is_body_like_metadata_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "body"
            | "raw"
            | "raw_rfc822"
            | "rawrfc822"
            | "rfc822"
            | "mime"
            | "payload"
            | "snippet"
            | "message"
            | "message_body"
            | "content"
    )
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

fn contains_rfc822_or_body_marker(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    [
        "\r\n\r\n",
        "\n\n",
        "\\r\\n\\r\\n",
        "\\n\\n",
        "raw_rfc822=",
        "raw rfc822:",
        "content-type:",
        "subject:",
        "body=",
        "body:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
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
    use serde_json::Value;

    use super::{
        safe_provider_error_key, safe_provider_error_message, safe_provider_metadata_json,
    };

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

    #[test]
    fn sanitizes_json_without_breaking_json_validity() {
        let safe = safe_provider_metadata_json(
            r#"{"detail":"Authorization: Bearer ya29.metadata-secret","body":"Subject: Private\n\nmessage body","nested":{"refresh_token":"1//metadata-refresh"}}"#,
        );
        let parsed: Value = serde_json::from_str(&safe).expect("safe JSON");
        assert_eq!(parsed["body"], "[redacted]");
        assert!(!safe.contains("Bearer"));
        assert!(!safe.contains("ya29.metadata-secret"));
        assert!(!safe.contains("1//metadata-refresh"));
        assert!(!safe.contains("message body"));
    }

    #[test]
    fn sanitizes_error_keys() {
        assert_eq!(safe_provider_error_key("rate limited"), "rate_limited");
        assert_eq!(
            safe_provider_error_key("Authorization: Bearer ya29.secret"),
            "provider_error"
        );
    }
}
