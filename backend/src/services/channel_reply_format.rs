//! Discord and WhatsApp syntax, after CommonMark display text is extracted.
use super::{TextLimit, expand_selected_links, supported_link_target, telegram_text};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use serde_json::Value;
use std::ops::Range;

fn urls(text: &str) -> Vec<Range<usize>> {
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    finder
        .links(text)
        .filter(|link| url::Url::parse(link.as_str()).is_ok())
        .map(|link| link.start()..link.end())
        .collect()
}

pub(super) fn parse(source: &str) -> (String, Vec<Value>) {
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    );
    let definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, def)| def.span.clone())
        .collect();
    let mut code = Vec::new();
    let mut autolinks = Vec::new();
    let mut links = definitions;
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Code(_) | Event::Start(Tag::CodeBlock(_)) => code.push(range),
            Event::Start(Tag::Link {
                link_type: LinkType::Autolink | LinkType::Email,
                ..
            }) => autolinks.push(range),
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => links.push(range),
            _ => {}
        }
    }
    let mut marked = String::with_capacity(source.len());
    let mut next = 0;
    for range in urls(source) {
        let overlaps = |p: &Range<usize>| p.start < range.end && range.start < p.end;
        if code.iter().any(overlaps) || autolinks.iter().any(overlaps) {
            continue;
        }
        marked.push_str(&source[next..range.start]);
        if links.iter().any(overlaps) {
            // CommonMark decodes entities in explicit destinations, unlike autolinks.
            marked.push_str(&source[range.clone()].replace('&', "&amp;"));
        } else {
            marked.push('<');
            marked.push_str(&source[range.clone()]);
            marked.push('>');
        }
        next = range.end;
    }
    marked.push_str(&source[next..]);
    let (body, mut entities) = telegram_text(&marked);
    for entity in &mut entities {
        if entity["type"] == "text_link" {
            let range = byte_range(&body, entity);
            if entity["url"].as_str() == Some(&body[range]) {
                entity["type"] = "url".into();
            }
        }
    }
    (body, entities)
}

pub(super) fn byte_range(text: &str, entity: &Value) -> Range<usize> {
    let offset = entity["offset"].as_u64().unwrap_or(0) as usize;
    let end = offset + entity["length"].as_u64().unwrap_or(0) as usize;
    let mut units = 0;
    let mut start_byte = text.len();
    let mut end_byte = text.len();
    for (byte, ch) in text.char_indices() {
        if units == offset {
            start_byte = byte;
        }
        if units == end {
            end_byte = byte;
            break;
        }
        units += ch.len_utf16();
    }
    start_byte..end_byte
}

pub(super) fn expand_links(
    body: &mut String,
    entities: &mut Vec<Value>,
    limit: TextLimit,
    platform: &str,
) {
    let links = entities
        .iter()
        .filter(|e| e["type"] == "text_link")
        .filter(|e| {
            let target = e["url"].as_str().unwrap_or_default();
            platform == "whatsapp"
                || !supported_link_target(target)
                || target.contains(['(', ')', '<', '>', '|', '\\'])
                || target.chars().any(char::is_whitespace)
                || limit.measure(target) > limit.budget() / 2
                || entities.iter().any(|other| {
                    other != *e && other["type"] == "text_link" && {
                        let a = byte_range(body, e);
                        let b = byte_range(body, other);
                        a.start < b.end && b.start < a.end
                    }
                })
        })
        .cloned()
        .collect();
    expand_selected_links(body, entities, links, true);
}

struct Span<'a> {
    range: Range<usize>,
    kind: &'a str,
    target: &'a str,
}

fn code_markers(text: &str, block: bool, whatsapp: bool) -> (String, String) {
    if whatsapp {
        if !block && !text.contains('`') {
            return ("`".into(), "`".into());
        }
        if !text.contains("```") && !text.starts_with('`') && !text.ends_with('`') {
            return ("```".into(), "```".into());
        }
        return (String::new(), String::new());
    }
    let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(if block {
        longest.max(2) + 1
    } else {
        longest + 1
    });
    if block {
        (format!("{fence}\n"), format!("\n{fence}"))
    } else if text.starts_with('`') || text.ends_with('`') {
        (format!("{fence} "), format!(" {fence}"))
    } else {
        (fence.clone(), fence)
    }
}

fn markers(span: &Span<'_>, text: &str, whatsapp: bool) -> (String, String) {
    let both = |s: &str| (s.to_owned(), s.to_owned());
    match span.kind {
        "bold" => both(if whatsapp { "*" } else { "**" }),
        "italic" => both(if whatsapp { "_" } else { "*" }),
        "strikethrough" => both(if whatsapp { "~" } else { "~~" }),
        "code" | "pre" => code_markers(&text[span.range.clone()], span.kind == "pre", whatsapp),
        "text_link" => ("[".into(), format!("]({})", span.target)),
        "url"
            if !whatsapp
                && &text[span.range.clone()] == span.target
                && span.target.contains(['(', ')'])
                && !span.target.contains(['<', '>']) =>
        {
            both_angle()
        }
        _ => (String::new(), String::new()),
    }
}

fn both_angle() -> (String, String) {
    ("<".into(), ">".into())
}

pub(super) fn render(text: &str, entities: &[Value], platform: &str) -> String {
    let whatsapp = platform == "whatsapp";
    let mut spans: Vec<_> = entities
        .iter()
        .map(|e| Span {
            range: byte_range(text, e),
            kind: e["type"].as_str().unwrap_or_default(),
            target: e["url"].as_str().unwrap_or_default(),
        })
        .filter(|s| !s.range.is_empty())
        .collect();
    spans.sort_by_key(|s| {
        (
            s.range.start,
            std::cmp::Reverse(s.range.end),
            s.kind == "url",
        )
    });
    let mut protected = vec![false; text.len()];
    for span in &spans {
        if matches!(span.kind, "code" | "pre" | "url") {
            protected[span.range.clone()].fill(true);
        }
    }
    let mut unsafe_bytes = literal_syntax(text, &protected, whatsapp);
    for span in &spans {
        if !whatsapp && span.kind == "text_link" {
            for (i, ch) in text[span.range.clone()].char_indices() {
                if matches!(ch, '[' | ']') {
                    unsafe_bytes[span.range.start + i] = true;
                }
            }
        }
        let delimiter = match span.kind {
            "bold" | "italic" if !whatsapp => '*',
            "bold" => '*',
            "italic" => '_',
            "strikethrough" => '~',
            _ => continue,
        };
        for byte in [
            span.range.start.saturating_sub(1),
            span.range.start,
            span.range.end.saturating_sub(1),
            span.range.end,
        ] {
            if text.as_bytes().get(byte) == Some(&(delimiter as u8)) && !protected[byte] {
                unsafe_bytes[byte] = true;
            }
        }
    }
    let framed_urls: Vec<_> = spans
        .iter()
        .filter(|s| {
            s.kind == "url"
                && !whatsapp
                && text[s.range.clone()] == *s.target
                && !s.target.contains(['<', '>'])
                && (s.target.contains(['(', ')'])
                    || spans.iter().any(|other| {
                        matches!(other.kind, "bold" | "italic" | "strikethrough")
                            && other.range.end == s.range.end
                    }))
        })
        .map(|s| s.range.clone())
        .collect();
    let delimiters = |span: &Span<'_>| {
        if span.kind == "url" && framed_urls.contains(&span.range) {
            both_angle()
        } else {
            markers(span, text, whatsapp)
        }
    };
    let mut result = String::with_capacity(text.len());
    for (byte, ch) in text.char_indices() {
        for span in spans.iter().rev().filter(|s| s.range.end == byte) {
            result.push_str(&delimiters(span).1);
        }
        for span in spans.iter().filter(|s| s.range.start == byte) {
            result.push_str(&delimiters(span).0);
        }
        if unsafe_bytes[byte] && !protected[byte] {
            if whatsapp {
                if ch != '`' {
                    let continues = |at: usize| {
                        unsafe_bytes.get(at) == Some(&true)
                            && !protected[at]
                            && text.as_bytes()[at] != b'`'
                    };
                    if byte == 0 || !continues(byte - 1) {
                        result.push('`');
                    }
                    result.push(ch);
                    if !continues(byte + ch.len_utf8()) {
                        result.push('`');
                    }
                } else {
                    // WhatsApp has no documented escape for colliding backticks.
                    result.push(ch);
                }
            } else {
                result.push('\\');
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    for span in spans.iter().rev().filter(|s| s.range.end == text.len()) {
        result.push_str(&delimiters(span).1);
    }
    result
}

fn word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn literal_syntax(text: &str, protected: &[bool], whatsapp: bool) -> Vec<bool> {
    let mut marked = vec![false; text.len()];
    let chars: Vec<_> = text.char_indices().collect();
    let previous = |i: usize| i.checked_sub(1).map(|p| chars[p].1);
    let following = |i: usize| chars.get(i + 1).map(|(_, c)| *c);
    for (i, &(byte, ch)) in chars.iter().enumerate() {
        if protected[byte] {
            continue;
        }
        if !whatsapp
            && ch == '\\'
            && following(i).is_some_and(|c| !c.is_ascii_alphanumeric() && !c.is_whitespace())
        {
            marked[byte] = true;
        }
        if !matches!(ch, '*' | '_' | '~' | '|' | '`') || (whatsapp && ch == '|') {
            continue;
        }
        if previous(i) == Some(ch) && !protected[chars[i - 1].0] {
            continue;
        }
        let run = chars[i..]
            .iter()
            .take_while(|(p, c)| *c == ch && !protected[*p])
            .count();
        let width = if ch == '`' {
            run
        } else if !whatsapp && (matches!(ch, '~' | '|') || run >= 2) {
            2
        } else {
            1
        };
        if run < width {
            continue;
        }
        let content = i + run;
        let edge_spaces = !whatsapp && width == 2 && matches!(ch, '*' | '_');
        if chars.get(content).is_none_or(|(_, c)| c.is_whitespace()) && ch != '`' && !edge_spaces {
            continue;
        }
        if ch == '_' && width == 1 && previous(i).is_some_and(word) {
            continue;
        }
        for j in content..chars.len() {
            let (end, c) = chars[j];
            if c == '\n' && whatsapp {
                break;
            }
            if c != ch || protected[end] {
                continue;
            }
            let close = chars[j..]
                .iter()
                .take_while(|(p, c)| *c == ch && !protected[*p])
                .count();
            if close < width || (ch == '`' && close != run) {
                continue;
            }
            if ch != '`' && !edge_spaces && previous(j).is_none_or(char::is_whitespace) {
                continue;
            }
            if ch == '_' && width == 1 && chars.get(j + close).is_some_and(|(_, c)| word(*c)) {
                continue;
            }
            for &(p, _) in &chars[i..i + run] {
                marked[p] = true;
            }
            for &(p, _) in &chars[j..j + close] {
                marked[p] = true;
            }
            break;
        }
    }
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let leading = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[leading..];
        let start = offset + leading;
        if start < text.len() && !protected[start] {
            let bullet = rest.starts_with("- ") || rest.starts_with("* ");
            let quote = rest.starts_with("> ") || (!whatsapp && rest.starts_with(">>> "));
            let hashes = rest.bytes().take_while(|b| *b == b'#').count();
            let heading = !whatsapp
                && leading == 0
                && ((1..=3).contains(&hashes) && rest.as_bytes().get(hashes) == Some(&b' ')
                    || rest.starts_with("-# "));
            if bullet || quote || heading {
                marked[start] = true;
            }
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && rest[digits..].starts_with(". ") && !protected[start + digits] {
                marked[start + digits] = true;
            }
        }
        offset += line.len();
    }
    // A literal Markdown link can be introduced by escaped source brackets.
    if !whatsapp {
        for (start, _) in text.match_indices('[') {
            if protected[start] {
                continue;
            }
            if let Some(end) = text[start..].find("](") {
                let target_start = start + end + 2;
                if let Some(end) = text[target_start..].find(')')
                    && url::Url::parse(&text[target_start..target_start + end]).is_ok()
                {
                    marked[start] = true;
                }
            }
        }
    }
    marked
}
