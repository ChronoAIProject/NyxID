use super::{
    channel_platform::OutboundReply, channel_reply_delivery::MAX_REPLY_PARTS,
    channel_text::TextLimit,
};
use crate::errors::{AppError, AppResult};

/// Fully plan before the first provider effect. Routing metadata is copied;
/// interactive content cannot be duplicated across multiple messages.
pub fn text_parts(reply: &OutboundReply, platform: &str) -> AppResult<Vec<OutboundReply>> {
    let text = reply.text.as_deref().unwrap_or_default();
    let chunks: Vec<_> = TextLimit::for_platform(platform)
        .chunks(text)
        .take(MAX_REPLY_PARTS + 1)
        .collect();
    if chunks.len() > MAX_REPLY_PARTS {
        return Err(AppError::ValidationError(
            "Reply exceeds the 64-part limit".into(),
        ));
    }
    if chunks.len() > 1 && chunks.iter().any(|part| part.trim().is_empty()) {
        return Err(AppError::ValidationError(
            "Reply would require a whitespace-only platform message".into(),
        ));
    }
    if chunks.len() > 1 && (!reply.attachments.is_empty() || has_structured_content(reply)) {
        return Err(AppError::ValidationError(
            "Long replies with attachments or interactive metadata require a separate complete text reply".into(),
        ));
    }
    if chunks.is_empty() {
        return Ok(vec![reply.clone()]);
    }
    let parts = chunks
        .into_iter()
        .map(|text| OutboundReply {
            text: Some(text.into()),
            attachments: reply.attachments.clone(),
            metadata: reply.metadata.clone(),
            reply_to_platform_message_id: reply.reply_to_platform_message_id.clone(),
        })
        .collect();
    Ok(parts)
}

pub fn has_structured_content(reply: &OutboundReply) -> bool {
    reply.metadata.as_ref().is_some_and(|metadata| {
        [
            "card",
            "blocks",
            "reply_markup",
            "entities",
            "parse_mode",
            "template",
            "interactive",
            "attachments",
        ]
        .iter()
        .any(|key| metadata.get(key).is_some())
    })
}

/// Rich/media and non-IM transports retain their adapter-owned send semantics.
/// Reject oversized mixed content rather than sending a prefix before failing.
pub fn ordinary(
    reply: &OutboundReply,
    platform: &str,
    chat: &str,
) -> AppResult<Option<Vec<OutboundReply>>> {
    if platform == "aurinko"
        || (platform == "x" && super::channel_adapters::x::is_public_conversation(chat))
    {
        return Ok(None);
    }
    let parts = text_parts(reply, platform)?;
    if !reply.attachments.is_empty() || has_structured_content(reply) {
        let limit = TextLimit::for_platform(platform);
        if reply
            .attachments
            .iter()
            .filter_map(|a| a.caption.as_deref())
            .any(|text| limit.chunks(text).count() > 1)
        {
            return Err(AppError::ValidationError(
                "Attachment caption exceeds the platform text limit".into(),
            ));
        }
        return Ok(None);
    }
    Ok(Some(parts))
}

/// Assistant output uses native Telegram entities rather than independently
/// parsing fragments of Markdown. One entity can continue across messages;
/// offsets and lengths are clipped in UTF-16 units for each part.
pub fn assistant_parts(text: &str, platform: &str, overflow_url: &str) -> Vec<OutboundReply> {
    let telegram = matches!(platform, "telegram" | "telegram-new");
    let markdown = matches!(platform, "discord" | "slack" | "whatsapp");
    let (mut body, mut entities) = if telegram || markdown {
        telegram_text(text)
    } else {
        (text.to_owned(), Vec::new())
    };
    let limit = TextLimit::for_platform(platform);
    let slack_plain = platform == "slack" && body.contains('`');
    if telegram || markdown {
        expand_long_links(&mut body, &mut entities, limit, markdown, slack_plain);
    }
    let mut rest = body.as_str();
    let mut offset = 0;
    let mut parts = Vec::new();
    while !rest.is_empty() && parts.len() <= MAX_REPLY_PARTS {
        let mut budget = limit.budget();
        let (chunk, rendered, local) = loop {
            let chunk = limit
                .with_budget(budget)
                .display_chunks(rest)
                .next()
                .unwrap_or_default();
            let length = chunk.encode_utf16().count();
            let mut local = local_entities(&entities, offset, length);
            if telegram && local.len() > 100 && budget > 8 {
                budget = (budget / 2).max(8);
                continue;
            }
            let mut rendered = if markdown && !slack_plain {
                render_markdown(chunk, &local, platform)
            } else {
                chunk.to_owned()
            };
            if rendered.trim().is_empty() {
                const CONTINUATION: &str = "Whitespace continuation:\n";
                let available = limit.budget() - limit.measure(CONTINUATION);
                if budget > available {
                    budget = available;
                    continue;
                }
                rendered.insert_str(0, CONTINUATION);
                for entity in &mut local {
                    entity["offset"] = (entity["offset"].as_u64().unwrap_or(0)
                        + CONTINUATION.encode_utf16().count() as u64)
                        .into();
                }
            }
            let cost = limit.measure(&rendered);
            if cost <= limit.budget() {
                break (chunk, rendered, local);
            }
            // Account for syntax repeated around clipped formatting spans.
            let next = (budget / 2).max(1);
            if next == budget {
                // Excessive nested styling can exceed the budget even for one
                // scalar. Retain the link; long/nested targets were expanded.
                let links: Vec<_> = local
                    .into_iter()
                    .filter(|e| e["type"] == "text_link")
                    .collect();
                let rendered = render_markdown(chunk, &links, platform);
                break (chunk, rendered, links);
            }
            budget = next;
        };
        offset += chunk.encode_utf16().count();
        rest = &rest[chunk.len()..];
        parts.push(OutboundReply {
            text: Some(rendered),
            attachments: vec![],
            reply_to_platform_message_id: None,
            metadata: if telegram {
                Some(serde_json::json!({"entities":local}))
            } else if slack_plain {
                Some(serde_json::json!({"mrkdwn":false}))
            } else {
                None
            },
        });
    }
    if parts.len() > MAX_REPLY_PARTS || !rest.is_empty() {
        parts.truncate(MAX_REPLY_PARTS - 1);
        let notice = format!(
            "The reply exceeds the 64-message delivery limit. The remaining text is in NyxID: {overflow_url}"
        );
        parts.push(OutboundReply {
            text: Some(limit.chunks(&notice).next().unwrap_or_default().into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            metadata: telegram.then(|| serde_json::json!({"entities":[]})),
        });
    }
    parts
}

fn expand_long_links(
    body: &mut String,
    entities: &mut Vec<serde_json::Value>,
    limit: TextLimit,
    markdown: bool,
    expand_all: bool,
) {
    let mut links = entities
        .iter()
        .enumerate()
        .filter(|(index, e)| {
            e["type"] == "text_link"
                && (expand_all
                    || e["url"]
                        .as_str()
                        .is_some_and(|url| !supported_link_target(url))
                    || e["url"].as_str().is_some_and(|url| {
                        let target = if markdown {
                            markdown_link_url(url)
                        } else {
                            url.to_owned()
                        };
                        limit.measure(&target) > limit.budget() / 2
                    })
                    || entities.iter().enumerate().any(|(other_index, other)| {
                        other_index != *index
                            && other["type"] == "text_link"
                            && other["offset"].as_u64().unwrap_or(0)
                                < e["offset"].as_u64().unwrap_or(0)
                                    + e["length"].as_u64().unwrap_or(0)
                            && e["offset"].as_u64().unwrap_or(0)
                                < other["offset"].as_u64().unwrap_or(0)
                                    + other["length"].as_u64().unwrap_or(0)
                    }))
        })
        .map(|(_, link)| link.clone())
        .collect::<Vec<_>>();
    links.sort_by_key(|e| std::cmp::Reverse(e["offset"].as_u64().unwrap_or(0)));
    for planned in links {
        // Inner expansion changes an enclosing link's length. Reload it before
        // inserting its target, so nested image links retain both complete URLs.
        let Some(link) = entities
            .iter()
            .find(|e| {
                e["type"] == "text_link"
                    && e["offset"] == planned["offset"]
                    && e["url"] == planned["url"]
            })
            .cloned()
        else {
            continue;
        };
        let end = link["offset"].as_u64().unwrap_or(0) + link["length"].as_u64().unwrap_or(0);
        let mut units = 0;
        let byte = body
            .char_indices()
            .find_map(|(i, ch)| {
                if units == end {
                    return Some(i);
                }
                units += ch.len_utf16() as u64;
                None
            })
            .unwrap_or(body.len());
        let target = link["url"].as_str().unwrap_or_default();
        let label_is_target = body
            .encode_utf16()
            .skip(link["offset"].as_u64().unwrap_or(0) as usize)
            .take(link["length"].as_u64().unwrap_or(0) as usize)
            .eq(target.encode_utf16());
        let extra = if label_is_target {
            String::new()
        } else {
            format!(" ({target})")
        };
        body.insert_str(byte, &extra);
        let added = extra.encode_utf16().count() as u64;
        entities.retain(|e| *e != link);
        for entity in entities.iter_mut() {
            let start = entity["offset"].as_u64().unwrap_or(0);
            let length = entity["length"].as_u64().unwrap_or(0);
            if start >= end {
                entity["offset"] = (start + added).into();
            } else if start + length >= end {
                entity["length"] = (length + added).into();
            }
        }
    }
}

fn supported_link_target(target: &str) -> bool {
    if target.chars().any(char::is_control) || target.trim() != target {
        return false;
    }
    let Ok(url) = url::Url::parse(target) else {
        return false;
    };
    match url.scheme() {
        "http" | "https" | "tg" => url.host_str().is_some(),
        "mailto" => !url.path().is_empty(),
        _ => false,
    }
}

fn local_entities(
    entities: &[serde_json::Value],
    offset: usize,
    length: usize,
) -> Vec<serde_json::Value> {
    entities
        .iter()
        .filter_map(|entity| {
            let start = entity["offset"].as_u64()? as usize;
            let end = start + entity["length"].as_u64()? as usize;
            let from = start.max(offset);
            let to = end.min(offset + length);
            if from >= to {
                return None;
            }
            let mut entity = entity.clone();
            entity["offset"] = (from - offset).into();
            entity["length"] = (to - from).into();
            Some(entity)
        })
        .collect()
}

fn render_markdown(text: &str, entities: &[serde_json::Value], platform: &str) -> String {
    let mut spans = entities.to_vec();
    spans.sort_by_key(|e| {
        (
            e["offset"].as_u64().unwrap_or(0),
            std::cmp::Reverse(e["length"].as_u64().unwrap_or(0)),
        )
    });
    let markers = |e: &serde_json::Value| -> (String, String) {
        match e["type"].as_str().unwrap_or_default() {
            "bold" => {
                let m = if platform == "discord" { "**" } else { "*" };
                (m.into(), m.into())
            }
            "italic" => ("_".into(), "_".into()),
            "strikethrough" => {
                let m = if platform == "discord" { "~~" } else { "~" };
                (m.into(), m.into())
            }
            "code" | "pre" => {
                let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(if e["type"] == "pre" {
                    longest.max(2) + 1
                } else {
                    longest + 1
                });
                if e["type"] == "pre" {
                    (format!("{fence}\n"), format!("\n{fence}"))
                } else {
                    (format!("{fence} "), format!(" {fence}"))
                }
            }
            "text_link" => {
                let url = markdown_link_url(e["url"].as_str().unwrap_or_default());
                if platform == "slack" {
                    (format!("<{url}|"), ">".into())
                } else {
                    ("[".into(), format!("]({url})"))
                }
            }
            _ => (String::new(), String::new()),
        }
    };
    let mut result = String::new();
    let mut position = 0;
    for ch in text.chars() {
        for span in spans.iter().rev().filter(|e| {
            e["offset"].as_u64().unwrap_or(0) + e["length"].as_u64().unwrap_or(0) == position
        }) {
            result.push_str(&markers(span).1);
        }
        for span in spans
            .iter()
            .filter(|e| e["offset"].as_u64() == Some(position))
        {
            result.push_str(&markers(span).0);
        }
        let in_code = spans.iter().any(|e| {
            matches!(e["type"].as_str(), Some("code" | "pre"))
                && e["offset"].as_u64().unwrap_or(0) <= position
                && position < e["offset"].as_u64().unwrap_or(0) + e["length"].as_u64().unwrap_or(0)
        });
        if platform == "slack" {
            match ch {
                '&' => result.push_str("&amp;"),
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                _ => result.push(ch),
            }
        } else {
            if !in_code && "\\`*_{}[]()#+-.!|>~".contains(ch) {
                result.push('\\');
            }
            result.push(ch);
        }
        position += ch.len_utf16() as u64;
    }
    for span in spans.iter().rev().filter(|e| {
        e["offset"].as_u64().unwrap_or(0) + e["length"].as_u64().unwrap_or(0) == position
    }) {
        result.push_str(&markers(span).1);
    }
    result
}

fn markdown_link_url(url: &str) -> String {
    url.replace('(', "%28")
        .replace(')', "%29")
        .replace('>', "%3E")
        .replace('|', "%7C")
        .replace(' ', "%20")
}

fn telegram_text(source: &str) -> (String, Vec<serde_json::Value>) {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut text = String::new();
    let mut entities = Vec::new();
    let mut stack: Vec<(TagEnd, usize, serde_json::Value)> = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    ) {
        match event {
            Event::Start(tag) => {
                let kind = match &tag {
                    Tag::Strong => Some("bold"),
                    Tag::Emphasis => Some("italic"),
                    Tag::Strikethrough => Some("strikethrough"),
                    Tag::CodeBlock(_) => Some("pre"),
                    Tag::Link { .. } | Tag::Image { .. } => Some("text_link"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let mut entity = serde_json::json!({"type": kind});
                    if let Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } = &tag {
                        entity["url"] = dest_url.to_string().into();
                    }
                    stack.push((tag.to_end(), text.encode_utf16().count(), entity));
                }
                if let Tag::List(start) = tag {
                    lists.push(start);
                }
                if matches!(tag, Tag::Item) {
                    if let Some(Some(number)) = lists.last_mut() {
                        text.push_str(&format!("{number}. "));
                        *number += 1;
                    } else {
                        text.push_str("• ");
                    }
                }
            }
            Event::End(end) => {
                if matches!(end, TagEnd::List(_)) {
                    lists.pop();
                }
                if stack.last().is_some_and(|(tag, _, _)| *tag == end) {
                    let (_, start, mut entity) = stack.pop().unwrap();
                    let mut length = text.encode_utf16().count() - start;
                    if length == 0 && entity["type"] == "text_link" {
                        let target = entity["url"].as_str().unwrap_or_default();
                        text.push_str(target);
                        length = target.encode_utf16().count();
                    }
                    if length > 0 {
                        entity["offset"] = start.into();
                        entity["length"] = length.into();
                        entities.push(entity);
                    }
                }
                match end {
                    TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::CodeBlock
                    | TagEnd::List(_) => {
                        if !text.ends_with("\n\n") {
                            text.push_str(if text.ends_with('\n') { "\n" } else { "\n\n" });
                        }
                    }
                    TagEnd::Item | TagEnd::TableRow => {
                        if !text.ends_with('\n') {
                            text.push('\n');
                        }
                    }
                    TagEnd::TableCell => text.push('\t'),
                    _ => {}
                }
            }
            Event::Text(value) | Event::Html(value) | Event::InlineHtml(value) => {
                text.push_str(&value)
            }
            Event::Code(value) => {
                let offset = text.encode_utf16().count();
                text.push_str(&value);
                // Telegram forbids code nested inside another entity. The
                // enclosing emphasis/link still preserves the display text.
                if stack.is_empty() {
                    entities.push(serde_json::json!({"type":"code", "offset":offset,"length":value.encode_utf16().count()}));
                }
            }
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::Rule => text.push_str("───\n"),
            Event::TaskListMarker(done) => text.push_str(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(label) => {
                text.push('[');
                text.push_str(&label);
                text.push(']');
            }
            Event::InlineMath(value) | Event::DisplayMath(value) => text.push_str(&value),
        }
    }
    // Block separation is renderer-owned; the final paragraph adds no content.
    let trimmed = text.trim_end_matches('\n').len();
    text.truncate(trimmed);
    if text.is_empty() {
        text = source.to_owned();
    }
    let total = text.encode_utf16().count();
    for entity in &mut entities {
        let start = entity["offset"].as_u64().unwrap_or(0) as usize;
        let length = entity["length"].as_u64().unwrap_or(0) as usize;
        entity["length"] = length.min(total.saturating_sub(start)).into();
    }
    entities.retain(|e| e["length"].as_u64().unwrap_or(0) > 0);
    (text, entities)
}

pub fn fingerprint(parts: &[OutboundReply]) -> String {
    super::mcp_service::canonical_sha256(serde_json::json!(
        parts
            .iter()
            .map(|part| serde_json::json!({"text":part.text,"metadata":part.metadata}))
            .collect::<Vec<_>>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: String) -> OutboundReply {
        OutboundReply {
            text: Some(text),
            attachments: vec![],
            metadata: None,
            reply_to_platform_message_id: Some("42".into()),
        }
    }

    #[test]
    fn assistant_empty_link_labels_keep_the_destination() {
        for platform in ["telegram", "discord", "slack", "whatsapp"] {
            for markup in ["![]", "[]"] {
                let parts = assistant_parts(
                    &format!("See this:\n\n{markup}(https://example.test/diagram.png)"),
                    platform,
                    "https://nyxid.test/thread",
                );
                let delivered: String = parts
                    .iter()
                    .map(|part| {
                        format!(
                            "{} {}",
                            part.text.as_deref().unwrap_or_default(),
                            part.metadata.as_ref().unwrap_or(&serde_json::Value::Null)
                        )
                    })
                    .collect();
                assert!(
                    delivered.contains("https://example.test/diagram.png"),
                    "{platform}: {delivered}"
                );
            }
        }
    }

    #[test]
    fn assistant_unsupported_link_targets_remain_visible_without_entities() {
        for target in [
            "./README.md",
            "/tmp/report.txt",
            "#details",
            "javascript:alert",
            "https://",
        ] {
            let parts = assistant_parts(
                &format!("Read [the file]({target}) for setup."),
                "telegram",
                "https://nyxid.test/thread",
            );
            let delivered: String = parts
                .iter()
                .filter_map(|part| part.text.as_deref())
                .collect();
            assert!(delivered.contains("the file") && delivered.contains(target));
            assert!(parts.iter().all(|part| {
                part.metadata.as_ref().unwrap()["entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|entity| entity["type"] != "text_link")
            }));
        }
        for target in [
            "https://example.test/path",
            "http://example.test/path",
            "tg://user?id=42",
            "mailto:help@example.test",
        ] {
            let parts = assistant_parts(
                &format!("Read [this]({target})."),
                "telegram",
                "https://nyxid.test/thread",
            );
            assert!(parts.iter().any(|part| {
                part.metadata.as_ref().unwrap()["entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entity| entity["url"] == target)
            }));
        }
    }

    #[test]
    fn assistant_whitespace_continuations_remain_deliverable() {
        let spaces = " ".repeat(8200);
        let source = format!("```text\nstart🦉{spaces}end\n```");
        let parts = assistant_parts(&source, "telegram", "https://nyxid.test/thread");
        assert!(parts.iter().all(|part| {
            let text = part.text.as_deref().unwrap();
            !text.trim().is_empty() && text.encode_utf16().count() <= 4096
        }));
        let delivered: String = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect();
        assert!(delivered.contains("start🦉") && delivered.ends_with("end"));
        assert_eq!(
            delivered.replace("Whitespace continuation:\n", ""),
            format!("start🦉{spaces}end")
        );
        for part in parts {
            let text: Vec<_> = part.text.unwrap().encode_utf16().collect();
            for entity in part.metadata.unwrap()["entities"].as_array().unwrap() {
                let start = entity["offset"].as_u64().unwrap() as usize;
                let end = start + entity["length"].as_u64().unwrap() as usize;
                assert!(String::from_utf16(&text[start..end]).is_ok());
            }
        }
    }

    #[test]
    fn assistant_telegram_entities_continue_without_surrogate_or_markup_damage() {
        let source = format!(
            "**{}**\n\n```rust\n{}\n```\n\n[reference](https://example.test/a?x=1&y=2)",
            "🦉".repeat(3000),
            "let x = 1;\n".repeat(800)
        );
        let parts = assistant_parts(&source, "telegram", "https://nyxid.test/assistant?c=thread");
        assert!(parts.len() > 3);
        let joined = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect::<String>();
        assert!(joined.contains(&"🦉".repeat(3000)));
        assert!(joined.contains(&"let x = 1;\n".repeat(800)));
        assert!(joined.contains("reference"));
        let mut bold = 0;
        for part in parts {
            let text = part.text.unwrap();
            let units = text.encode_utf16().count();
            assert!(units <= 4096);
            let entities = part.metadata.unwrap()["entities"]
                .as_array()
                .unwrap()
                .clone();
            for entity in entities {
                let start = entity["offset"].as_u64().unwrap() as usize;
                let length = entity["length"].as_u64().unwrap() as usize;
                assert!(length > 0 && start + length <= units);
                assert!(
                    String::from_utf16(
                        &text.encode_utf16().collect::<Vec<_>>()[start..start + length]
                    )
                    .is_ok()
                );
                if entity["type"] == "bold" {
                    bold += length;
                }
                if entity["type"] == "text_link" {
                    assert_eq!(entity["url"], "https://example.test/a?x=1&y=2");
                }
            }
        }
        assert_eq!(bold, 6000);
    }

    #[test]
    fn assistant_formats_literal_syntax_code_fences_and_long_links_without_losing_content() {
        let source = format!(
            "Literal \\*stars\\* &amp; [reference](https://example.test/a)\n\n**{}**\n\n````rust\n{}\n````",
            "bold 🦉 (value) ".repeat(600),
            "let value = \"```\";\n".repeat(600),
        );
        let (expected, _) = telegram_text(&source);
        let squash = |value: &str| {
            value
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        };
        let parts = assistant_parts(&source, "discord", "https://nyxid.test/assistant?c=thread");
        let display = parts
            .iter()
            .map(|part| telegram_text(part.text.as_deref().unwrap()).0)
            .collect::<String>();
        assert_eq!(squash(&display), squash(&expected));
        assert!(
            parts
                .iter()
                .all(|p| p.text.as_ref().unwrap().encode_utf16().count() <= 2000)
        );
        let url = format!("https://example.test/{}", "long".repeat(2000));
        for platform in ["telegram", "discord", "slack"] {
            let parts = assistant_parts(
                &format!("[full target]({url})"),
                platform,
                "https://nyxid.test/assistant?c=thread",
            );
            let display = parts
                .iter()
                .map(|p| {
                    if platform == "discord" {
                        telegram_text(p.text.as_deref().unwrap()).0
                    } else {
                        p.text.clone().unwrap()
                    }
                })
                .collect::<String>();
            assert!(display.contains(&url), "{platform}");
        }
    }

    #[test]
    fn dense_and_nested_links_keep_every_target_within_the_platform_limit() {
        for (platform, target_length) in [("discord", 900), ("slack", 19_000)] {
            let targets: Vec<_> = (0..12)
                .map(|index| format!("https://example.test/{index}/{}", "a".repeat(target_length)))
                .collect();
            let source: String = targets.iter().map(|url| format!("[x]({url})")).collect();
            let parts = assistant_parts(&source, platform, "https://nyxid.test/assistant?c=thread");
            let rendered: String = parts.iter().filter_map(|p| p.text.as_deref()).collect();
            assert!(
                targets.iter().all(|url| rendered.contains(url)),
                "{platform}"
            );
            assert!(parts.iter().all(|p| {
                TextLimit::for_platform(platform).measure(p.text.as_deref().unwrap())
                    <= TextLimit::for_platform(platform).budget()
            }));
        }
        let source = "[![diagram](https://example.test/image.png)](https://example.test/page)";
        let parts = assistant_parts(source, "telegram", "https://nyxid.test/assistant?c=thread");
        let rendered: String = parts.iter().filter_map(|p| p.text.as_deref()).collect();
        assert!(rendered.contains("diagram"));
        assert!(rendered.contains("https://example.test/image.png"));
        assert!(rendered.contains("https://example.test/page"));
        assert!(parts.iter().all(|p| {
            p.metadata.as_ref().unwrap()["entities"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["type"] != "text_link")
        }));
    }

    #[test]
    fn slack_plain_fallback_keeps_literal_backticks_and_every_link_target() {
        let source =
            "`` `literal` <value> & more `` and [guide](https://example.test/guide?x=1&y=2)\n\n"
                .repeat(900);
        let parts = assistant_parts(&source, "slack", "https://nyxid.test/assistant?c=thread");
        assert!(parts.len() > 1);
        let delivered: String = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect();
        assert_eq!(delivered.matches("`literal` <value> & more").count(), 900);
        assert_eq!(
            delivered
                .matches("https://example.test/guide?x=1&y=2")
                .count(),
            900
        );
        assert!(parts.iter().all(|part| {
            part.metadata.as_ref().unwrap()["mrkdwn"] == false
                && TextLimit::for_platform("slack").measure(part.text.as_deref().unwrap()) <= 40_000
        }));
    }

    #[test]
    fn assistant_delivery_cap_is_explicit_and_uses_each_platforms_units() {
        for platform in [
            "telegram",
            "telegram-new",
            "discord",
            "slack",
            "whatsapp",
            "x",
            "lark",
            "feishu",
        ] {
            let text = "🦉中\n\n".repeat(20000);
            let parts = assistant_parts(&text, platform, "https://nyxid.test/assistant?c=thread");
            assert!(parts.len() > 1);
            assert!(parts.iter().all(|part| {
                TextLimit::for_platform(platform)
                    .chunks(part.text.as_deref().unwrap())
                    .count()
                    == 1
            }));
        }
        let parts = assistant_parts(
            &"a".repeat(4096 * 65),
            "telegram",
            "https://nyxid.test/assistant?c=thread",
        );
        assert_eq!(parts.len(), 64);
        assert!(
            parts
                .last()
                .unwrap()
                .text
                .as_deref()
                .unwrap()
                .contains("remaining text is in NyxID: https://nyxid.test/assistant?c=thread")
        );
        assert_eq!(
            parts[..63]
                .iter()
                .map(|p| p.text.as_ref().unwrap().len())
                .sum::<usize>(),
            4096 * 63
        );
    }

    #[test]
    fn preflight_rejects_part_limit_and_interactive_overflow() {
        assert!(text_parts(&reply("a".repeat(4096 * 64 + 1)), "telegram").is_err());
        let mut message = reply("a".repeat(5000));
        message.metadata = Some(serde_json::json!({"reply_markup": {"inline_keyboard": []}}));
        assert!(text_parts(&message, "telegram").is_err());
    }

    #[test]
    fn preflight_keeps_templates_interactions_and_media_on_their_existing_paths() {
        for (platform, key) in [
            ("whatsapp", "template"),
            ("whatsapp", "interactive"),
            ("x", "attachments"),
        ] {
            let mut message = reply("short".into());
            message.metadata = Some(serde_json::json!({key: {}}));
            assert!(ordinary(&message, platform, "123").unwrap().is_none());
            message.text = Some("a".repeat(10_001));
            assert!(ordinary(&message, platform, "123").is_err());
        }
    }

    #[test]
    fn preflight_rejects_whitespace_only_parts_before_sending_a_prefix() {
        let message = reply("a".repeat(4096) + &" ".repeat(4096) + "tail");
        assert!(text_parts(&message, "telegram").is_err());
    }

    #[test]
    fn preserves_full_text_and_routing_in_all_parts() {
        let mut message = reply("🦉界\\* paragraph\n\n".repeat(1000));
        message.metadata = Some(serde_json::json!({"message_thread_id": 123}));
        let parts = text_parts(&message, "telegram").unwrap();
        assert_eq!(
            parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<String>(),
            message.text.unwrap()
        );
        assert!(
            parts
                .iter()
                .all(|p| p.reply_to_platform_message_id.as_deref() == Some("42")
                    && p.metadata == message.metadata)
        );
    }
}
