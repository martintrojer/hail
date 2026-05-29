//! Mail rendering primitives shared by API and worker code.
//!
//! This module owns the server-side defense for message bodies shown in the
//! thread pane: obvious tracking images are removed and counted, then the
//! remaining fragment is passed through `ammonia` so untrusted senders cannot
//! execute script in the SPA origin. It also exposes quote/history stripping
//! helpers for the thread-as-document view.

pub mod quote;
pub use quote::{StrippedText, strip_quoted_history};

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use ammonia::{Builder, UrlRelative};
use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::TendrilSink;
use html5ever::{QualName, local_name, ns, parse_fragment, serialize};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use regex::Regex;

/// Sanitized HTML plus metadata about tracking resources removed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedHtml {
    pub html: String,
    pub html_with_remote_images: String,
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
    let (stripped_html, blocked_trackers) = strip_tracking_images(input_html, true);
    let (remote_image_html, _) = strip_tracking_images(input_html, false);
    let html = sanitizer().clean(&stripped_html).to_string();
    let html_with_remote_images = sanitizer().clean(&remote_image_html).to_string();

    SanitizedHtml {
        html,
        html_with_remote_images,
        blocked_trackers,
    }
}

/// Sanitize an outbound compose/draft HTML body before storing or sending.
///
/// Outbound HTML is authored by our composer but still treated as untrusted
/// request input. The allow-list is intentionally narrower than inbound mail
/// rendering: no images, tables, forms, iframes, scripts, event handlers, or
/// remote resource-loading attributes. Links may target only explicit safe
/// schemes and always receive `rel="noopener noreferrer"` and
/// `target="_blank"`.
pub fn sanitize_outgoing_html(input_html: &str) -> String {
    outgoing_sanitizer().clean(input_html).to_string()
}

/// Build an HTML reply quote around already-sanitized message HTML.
///
/// The `previous_message_html` input must come from the inbound render pipeline
/// (`strip_quoted_history` + `sanitize_and_strip_trackers`) so the blockquote
/// cannot reintroduce scripts, event handlers, or remote-image loaders. This
/// helper only escapes the human-readable attribution line and wraps the trusted
/// fragment for composer prefill.
pub fn build_reply_quote_html(
    date_label: &str,
    sender: &str,
    previous_message_html: &str,
) -> String {
    let mut html = String::with_capacity(
        date_label.len() + sender.len() + previous_message_html.len() + 42,
    );
    html.push_str("<p>On ");
    escape_text_into(date_label, &mut html);
    html.push_str(", ");
    escape_text_into(sender, &mut html);
    html.push_str(" wrote:</p><blockquote>");
    html.push_str(previous_message_html);
    html.push_str("</blockquote>");
    html
}

/// Convert a plain-text mail body into a safe HTML fragment.
///
/// The output escapes HTML metacharacters and preserves author line breaks with
/// `<br>` tags. Quote/history stripping is intentionally left to callers so the
/// same thread render pipeline can apply one policy to both native HTML and
/// text/plain fallback bodies.
pub fn plaintext_body_to_html(input_text: &str) -> String {
    let mut html = String::with_capacity(input_text.len());
    let normalized = input_text.replace("\r\n", "\n").replace('\r', "\n");

    for (idx, line) in normalized.split('\n').enumerate() {
        if idx > 0 {
            html.push_str("<br>");
        }
        escape_text_into(line, &mut html);
    }

    html
}

/// Convert a sanitized HTML fragment into plain text suitable for compact
/// previews. Tags are treated as structural whitespace and text nodes are
/// decoded by the HTML parser before being appended.
pub fn html_fragment_to_text(input_html: &str) -> String {
    let dom = parse_fragment(
        RcDom::default(),
        Default::default(),
        QualName::new(None, ns!(html), local_name!("body")),
        Vec::new(),
        false,
    )
    .one(input_html);

    let mut output = String::new();
    collect_text_from_html(&dom.document, &mut output);
    output
}

fn collect_text_from_html(handle: &Handle, output: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => output.push_str(&contents.borrow()),
        NodeData::Element { name, .. } if is_html_text_break(name.local.as_ref()) => {
            output.push(' ');
        }
        _ => {}
    }

    for child in handle.children.borrow().iter() {
        collect_text_from_html(child, output);
    }

    if let NodeData::Element { name, .. } = &handle.data
        && is_html_text_break(name.local.as_ref())
    {
        output.push(' ');
    }
}

fn is_html_text_break(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "br"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "td"
            | "th"
            | "tr"
            | "ul"
    )
}

fn escape_text_into(input: &str, output: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(ch),
        }
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
        })
        .add_allowed_classes("img", &["hail-cid-inline-image"]);
    builder
}

fn outgoing_sanitizer() -> Builder<'static> {
    let mut tag_attributes = HashMap::new();
    tag_attributes.insert("a", HashSet::from(["href", "title"]));

    let mut builder = Builder::empty();
    builder
        .tags(HashSet::from([
            "a",
            "b",
            "strong",
            "i",
            "em",
            "u",
            "s",
            "code",
            "pre",
            "blockquote",
            "p",
            "br",
            "hr",
            "h1",
            "h2",
            "h3",
            "h4",
            "ul",
            "ol",
            "li",
            "span",
            "div",
        ]))
        .tag_attributes(tag_attributes)
        .clean_content_tags(HashSet::from(["script", "style", "iframe"]))
        .generic_attributes(HashSet::from(["style"]))
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .url_relative(UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .attribute_filter(|_element, attribute, value| match attribute {
            "style" => sanitize_outgoing_style(value).map(Cow::Owned),
            attr if attr.to_ascii_lowercase().starts_with("on") => None,
            _ => Some(Cow::Borrowed(value)),
        });
    builder
}

fn sanitize_outgoing_style(value: &str) -> Option<String> {
    let mut declarations = Vec::new();

    for declaration in value.split(';') {
        let Some((property, raw_value)) = declaration.split_once(':') else {
            continue;
        };
        if !property.trim().eq_ignore_ascii_case("text-align") {
            continue;
        }

        let align = raw_value.trim().to_ascii_lowercase();
        if matches!(align.as_str(), "left" | "center" | "right") {
            declarations.push(format!("text-align:{align}"));
        }
    }

    (!declarations.is_empty()).then(|| declarations.join(";"))
}

fn strip_tracking_images(
    input_html: &str,
    block_remote_images: bool,
) -> (String, Vec<BlockedTracker>) {
    let dom = parse_fragment(
        RcDom::default(),
        Default::default(),
        QualName::new(None, ns!(html), local_name!("body")),
        Vec::new(),
        false,
    )
    .one(input_html);

    let mut blocked_trackers = Vec::new();
    strip_tracking_images_from(&dom.document, block_remote_images, &mut blocked_trackers);

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

fn strip_tracking_images_from(
    handle: &Handle,
    block_remote_images: bool,
    blocked_trackers: &mut Vec<BlockedTracker>,
) {
    let children = handle.children.borrow().clone();
    let mut remove_indices = Vec::new();

    for (index, child) in children.iter().enumerate() {
        if let Some(blocked) = blocked_tracker_for(child, block_remote_images) {
            blocked_trackers.push(blocked);
            remove_indices.push(index);
        } else {
            strip_tracking_images_from(child, block_remote_images, blocked_trackers);
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

fn blocked_tracker_for(handle: &Handle, block_remote_images: bool) -> Option<BlockedTracker> {
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
    } else if block_remote_images && is_remote_http_image(&src) {
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
    let src = normalize_url_for_scheme_detection(src);
    src.starts_with("http://") || src.starts_with("https://")
}

fn tracking_url(src: &str) -> bool {
    let src = normalize_url_for_scheme_detection(src);
    ["open", "pixel", "track", "beacon"]
        .iter()
        .any(|needle| src.contains(needle))
}

fn normalize_url_for_scheme_detection(src: &str) -> String {
    src.chars()
        .filter(|ch| !ch.is_ascii_control() && !ch.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_quote_wraps_sanitized_html_and_escapes_attribution() {
        let quote = build_reply_quote_html(
            "2026-05-25T12:30:00Z",
            "Alice <alice@example.org>",
            "<p>Hello <strong>Bob</strong></p>",
        );

        assert_eq!(
            quote,
            "<p>On 2026-05-25T12:30:00Z, Alice &lt;alice@example.org&gt; wrote:</p><blockquote><p>Hello <strong>Bob</strong></p></blockquote>"
        );
    }

    #[test]
    fn sanitize_outgoing_drops_script_content() {
        let sanitized = sanitize_outgoing_html("<p>Hello</p><script>alert(1)</script>");

        assert_eq!(sanitized, "<p>Hello</p>");
    }

    #[test]
    fn sanitize_outgoing_drops_iframe_but_keeps_surrounding_text() {
        let sanitized = sanitize_outgoing_html(
            r#"before<iframe src="https://evil.example/embed">frame</iframe>after"#,
        );

        assert_eq!(sanitized, "beforeafter");
    }

    #[test]
    fn sanitize_outgoing_drops_event_handlers() {
        let sanitized = sanitize_outgoing_html(r#"<p onclick="alert(1)">Click me</p>"#);

        assert_eq!(sanitized, "<p>Click me</p>");
    }

    #[test]
    fn sanitize_outgoing_strips_javascript_hrefs() {
        let sanitized = sanitize_outgoing_html(r#"<a href="javascript:alert(1)">click</a>"#);

        assert!(sanitized.starts_with("<a "));
        assert!(sanitized.ends_with(">click</a>"));
        assert!(sanitized.contains(r#"rel="noopener noreferrer""#));
        assert!(sanitized.contains(r#"target="_blank""#));
        assert!(!sanitized.to_ascii_lowercase().contains("javascript"));
    }

    #[test]
    fn sanitize_outgoing_blocks_high_risk_bypass_shapes() {
        struct Case {
            name: &'static str,
            input: &'static str,
            must_contain: &'static [&'static str],
        }

        let cases = [
            Case {
                name: "svg_onload",
                input: r#"<svg onload=alert(1)><circle></circle></svg>"#,
                must_contain: &[],
            },
            Case {
                name: "data_svg_image",
                input: r#"<img src="data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9YWxlcnQoMSk+" />"#,
                must_contain: &[],
            },
            Case {
                name: "data_html_link",
                input: r#"<a href="data:text/html,<script>alert(1)</script>">x</a>"#,
                must_contain: &[">x</a>"],
            },
            Case {
                name: "mixed_case_javascript_link",
                input: r#"<a href="JaVaScRiPt:alert(1)">x</a>"#,
                must_contain: &[">x</a>"],
            },
            Case {
                name: "entity_encoded_javascript_link",
                input: r#"<a href="javascript&#58;alert(1)">x</a>"#,
                must_contain: &[">x</a>"],
            },
            Case {
                name: "mixed_case_script_tag",
                input: r#"<ScRiPt>alert(1)</ScRiPt>"#,
                must_contain: &[],
            },
            Case {
                name: "nested_script_preserves_surrounding_text",
                input: r#"<p>safe<script>alert(1)</script>after</p>"#,
                must_contain: &["safe", "after"],
            },
            Case {
                name: "leading_whitespace_javascript_link",
                input: r#"<a href=" javascript:alert(1)">x</a>"#,
                must_contain: &[">x</a>"],
            },
            Case {
                name: "malformed_event_attribute_preserves_bold",
                input: r#"<p onclick=foo() x><b>hi</b>"#,
                must_contain: &["<b>hi</b>"],
            },
            Case {
                name: "iframe_srcdoc",
                input: r#"<iframe srcdoc="<script>alert(1)</script>">frame</iframe>"#,
                must_contain: &[],
            },
            Case {
                name: "math_nested_script",
                input: r#"<math><mtext><script>alert(1)</script></mtext></math>"#,
                must_contain: &[],
            },
            Case {
                name: "details_ontoggle",
                input: r#"<details ontoggle="alert(1)">x</details>"#,
                must_contain: &["x"],
            },
            Case {
                name: "vbscript_link",
                input: r#"<a href="vbscript:alert(1)">x</a>"#,
                must_contain: &[">x</a>"],
            },
        ];

        for case in cases {
            let output = sanitize_outgoing_html(case.input);
            let lower = output.to_ascii_lowercase();

            assert!(
                !lower.contains("script"),
                "{} leaked script token in {output:?}",
                case.name
            );
            assert!(
                !lower.contains("javascript:"),
                "{} leaked javascript: URL in {output:?}",
                case.name
            );
            assert!(
                !output.contains("data:"),
                "{} leaked data: URL in {output:?}",
                case.name
            );
            assert!(
                !output.contains("vbscript:"),
                "{} leaked vbscript: URL in {output:?}",
                case.name
            );
            assert!(
                !output.contains("onload"),
                "{} leaked onload attribute in {output:?}",
                case.name
            );
            assert!(
                !output.contains("onclick"),
                "{} leaked onclick attribute in {output:?}",
                case.name
            );
            assert!(
                !output.contains("ontoggle"),
                "{} leaked ontoggle attribute in {output:?}",
                case.name
            );
            assert!(
                !lower.contains("alert"),
                "{} leaked alert payload in {output:?}",
                case.name
            );

            for expected in case.must_contain {
                assert!(
                    output.contains(expected),
                    "{} lost expected safe fragment {expected:?} in {output:?}",
                    case.name
                );
            }
        }
    }

    #[test]
    fn sanitize_outgoing_keeps_compose_formatting_tags() {
        let sanitized = sanitize_outgoing_html(
            r#"<blockquote><ul><li><code>let x = 1;</code></li></ul><pre>code block</pre></blockquote>"#,
        );

        assert_eq!(
            sanitized,
            "<blockquote><ul><li><code>let x = 1;</code></li></ul><pre>code block</pre></blockquote>"
        );
    }

    #[test]
    fn sanitize_outgoing_preserves_real_world_compose_structure() {
        let input = r#"<p>Hi team,</p><ul><li><p>Review the <a href="https://example.org/doc">launch notes</a></p></li><li><p>Run <code>cargo test</code></p></li></ul><blockquote><p>Alice wrote:</p><p>Ship it.</p></blockquote>"#;
        let sanitized = sanitize_outgoing_html(input);

        assert!(sanitized.contains("<p>Hi team,</p>"));
        assert!(sanitized.contains("<ul><li><p>Review the <a href=\"https://example.org/doc\""));
        assert!(sanitized.contains("rel=\"noopener noreferrer\""));
        assert!(sanitized.contains("target=\"_blank\""));
        assert!(sanitized.contains("launch notes</a></p></li><li><p>Run <code>cargo test</code></p></li></ul>"));
        assert!(sanitized.contains("<blockquote><p>Alice wrote:</p><p>Ship it.</p></blockquote>"));
    }

    #[test]
    fn sanitize_outgoing_leaves_text_untouched() {
        let sanitized = sanitize_outgoing_html("Hello Alice & Bob < Carol");

        assert_eq!(sanitized, "Hello Alice &amp; Bob &lt; Carol");
    }

    #[test]
    fn sanitize_outgoing_normalizes_link_rel_and_target() {
        let sanitized = sanitize_outgoing_html(
            r#"<a href="https://example.com/path?q=1&x=2" rel="opener" target="_self">site</a>"#,
        );

        assert!(sanitized.starts_with(r#"<a href="https://example.com/path?q=1&amp;x=2""#));
        assert!(sanitized.ends_with(">site</a>"));
        assert!(sanitized.contains(r#"rel="noopener noreferrer""#));
        assert!(sanitized.contains(r#"target="_blank""#));
    }

    #[test]
    fn sanitize_outgoing_strips_images_and_remote_loaders() {
        let sanitized = sanitize_outgoing_html(
            r#"<p>Logo</p><img src="https://example.com/logo.png" alt="logo"><form action="https://example.com"><input name="x"></form>"#,
        );

        assert_eq!(sanitized, "<p>Logo</p>");
        assert!(!sanitized.contains("https://example.com/logo.png"));
        assert!(!sanitized.contains("<form"));
    }

    #[test]
    fn sanitize_outgoing_allows_only_safe_text_align_style() {
        let sanitized = sanitize_outgoing_html(
            r#"<p style="color:red; text-align: center; background:url(https://evil.example/x)">Centered</p><div style="text-align:justify">No style</div>"#,
        );

        assert_eq!(
            sanitized,
            r#"<p style="text-align:center">Centered</p><div>No style</div>"#
        );
    }

    #[test]
    fn plaintext_body_is_escaped_and_preserves_line_breaks() {
        let html = plaintext_body_to_html("Hello <Alice> & Bob\r\nSecond line\n\nFinal > line");

        assert_eq!(
            html,
            "Hello &lt;Alice&gt; &amp; Bob<br>Second line<br><br>Final &gt; line"
        );
    }

    #[test]
    fn plaintext_body_can_feed_quote_stripper() {
        let html = plaintext_body_to_html("Fresh reply\n\n> old quoted text");
        let stripped = strip_quoted_history(&html);

        assert_eq!(stripped.html, "Fresh reply");
        assert!(stripped.stripped);
    }

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
    fn strips_mixed_case_javascript_and_data_links() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<a href="JaVaScRiPt:alert(1)">script</a><a href="DaTa:text/html,hello">data</a>"#,
        );

        assert!(sanitized.html.contains(">script</a>"));
        assert!(sanitized.html.contains(">data</a>"));
        assert!(!sanitized.html.contains("href"));
        assert!(!sanitized.html.to_ascii_lowercase().contains("javascript"));
        assert!(
            !sanitized
                .html
                .to_ascii_lowercase()
                .contains("data:text/html")
        );
    }

    #[test]
    fn strips_control_character_javascript_links() {
        let sanitized = sanitize_and_strip_trackers(
            "<a href=\"java\nscript:alert(1)\">newline</a><a href=\"jav&#x09;ascript:alert(1)\">tab</a>",
        );

        assert!(sanitized.html.contains(">newline</a>"));
        assert!(sanitized.html.contains(">tab</a>"));
        assert!(!sanitized.html.contains("href"));
        assert!(!sanitized.html.to_ascii_lowercase().contains("script:alert"));
    }

    #[test]
    fn strips_data_svg_with_control_character_scheme() {
        let sanitized = sanitize_and_strip_trackers(
            "<img src=\"da\nta:image/svg+xml,<svg onload=alert(1)>\" alt=\"svg\">",
        );

        assert!(sanitized.html.contains("<img"));
        assert!(sanitized.html.contains(r#"alt="svg""#));
        assert!(!sanitized.html.contains("src"));
        assert!(
            !sanitized
                .html
                .to_ascii_lowercase()
                .contains("data:image/svg")
        );
        assert!(!sanitized.html.to_ascii_lowercase().contains("onload"));
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
        assert!(
            sanitized.blocked_trackers[0]
                .reason
                .contains("remote image")
        );
    }

    #[test]
    fn strips_remote_http_images_with_mixed_case_and_control_chars() {
        let sanitized = sanitize_and_strip_trackers(
            "<img src=\" HtTpS://cdn.example/logo.png?utm=1&open=abc\" alt=\"logo\"><img src=\"h\nttps://cdn.example/banner.jpg?x=1\" alt=\"banner\">",
        );

        assert!(!sanitized.html.contains("<img"));
        assert!(!sanitized.html.contains("cdn.example"));
        assert_eq!(sanitized.blocked_trackers.len(), 2);
        assert!(
            sanitized.blocked_trackers[0]
                .reason
                .contains("tracking beacon")
        );
        assert!(
            sanitized.blocked_trackers[1]
                .reason
                .contains("remote image")
        );
    }

    #[test]
    fn strips_remote_http_image_query_tracking_variants() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<img src="https://images.example/newsletter.png?utm_source=list&OpenId=abc" width="640" height="320" alt="newsletter">"#,
        );

        assert!(!sanitized.html.contains("<img"));
        assert_eq!(sanitized.blocked_trackers.len(), 1);
        assert_eq!(
            sanitized.blocked_trackers[0].src,
            "https://images.example/newsletter.png?utm_source=list&OpenId=abc"
        );
        assert!(
            sanitized.blocked_trackers[0]
                .reason
                .contains("tracking beacon")
        );
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
    fn cid_image_with_query_is_preserved() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<img src="CID:logo.123@example?part=1&name=logo.png" width="640" height="320" alt="Logo">"#,
        );

        assert!(sanitized.html.contains("<img"));
        assert!(
            sanitized
                .html
                .contains(r#"src="CID:logo.123@example?part=1&amp;name=logo.png""#)
        );
        assert!(sanitized.blocked_trackers.is_empty());
    }

    #[test]
    fn strips_picture_source_and_srcset_rather_than_keep_remote_candidates() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<picture><source srcset="https://cdn.example/hero.webp 1x, cid:hero 2x"><img src="cid:fallback" srcset="https://cdn.example/fallback.png 1x" alt="Hero"></picture>"#,
        );

        assert!(!sanitized.html.contains("<picture"));
        assert!(!sanitized.html.contains("<source"));
        assert!(sanitized.html.contains("<img"));
        assert!(sanitized.html.contains(r#"src="cid:fallback""#));
        assert!(!sanitized.html.contains("srcset"));
        assert!(!sanitized.html.contains("cdn.example"));
    }

    #[test]
    fn strips_svg_tags_and_data_svg_references() {
        let sanitized = sanitize_and_strip_trackers(
            r#"<svg><a href="javascript:alert(1)"><circle onload="alert(2)"></circle></a></svg><img src="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=" alt="svg">"#,
        );

        assert!(sanitized.html.contains(r#"alt="svg""#));
        assert!(!sanitized.html.contains("<svg"));
        assert!(!sanitized.html.contains("<circle"));
        assert!(!sanitized.html.contains("src="));
        assert!(!sanitized.html.to_ascii_lowercase().contains("javascript"));
        assert!(
            !sanitized
                .html
                .to_ascii_lowercase()
                .contains("data:image/svg")
        );
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
        assert!(
            sanitized
                .html
                .contains(r#"rel="noopener noreferrer nofollow""#)
        );
        assert!(sanitized.html.contains(r#"target="_blank""#));
    }

    mod fixture_corpus {
        use hail_test::mail_fixture;
        use mail_parser::MessageParser;

        use super::{plaintext_body_to_html, sanitize_and_strip_trackers, strip_quoted_history};

        #[derive(Debug)]
        struct RenderedFixture {
            html: String,
            blocked_tracker_srcs: Vec<String>,
        }

        fn render_fixture(name: &str) -> RenderedFixture {
            let fixture = mail_fixture(name).unwrap_or_else(|| panic!("{name} fixture exists"));
            let message = MessageParser::default()
                .parse(fixture.bytes())
                .unwrap_or_else(|| panic!("{name} parses as RFC822"));
            let body_html = if message.html_body_count() > 0 {
                message
                    .body_html(0)
                    .unwrap_or_else(|| panic!("{name} has readable html body"))
                    .into_owned()
            } else {
                let text = message
                    .body_text(0)
                    .unwrap_or_else(|| panic!("{name} has readable text body"));
                plaintext_body_to_html(&text)
            };

            let stripped = strip_quoted_history(&body_html);
            let sanitized = sanitize_and_strip_trackers(&stripped.html);

            RenderedFixture {
                html: sanitized.html,
                blocked_tracker_srcs: sanitized
                    .blocked_trackers
                    .into_iter()
                    .map(|tracker| tracker.src)
                    .collect(),
            }
        }

        #[test]
        fn newsletter_tracking_pixel_is_blocked() {
            let rendered = render_fixture("newsletter-tracking-pixel.eml");

            assert!(rendered.html.contains("Northwind Weekly"));
            assert!(rendered.html.contains("Read the full issue"));
            assert!(!rendered.html.contains("track.northwind.example"));
            assert!(!rendered.html.contains("open.gif"));
            assert_eq!(
                rendered.blocked_tracker_srcs,
                [
                    "https://track.northwind.example/open.gif?recipient=alex%40hail.test&campaign=2025-05-21"
                ]
            );
        }

        #[test]
        fn malicious_html_fixture_is_sanitized() {
            let rendered = render_fixture("malicious-html.eml");
            let lower_html = rendered.html.to_ascii_lowercase();

            assert!(rendered.html.contains("Mailbox verification required"));
            assert!(rendered.html.contains("Verify mailbox"));
            assert!(!lower_html.contains("<script"));
            assert!(!lower_html.contains("__stolen"));
            assert!(!lower_html.contains("onerror"));
            assert!(!lower_html.contains("javascript:"));
            assert!(!lower_html.contains("background-image"));
            assert!(!rendered.html.contains("phish.example/pixel/open.png"));
            assert_eq!(
                rendered.blocked_tracker_srcs,
                ["https://phish.example/pixel/open.png?id=alex"]
            );
        }

        #[test]
        fn quoted_gmail_fixture_strips_reply_history() {
            let rendered = render_fixture("quoted-gmail.eml");

            assert!(rendered.html.contains("Looks good to me"));
            assert!(rendered.html.contains("Priya"));
            assert!(!rendered.html.contains("gmail_quote"));
            assert!(
                !rendered
                    .html
                    .contains("Can you sanity-check the launch checklist")
            );
            assert!(!rendered.html.contains("Approve a sender"));
        }

        #[test]
        fn quoted_outlook_fixture_strips_reply_history() {
            let rendered = render_fixture("quoted-outlook.eml");

            assert!(rendered.html.contains("Approved. Please keep the invoice"));
            assert!(rendered.html.contains("Sam"));
            assert!(!rendered.html.contains("Alex Rivera"));
            assert!(!rendered.html.contains("Budget approval</p>"));
            assert!(!rendered.html.contains("prototype documentation shelf"));
        }

        #[test]
        fn plaintext_simple_fixture_renders_body_safely() {
            let rendered = render_fixture("personal-simple.eml");

            assert!(rendered.html.contains("Hey Alex"));
            assert!(
                rendered
                    .html
                    .contains("Are you still free for dinner on Thursday?")
            );
            assert!(rendered.html.contains("Bring the photos from the hike"));
            assert!(rendered.html.contains("<br>"));
            assert!(!rendered.html.contains("<script"));
            assert!(rendered.blocked_tracker_srcs.is_empty());
        }

        #[test]
        fn receipt_fixture_preserves_table_and_basic_formatting_safely() {
            let rendered = render_fixture("receipt-papertrail.eml");

            assert!(rendered.html.contains("Receipt PT-1042"));
            assert!(rendered.html.contains("<table>"));
            assert!(rendered.html.contains("<th>Item</th>"));
            assert!(rendered.html.contains("The Rust Programming Language"));
            assert!(rendered.html.contains("<strong>Total</strong>"));
            assert!(rendered.html.contains("<strong>$55.08</strong>"));
            assert!(rendered.html.contains("Paid with Visa ending in 4242"));
            assert!(!rendered.html.contains("<script"));
            assert!(rendered.blocked_tracker_srcs.is_empty());
        }
    }
}
