//! Slack's text-only mrkdwn for NyxID-owned assistant plans.
use super::{commonmark_text, render_markdown_impl};
use pulldown_cmark::{Event, Options, Parser, Tag};
use serde_json::{Value, json};
use std::ops::Range;

const STYLE: [&str; 3] = ["bold", "italic", "strikethrough"];
pub(super) fn parse(source: &str) -> Option<(String, Vec<Value>)> {
    if !source
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && matches!(pair[1], b'*' | b'_' | b'~'))
    {
        return None;
    }
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    );
    let mut excluded: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.clone())
        .collect();
    let mut style_ends = Vec::new();
    let mut keycaps = Vec::new();
    let mut code_block = false;
    let mut has_literal = false;
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                code_block = true;
                excluded.push(range);
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => code_block = false,
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) | Event::Code(_) => {
                excluded.push(range);
            }
            Event::Text(value) if !code_block => {
                has_literal |= escaped_delimiter(source, range.start, &value).is_some();
            }
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough) => {
                let width = match event {
                    Event::Start(Tag::Strong) => 2,
                    Event::Start(Tag::Strikethrough) if source[range.clone()].ends_with("~~") => 2,
                    _ => 1,
                };
                style_ends.push((range.start, range.end - width));
                // A keycap '*' can itself be mistaken for a CommonMark delimiter.
                keycaps.extend(
                    [
                        range.start..range.start + width,
                        range.end - width..range.end,
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|&at| {
                        source.as_bytes()[at] == b'*'
                            && !keycap_suffix(&source[at + 1..]).is_empty()
                    }),
                );
            }
            _ => {}
        }
    }
    if !has_literal {
        return None;
    }
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    let mut urls = Vec::new();
    for link in finder.links(source) {
        let start = link.start();
        // Linkify strips trailing asterisks. Keep them when CommonMark does
        // not identify a matching style that opened outside the URL.
        let mut end = link.end()
            + source.as_bytes()[link.end()..]
                .iter()
                .take_while(|&&b| b == b'*')
                .count();
        if let Some(close) = style_ends
            .iter()
            .filter_map(|&(open, close)| {
                (open < start && (start..end).contains(&close)).then_some(close)
            })
            .min()
        {
            end = close;
        }
        if !excluded.iter().any(|r| r.start < end && start < r.end)
            && !source[start..end].contains(['<', '>'])
            && url::Url::parse(&source[start..end]).is_ok()
        {
            urls.push(start..end);
        }
    }
    let mut inserts = Vec::new();
    for range in &urls {
        inserts.push((range.start, '<'));
        inserts.push((range.end, '>'));
    }
    keycaps.sort_unstable();
    keycaps.dedup();
    for at in keycaps {
        if !urls.iter().any(|url| url.contains(&at)) {
            inserts.push((at, '\\'));
        }
    }
    inserts.sort_unstable();
    let mut marked = String::with_capacity(source.len() + inserts.len());
    let mut next = 0;
    for (at, ch) in inserts {
        marked.push_str(&source[next..at]);
        marked.push(ch);
        next = at;
    }
    marked.push_str(&source[next..]);
    Some(commonmark_text(&marked, true))
}

pub(super) fn escaped_delimiter(source: &str, at: usize, value: &str) -> Option<usize> {
    let first = value.as_bytes().first().copied()?;
    if !matches!(first, b'*' | b'_' | b'~')
        || source.as_bytes().get(at) != Some(&first)
        || source.as_bytes()[..at]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count()
            % 2
            == 0
    {
        return None;
    }
    let suffix = if first == b'*' {
        keycap_suffix(&value[1..])
    } else {
        ""
    };
    Some(1 + suffix.chars().count())
}

fn keycap_suffix(text: &str) -> &'static str {
    if text.starts_with("\u{fe0f}\u{20e3}") {
        "\u{fe0f}\u{20e3}"
    } else if text.starts_with('\u{20e3}') {
        "\u{20e3}"
    } else {
        ""
    }
}

pub(super) fn scalar_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    text.chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= utf16
        })
        .count()
}

fn entity_range(entity: &Value) -> Range<usize> {
    let start = entity["offset"].as_u64().unwrap_or(0) as usize;
    start..start + entity["length"].as_u64().unwrap_or(0) as usize
}

fn span(kind: &str, range: Range<usize>) -> Value {
    json!({"type":kind, "offset":range.start, "length":range.len()})
}

pub(super) fn url_touches_style(url: &Value, entities: &[Value]) -> bool {
    let range = entity_range(url);
    entities.iter().any(|other| {
        other["type"]
            .as_str()
            .is_some_and(|kind| STYLE.contains(&kind))
            && {
                let style = entity_range(other);
                [style.start, style.end]
                    .into_iter()
                    .any(|edge| edge == range.start || edge == range.end)
            }
    })
}

pub(super) fn render(text: &str, entities: &[Value]) -> String {
    if !entities.iter().any(|e| e["type"] == "slack_literal") {
        return render_markdown_impl(text, entities, "slack", true);
    }
    // Adjacent backticks would turn an inline escape into a different fence.
    // Merge literal punctuation into adjoining inline code when applicable.
    let mut code: Vec<_> = entities
        .iter()
        .filter(|e| matches!(e["type"].as_str(), Some("code" | "slack_literal")))
        .map(|e| (entity_range(e), e["type"] == "slack_literal"))
        .collect();
    code.sort_by_key(|(range, _)| range.start);
    let mut merged: Vec<(Range<usize>, bool)> = Vec::new();
    for (range, literal) in code {
        if let Some((last, has_literal)) = merged.last_mut()
            && last.end == range.start
            && (*has_literal || literal)
        {
            last.end = range.end;
            *has_literal = true;
        } else {
            merged.push((range, literal));
        }
    }

    let mut rendered = Vec::new();
    for entity in entities {
        let kind = entity["type"].as_str().unwrap_or_default();
        if matches!(kind, "code" | "slack_literal") {
            continue;
        }
        let range = entity_range(entity);
        if !STYLE.contains(&kind)
            || !merged
                .iter()
                .any(|(code, literal)| *literal && range.start < code.end && code.start < range.end)
        {
            rendered.push(entity.clone());
            continue;
        }
        // Slack's code span disables style on the punctuation itself. Close
        // and reopen the authored style around it so the remaining words keep it.
        let mut start = range.start;
        let mut push_style = |start, end| {
            if start >= end {
                return;
            }
            let local = &text[super::format::byte_range(text, &span(kind, start..end))];
            let trimmed = local.trim();
            let leading = local[..local.len() - local.trim_start().len()]
                .encode_utf16()
                .count();
            if !trimmed.is_empty() {
                let from = start + leading;
                rendered.push(span(kind, from..from + trimmed.encode_utf16().count()));
            }
        };
        for (code, literal) in &merged {
            if *literal && start < code.end && code.start < range.end {
                push_style(start, code.start);
                start = code.end;
            }
        }
        push_style(start, range.end);
    }
    rendered.extend(
        merged
            .into_iter()
            .map(|(range, literal)| span(if literal { "slack_literal" } else { "code" }, range)),
    );
    render_markdown_impl(text, &rendered, "slack", true)
}
