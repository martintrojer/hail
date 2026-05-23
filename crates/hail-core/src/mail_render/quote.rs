//! Conservative quoted-reply/history stripping for thread-as-document rendering.
//!
//! The stripper recognizes only common, high-confidence reply markers: Gmail
//! quote containers, citation blockquotes, Outlook header separators, trailing
//! `On ..., wrote:` history, and trailing plaintext `>` quote blocks. Ordinary
//! author-written quotes are preserved unless explicitly cite/history-marked.
//! When in doubt, leave the body unchanged.

/// Result of quoted-history stripping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrippedText {
    /// HTML after removing recognized quoted reply/history fragments.
    pub html: String,
    /// True when at least one high-confidence quoted-history fragment was removed.
    pub stripped: bool,
}

/// Strip common quoted reply/history fragments from a message HTML body.
///
/// Heuristics are deliberately conservative:
/// - remove complete elements only for provider/citation markers such as
///   `class="gmail_quote"`, `blockquote[type="cite"]`, or common quote classes;
/// - remove Outlook history only when an `<hr>` is followed by at least three
///   standard header labels (`From:`, `Sent:`, `To:`, `Subject:`);
/// - remove plaintext-style quote blocks only when they are trailing content;
/// - remove `On DATE, NAME wrote:` only when it starts trailing quoted history.
///
/// This is not a full HTML parser. It is a reusable render primitive designed
/// to run before thread assembly, where keeping questionable user text is safer
/// than hiding it.
pub fn strip_quoted_history(input_html: &str) -> StrippedText {
    let mut html = input_html.to_string();
    let mut stripped = false;

    stripped |= strip_gmail_quotes(&mut html);
    stripped |= strip_outlook_history(&mut html);
    stripped |= strip_on_wrote_trailing_history(&mut html);
    stripped |= strip_cite_blockquotes(&mut html);
    stripped |= strip_trailing_plaintext_quote(&mut html);

    StrippedText { html, stripped }
}

fn strip_gmail_quotes(html: &mut String) -> bool {
    strip_elements_matching(html, "div", |attrs| {
        class_attr(attrs).is_some_and(|class| has_class(&class, "gmail_quote"))
    })
}

fn strip_cite_blockquotes(html: &mut String) -> bool {
    strip_elements_matching(html, "blockquote", |attrs| {
        let attrs_lower = attrs.to_ascii_lowercase();
        attrs_lower.contains("type=\"cite\"")
            || attrs_lower.contains("type='cite'")
            || class_attr(attrs).is_some_and(|class| {
                has_class(&class, "gmail_quote")
                    || has_class(&class, "yahoo_quoted")
                    || has_class(&class, "protonmail_quote")
                    || class.contains("gmail_quote")
            })
    })
}

fn strip_outlook_history(html: &mut String) -> bool {
    let mut cursor = 0;
    while let Some(rel_hr_start) = find_next_tag_start(&html[cursor..], "hr") {
        let hr_start = cursor + rel_hr_start;
        let Some(hr_end) = find_tag_end(html, hr_start) else {
            break;
        };
        let after_hr = hr_end + 1;
        let probe_end = (after_hr + 2_000).min(html.len());
        if outlook_header_score(&html[after_hr..probe_end]) >= 3 {
            html.truncate(hr_start);
            trim_trailing_breaks(html);
            return true;
        }
        cursor = after_hr;
    }
    false
}

fn strip_on_wrote_trailing_history(html: &mut String) -> bool {
    for marker in [" wrote:", " wrote:&nbsp;"] {
        let mut search_end = html.len();
        while let Some(pos) = html[..search_end].rfind(marker) {
            let line_start = find_lineish_start(html, pos);
            let line_prefix = visible_text(&html[line_start..pos]).to_ascii_lowercase();
            if line_prefix.trim_start().starts_with("on ")
                && has_trailing_history_after(html, pos + marker.len())
            {
                html.truncate(line_start);
                trim_trailing_breaks(html);
                return true;
            }
            if pos == 0 {
                break;
            }
            search_end = pos;
        }
    }
    false
}

fn strip_trailing_plaintext_quote(html: &mut String) -> bool {
    let segments = line_segments(html);
    let mut quote_start = None;
    let mut seen_quote = false;

    for segment in segments.iter().rev() {
        let text = visible_text(&html[segment.start..segment.end]);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            if seen_quote {
                quote_start = Some(segment.start);
            }
            continue;
        }
        if starts_with_quote_marker(trimmed) {
            seen_quote = true;
            quote_start = Some(segment.start);
            continue;
        }
        break;
    }

    if let (true, Some(start)) = (seen_quote, quote_start) {
        html.truncate(start);
        trim_trailing_breaks(html);
        return true;
    }
    false
}

fn has_trailing_history_after(html: &str, start: usize) -> bool {
    let after = &html[start..];
    let visible = visible_text(after);
    let trimmed = visible.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = after.to_ascii_lowercase();
    lower.contains("<blockquote")
        || lower.contains("gmail_quote")
        || lower.contains("type=\"cite\"")
        || lower.contains("type='cite'")
        || trimmed
            .lines()
            .any(|line| starts_with_quote_marker(line.trim()))
}

fn outlook_header_score(html: &str) -> usize {
    let visible = visible_text(html).to_ascii_lowercase();
    ["from:", "sent:", "to:", "subject:"]
        .iter()
        .filter(|label| visible.contains(**label))
        .count()
}

fn strip_elements_matching(
    html: &mut String,
    tag: &str,
    mut matches: impl FnMut(&str) -> bool,
) -> bool {
    let mut stripped = false;
    let mut cursor = 0;
    while let Some(rel_start) = find_next_tag_start(&html[cursor..], tag) {
        let start = cursor + rel_start;
        let Some(open_end) = find_tag_end(html, start) else {
            break;
        };
        let open_tag = &html[start..=open_end];
        let attrs = open_tag
            .strip_prefix('<')
            .and_then(|s| s.strip_suffix('>'))
            .and_then(|s| s.trim().strip_prefix(tag))
            .unwrap_or_default();

        if matches(attrs) {
            if let Some(end) = find_matching_close(html, tag, open_end + 1) {
                html.replace_range(start..end, "");
                stripped = true;
                cursor = start;
                continue;
            }
        }
        cursor = open_end + 1;
    }
    stripped
}

fn find_next_tag_start(html: &str, tag: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{tag}");
    let mut offset = 0;
    while let Some(rel) = lower[offset..].find(&needle) {
        let pos = offset + rel;
        let next = lower.as_bytes().get(pos + needle.len()).copied();
        if matches!(next, Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')) {
            return Some(pos);
        }
        offset = pos + needle.len();
    }
    None
}

fn find_tag_end(html: &str, tag_start: usize) -> Option<usize> {
    html[tag_start..].find('>').map(|rel| tag_start + rel)
}

fn find_matching_close(html: &str, tag: &str, content_start: usize) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    let open_needle = format!("<{tag}");
    let close_needle = format!("</{tag}>");
    let mut depth = 1usize;
    let mut cursor = content_start;

    while cursor < html.len() {
        let next_open = lower[cursor..].find(&open_needle).map(|rel| cursor + rel);
        let next_close = lower[cursor..].find(&close_needle).map(|rel| cursor + rel);
        match (next_open, next_close) {
            (Some(open), Some(close)) if open < close => {
                let next = lower.as_bytes().get(open + open_needle.len()).copied();
                if matches!(next, Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')) {
                    depth += 1;
                }
                cursor = open + open_needle.len();
            }
            (_, Some(close)) => {
                depth -= 1;
                let end = close + close_needle.len();
                if depth == 0 {
                    return Some(end);
                }
                cursor = end;
            }
            _ => return None,
        }
    }
    None
}

fn class_attr(attrs: &str) -> Option<String> {
    attr_value(attrs, "class")
}

fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let pattern = format!("{name}=");
    let pos = lower.find(&pattern)?;
    let value_start = pos + pattern.len();
    let bytes = attrs.as_bytes();
    let quote = bytes.get(value_start).copied()?;
    if quote == b'\"' || quote == b'\'' {
        let rest = &attrs[value_start + 1..];
        let end = rest.find(char::from(quote))?;
        Some(rest[..end].to_string())
    } else {
        let rest = &attrs[value_start..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end].trim_end_matches('>').to_string())
    }
}

fn has_class(class_attr: &str, expected: &str) -> bool {
    class_attr
        .split_ascii_whitespace()
        .any(|class| class == expected)
}

#[derive(Debug)]
struct Segment {
    start: usize,
    end: usize,
}

fn line_segments(html: &str) -> Vec<Segment> {
    let mut starts = vec![0usize];
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            starts.push(i + 1);
            i += 1;
        } else if bytes[i] == b'<'
            && (lower[i..].starts_with("<br")
                || lower[i..].starts_with("</p>")
                || lower[i..].starts_with("</div>"))
        {
            if let Some(end) = lower[i..].find('>') {
                starts.push(i + end + 1);
                i += end + 1;
            } else {
                break;
            }
        } else {
            i += 1;
        }
    }

    starts.sort_unstable();
    starts.dedup();
    starts
        .iter()
        .copied()
        .enumerate()
        .map(|(idx, start)| Segment {
            start,
            end: starts.get(idx + 1).copied().unwrap_or(html.len()),
        })
        .collect()
}

fn find_lineish_start(html: &str, pos: usize) -> usize {
    let lower = html[..pos].to_ascii_lowercase();
    let mut start = 0;
    for marker in ["<br", "</p>", "</div>", "\n"] {
        if let Some(found) = lower.rfind(marker) {
            let after = if marker == "\n" {
                found + 1
            } else {
                lower[found..]
                    .find('>')
                    .map_or(found, |rel| found + rel + 1)
            };
            start = start.max(after);
        }
    }
    start
}

fn starts_with_quote_marker(line: &str) -> bool {
    line.starts_with('>') || line.starts_with("&gt;") || line.starts_with("&#62;")
}

fn trim_trailing_breaks(html: &mut String) {
    loop {
        let trimmed = html.trim_end();
        if trimmed.len() != html.len() {
            html.truncate(trimmed.len());
            continue;
        }
        let lower = html.to_ascii_lowercase();
        let suffix = ["<br>", "<br/>", "<br />", "<p></p>", "<div></div>"]
            .into_iter()
            .find(|suffix| lower.ends_with(suffix));
        if let Some(suffix) = suffix {
            html.truncate(html.len() - suffix.len());
        } else {
            break;
        }
    }
}

fn visible_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    out.replace("&gt;", ">")
        .replace("&#62;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::{strip_quoted_history, StrippedText};

    #[test]
    fn gmail_quote_removed() {
        let input = r#"<div>Fresh reply</div><div class="gmail_quote"><div>On Tue, Bob wrote:</div><blockquote>old</blockquote></div>"#;

        let stripped = strip_quoted_history(input);

        assert_eq!(
            stripped,
            StrippedText {
                html: "<div>Fresh reply</div>".to_string(),
                stripped: true,
            }
        );
    }

    #[test]
    fn blockquote_cite_removed() {
        let input = r#"<p>Answer</p><blockquote type="cite"><p>Prior message</p><blockquote type="cite">older</blockquote></blockquote>"#;

        let stripped = strip_quoted_history(input);

        assert_eq!(stripped.html, "<p>Answer</p>");
        assert!(stripped.stripped);
    }

    #[test]
    fn outlook_header_block_removed() {
        let input = "<div>Done.</div><hr><div><b>From:</b> Alice &lt;a@example.test&gt;<br><b>Sent:</b> Monday<br><b>To:</b> Bob<br><b>Subject:</b> Re: Hi</div><div>old body</div>";

        let stripped = strip_quoted_history(input);

        assert_eq!(stripped.html, "<div>Done.</div>");
        assert!(stripped.stripped);
    }

    #[test]
    fn on_date_wrote_block_removed() {
        let input = "<p>Yes.</p><div>On Tue, Alice wrote:</div><blockquote type=\"cite\"><p>old</p></blockquote>";

        let stripped = strip_quoted_history(input);

        assert_eq!(stripped.html, "<p>Yes.</p>");
        assert!(stripped.stripped);
    }

    #[test]
    fn plaintext_quote_block_removed() {
        let input = "<div>Sounds good.</div><br>&gt; Old line<br>&gt; another old line";

        let stripped = strip_quoted_history(input);

        assert_eq!(stripped.html, "<div>Sounds good.</div>");
        assert!(stripped.stripped);
    }

    #[test]
    fn ordinary_blockquote_preserved_when_not_history() {
        let input = "<p>I keep this quote:</p><blockquote><p>Ship small, learn fast.</p></blockquote><p>Agree.</p>";

        let stripped = strip_quoted_history(input);

        assert_eq!(stripped.html, input);
        assert!(!stripped.stripped);
    }

    #[test]
    fn no_quote_returns_unchanged_with_stripped_false() {
        let input = "<p>Hello Alice,</p><p>Let's meet tomorrow.</p>";

        let stripped = strip_quoted_history(input);

        assert_eq!(
            stripped,
            StrippedText {
                html: input.to_string(),
                stripped: false,
            }
        );
    }
}
