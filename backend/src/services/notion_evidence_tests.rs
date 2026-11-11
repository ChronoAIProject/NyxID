use super::*;
use crate::models::assistant_resource::ResourceSource;

fn read() -> ResourceRead {
    ResourceRead {
        source: ResourceSource {
            adapter: "notion.v1".into(),
            service_id: "connection".into(),
            endpoint_id: "endpoint".into(),
            generation: 1,
            endpoint_digest: "contract".into(),
            authority_digest: "authority".into(),
            tool_name: "renamed-notion__read".into(),
        },
        arguments: json!({"block_id":"3fb20833-3b58-80e5-9f6f-f891a03c1831"}),
        candidates: vec![],
    }
}

fn block(kind: &str, text: &str) -> Value {
    json!({"object":"block","id":uuid::Uuid::new_v4().to_string(),"type":kind,
        "last_edited_time":"2026-10-21T00:00:00Z","has_children":false,
        kind:{"rich_text":[{"plain_text":text}]},
        "created_by":{"id":"irrelevant-author-metadata"}})
}

fn page(blocks: Vec<Value>) -> String {
    json!({"object":"list","results":blocks,"has_more":true,"next_cursor":"provider-cursor"})
        .to_string()
}

fn present_json(text: &str, query: Option<&str>) -> Value {
    serde_json::from_str(&present(&read(), text, query).unwrap()).unwrap()
}

#[test]
fn notion_evidence_finds_late_sections_and_keeps_source_revision_and_cursor() {
    let mut blocks = vec![block("heading_2", "Opening hours")];
    blocks.extend((0..70).map(|_| block("paragraph", "Unrelated early content")));
    blocks.extend([
        block("heading_2", "Menu"),
        block("heading_3", "MENU-01 | Signature dishes"),
        block("paragraph", "Coffee pork ribs and Moonlight Horfun."),
        block(
            "heading_3",
            "MENU-03 | Vegetarian and allergy-sensitive diners",
        ),
        block(
            "paragraph",
            "Vegetable dishes may contain animal stock. Staff must confirm dietary suitability.",
        ),
    ]);
    let text = page(blocks);
    let original = text.clone();
    let evidence = present_json(&text, Some("signature dishes vegetarian"));
    let sections = evidence["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 2);
    let rendered = evidence.to_string();
    assert!(rendered.contains("Coffee pork ribs"));
    assert!(rendered.contains("Staff must confirm"));
    assert!(!rendered.contains("Unrelated early content"));
    assert!(!rendered.contains("irrelevant-author-metadata"));
    assert!(rendered.contains("2026-10-21T00:00:00Z"));
    assert_eq!(evidence["has_more"], true);
    assert_eq!(evidence["next_cursor"], "provider-cursor");
    assert_eq!(evidence["scope"], "returned_block_page");
    assert!(
        evidence["source"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://www.notion.so/")
    );
    assert_eq!(text, original);
}

#[test]
fn notion_evidence_index_exposes_late_headings_without_claiming_full_content() {
    let text = page(vec![
        block("heading_2", "Company"),
        block("paragraph", &"Large text. ".repeat(300)),
        block("heading_2", "Dietary policies"),
        block("paragraph", "Ask staff."),
    ]);
    let evidence = present_json(&text, None);
    assert!(
        evidence["section_index"]
            .to_string()
            .contains("Dietary policies")
    );
    assert_eq!(evidence["sections"][0]["blocks"][1]["text_truncated"], true);
    assert!(wire_bytes(&evidence) <= MAX_WIRE_BYTES);
}

#[test]
fn notion_evidence_returns_late_matches_with_utf8_safe_source_offsets() {
    let prefix = "普通资料 unrelated background。".repeat(100);
    let answer = "Birthday groups need staff confirmation. Capacity is not guaranteed.";
    let long_text = format!("{prefix}{answer}{}", "后续资料。".repeat(100));
    let mut blocks = vec![block("heading_2", "Dining information")];
    blocks.extend((0..60).map(|_| block("paragraph", "General background.")));
    let matching_block = block("paragraph", &long_text);
    let matching_id = matching_block["id"].clone();
    blocks.push(matching_block);
    let evidence = present_json(&page(blocks), Some("BIRTHDAY groups"));
    let first = &evidence["sections"][0]["blocks"][0];
    assert_eq!(first["id"], matching_id);
    let excerpt = first["text"].as_str().unwrap();
    assert!(excerpt.contains(answer), "{excerpt}");
    let offset = first["text_offset_bytes"].as_u64().unwrap() as usize;
    assert!(offset > 0);
    assert!(long_text[offset..].starts_with(excerpt));
    assert_eq!(first["text_truncated"], true);
    assert!(wire_bytes(&evidence) <= MAX_WIRE_BYTES);
}

#[test]
fn notion_evidence_no_match_never_returns_unrelated_facts() {
    let evidence = present_json(
        &page(vec![block("paragraph", "Closed on Tuesday")]),
        Some("parking"),
    );
    assert_eq!(evidence["sections"], json!([]));
    assert_eq!(evidence["matched_sections"], 0);
    assert_eq!(evidence["omitted_blocks"], 1);
    assert_eq!(evidence["next_cursor"], "provider-cursor");
}

#[test]
fn notion_evidence_preserves_links_tables_nested_reads_and_checkboxes() {
    let mut paragraph = block("paragraph", "Order online");
    paragraph["paragraph"]["rich_text"][0]["href"] = json!("https://example.com/menu");
    let mut table = block("table", "");
    table["has_children"] = json!(true);
    let mut row = block("table_row", "");
    row["table_row"]["cells"] = json!([[{"plain_text":"Outlet"}],[{"plain_text":"Alexandra"}]]);
    let mut todo = block("to_do", "Staff verified");
    todo["to_do"]["checked"] = json!(true);
    let evidence = present_json(&page(vec![paragraph, table, row, todo]), None);
    let blocks: Vec<_> = evidence["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|section| section["blocks"].as_array().unwrap())
        .collect();
    assert_eq!(blocks[0]["links"][0], "https://example.com/menu");
    assert_eq!(blocks[1]["has_children"], true);
    assert_eq!(blocks[2]["text"], "Outlet | Alexandra");
    assert_eq!(blocks[3]["checked"], true);
}

#[test]
fn notion_evidence_bounds_unicode_quotes_and_untrusted_long_sections() {
    let text = page(
        (0..100)
            .map(|i| {
                block(
                    if i % 2 == 0 { "heading_3" } else { "paragraph" },
                    &"素食\"\\\n".repeat(400),
                )
            })
            .collect(),
    );
    for query in [None, Some("素食")] {
        let evidence = present_json(&text, query);
        assert!(wire_bytes(&evidence) <= MAX_WIRE_BYTES);
        assert!(evidence["omitted_blocks"].as_u64().unwrap() > 0);
    }
}

#[test]
fn notion_evidence_finds_late_table_rows_and_omits_overlong_links() {
    let mut blocks: Vec<_> = (0..90)
        .map(|i| {
            let mut row = block("table_row", "");
            row["table_row"]["cells"] =
                json!([[{"plain_text":format!("Dish {i}")}],[{"plain_text":"12 SGD"}]]);
            row
        })
        .collect();
    let mut row = block("table_row", "");
    row["table_row"]["cells"] = json!([[{"plain_text":"Coffee pork ribs"}],[{"plain_text":"Staff must confirm current price"}]]);
    blocks.push(row);
    let evidence = present_json(&page(blocks), Some("coffee pork ribs"));
    assert_eq!(evidence["matched_sections"], 1);
    assert!(
        evidence
            .to_string()
            .contains("Staff must confirm current price")
    );
    assert!(!evidence.to_string().contains("Dish 0"));

    let mut paragraph = block("paragraph", "Source");
    paragraph["paragraph"]["rich_text"][0]["href"] =
        json!(format!("https://example.com/{}", "x".repeat(600)));
    let evidence = present_json(&page(vec![paragraph]), None);
    assert_eq!(evidence["sections"][0]["blocks"][0]["links"], json!([]));
    assert_eq!(
        evidence["sections"][0]["blocks"][0]["links_truncated"],
        true
    );
}

#[test]
fn notion_evidence_requires_verified_adapter_and_valid_bounded_block_page() {
    let mut read = read();
    let text = page(vec![block("paragraph", "hello")]);
    read.source.adapter = "lookalike".into();
    assert!(present(&read, &text, None).is_none());
    read.source.adapter = "notion.v1".into();
    for text in [
        "not json".into(),
        "{}".into(),
        "x".repeat(MAX_SOURCE_BYTES + 1),
        page((0..101).map(|_| block("paragraph", "hello")).collect()),
        page(vec![json!({"id":"not-a-block"})]),
    ] {
        assert!(present(&read, &text, None).is_none());
    }
    read.arguments = json!({"page_id":"id"});
    assert!(present(&read, &text, None).is_none());
}

#[test]
fn notion_evidence_query_is_bounded_and_empty_input_is_rejected() {
    assert_eq!(validate_query(None), Ok(None));
    assert_eq!(validate_query(Some(&json!(" menu "))), Ok(Some("menu")));
    for value in [json!(null), json!(true), json!(" "), json!("x".repeat(257))] {
        assert!(validate_query(Some(&value)).is_err());
    }
}

#[test]
#[ignore = "manual benchmark with a local Notion block-list JSON fixture"]
fn notion_evidence_local_fixture_probe() {
    let text =
        std::fs::read_to_string(std::env::var("NYXID_NOTION_EVIDENCE_FIXTURE").unwrap()).unwrap();
    let query = std::env::var("NYXID_NOTION_EVIDENCE_QUERY").unwrap();
    let start = std::time::Instant::now();
    let result = present_json(&text, Some(&query));
    let elapsed = start.elapsed();
    let index = present_json(&text, None);
    println!(
        "source_bytes={} evidence_bytes={} escaped_evidence_bytes={} matched_sections={} returned_sections={} omitted_blocks={} section_index_omitted={} extraction_micros={}",
        text.len(),
        result.to_string().len(),
        wire_bytes(&result),
        result["matched_sections"],
        result["returned_sections"],
        result["omitted_blocks"],
        index["section_index_omitted"],
        elapsed.as_micros()
    );
    assert!(wire_bytes(&result) <= MAX_WIRE_BYTES);
    assert!(result["returned_sections"].as_u64().unwrap() > 0);
}
