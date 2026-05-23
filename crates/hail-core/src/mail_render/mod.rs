//! Mail rendering primitives shared by API and worker code.
//!
//! This module owns the server-side defense for message bodies shown in the
//! thread pane: obvious tracking images are removed and counted, then the
//! remaining fragment is passed through `ammonia` so untrusted senders cannot
//! execute script in the SPA origin. It also exposes quote/history stripping
//! helpers for the thread-as-document view.

pub mod quote;
pub use quote::{strip_quoted_history, StrippedText};

use std::borrow::Cow;

use ammonia::{Builder, UrlRelative};
use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::TendrilSink;
use html5ever::{local_name, ns, parse_fragment, serialize, QualName};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use regex::Regex;

/// Sanitized HTML plus metadata about tracking resources removed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedHtml {
    pub html: String,
    pub blocked_trackers: Vec<BlockedTracker>,
}

/// One stripped image that looked like a tracking beacon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedTracker {
    pub src: String,
    pub reason: String,
}

/// Strip likely tracking images from an email HTML fragment, then sanitize it.
///
/// This function is intentionally route-agnostic so both API thread assembly and
/// any future worker-side render/cache job can share exactly the same policy.
pub fn sanitize_and_strip_trackers(input_html: &str) -> SanitizedHtml {
    let (stripped_html, blocked_trackers) = strip_tracking_images(input_html);
    let html = sanitizer().clean(&stripped_html).to_string();

    SanitizedHtml {
        html,
        blocked_trackers,
    }
}

fn sanitizer() -> Builder<'static> {
    let mut builder = Builder::default();
    builder
        .link_rel(Some("noopener noreferrer nofollow"))
        .set_tag_attribute_value("a", "target", "_blank")
        // We render fragments inside hail, not standalone pages. Relative URLs in
        // remote email are ambiguous and can accidentally target our origin.
        .url_relative(UrlRelative::Deny)
        // Do not add a blanket `data:` URL scheme allowance here: ammonia
        // applies scheme policy across URL attributes, so that would also make
        // `data:` links clickable. Inline data images stay blocked unless we
        // later add an explicit img-only data:image/png/jpeg/gif/webp base64
        // predicate that rejects SVG and every non-image consumer.
        .add_url_schemes(&["cid"])
        .attribute_filter(|element, attribute, value| match (element, attribute) {
            // Style attributes have a large CSS attack surface and are not in
            // ammonia's defaults. Keep that default even if upstream changes.
            (_, "style") => None,
            // Defense in depth: reject event handlers regardless of tag.
            (_, attr) if attr.to_ascii_lowercase().starts_with("on") => None,
            _ => Some(Cow::Borrowed(value)),
        });
    builder
}

fn strip_tracking_images(input_html: &str) -> (String, Vec<BlockedTracker>) {
    let dom = parse_fragment(
        RcDom::default(),
        Default::default(),
        QualName::new(None, ns!(html), local_name!("body")),
        Vec::new(),
        false,
    )
    .one(input_html);

    let mut blocked_trackers = Vec::new();
    strip_tracking_images_from(&dom.document, &mut blocked_trackers);

    let mut bytes = Vec::new();
    serialize(
        &mut bytes,
        &SerializableHandle::from(dom.document),
        SerializeOpts {
            traversal_scope: TraversalScope::ChildrenOnly(None),
            ..SerializeOpts::default()
        },
    )
    .expect("serializing parsed HTML fragment into Vec cannot fail");

    (
        String::from_utf8(bytes).expect("html5ever serializes UTF-8"),
        blocked_trackers,
    )
}

fn strip_tracking_images_from(handle: &Handle, blocked_trackers: &mut Vec<BlockedTracker>) {
    let children = handle.children.borrow().clone();
    let mut remove_indices = Vec::new();

    for (index, child) in children.iter().enumerate() {
        if let Some(blocked) = blocked_tracker_for(child) {
            blocked_trackers.push(blocked);
            remove_indices.push(index);
        } else {
            strip_tracking_images_from(child, blocked_trackers);
        }
    }

    if !remove_indices.is_empty() {
        let mut children = handle.children.borrow_mut();
        for index in remove_indices.into_iter().rev() {
            let removed = children.remove(index);
            removed.parent.set(None);
        }
    }
}

fn blocked_tracker_for(handle: &Handle) -> Option<BlockedTracker> {
    let NodeData::Element { name, attrs, .. } = &handle.data else {
        return None;
    };

    if name.local.as_ref() != "img" {
        return None;
    }

    let attrs = attrs.borrow();
    let src = attr_value(&attrs, "src").unwrap_or_default();
    let width = attr_value(&attrs, "width").and_then(parse_dimension);
    let height = attr_value(&attrs, "height").and_then(parse_dimension);
    let style = attr_value(&attrs, "style").unwrap_or_default();

    let reason = if matches!((width, height), (Some(w), Some(h)) if w <= 2 && h <= 2) {
        Some("image dimensions are 2x2 or smaller")
    } else if width == Some(0) || height == Some(0) {
        Some("image has a zero dimension")
    } else if is_hidden_style(&style) {
        Some("image is hidden by inline style")
    } else if tracking_url(&src) {
        Some("image URL looks like a tracking beacon")
    } else if is_remote_http_image(&src) {
        Some("remote image blocked by default")
    } else {
        None
    }?;

    Some(BlockedTracker {
        src,
        reason: reason.to_string(),
    })
}

fn attr_value(attrs: &[html5ever::Attribute], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|attr| attr.value.to_string())
}

fn parse_dimension(value: String) -> Option<u32> {
    static DIMENSION_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^\s*(\d+)(?:\.\d+)?(?:\s*px)?\s*$").expect("valid dimension regex")
    });

    DIMENSION_RE
        .captures(&value)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

fn is_hidden_style(style: &str) -> bool {
    let normalized: String = style
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();

    normalized.contains("display:none") || normalized.contains("visibility:hidden")
}

fn is_remote_http_image(src: &str) -> bool {
    let src = src.trim_start().to_ascii_lowercase();
    src.starts_with("http://") || src.starts_with("https://")
}

fn tracking_url(src: &str) -> bool {
    let src = src.to_ascii_lowercase();
    ["open", "pixel", "track", "beacon"]
        .iter()
        .any(|needle| src.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_script_tags() {
        let sanitized = sanitize_and_strip_trackers("<p>Hello</p><script>alert(1)</script>");

        assert!(sanitized.html.contains("<p>Hello</p>"));
        assert!(!sanitized.html.contains("script"));
        assert!(!sanitized.html.contains("alert"));
    }

    #[test]
    fn removes_event_handler_attributes() {
        let sanitized = sanitize_and_strip_trackers(r#"<img src="cid:logo" onerror="alert(1)">"#);

        assert!(sanitized.html.contains("<img"));
        assert!(!sanitized.html.contains("onerror"));
        assert!(!sanitized.html.contains("alert"));
    }

    #[test]
    fn removes_javascript_links() {
        let sanitized = sanitize_and_strip_trackers(r#"<a href="javascript:alert(1)">click</a>"#);

        assert!(sanitized.html.contains(">click</a>"));
        assert!(!sanitized.html.contains("href"));
        assert!(!sanitized.html.contains("javascript"));
    }

    #[test]
    fn strips_data_html_links() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">open</a>"#,
        );

        assert!(sanitized.html.contains(">open</a>"));
        assert!(!sanitized.html.contains("href"));
        assert!(!sanitized.html.contains("data:text/html"));
    }

    #[test]
    fn strips_data_png_images_until_img_only_predicate_exists() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<img src="data:image/png;base64,iVBORw0KGgo=" alt="inline pixel">"#,
        );

        assert!(sanitized.html.contains("<img"));
        assert!(sanitized.html.contains(r#"alt="inline pixel""#));
        assert!(!sanitized.html.contains("src"));
        assert!(!sanitized.html.contains("data:image/png"));
        assert!(sanitized.blocked_trackers.is_empty());
    }

    #[test]
    fn strips_data_svg_images() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<img src="data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9YWxlcnQoMSk+" alt="svg">"#,
        );

        assert!(sanitized.html.contains("<img"));
        assert!(sanitized.html.contains(r#"alt="svg""#));
        assert!(!sanitized.html.contains("src"));
        assert!(!sanitized.html.contains("data:image/svg"));
        assert!(!sanitized.html.contains("onload"));
    }

    #[test]
    fn strips_and_counts_tiny_tracking_pixels() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<p>Hi</p><img src="https://tracker.example/open.gif" width="1" height="1">"#,
        );

        assert!(!sanitized.html.contains("tracker.example"));
        assert_eq!(sanitized.blocked_trackers.len(), 1);
        assert_eq!(
            sanitized.blocked_trackers[0].src,
            "https://tracker.example/open.gif"
        );
        assert!(sanitized.blocked_trackers[0].reason.contains("2x2"));
    }

    #[test]
    fn strips_and_counts_hidden_tracking_images() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<img src="https://cdn.example/newsletter.png" style="display: none">"#,
        );

        assert!(!sanitized.html.contains("newsletter.png"));
        assert_eq!(sanitized.blocked_trackers.len(), 1);
        assert!(sanitized.blocked_trackers[0].reason.contains("hidden"));
    }

    #[test]
    fn strips_and_counts_remote_http_images() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<p>Logo</p><img src="https://example.com/logo.png" width="640" height="320" alt="Logo">"#,
        );

        assert!(!sanitized.html.contains(r#"<img"#));
        assert!(!sanitized.html.contains("https://example.com/logo.png"));
        assert_eq!(sanitized.blocked_trackers.len(), 1);
        assert_eq!(
            sanitized.blocked_trackers[0].src,
            "https://example.com/logo.png"
        );
        assert!(sanitized.blocked_trackers[0].reason.contains("remote image"));
    }

    #[test]
    fn preserves_cid_images() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<p>Logo</p><img src="cid:logo" width="640" height="320" alt="Logo">"#,
        );

        assert!(sanitized.html.contains(r#"<img"#));
        assert!(sanitized.html.contains(r#"src="cid:logo""#));
        assert!(sanitized.html.contains(r#"alt="Logo""#));
        assert!(sanitized.blocked_trackers.is_empty());
    }

    #[test]
    fn preserves_table_and_basic_formatting() {
        let sanitized = sanitize_and_strip_trackers(
            "<table><tbody><tr><td><strong>Total</strong></td><td><em>$5</em></td></tr></tbody></table>",
        );

        assert!(sanitized.html.contains("<table>"));
        assert!(sanitized.html.contains("<td><strong>Total</strong></td>"));
        assert!(sanitized.html.contains("<td><em>$5</em></td>"));
    }

    #[test]
    fn adds_link_rel_and_target() {
        let sanitized = sanitize_and_strip_trackers(r#"<a href="https://example.com">site</a>"#);

        assert!(sanitized.html.contains(r#"href="https://example.com""#));
        assert!(sanitized
            .html
            .contains(r#"rel="noopener noreferrer nofollow""#));
        assert!(sanitized.html.contains(r#"target="_blank""#));
    }
}
