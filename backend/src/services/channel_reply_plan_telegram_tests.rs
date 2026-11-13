use super::*;

const QA: &str = "BEGIN-QA-FMT\n**中文标题**\n- 食材：米100克。\n**English title**\n- Ingredient: rice 100 g.\n**မြန်မာခေါင်းစဉ်**\n- ပါဝင်ပစ္စည်း — ဆန် ၁၀၀ ဂရမ်။\n[D1-B-ZH]\nEND-QA-FMT";

fn spans(part: &OutboundReply) -> Vec<(String, String)> {
    let units: Vec<_> = part.text.as_deref().unwrap().encode_utf16().collect();
    assert!(units.len() <= 4096);
    part.metadata.as_ref().unwrap()["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let start = e["offset"].as_u64().unwrap() as usize;
            let end = start + e["length"].as_u64().unwrap() as usize;
            assert!(end <= units.len() && end > start);
            (
                e["type"].as_str().unwrap().into(),
                String::from_utf16(&units[start..end]).unwrap(),
            )
        })
        .collect()
}

#[test]
fn qa_2317_verbatim_and_literal_paths_have_no_conversion_escapes() {
    let parts = assistant_parts(QA, "telegram", "");
    assert_eq!(parts.len(), 1);
    let body = parts[0].text.as_deref().unwrap();
    assert!(body.contains("[D1-B-ZH]"));
    assert!(body.contains("ပါဝင်ပစ္စည်း — ဆန် ၁၀၀ ဂရမ်။"));
    assert!(!body.contains('\\') && !body.contains("**"));
    let bold: Vec<_> = spans(&parts[0])
        .into_iter()
        .filter(|(kind, _)| kind == "bold")
        .map(|(_, text)| text)
        .collect();
    assert_eq!(bold, ["中文标题", "English title", "မြန်မာခေါင်းစဉ်"]);
    let source = r#"🦉 [D1-B-ZH] | menu_01 my_identifier

`C:\qa\menu_01.txt` and `a|b]_[x]`

````txt
C:\qa\menu_01.txt
let fence = "```";
````"#;
    let parts = assistant_parts(source, "telegram", "");
    let code = spans(&parts[0]);
    assert!(code.contains(&("code".into(), r"C:\qa\menu_01.txt".into())));
    assert!(
        code.iter()
            .any(|(kind, text)| kind == "pre" && text.contains("\"```\""))
    );
    let text = parts[0].text.as_deref().unwrap();
    assert!(text.contains("[D1-B-ZH] | menu_01 my_identifier"));
    assert_eq!(text.matches('\\').count(), 4);
}

#[test]
fn headings_nested_emphasis_code_links_and_unbalanced_markup() {
    let source = "# 🦉 မြန်မာ **nested**\n\n**bold *italic* `a_b|c` tail**\n\n[link](https://example.test/a_(b))\n\nunbalanced **literal [D1-B-ZH] `tick";
    let parts = assistant_parts(source, "telegram", "");
    let spans = spans(&parts[0]);
    for expected in [
        ("bold", "🦉 မြန်မာ nested"),
        ("italic", "italic"),
        ("code", "a_b|c"),
        ("text_link", "link"),
    ] {
        assert!(
            spans.contains(&(expected.0.into(), expected.1.into())),
            "{spans:?}"
        );
    }
    let metadata = parts[0].metadata.as_ref().unwrap();
    assert!(
        metadata["entities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["url"] == "https://example.test/a_(b)")
    );
    assert!(
        parts[0]
            .text
            .as_deref()
            .unwrap()
            .ends_with("unbalanced **literal [D1-B-ZH] `tick")
    );
    let code = metadata["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "code")
        .unwrap();
    let start = code["offset"].as_u64().unwrap();
    let end = start + code["length"].as_u64().unwrap();
    for entity in metadata["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] != "code")
    {
        let from = entity["offset"].as_u64().unwrap();
        let to = from + entity["length"].as_u64().unwrap();
        assert!(to <= start || from >= end);
    }
}

#[test]
fn eleven_thousand_trilingual_characters_keep_complete_ordered_content_and_utf16() {
    let section = "**中文标题 🦉 English မြန်မာခေါင်းစဉ်**\n- 食材：米100克。\n- Ingredient: rice 100 g.\n- ပါဝင်ပစ္စည်း — ဆန် ၁၀၀ ဂရမ်။\n[D1-B-ZH] | menu_01\n\n";
    let source = format!("BEGIN\n\n{}END", section.repeat(110));
    assert!(source.chars().count() >= 11_000);
    let display_section = "中文标题 🦉 English မြန်မာခေါင်းစဉ်\n\n• 食材：米100克。\n• Ingredient: rice 100 g.\n• ပါဝင်ပစ္စည်း — ဆန် ၁၀၀ ဂရမ်။\n[D1-B-ZH] | menu_01\n\n";
    let expected = format!("BEGIN\n\n{}END", display_section.repeat(110));
    let parts = assistant_parts(&source, "telegram", "");
    assert!(parts.len() >= 3);
    assert_eq!(
        parts
            .iter()
            .filter_map(|p| p.text.as_deref())
            .collect::<String>(),
        expected
    );
    for part in &parts {
        spans(part);
    }
    assert_eq!(
        parts
            .iter()
            .flat_map(spans)
            .filter(|(kind, _)| kind == "bold")
            .count(),
        110
    );
}

#[test]
fn fitting_entity_moves_whole_and_oversized_code_gets_valid_continuations() {
    // A preferred newline must not cut a code block that fits in one part.
    let fitting_code = format!("{}\n{}", "a".repeat(2000), "b".repeat(2000));
    let source = format!("```\n{fitting_code}\n```\n\n{}", "tail".repeat(1100));
    let parts = assistant_parts(&source, "telegram", "");
    let code_spans: Vec<_> = parts
        .iter()
        .flat_map(spans)
        .filter(|(kind, _)| kind == "pre")
        .collect();
    assert_eq!(
        code_spans,
        vec![("pre".into(), format!("{fitting_code}\n"))]
    );
    // Start emphasis with a letter: an emoji directly after `a**` is not a
    // CommonMark opening delimiter (it is punctuation after a non-space).
    let emphasized = format!("bold 🦉မြန်မာ {}", "z".repeat(60));
    let source = format!("{}**{emphasized}** tail", "a".repeat(4080));
    let parts = assistant_parts(&source, "telegram", "");
    assert_eq!(parts.len(), 2);
    assert!(spans(&parts[0]).is_empty());
    assert_eq!(spans(&parts[1]), vec![("bold".into(), emphasized)]);
    let code = "🦉မြန်မာ|_\\`".repeat(1500);
    let source = format!("````\n{code}\n````");
    let parts = assistant_parts(&source, "telegram", "");
    assert_eq!(
        parts
            .iter()
            .filter_map(|p| p.text.as_deref())
            .collect::<String>(),
        code
    );
    assert!(
        parts
            .iter()
            .all(|p| spans(p).iter().any(|(kind, _)| kind == "pre"))
    );
}

#[test]
fn public_legacy_bytes_are_unchanged_and_commonmark_is_explicit() {
    let original = OutboundReply {
        text: Some(QA.into()),
        metadata: None,
        attachments: vec![],
        reply_to_platform_message_id: None,
    };
    assert_eq!(
        text_parts(&original, "telegram").unwrap()[0].text,
        original.text
    );
    let mut opted = original.clone();
    opted.metadata = assistant_metadata("telegram");
    let parts = ordinary(&opted, "telegram", "chat").unwrap().unwrap();
    assert!(!parts[0].text.as_deref().unwrap().contains("**"));
    opted.metadata.as_mut().unwrap()["parse_mode"] = "MarkdownV2".into();
    assert!(text_parts(&opted, "telegram").is_err());
}
