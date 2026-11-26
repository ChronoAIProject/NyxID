//! Slack rich text for assistant parts that need literal punctuation.
use super::{slack, supported_link_target};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn needs_block(text: &str, entities: &[Value]) -> bool {
    text.contains('`') || entities.iter().any(|e| e["type"] == "slack_literal")
}

pub(super) fn supported_target(target: &str) -> bool {
    supported_link_target(target)
        && url::Url::parse(target)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

pub(super) fn fallback_scalars(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch {
            '&' => 5,
            '<' | '>' => 4,
            _ => 1,
        })
        .sum()
}

pub(super) struct Rendered {
    pub blocks: Value,
    pub fallback: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run<'a> {
    preformatted: bool,
    style: [bool; 4],
    target: Option<&'a str>,
}

impl Run<'_> {
    fn element(self, text: &str) -> Value {
        let mut element = if let Some(url) = self.target {
            json!({"type":"link","url":url,"text":text})
        } else {
            json!({"type":"text","text":text})
        };
        let style: Map<_, _> = ["bold", "italic", "strike", "code"]
            .into_iter()
            .zip(self.style)
            .filter(|(_, enabled)| *enabled)
            .map(|(name, _)| (name.into(), true.into()))
            .collect();
        if !style.is_empty() {
            element["style"] = Value::Object(style);
        }
        element
    }
}

#[derive(Clone, Copy)]
enum Span<'a> {
    Style(usize),
    Preformatted,
    Link(Option<&'a str>),
}

pub(super) fn render(text: &str, entities: &[Value]) -> Rendered {
    let bytes: Vec<_> = text
        .char_indices()
        .flat_map(|(byte, ch)| std::iter::repeat_n(byte, ch.len_utf16()))
        .chain(std::iter::once(text.len()))
        .collect();
    let mut code: Vec<_> = entities
        .iter()
        .filter(|e| matches!(e["type"].as_str(), Some("code" | "pre")))
        .filter_map(|e| {
            Some((
                e["offset"].as_u64()? as usize,
                e["length"].as_u64()? as usize,
            ))
        })
        .map(|(start, length)| start..start + length)
        .collect();
    code.sort_unstable_by_key(|span| span.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for span in code {
        if let Some(last) = merged.last_mut()
            && last.end == span.start
        {
            last.end = span.end;
        } else {
            merged.push(span);
        }
    }

    let mut spans = Vec::new();
    let mut boundaries = Vec::new();
    let mut suffixes: BTreeMap<usize, Vec<(&str, bool)>> = BTreeMap::new();
    for entity in entities {
        let Some(start) = entity["offset"].as_u64().map(|offset| offset as usize) else {
            continue;
        };
        let end = start + entity["length"].as_u64().unwrap_or_default() as usize;
        let span = match entity["type"].as_str().unwrap_or_default() {
            "bold" => Span::Style(0),
            "italic" => Span::Style(1),
            "strikethrough" => Span::Style(2),
            "code" => Span::Style(3),
            "pre" => Span::Preformatted,
            "text_link" | "url" => {
                let Some(target) = entity["url"].as_str() else {
                    continue;
                };
                let clickable = supported_target(target);
                let label = &text[bytes[start]..bytes[end]];
                let tail = merged.partition_point(|span| span.start < end);
                let last_label_end = merged[..tail]
                    .last()
                    .filter(|span| span.end >= end)
                    .map_or(end, |span| span.start);
                // The shared renderer leaves wholly code labels unlinked.
                if last_label_end <= start {
                    continue;
                }
                if label != target
                    && (entity["type"] != "url" || end - start == target.encode_utf16().count())
                {
                    let at = if clickable { last_label_end } else { end };
                    suffixes.entry(at).or_default().push((target, !clickable));
                }
                // A clipped bare URL cannot represent the original destination
                // as independent clickable URLs.
                Span::Link(
                    (clickable && (entity["type"] != "url" || label == target)).then_some(target),
                )
            }
            _ => continue,
        };
        boundaries.extend([(start, spans.len(), true), (end, spans.len(), false)]);
        spans.push((end - start, span));
    }
    boundaries.sort_unstable();

    let mut render = BlockBuilder::default();
    let mut styles = [0_usize; 4];
    let mut preformatted = 0;
    let mut links = BTreeSet::new();
    let mut range_start = 0;
    let mut utf16 = 0;
    let mut boundaries = boundaries.into_iter().peekable();
    let mut prior: Option<Run<'_>> = None;
    for (byte, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        while let Some((_, index, opening)) = boundaries.next_if(|edge| edge.0 == utf16) {
            let (length, span) = spans[index];
            let count = match span {
                Span::Style(style) => &mut styles[style],
                Span::Preformatted => &mut preformatted,
                Span::Link(_) => {
                    let key = (length, index);
                    if opening {
                        links.insert(key);
                    } else {
                        links.remove(&key);
                    }
                    continue;
                }
            };
            if opening {
                *count += 1;
            } else {
                *count -= 1;
            }
        }
        let style = styles.map(|count| count > 0);
        let target = links.first().and_then(|&(_, index)| match spans[index].1 {
            Span::Link(target) if !style[3] => target,
            _ => None,
        });
        let current = Run {
            preformatted: preformatted > 0,
            style,
            target,
        };
        let suffix = suffixes.get(&utf16);
        let visible_suffix = suffix.is_some_and(|items| items.iter().any(|(_, visible)| *visible));
        if (byte == text.len() || prior != Some(current) || visible_suffix)
            && let Some(prior) = prior
        {
            render.push(prior, &text[range_start..byte]);
            range_start = byte;
        }
        if let Some(suffix) = suffix {
            for &(target, visible) in suffix {
                let extra = format!(" ({target})");
                extra
                    .chars()
                    .for_each(|ch| slack::push_text_char(&mut render.fallback, ch));
                if visible && let Some(prior) = prior {
                    let run = Run {
                        target: None,
                        ..prior
                    };
                    render.push(run, &extra);
                }
            }
        }
        prior = Some(current);
        if byte < text.len() {
            slack::push_text_char(&mut render.fallback, ch);
            utf16 += ch.len_utf16();
        }
    }
    render.finish()
}

#[derive(Default)]
struct BlockBuilder {
    sections: Vec<Value>,
    inline: Vec<Value>,
    preformatted: bool,
    fallback: String,
}

impl BlockBuilder {
    fn push(&mut self, run: Run<'_>, text: &str) {
        if text.is_empty() {
            return;
        }
        if run.preformatted != self.preformatted {
            self.close_section();
            self.preformatted = run.preformatted;
        }
        self.inline.push(run.element(text));
    }

    fn close_section(&mut self) {
        if !self.inline.is_empty() {
            let kind = if self.preformatted {
                "rich_text_preformatted"
            } else {
                "rich_text_section"
            };
            self.sections
                .push(json!({"type":kind,"elements":std::mem::take(&mut self.inline)}));
        }
    }

    fn finish(mut self) -> Rendered {
        self.close_section();
        Rendered {
            blocks: json!([{"type":"rich_text","elements":self.sections}]),
            fallback: self.fallback,
        }
    }
}
