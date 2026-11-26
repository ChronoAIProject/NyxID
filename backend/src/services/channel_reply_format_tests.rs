//! Wire fixtures and a small, independent reader for the documented platform syntax.
//! Discord: https://support.discord.com/hc/en-us/articles/210298617
//! Escape, code and link precedence: discord/simple-markdown's defaultRules.
//! Slack: https://docs.slack.dev/messaging/formatting-message-text/
use super::*;
use std::sync::LazyLock;

#[derive(Default, Debug)]
pub(crate) struct Display {
    pub(crate) text: String,
    pub(crate) urls: Vec<String>,
}

// This reader deliberately consumes the wire string, without planner entities,
// linkify, or CommonMark. It covers the syntax exercised by the fixtures below.
pub(crate) fn discord_read(mut wire: &str) -> Display {
    static BARE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"^https?://[^\s<]+[^<.,:;"')\]\s]"#).unwrap());
    let mut result = Display::default();
    while !wire.is_empty() {
        if let Some(rest) = wire.strip_prefix('\\') {
            let ch = rest.chars().next().unwrap_or(' ');
            if !ch.is_ascii_alphanumeric() && !ch.is_whitespace() {
                result.text.push(ch);
                wire = &rest[ch.len_utf8()..];
                continue;
            }
        }
        if wire.starts_with('`') {
            let ticks = wire.bytes().take_while(|b| *b == b'`').count();
            let fence = &wire[..ticks];
            if let Some(end) = wire[ticks..].find(fence) {
                let mut body = &wire[ticks..ticks + end];
                if ticks >= 3 && body.starts_with('\n') && body.ends_with('\n') {
                    body = &body[1..body.len() - 1];
                } else if body.starts_with(" `") || body.ends_with("` ") {
                    body = body.strip_prefix(' ').unwrap_or(body);
                    body = body.strip_suffix(' ').unwrap_or(body);
                }
                result.text.push_str(body);
                wire = &wire[ticks + end + ticks..];
                continue;
            }
        }
        if let Some(rest) = wire.strip_prefix('<')
            && let Some(end) = rest.find('>')
            && rest[..end].contains(":/")
        {
            result.text.push_str(&rest[..end]);
            result.urls.push(rest[..end].to_owned());
            wire = &rest[end + 1..];
            continue;
        }
        if let Some(rest) = wire.strip_prefix('[')
            && let Some(mid) = rest.find("](")
            && let Some(end) = rest[mid + 2..].find(')')
        {
            let label = discord_read(&rest[..mid]);
            result.text.push_str(&label.text);
            result.urls.push(rest[mid + 2..mid + 2 + end].to_owned());
            wire = &rest[mid + 3 + end..];
            continue;
        }
        if let Some(link) = BARE.find(wire) {
            result.text.push_str(link.as_str());
            result.urls.push(link.as_str().to_owned());
            wire = &wire[link.end()..];
            continue;
        }
        let mut style = None;
        for delimiter in ["***", "**", "__", "~~", "||", "*", "_"] {
            if let Some(rest) = wire.strip_prefix(delimiter)
                && !rest.starts_with(char::is_whitespace)
            {
                let mut from = 0;
                while let Some(end) = rest[from..].find(delimiter) {
                    let end = from + end;
                    if end > 0 && !rest[..end].ends_with(['\\', ' ', '\n']) {
                        style = Some((delimiter, end));
                        break;
                    }
                    from = end + delimiter.len();
                }
            }
            if style.is_some() {
                break;
            }
        }
        if let Some((delimiter, end)) = style {
            let inside = discord_read(&wire[delimiter.len()..delimiter.len() + end]);
            result.text.push_str(&inside.text);
            result.urls.extend(inside.urls);
            wire = &wire[2 * delimiter.len() + end..];
            continue;
        }
        if result.text.ends_with('\n') || result.text.is_empty() {
            for prefix in ["-# ", "### ", "## ", "# ", ">>> ", "> ", "- ", "* "] {
                if let Some(rest) = wire.strip_prefix(prefix) {
                    wire = rest;
                    break;
                }
            }
        }
        let ch = wire.chars().next().unwrap();
        result.text.push(ch);
        wire = &wire[ch.len_utf8()..];
    }
    result
}

fn wire(source: &str, platform: &str) -> String {
    let parts = assistant_parts(source, platform, "https://nyxid.test/thread");
    assert_eq!(parts.len(), 1, "{source}");
    parts[0].text.clone().unwrap()
}

#[test]
fn channel_format_incident_strings_are_exact_on_each_platform() {
    for source in [
        "… November 16 (Singapore time).",
        "Thanks for joining the ChronoAI launch event! … once it's ready.",
        "… is now available: https://www.youtube.com/watch?v=Vre-ZFYv9as",
    ] {
        for platform in ["discord", "whatsapp", "slack"] {
            assert_eq!(wire(source, platform), source, "{platform}");
        }
        assert_eq!(discord_read(&wire(source, "discord")).text, source);
    }
    assert_eq!(
        discord_read(&wire(
            "… is now available: https://www.youtube.com/watch?v=Vre-ZFYv9as",
            "discord"
        ))
        .urls,
        ["https://www.youtube.com/watch?v=Vre-ZFYv9as"]
    );
}

#[test]
fn proactive_posts_preserve_gateway_incident_links() {
    let source = "([chrono-ai.fun](https://chrono-ai.fun/zh/?utm_source=openai))\nhttps://booking.example/test-link?a=b#share=_yNR5\nLiteral _ * [ ]";
    for platform in ["lark", "feishu"] {
        assert_eq!(wire(source, platform), source);
    }
    let display = discord_read(&wire(source, "discord"));
    assert_eq!(
        display.urls,
        [
            "https://chrono-ai.fun/zh/?utm_source=openai",
            "https://booking.example/test-link?a=b#share=_yNR5",
        ]
    );
    assert_eq!(
        display.text,
        "(chrono-ai.fun)\nhttps://booking.example/test-link?a=b#share=_yNR5\nLiteral _ * [ ]"
    );
}

#[test]
fn channel_format_exact_literals_styles_and_code() {
    let fixtures = [
        (
            "snake_case and 2 * 3 * 4; . - (x)! # + {}",
            "snake_case and 2 * 3 * 4; . - (x)! # + {}",
            "snake_case and 2 * 3 * 4; . - (x)! # + {}",
            "snake_case and 2 * 3 * 4; . - (x)! # + {}",
        ),
        (
            r"\*stars\* \_under\_ \~\~strike\~\~ \|\|secret\|\|",
            r"\*stars\* \_under\_ \~\~strike\~\~ \|\|secret\|\|",
            "`*`stars`*` `_`under`_` `~~`strike`~~` ||secret||",
            "*stars* _under_ ~~strike~~ ||secret||",
        ),
        (
            "*em* **bold** ~~strike~~ ***both***",
            "*em* **bold** ~~strike~~ ***both***",
            "_em_ *bold* ~strike~ *_both_*",
            "em bold strike both",
        ),
        (r"\- item", r"\- item", "`-` item", "- item"),
        (r"\# heading", r"\# heading", "# heading", "# heading"),
        (r"\> quote", r"\> quote", "`>` quote", "> quote"),
        (r"1\. item", r"1\. item", "1`.` item", "1. item"),
        (
            r"x > y and x - y",
            "x > y and x - y",
            "x > y and x - y",
            "x > y and x - y",
        ),
        (
            "`* _ ~ | https://example.test/a_b-x?q=1#f!`",
            "`* _ ~ | https://example.test/a_b-x?q=1#f!`",
            "`* _ ~ | https://example.test/a_b-x?q=1#f!`",
            "* _ ~ | https://example.test/a_b-x?q=1#f!",
        ),
        (
            "```text\n* _ ~ | https://example.test/a_b-x?q=1#f!\n```",
            "```\n* _ ~ | https://example.test/a_b-x?q=1#f!\n```",
            "```* _ ~ | https://example.test/a_b-x?q=1#f!```",
            "* _ ~ | https://example.test/a_b-x?q=1#f!",
        ),
        (r"\`literal\`", r"\`literal\`", "`literal`", "`literal`"),
        (r"a \\! b", r"a \\! b", r"a \! b", r"a \! b"),
    ];
    for (source, discord, whatsapp, display) in fixtures {
        assert_eq!(wire(source, "discord"), discord, "Discord input {source}");
        assert_eq!(
            wire(source, "whatsapp"),
            whatsapp,
            "WhatsApp input {source}"
        );
        assert_eq!(
            discord_read(discord).text,
            display,
            "Discord input {source}"
        );
    }
}

#[test]
fn channel_format_url_boundaries_keep_original_bytes() {
    for target in [
        "https://example.test/a-b_c.d~e?q=x_y-z&n=1#frag!end",
        "https://example.test/a_(b)?x=1&y=2#f!end",
        "https://example.test/a?x=&copy;&y=2#f",
    ] {
        let bare = format!("See {target}., next ({target}).");
        let quoted = if target.contains('(') {
            format!("<{target}>")
        } else {
            target.into()
        };
        let expected = format!("See {quoted}., next ({quoted}).");
        assert_eq!(wire(&bare, "discord"), expected);
        let read = discord_read(&expected);
        assert_eq!(read.text, bare);
        assert_eq!(read.urls, [target, target]);
        assert_eq!(wire(&bare, "whatsapp"), bare);
        let labelled = format!("[read]({target})!");
        let expected = if target.contains('(') {
            format!("read (<{target}>)!")
        } else {
            format!("[read]({target})!")
        };
        assert_eq!(wire(&labelled, "discord"), expected);
        assert_eq!(discord_read(&expected).urls, [target]);
        assert_eq!(wire(&labelled, "whatsapp"), format!("read ({target})!"));
    }
}

#[test]
fn channel_format_boundaries_code_overlong_urls_and_cap() {
    for platform in ["discord", "whatsapp"] {
        let limit = TextLimit::for_platform(platform);
        let url = "https://example.test/a-b_c.d~e?q=1&x=2#f!";
        let source = format!(
            "{} {url}\n\n```\n{}\n```",
            "a".repeat(limit.budget() - 20),
            "x * _ | ~ 🦉\n".repeat(1000)
        );
        let parts = assistant_parts(&source, platform, "https://nyxid.test/thread");
        assert!(parts.len() > 2);
        assert!(
            parts
                .iter()
                .all(|p| limit.measure(p.text.as_deref().unwrap()) <= limit.budget())
        );
        assert_eq!(
            parts
                .iter()
                .filter(|p| p.text.as_deref().unwrap().contains(url))
                .count(),
            1
        );
        if platform == "discord" {
            let display: String = parts
                .iter()
                .map(|p| discord_read(p.text.as_deref().unwrap()).text)
                .collect();
            assert_eq!(
                display,
                format!(
                    "{} {url}\n\n{}",
                    "a".repeat(limit.budget() - 20),
                    "x * _ | ~ 🦉\n".repeat(1000).trim_end_matches('\n')
                )
            );
        }
        let long_url = format!("https://example.test/{}", "a_b-~x!".repeat(limit.budget()));
        let parts = assistant_parts(&long_url, platform, "https://nyxid.test/thread");
        let joined: String = parts.iter().filter_map(|p| p.text.as_deref()).collect();
        assert_eq!(joined, long_url);
        let parts = assistant_parts(
            &"a".repeat(limit.budget() * 65),
            platform,
            "https://nyxid.test/thread",
        );
        assert_eq!(parts.len(), 64);
        assert_eq!(
            parts.last().unwrap().metadata.as_ref().unwrap()["text_overflow"],
            true
        );
    }
}

#[test]
fn channel_format_whatsapp_unrepresentable_code_preserves_characters() {
    for (source, expected) in [
        ("`` `literal` ``", "`literal`"),
        ("````\na ``` b\n````", "a ``` b"),
        (r"\`literal\`", "`literal`"),
    ] {
        assert_eq!(wire(source, "whatsapp"), expected);
    }
}

fn slack_read(wire: &str, plain: bool) -> Display {
    if plain {
        return Display {
            text: wire.into(),
            urls: vec![],
        };
    }
    let mut output = Display::default();
    let mut rest = wire;
    while !rest.is_empty() {
        if let Some(link) = rest.strip_prefix('<')
            && let Some(end) = link.find('>')
            && let Some((target, label)) = link[..end].split_once('|')
        {
            output.text.push_str(label);
            output.urls.push(target.replace("&amp;", "&"));
            rest = &link[end + 1..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        if matches!(ch, '*' | '_' | '~')
            && let Some(end) = rest[1..].find(ch)
            && end > 0
        {
            output.text.push_str(&rest[1..end + 1]);
            rest = &rest[end + 2..];
            continue;
        }
        output.text.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    output.text = output
        .text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    // Bare URL fixture boundaries are spaces, matching Slack's documented examples.
    output.urls.extend(
        output
            .text
            .split_whitespace()
            .filter(|s| s.starts_with("https://"))
            .map(str::to_owned),
    );
    output
}

#[test]
fn channel_format_slack_payloads_and_fingerprints_remain_frozen() {
    for (source, expected, digest, display, plain) in [
        (
            "… November 16 (Singapore time).",
            "… November 16 (Singapore time).",
            "sha256:e437b5c439c628a4937ec8c00c2bb0ff27eaffc40a66f3ad969454ead040b7c6",
            "… November 16 (Singapore time).",
            false,
        ),
        (
            "Thanks for joining the ChronoAI launch event! … once it's ready.",
            "Thanks for joining the ChronoAI launch event! … once it's ready.",
            "sha256:a868aa72529074b14a97d3bfb85834d7882c23fb2cfc4f511735df628961c747",
            "Thanks for joining the ChronoAI launch event! … once it's ready.",
            false,
        ),
        (
            "… is now available: https://www.youtube.com/watch?v=Vre-ZFYv9as",
            "… is now available: https://www.youtube.com/watch?v=Vre-ZFYv9as",
            "sha256:f8fd7eea03a9054b14210f647753adb43f2369e34e079c91ad65f211f2b96f2c",
            "… is now available: https://www.youtube.com/watch?v=Vre-ZFYv9as",
            false,
        ),
        (
            "See https://example.test/a_b-x?q=1&y=2#f!",
            "See https://example.test/a_b-x?q=1&amp;y=2#f!",
            "sha256:843dc3121f36ad098685166cdaa21704e5fd8bef88b6b6062781041b20f81b51",
            "See https://example.test/a_b-x?q=1&y=2#f!",
            false,
        ),
        (
            "**bold** [read](https://example.test/a?x=1&y=2).",
            "*bold* <https://example.test/a?x=1&y=2|read>.",
            "sha256:46475aace6b62838dbd045fdf6482100639f997fda64d1af11248b4d7cd5ece8",
            "bold read.",
            false,
        ),
        (
            r"\*stars\*",
            "*stars*",
            "sha256:708821efb2567d82dca428f3b75f30cdc97388510dad4ed5c1839d23a3ccb314",
            "stars",
            false,
        ),
        (
            "`` `literal` <value> & more `` and [guide](https://example.test/guide?x=1&y=2)",
            "`literal` <value> & more and guide (https://example.test/guide?x=1&y=2)",
            "sha256:4169e7293225538e531ee8b1c37c6d320e520efe07f0f44718ccdaa32d22d6bf",
            "`literal` <value> & more and guide (https://example.test/guide?x=1&y=2)",
            true,
        ),
    ] {
        let parts = assistant_parts(source, "slack", "https://nyxid.test/thread");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text.as_deref(), Some(expected));
        assert_eq!(
            parts[0].metadata,
            plain.then(|| serde_json::json!({"mrkdwn":false}))
        );
        assert_eq!(fingerprint(&parts), digest);
        assert_eq!(slack_read(expected, plain).text, display);
        assert_eq!(
            fingerprint(
                &recorded_assistant_parts(source, "slack", "https://nyxid.test/thread", None)
                    .unwrap()
            ),
            digest
        );
    }
    assert_eq!(
        slack_read(
            &wire("See https://example.test/a-b?q=1&y=2#f!", "slack"),
            false
        )
        .urls,
        ["https://example.test/a-b?q=1&y=2#f!"]
    );
    assert_eq!(
        slack_read(
            &wire("[read](https://example.test/a?x=1&y=2)", "slack"),
            false
        )
        .urls,
        ["https://example.test/a?x=1&y=2"]
    );
}

#[test]
fn channel_format_delimiter_adjacency_and_styled_urls_keep_display_and_target() {
    for (source, expected, display) in [
        (r"\***bold**", r"\***bold**", "*bold"),
        (r"**bold**\*", r"**bold**\*", "bold*"),
        (r"foo\_\_bar\_\_baz", r"foo\_\_bar\_\_baz", "foo__bar__baz"),
        (r"\*\* spaced \*\*", r"\*\* spaced \*\*", "** spaced **"),
        (
            "**https://example.test/a_b?x=1&y=2**",
            "**<https://example.test/a_b?x=1&y=2>**",
            "https://example.test/a_b?x=1&y=2",
        ),
        (
            "**bold *inner* end**",
            "**bold *inner* end**",
            "bold inner end",
        ),
    ] {
        let actual = wire(source, "discord");
        assert_eq!(actual, expected, "{source}");
        let read = discord_read(&actual);
        assert_eq!(read.text, display, "{source}");
        if source.contains("https://") {
            assert_eq!(read.urls, [display]);
        }
    }
}

#[test]
fn channel_format_recorded_v1_is_frozen_and_v2_is_distinct_even_without_syntax() {
    for platform in ["discord", "whatsapp"] {
        for (source, expected) in [
            (
                "… November 16 (Singapore time).",
                r"… November 16 \(Singapore time\)\.",
            ),
            (
                "… is now available: https://www.youtube.com/watch?v=Vre-ZFYv9as",
                r"… is now available: https://www\.youtube\.com/watch?v=Vre\-ZFYv9as",
            ),
        ] {
            for version in [None, Some(1)] {
                let old = recorded_assistant_parts(source, platform, "", version).unwrap();
                assert_eq!(old.len(), 1);
                assert_eq!(old[0].text.as_deref(), Some(expected));
                assert!(old[0].metadata.is_none());
                assert_ne!(
                    fingerprint(&old),
                    fingerprint(&assistant_parts(source, platform, ""))
                );
            }
        }
        let old = recorded_assistant_parts("hello", platform, "", None).unwrap();
        let new = assistant_parts("hello", platform, "");
        assert_eq!(old[0].text, new[0].text);
        assert_ne!(fingerprint(&old), fingerprint(&new));
    }
}

#[test]
fn gateway_commonmark_multipart_preserves_routing_and_renderer() {
    use crate::services::channel_platform::{MaterializedAttachment, MediaKind};

    let mut reply = OutboundReply {
        text: Some("a".repeat(2000 * 64)),
        metadata: Some(
            serde_json::json!({"text_format":"commonmark","interaction_thread_id":"route","nyxid_text_renderer":1}),
        ),
        reply_to_platform_message_id: Some("42".into()),
        attachments: vec![],
    };
    let parts = text_parts(&reply, "discord").unwrap();
    assert_eq!(parts.len(), 64);
    for part in parts {
        let metadata = part.metadata.unwrap();
        assert_eq!(metadata["nyxid_text_renderer"], 2);
        assert_eq!(metadata["interaction_thread_id"], "route");
        assert!(metadata.get("text_format").is_none());
        assert_eq!(part.reply_to_platform_message_id.as_deref(), Some("42"));
        assert_eq!(part.text.unwrap().len(), 2000);
    }
    reply.text.as_mut().unwrap().push('b');
    assert!(text_parts(&reply, "discord").is_err());
    reply.text = Some("x".repeat(2001));
    reply.attachments.push(MaterializedAttachment {
        kind: MediaKind::Image,
        bytes: Default::default(),
        filename: None,
        mime_type: None,
        caption: None,
    });
    assert!(text_parts(&reply, "discord").is_err());
    reply.attachments.clear();
    reply.metadata.as_mut().unwrap()["card"] = serde_json::json!({});
    reply.text = Some("x".repeat(32000));
    assert!(text_parts(&reply, "feishu").is_err());
}
