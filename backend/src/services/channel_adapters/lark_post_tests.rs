//! Post flattening unit tests: each element and layout rule, and a seeded
//! invariant that every flattened post fits the activity's text and attachment
//! domain.
use super::*;
use serde_json::json;

const BOT: &str = "ou_bot";

fn mentions() -> Vec<Value> {
    vec![
        json!({"key": "@_user_1", "id": {"open_id": BOT, "union_id": "on_bot"}, "name": "Support Bot"}),
        json!({"key": "@_user_2", "id": {"open_id": "ou_alice"}, "name": "Alice"}),
    ]
}

/// The flattened text of one post whose single paragraph is `elements`.
fn paragraph(elements: Value) -> String {
    parse_post(&json!({ "content": [elements] }), &mentions(), Some(BOT))
        .map(|post| post.text)
        .unwrap_or_default()
}

fn at(user_id: &str) -> Value {
    json!({"tag": "at", "user_id": user_id, "user_name": ""})
}

#[test]
fn a_title_and_inline_elements_join_into_paragraph_lines() {
    let post = json!({"title": "Weekly \t update", "content": [
        [{"tag": "text", "text": "Hello "}, {"tag": "a", "text": "docs", "href": "https://docs.example/a?b=1"},
         {"tag": "text", "text": "  and\u{a0}"}, {"tag": "emotion", "emoji_type": "SMILE"}],
        [],
        [{"tag": "text", "text": "   "}],
        [{"tag": "text", "text": "second"}],
    ]});
    let flattened = parse_post(&post, &[], None).unwrap();
    assert_eq!(
        flattened.text,
        "Weekly update\nHello docs (https://docs.example/a?b=1) and :SMILE:\nsecond"
    );
    assert!(flattened.attachments.is_empty());
}

#[test]
fn links_keep_only_absolute_web_targets() {
    let link = |href: &str| paragraph(json!([{"tag": "a", "text": "site", "href": href}]));
    assert_eq!(link("http://a.example"), "site (http://a.example)");
    let long = format!("https://a.example/{}", "p".repeat(2_048));
    for href in [
        "javascript:alert(1)",
        "/relative/path",
        "ftp://a.example/file",
        "https://a.example/\nnext",
        "https://a.example/\u{202E}gpj",
        "not a url",
        "",
        long.as_str(),
    ] {
        assert_eq!(link(href), "site", "{href}");
    }
    assert_eq!(paragraph(json!([{"tag": "a", "href": 7}])), "");
}

#[test]
fn mentions_render_names_without_the_pinned_bot() {
    let elements = json!([at("@_user_1"), {"tag": "text", "text": " ping "}, at("@_user_2")]);
    assert_eq!(paragraph(elements.clone()), "ping @Alice");
    // Unpinned, the gateway cannot tell which mention is the bot.
    let unpinned = parse_post(&json!({ "content": [elements] }), &mentions(), None).unwrap();
    assert_eq!(unpinned.text, "@Support Bot ping @Alice");
}

#[test]
fn all_and_unresolvable_mentions() {
    for user_id in ["@_all", "all"] {
        assert_eq!(paragraph(json!([at(user_id)])), "@all");
    }
    let named = json!([{"tag": "at", "user_id": "@_user_9", "user_name": "Bob"}]);
    assert_eq!(paragraph(named), "@Bob");
    for user_name in [json!(""), json!("\u{202E}\u{7}"), json!(7)] {
        let unnamed = json!([{"tag": "at", "user_id": "@_user_9", "user_name": user_name}, {"tag": "text", "text": "hi"}]);
        assert_eq!(paragraph(unnamed), "hi");
    }
    assert_eq!(
        paragraph(json!([{"tag": "at"}, {"tag": "text", "text": "x"}])),
        "x"
    );
}

#[test]
fn titles_and_names_lose_controls_and_bidirectional_characters() {
    let post =
        json!({"title": "Re\u{202E}port\u{7} \u{2066}Q3\u{85}", "content": [[at("@_user_3")]]});
    let long = "N".repeat(70);
    for (name, expected) in [
        ("Al\u{200F}ice\n  Smith\u{1b}", "@Alice Smith".to_owned()),
        (long.as_str(), format!("@{}", "N".repeat(64))),
    ] {
        let mention = json!([{"key": "@_user_3", "id": {"open_id": "ou_3"}, "name": name}]);
        let text = parse_post(&post, mention.as_array().unwrap(), Some(BOT))
            .unwrap()
            .text;
        assert_eq!(text, format!("Report Q3\n{expected}"));
    }
}

#[test]
fn code_blocks_and_rules_stand_on_their_own_lines() {
    let code = "func main() {\n\treturn 0\u{7}\n}";
    let text = paragraph(json!([
        {"tag": "text", "text": "before"},
        {"tag": "code_block", "language": "GO", "text": code},
        {"tag": "text", "text": " after "},
        {"tag": "hr"},
    ]));
    assert_eq!(
        text,
        "before\n```go\nfunc main() {\n\treturn 0\n}\n```\nafter\n---"
    );
    for (language, fence) in [
        ("C++", "```c++"),
        ("objective-c#_1", "```objective-c#_1"),
        ("bad lang", "```"),
        ("x```", "```"),
        ("文", "```"),
    ] {
        let block = paragraph(json!([{"tag": "code_block", "language": language, "text": "x"}]));
        assert_eq!(block, format!("{fence}\nx\n```"), "{language}");
    }
    let long = "a".repeat(33);
    let block = paragraph(json!([{"tag": "code_block", "language": long, "text": "x"}]));
    assert_eq!(block, "```\nx\n```");
}

#[test]
fn emotions_render_only_plain_identifiers() {
    assert_eq!(
        paragraph(json!([{"tag": "emotion", "emoji_type": "THUMBSUP_2"}])),
        ":THUMBSUP_2:"
    );
    let long = "A".repeat(65);
    for invalid in [json!("a b"), json!(":x:"), json!(""), json!(long), json!(3)] {
        assert_eq!(
            paragraph(json!([{"tag": "emotion", "emoji_type": invalid}])),
            ""
        );
    }
}

#[test]
fn embedded_media_preserve_document_order_and_invalid_handles() {
    let post = json!({"content": [[
        {"tag": "img", "image_key": "img_v3_a"},
        {"tag": "media", "file_key": "file_v3_b", "image_key": "img_v3_cover"},
        {"tag": "img", "image_key": "has space"},
        {"tag": "media"},
        {"tag": "note", "elements": []},
    ]]});
    let flattened = parse_post(&post, &[], None).unwrap();
    assert_eq!(flattened.text, "");
    let handles: Vec<_> = flattened
        .attachments
        .iter()
        .map(|item| {
            (
                item.content_type.as_str(),
                item.image_key.as_deref(),
                item.file_key.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        handles,
        [
            ("image", Some("img_v3_a"), None),
            ("file", None, Some("file_v3_b")),
            ("image", None, None),
            ("file", None, None)
        ]
    );
}

#[test]
fn media_beyond_the_attachment_bound_are_noted_once() {
    let images: Vec<_> = (0..20)
        .map(|index| json!([{"tag": "img", "image_key": format!("img_{index}")}]))
        .collect();
    let mut paragraphs = vec![json!([{"tag": "text", "text": "photos"}])];
    paragraphs.extend(images);
    paragraphs.push(json!([{"tag": "text", "text": "thanks"}]));
    let flattened = parse_post(&json!({ "content": paragraphs }), &[], None).unwrap();
    assert_eq!(flattened.attachments.len(), 10);
    assert_eq!(flattened.omitted, 10);
    assert_eq!(flattened.attachments[9].image_key.as_deref(), Some("img_9"));
    assert_eq!(
        flattened.text,
        "photos\n[more embedded media omitted]\nthanks"
    );
}

#[test]
fn oversized_posts_are_truncated_on_a_char_boundary() {
    let text = "文".repeat(14_000);
    let post = json!({"title": "title", "content": [[{"tag": "text", "text": text}], [{"tag": "text", "text": "lost"}]]});
    let flattened = parse_post(&post, &[], None).unwrap();
    assert!(flattened.text.len() <= MAX_TEXT_BYTES);
    assert!(flattened.text.ends_with(MARKER));
    assert!(flattened.text.starts_with("title\n文文"));
    assert!(!flattened.text.contains("lost"));
    // Exactly at the budget, nothing is cut.
    let fits = "a".repeat(MAX_TEXT_BYTES - MARKER.len());
    let exact = parse_post(
        &json!({"content": [[{"tag": "text", "text": fits}]]}),
        &[],
        None,
    );
    assert_eq!(exact.unwrap().text, fits);
}

#[test]
fn malformed_and_empty_posts_are_none() {
    for content in [
        json!("text"),
        json!([[{"tag": "text", "text": "x"}]]),
        json!({"title": "x"}),
        json!({"content": "x"}),
        json!({"content": [{"tag": "text", "text": "x"}]}),
        json!({"content": [["x"]]}),
        json!({"title": 7, "content": [[{"tag": "text", "text": "x"}]]}),
        json!({"title": "", "content": []}),
        json!({"content": [[], []]}),
        json!({"content": [[{"tag": "unknown", "text": "x"}, {"tag": "text", "text": " \u{7} "}]]}),
    ] {
        assert!(parse_post(&content, &[], None).is_none(), "{content}");
    }
    let titled = parse_post(&json!({"title": "only a title", "content": []}), &[], None);
    assert_eq!(titled.unwrap().text, "only a title");
}

#[test]
fn cma_relay_rich_post_fixture_parity() {
    // Copied from cmaeg/tests/relay/lark_post.rs::rich_post (CMA d2db9ee1).
    let content = json!({"title": "Weekly update", "content": [
        [{"tag": "at", "user_id": "@_user_1", "user_name": ""}, {"tag": "text", "text": " please check "},
         {"tag": "a", "text": "the report", "href": "https://docs.example/r"},
         {"tag": "text", "text": " with "}, {"tag": "at", "user_id": "@_user_2", "user_name": ""}],
        [{"tag": "img", "image_key": "img_v3_receipt"}],
    ]});
    let post = parse_post(&content, &mentions(), Some(BOT)).unwrap();
    assert_eq!(
        post.text,
        "Weekly update\nplease check the report (https://docs.example/r) with @Alice"
    );
    assert_eq!(post.attachments.len(), 1);
    let post = parse_post(&content, &mentions(), None).unwrap();
    assert_eq!(
        post.text,
        "Weekly update\n@Support Bot please check the report (https://docs.example/r) with @Alice"
    );
    // The relay fixture has seventeen images. NyxID's contract caps at ten.
    let images: Vec<_> = (0..17)
        .map(|i| json!([{"tag":"img","image_key":format!("img_v3_{i}")}]))
        .collect();
    let capped = parse_post(&json!({"content":images}), &[], None).unwrap();
    assert_eq!(capped.attachments.len(), 10);
    assert_eq!(capped.omitted, 7);
    assert_eq!(capped.text, OMITTED);
}

#[test]
fn malformed_post_shapes_never_panic() {
    for value in [
        Value::Null,
        json!(false),
        json!(1),
        json!({"content":[null]}),
        json!({"content":[[null]]}),
    ] {
        assert!(parse_post(&value, &[], None).is_none());
    }
    for n in 0..200 {
        let items:Vec<_>=(0..n).map(|i| json!({"tag": if i%2==0 {"img"} else {"text"}, "text":"文".repeat(i*100), "image_key":null})).collect();
        if let Some(post) = parse_post(&json!({"content":[items]}), &[], None) {
            assert!(post.text.len() <= MAX_TEXT_BYTES);
            assert!(post.attachments.len() <= MAX_ATTACHMENTS);
        }
    }
}
