//! Slack's text-only mrkdwn for NyxID-owned assistant plans.
use super::{commonmark_text, supported_link_target};
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag};
use serde_json::Value;
use std::collections::HashSet;
use std::ops::Range;

const STYLE: [&str; 3] = ["bold", "italic", "strikethrough"];

pub(super) struct BareUrl {
    pub start: usize,
    pub normalized: Option<NormalizedUrl>,
}

pub(super) struct NormalizedUrl {
    pub display: String,
    pub literals: Vec<Range<usize>>,
}

pub(super) fn parse(source: &str, rich: bool) -> Option<(String, Vec<Value>)> {
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
    let mut character_references = Vec::new();
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
                let raw = &source[range.clone()];
                if raw.starts_with('&') && raw.ends_with(';') && raw != value.as_ref() {
                    character_references.push((range, value));
                }
            }
            Event::HardBreak if source.as_bytes()[range.start] == b'\\' => {
                escape_slashes.push(range.start);
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
    character_references.sort_unstable_by_key(|(range, _)| range.start);

    let mut scan = String::new();
    let mut mapping = Vec::new();
    let mut next = 0;
    for (range, value) in &character_references {
        // Keep decoded controls inside the candidate so target validation can
        // reject it instead of opening a link to a different destination.
        if value.chars().any(char::is_control) {
            continue;
        }
        scan.push_str(&source[next..range.start]);
        let start = scan.len();
        scan.push_str(value);
        mapping.push((start..scan.len(), range.clone()));
        next = range.end;
    }
    let scan = if mapping.is_empty() {
        source
    } else {
        scan.push_str(&source[next..]);
        &scan
    };

    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    // Linkify doesn't treat every Unicode whitespace scalar as a separator.
    // Keep the same boundaries for decoded references and literal whitespace.
    let words = scan
        .split_inclusive(char::is_whitespace)
        .scan(0, |offset, word| {
            let start = *offset;
            *offset += word.len();
            Some((start, word.trim_end_matches(char::is_whitespace)))
        });
    let mut urls = Vec::new();
    let links = words.flat_map(|(start, word)| finder.links(word).map(move |link| (start, link)));
    for (word_start, link) in links.filter(|(_, link)| supported_link_target(link.as_str())) {
        // Linkify strips trailing asterisks. Keep them when CommonMark does
        // not identify a matching style that opened outside the URL.
        let start = word_start + link.start();
        let end = word_start + link.end();
        let end = end
            + scan.as_bytes()[end..]
                .iter()
                .take_while(|&&b| b == b'*')
                .count();
        let (Some(start), Some(mut end)) = (
            source_boundary(&mapping, start),
            source_boundary(&mapping, end),
        ) else {
            continue;
        };
        let Some((scheme, _)) = link.as_str().split_once(':') else {
            continue;
        };
        // The temporary angle autolink needs a literal CommonMark scheme.
        // An encoded scheme stays ordinary decoded display text.
        if !source[start..end]
            .strip_prefix(scheme)
            .is_some_and(|rest| rest.starts_with(':'))
        {
            continue;
        }
        let first_close = style_ends.partition_point(|&(close, _)| close < start);
        if let Some(&(close, _)) = style_ends[first_close..]
            .iter()
            .take_while(|&&(close, _)| close < end)
            .find(|&&(_, open)| open < start)
        {
            end = close;
        }
        // Leave trailing punctuation escapes and hard breaks intact outside
        // the autolink so CommonMark can consume their backslashes.
        if end > start && escape_slashes.binary_search(&(end - 1)).is_ok() {
            end -= 1;
        }
        let first_reference =
            character_references.partition_point(|(range, _)| range.start < start);
        let after_references = character_references.partition_point(|(range, _)| range.start < end);
        let references = &character_references[first_reference..after_references];
        let first_opaque = opaque.partition_point(|range| range.end <= start);
        if opaque
            .get(first_opaque)
            .is_none_or(|range| range.start >= end)
            && !source[start..end].contains(['<', '>'])
        {
            urls.push((start..end, references));
        }
    }
    let mut edits = Vec::new();
    let mut decoded_urls = Vec::with_capacity(urls.len());
    for (range, references) in &urls {
        edits.push((range.start, Some('<')));
        edits.push((range.end, Some('>')));
        // Autolinks retain backslashes verbatim. Consume only slashes whose
        // following punctuation was escaped by the original CommonMark parse.
        let first = escape_slashes.partition_point(|&at| at < range.start);
        let count = escape_slashes[first..].partition_point(|&at| at + 1 < range.end);
        let slashes = &escape_slashes[first..first + count];
        edits.extend(slashes.iter().copied().map(|at| (at, None)));
        decoded_urls.push(
            (!references.is_empty() || !slashes.is_empty())
                .then(|| normalize_bare_url(source, range.clone(), references, slashes)),
        );
    }
    let mut decoded_urls = decoded_urls.into_iter();
    keycaps.sort_unstable();
    keycaps.dedup();
    for at in keycaps {
        let next_url = urls.partition_point(|(url, _)| url.end <= at);
        if urls.get(next_url).is_none_or(|(url, _)| !url.contains(&at)) {
            edits.push((at, Some('\\')));
        }
    }
    edits.sort_unstable();
    let mut marked = String::with_capacity(source.len() + edits.len());
    let mut bare_urls = Vec::new();
    let mut next = 0;
    for (at, replacement) in edits {
        marked.push_str(&source[next..at]);
        next = at;
        if let Some(ch) = replacement {
            if ch == '<' {
                // A literal odd backslash would escape our inserted '<'.
                // Pair it so CommonMark emits the original literal slash.
                if preceding_backslashes(source, at) % 2 != 0 {
                    marked.push('\\');
                }
                bare_urls.push(BareUrl {
                    start: marked.len(),
                    normalized: decoded_urls.next().unwrap(),
                });
            }
            marked.push(ch);
        } else {
            next += 1;
        }
    }
    marked.push_str(&source[next..]);
    Some(commonmark_text(&marked, Some(&bare_urls), rich))
}

fn source_boundary(mapping: &[(Range<usize>, Range<usize>)], at: usize) -> Option<usize> {
    let index = mapping.partition_point(|(decoded, _)| decoded.start <= at);
    let Some((decoded, source)) = index.checked_sub(1).map(|index| &mapping[index]) else {
        return Some(at);
    };
    if at == decoded.start {
        return Some(source.start);
    }
    // A source reference is indivisible. Leave it to CommonMark if the
    // scanner placed a boundary between two scalars it decodes to.
    Some(source.end + at.checked_sub(decoded.end)?)
}

fn normalize_bare_url(
    source: &str,
    url: Range<usize>,
    references: &[(Range<usize>, CowStr<'_>)],
    slashes: &[usize],
) -> NormalizedUrl {
    // Reuse the original CommonMark text events. Inserting their values into
    // the markup would let decoded angle brackets or whitespace change syntax.
    let mut replacements: Vec<_> = references
        .iter()
        .map(|(range, value)| (range.clone(), Some(value.as_ref())))
        .chain(slashes.iter().map(|&at| (at..at + 1, None)))
        .collect();
    replacements.sort_unstable_by_key(|(range, _)| range.start);
    let mut display = String::with_capacity(url.len());
    let mut literals = Vec::new();
    let mut units = 0;
    let mut next = url.start;
    for (range, replacement) in replacements {
        let prefix = &source[next..range.start];
        display.push_str(prefix);
        units += prefix.encode_utf16().count();
        if let Some(replacement) = replacement {
            display.push_str(replacement);
            units += replacement.encode_utf16().count();
        } else if let Some(length) =
            escaped_delimiter(source, range.end, &source[range.end..url.end])
        {
            literals.push(units..units + length);
        }
        next = range.end;
    }
    display.push_str(&source[next..url.end]);
    NormalizedUrl { display, literals }
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
        || preceding_backslashes(source, at) % 2 == 0
    {
        return None;
    }
    Some(first)
}

fn preceding_backslashes(source: &str, at: usize) -> usize {
    source.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
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
