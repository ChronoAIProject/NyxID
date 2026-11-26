//! Slack's text-only mrkdwn for NyxID-owned assistant plans.
use super::{commonmark_text, render_markdown_impl, supported_link_target};
use pulldown_cmark::{Event, Options, Parser, Tag};
use serde_json::{Value, json};
use std::collections::HashSet;
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
    let mut escape_slashes = Vec::new();
    let mut has_literal = false;
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                code_block = true;
                excluded.push(range);
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => code_block = false,
            Event::Start(Tag::Link { .. } | Tag::Image { .. })
            | Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_) => {
                excluded.push(range);
            }
            Event::Text(value) if !code_block => {
                if let Some(delimiter) = escaped_punctuation(source, range.start, &value) {
                    escape_slashes.push(range.start - 1);
                    has_literal |= matches!(delimiter, b'*' | b'_' | b'~');
                }
            }
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough) => {
                let width = match event {
                    Event::Start(Tag::Strong) => 2,
                    Event::Start(Tag::Strikethrough) if source[range.clone()].ends_with("~~") => 2,
                    _ => 1,
                };
                style_ends.push((range.end - width, range.start));
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
    style_ends.sort_unstable();
    excluded.sort_unstable_by_key(|range| range.start);
    let mut opaque: Vec<Range<usize>> = Vec::new();
    for range in excluded {
        if let Some(previous) = opaque.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            opaque.push(range);
        }
    }
    escape_slashes.sort_unstable();
    escape_slashes.dedup();

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
        let first_close = style_ends.partition_point(|&(close, _)| close < start);
        if let Some(&(close, _)) = style_ends[first_close..]
            .iter()
            .take_while(|&&(close, _)| close < end)
            .find(|&&(_, open)| open < start)
        {
            end = close;
        }
        let first_opaque = opaque.partition_point(|range| range.end <= start);
        if opaque
            .get(first_opaque)
            .is_none_or(|range| range.start >= end)
            && !source[start..end].contains(['<', '>'])
            && supported_link_target(&source[start..end])
        {
            urls.push(start..end);
        }
    }
    let mut edits = Vec::new();
    for range in &urls {
        edits.push((range.start, Some('<')));
        edits.push((range.end, Some('>')));
        // Autolinks retain backslashes verbatim. Consume only slashes whose
        // following punctuation was escaped by the original CommonMark parse.
        let first = escape_slashes.partition_point(|&at| at < range.start);
        edits.extend(
            escape_slashes[first..]
                .iter()
                .copied()
                .take_while(|&at| at + 1 < range.end)
                .map(|at| (at, None)),
        );
    }
    keycaps.sort_unstable();
    keycaps.dedup();
    for at in keycaps {
        let next_url = urls.partition_point(|url| url.end <= at);
        if urls.get(next_url).is_none_or(|url| !url.contains(&at)) {
            edits.push((at, Some('\\')));
        }
    }
    edits.sort_unstable();
    let mut marked = String::with_capacity(source.len() + edits.len());
    let mut bare_starts = Vec::new();
    let mut next = 0;
    for (at, replacement) in edits {
        marked.push_str(&source[next..at]);
        next = at;
        if let Some(ch) = replacement {
            if ch == '<' {
                bare_starts.push(marked.len());
            }
            marked.push(ch);
        } else {
            next += 1;
        }
    }
    marked.push_str(&source[next..]);
    Some(commonmark_text(&marked, Some(&bare_starts)))
}

pub(super) fn escaped_delimiter(source: &str, at: usize, value: &str) -> Option<usize> {
    let first = escaped_punctuation(source, at, value)?;
    match first {
        b'*' => Some(1 + keycap_suffix(&value[1..]).chars().count()),
        b'_' | b'~' => Some(1),
        _ => None,
    }
}

fn escaped_punctuation(source: &str, at: usize, value: &str) -> Option<u8> {
    let first = value.as_bytes().first().copied()?;
    if !first.is_ascii_punctuation()
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
    Some(first)
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

pub(super) fn style_edges(entities: &[Value]) -> HashSet<u64> {
    entities
        .iter()
        .filter(|entity| {
            entity["type"]
                .as_str()
                .is_some_and(|kind| STYLE.contains(&kind))
        })
        .flat_map(|entity| {
            let range = entity_range(entity);
            [range.start as u64, range.end as u64]
        })
        .collect()
}

pub(super) fn push_text_char(result: &mut String, ch: char) {
    match ch {
        '&' => result.push_str("&amp;"),
        '<' => result.push_str("&lt;"),
        '>' => result.push_str("&gt;"),
        _ => result.push(ch),
    }
}

pub(super) fn render_markers(
    text: &str,
    spans: &[Value],
    markers: impl Fn(&Value) -> (String, String),
) -> String {
    let mut boundaries: Vec<_> = spans
        .iter()
        .enumerate()
        .flat_map(|(index, span)| {
            let range = entity_range(span);
            let (open, close) = markers(span);
            [
                (range.start, true, index, open),
                (range.end, false, usize::MAX - index, close),
            ]
        })
        .filter(|(_, _, _, marker)| !marker.is_empty())
        .collect();
    // At a shared boundary, close inner spans before opening new spans.
    boundaries.sort_unstable_by_key(|&(position, opening, order, _)| (position, opening, order));
    let mut boundaries = boundaries.into_iter().peekable();
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    for ch in text.chars() {
        while let Some((_, _, _, marker)) = boundaries.next_if(|edge| edge.0 == position) {
            result.push_str(&marker);
        }
        push_text_char(&mut result, ch);
        position += ch.len_utf16();
    }
    for (_, _, _, marker) in boundaries {
        result.push_str(&marker);
    }
    result
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

    let utf16_bytes: Vec<_> = text
        .char_indices()
        .flat_map(|(byte, ch)| std::iter::repeat_n(byte, ch.len_utf16()))
        .chain(std::iter::once(text.len()))
        .collect();
    let mut rendered = Vec::new();
    for entity in entities {
        let kind = entity["type"].as_str().unwrap_or_default();
        if matches!(kind, "code" | "slack_literal") {
            continue;
        }
        let range = entity_range(entity);
        if !STYLE.contains(&kind) {
            rendered.push(entity.clone());
            continue;
        }
        let first = merged.partition_point(|(code, _)| code.end <= range.start);
        let overlapping = merged[first..]
            .iter()
            .take_while(|(code, _)| code.start < range.end)
            .filter(|(_, literal)| *literal);
        if overlapping.clone().next().is_none() {
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
            let local = &text[utf16_bytes[start]..utf16_bytes[end]];
            let trimmed = local.trim();
            let leading = local[..local.len() - local.trim_start().len()]
                .encode_utf16()
                .count();
            if !trimmed.is_empty() {
                let from = start + leading;
                rendered.push(span(kind, from..from + trimmed.encode_utf16().count()));
            }
        };
        for (code, _) in overlapping {
            push_style(start, code.start);
            start = code.end;
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
