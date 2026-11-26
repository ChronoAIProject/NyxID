use std::borrow::Cow;
use std::sync::LazyLock;

use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;

// Reference kinds (search, view, news, etc.) have decimal indices on both sides.
// An interrupted response can end partway through the last reference.
static MARKER: LazyLock<Regex> = LazyLock::new(|| {
    let open = '\u{e200}';
    let close = '\u{e201}';
    let sep = '\u{e202}';
    let reference = r"\u{e202}turn[0-9]+[A-Za-z]+[0-9]+";
    let unfinished = r"\u{e202}(?:t|tu|tur|turn(?:[0-9]+[A-Za-z]*[0-9]*)?)?\z";
    Regex::new(&format!(
        r"(?x)
        {open}cite(?:{reference})+ (?:{close}|{unfinished})?
        | {open}cite (?:{unfinished}|\z)
        | [{open}{close}{sep}]"
    ))
    .expect("citation marker syntax")
});

pub(super) fn sanitize(source: &str) -> Cow<'_, str> {
    if !source.contains(['\u{e200}', '\u{e201}', '\u{e202}']) {
        return Cow::Borrowed(source);
    }
    let mut result = String::with_capacity(source.len());
    let mut start = 0;
    for (event, range) in Parser::new(source).into_offset_iter() {
        if matches!(event, Event::Code(_) | Event::Start(Tag::CodeBlock(_))) {
            append_prose(&mut result, &source[start..range.start]);
            result.push_str(&source[range.clone()]);
            start = range.end;
        }
    }
    append_prose(&mut result, &source[start..]);
    Cow::Owned(result)
}

fn append_prose(result: &mut String, prose: &str) {
    let start = result.len();
    let mut next = 0;
    for marker in MARKER.find_iter(prose) {
        result.push_str(&prose[next..marker.start()]);
        next = marker.end();
        let horizontal = |c: char| c.is_whitespace() && !matches!(c, '\r' | '\n');
        let following = prose[next..].trim_start_matches(horizontal);
        if following.starts_with([
            '.', ',', ';', ':', '?', '!', '。', '，', '、', '；', '：', '？', '！', '…',
        ]) {
            result.truncate(start + result[start..].trim_end_matches(horizontal).len());
            next = prose.len() - following.len();
        }
    }
    result.push_str(&prose[next..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = "\u{e200}cite\u{e202}turn347841search1\u{e202}turn347841search2\u{e202}turn347841search3\u{e201}";
    const VIEW: &str = "\u{e200}cite\u{e202}turn652437view1\u{e202}turn652437view2\u{e201}";

    #[test]
    fn removes_web_and_lark_markers_and_spaces_before_punctuation() {
        let raw = format!("结果  {SEARCH}　，来源。 {VIEW}\nA {VIEW}{SEARCH} ! B  .");
        assert_eq!(sanitize(&raw), "结果，来源。 \nA! B  .");
        assert_eq!(sanitize("a\u{e200}cite\u{e202}turn8news2\u{e201} b"), "a b");
    }

    #[test]
    fn handles_truncated_markers_and_isolated_delimiters() {
        for tail in ["", "turn42view3", "turn42news1\u{e202}turn42se"] {
            assert_eq!(
                sanitize(&format!("Answer\u{e200}cite\u{e202}{tail}")),
                "Answer"
            );
        }
        assert_eq!(
            sanitize("a\u{e200}b\u{e202}c\u{e201}d\u{e203}e\u{e000}"),
            "abcd\u{e203}e\u{e000}"
        );
        assert_eq!(sanitize("citeturn347841search1"), "citeturn347841search1");
    }

    #[test]
    fn preserves_markdown_code_verbatim_and_is_idempotent() {
        let code = format!(
            "«``one ` {VIEW}``»\r\n\r\n~~~text\r\n{SEARCH}\r\n~~~\r\n\n```md\n{VIEW}\n```\n"
        );
        let raw = format!("before {SEARCH}. {code}after {VIEW}。");
        let clean = sanitize(&raw);
        assert_eq!(clean, format!("before. {code}after。"));
        assert_eq!(sanitize(&clean), clean);
    }
}
