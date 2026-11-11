//! Bounded lexical evidence from one authorized Notion block page.
//! This is presentation only: provider text and execution receipts stay unchanged.
use super::resource_adapter::{MAX_SOURCE_BYTES, ResourceRead};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

const MAX_WIRE_BYTES: usize = 6_000;
const MAX_BLOCK_TEXT: usize = 1_200;
const MAX_SECTIONS: usize = 4;

pub fn validate_query(value: Option<&Value>) -> Result<Option<&str>, &'static str> {
    match value {
        None => Ok(None),
        Some(Value::String(query)) if !query.trim().is_empty() && query.chars().count() <= 256 => {
            Ok(Some(query.trim()))
        }
        _ => Err("result_query must contain 1–256 characters"),
    }
}

fn excerpt(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn matching_excerpt(text: &str, matcher: Option<&Regex>) -> (String, usize) {
    let mut start = matcher
        .and_then(|matcher| matcher.find(text))
        .map_or(0, |found| {
            // Keep context around a late match without changing source offsets.
            if found.end() <= MAX_BLOCK_TEXT {
                0
            } else {
                found.start().saturating_sub(200)
            }
        });
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    (excerpt(&text[start..], MAX_BLOCK_TEXT), start)
}

fn rich_text(runs: &Value) -> String {
    runs.as_array()
        .into_iter()
        .flatten()
        .map(|run| {
            run["plain_text"]
                .as_str()
                .or_else(|| run.pointer("/text/content").and_then(Value::as_str))
                .unwrap_or("")
        })
        .collect()
}

fn block_text(block: &Value, kind: &str) -> String {
    let data = &block[kind];
    match kind {
        "table_row" => data["cells"]
            .as_array()
            .into_iter()
            .flatten()
            .map(rich_text)
            .collect::<Vec<_>>()
            .join(" | "),
        "child_page" | "child_database" => data["title"].as_str().unwrap_or("").into(),
        "image" | "file" | "pdf" | "video" | "audio" => rich_text(&data["caption"]),
        "bookmark" | "embed" | "link_preview" => data["url"].as_str().unwrap_or("").into(),
        _ => rich_text(&data["rich_text"]),
    }
}

struct Section {
    heading: String,
    parent_heading: String,
    id: String,
    blocks: Vec<Value>,
    searchable: String,
}

fn terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.chars().count() < 2
            || matches!(
                word.as_str(),
                "the"
                    | "and"
                    | "are"
                    | "can"
                    | "you"
                    | "for"
                    | "what"
                    | "does"
                    | "is"
                    | "to"
                    | "of"
                    | "in"
                    | "me"
                    | "it"
                    | "at"
                    | "please"
                    | "would"
                    | "like"
            )
            || terms.contains(&word)
        {
            continue;
        }
        terms.push(word);
        if terms.len() == 16 {
            break;
        }
    }
    terms
}

fn score(section: &Section, terms: &[String]) -> usize {
    let heading = section.heading.to_lowercase();
    let parent = section.parent_heading.to_lowercase();
    terms
        .iter()
        .map(|term| {
            usize::from(section.searchable.contains(term))
                + 5 * usize::from(heading.contains(term))
                + 2 * usize::from(parent.contains(term))
        })
        .sum()
}

fn wire_bytes(value: &Value) -> usize {
    // NyxAgent receives MCP text as an escaped string. Budget that representation.
    serde_json::to_string(&value.to_string())
        .map(|text| text.len())
        .unwrap_or(usize::MAX)
}

/// Uses the same server-verified source provenance as image refs. Never interpret
/// lookalike JSON from arbitrary services, failures, or caller-provided bodies.
pub fn present(read: &ResourceRead, text: &str, query: Option<&str>) -> Option<String> {
    if read.source.adapter != "notion.v1"
        || read.arguments.get("block_id").is_none()
        || text.len() > MAX_SOURCE_BYTES
    {
        return None;
    }
    let root: Value = serde_json::from_str(text).ok()?;
    let blocks = root["results"].as_array()?;
    if root["object"] != "list"
        || root["has_more"].as_bool().is_none()
        || blocks.len() > 100
        || blocks.iter().any(|block| {
            block["object"] != "block"
                || block["type"].as_str().is_none()
                || block["id"].as_str().is_none()
        })
    {
        return None;
    }
    let mut sections = Vec::<Section>::new();
    let query_terms = query.map(terms);
    let matcher = query_terms
        .as_ref()
        .filter(|terms| !terms.is_empty())
        .and_then(|terms| {
            RegexBuilder::new(
                &terms
                    .iter()
                    .map(|term| regex::escape(term))
                    .collect::<Vec<_>>()
                    .join("|"),
            )
            .case_insensitive(true)
            .build()
            .ok()
        });
    let mut parent_heading = String::new();
    for block in blocks {
        let kind = block["type"].as_str()?;
        let full_text = block_text(block, kind);
        let heading = matches!(kind, "heading_1" | "heading_2" | "heading_3");
        if heading || kind == "table_row" || sections.is_empty() {
            if matches!(kind, "heading_1" | "heading_2") {
                parent_heading = full_text.clone();
            }
            sections.push(Section {
                heading: if heading || kind == "table_row" {
                    excerpt(&full_text, 200)
                } else {
                    String::new()
                },
                parent_heading: excerpt(&parent_heading, 200),
                id: block["id"].as_str()?.into(),
                blocks: Vec::new(),
                searchable: String::new(),
            });
        }
        let section = sections.last_mut()?;
        section.searchable.push_str(&full_text.to_lowercase());
        section.searchable.push('\n');
        let data = &block[kind];
        let links: Vec<&str> = data["rich_text"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|run| run["href"].as_str())
            .collect();
        let (text, offset) = matching_excerpt(&full_text, matcher.as_ref());
        let mut evidence = json!({
            "id":block["id"], "type":kind, "last_edited_time":block["last_edited_time"],
            "has_children":block["has_children"].as_bool().unwrap_or(false),
            "text":text, "text_offset_bytes":offset,
            "text_truncated":offset > 0 || full_text.len() > MAX_BLOCK_TEXT,
        });
        if !links.is_empty() {
            evidence["links"] = json!(
                links
                    .iter()
                    .take(8)
                    .filter(|url| url.len() <= 512)
                    .copied()
                    .collect::<Vec<_>>()
            );
            evidence["links_truncated"] =
                json!(links.len() > 8 || links.iter().any(|url| url.len() > 512));
        }
        if kind == "to_do" {
            evidence["checked"] = data["checked"].clone();
        }
        if kind == "synced_block" {
            evidence["synced_from"] = data["synced_from"].clone();
        }
        section.blocks.push(evidence);
    }
    let mut ranked: Vec<_> = sections
        .iter()
        .enumerate()
        .filter_map(|(index, section)| {
            let relevance = query_terms
                .as_ref()
                .map_or(1, |terms| score(section, terms));
            (relevance > 0).then_some((index, relevance))
        })
        .collect();
    if query.is_some() {
        ranked.sort_by_key(|(index, relevance)| (std::cmp::Reverse(*relevance), *index));
    }
    let block_id = read.arguments["block_id"].as_str()?;
    let mut result = json!({
        "source": {"block_id":block_id,"url":format!("https://www.notion.so/{}",block_id.replace('-',""))},
        "scope":"returned_block_page", "fetched_at":chrono::Utc::now().to_rfc3339(),
        "has_more":root["has_more"], "next_cursor":root["next_cursor"],
        "query":query, "matched_sections":ranked.len(), "returned_sections":0,
        "omitted_blocks":blocks.len(), "sections":[],
        "instructions":"Evidence from this live response only, not a workspace search. Use the returned cursor for further pages, block IDs with has_children for nested content, or result_query on nyx__call_tool for relevant sections. Omitted or truncated content is not evidence of absence. Image refs above require nyx__resource_get(output=attach) to display; links alone do not attach images.",
    });
    if query.is_none() {
        // The index leads the evidence so later FAQ headings survive client truncation.
        let mut index = Vec::<Value>::new();
        for section in sections
            .iter()
            .filter(|section| !section.heading.is_empty())
        {
            let entry = json!(excerpt(&section.heading, 160));
            index.push(entry);
            result["section_index"] = json!(index);
            if wire_bytes(&result) > MAX_WIRE_BYTES * 2 / 3 {
                index.pop();
                break;
            }
        }
        result["section_index"] = json!(index);
        result["section_index_omitted"] = json!(
            sections
                .iter()
                .filter(|section| !section.heading.is_empty())
                .count()
                - index.len()
        );
    }
    let mut returned_blocks = 0;
    let mut selected = Vec::<Value>::new();
    for (index, _) in ranked.iter().take(MAX_SECTIONS) {
        let section = &sections[*index];
        let mut item = json!({"heading":section.heading,"parent_heading":section.parent_heading,
            "id":section.id,"blocks":[],"omitted_blocks":section.blocks.len()});
        let mut evidence = Vec::<Value>::new();
        let mut block_order: Vec<_> = section.blocks.iter().enumerate().collect();
        if let Some(matcher) = &matcher {
            // A heading can contain many paragraphs. Surface matching blocks
            // before unrelated early paragraphs consume the output budget.
            block_order.sort_by_key(|(index, block)| {
                (
                    std::cmp::Reverse(matcher.is_match(block["text"].as_str().unwrap_or(""))),
                    *index,
                )
            });
        }
        for (_, block) in block_order {
            evidence.push(block.clone());
            item["blocks"] = json!(evidence);
            item["omitted_blocks"] = json!(section.blocks.len() - evidence.len());
            let mut trial = selected.clone();
            trial.push(item.clone());
            result["sections"] = json!(trial);
            if wire_bytes(&result) > MAX_WIRE_BYTES - 128 {
                evidence.pop();
                break;
            }
        }
        item["blocks"] = json!(evidence);
        item["omitted_blocks"] = json!(section.blocks.len() - evidence.len());
        if evidence.is_empty() {
            break;
        }
        returned_blocks += evidence.len();
        selected.push(item);
    }
    result["returned_sections"] = json!(selected.len());
    result["omitted_blocks"] = json!(blocks.len() - returned_blocks);
    result["sections"] = json!(selected);
    (wire_bytes(&result) <= MAX_WIRE_BYTES).then(|| result.to_string())
}

#[cfg(test)]
#[path = "notion_evidence_tests.rs"]
mod tests;
