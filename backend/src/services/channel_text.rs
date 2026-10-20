//! Lossless plain-text boundaries for native IM messages. Limits count the
//! provider's units, including the serialized content wrapper on Lark/Feishu.

#[derive(Clone, Copy)]
enum Units {
    Utf16,
    Scalars,
    JsonBytes,
}

#[derive(Clone, Copy)]
pub struct TextLimit {
    units: Units,
    budget: usize,
}

impl TextLimit {
    pub fn for_platform(platform: &str) -> Self {
        let (units, budget) = match platform {
            "lark" | "feishu" => (Units::JsonBytes, 30 * 1024 - "{\"text\":\"\"}".len()),
            "telegram" | "telegram-new" => (Units::Utf16, 4096),
            "discord" => (Units::Utf16, 2000),
            "slack" => (Units::Scalars, 40_000),
            "whatsapp" => (Units::Scalars, 4096),
            "x" => (Units::Scalars, 10_000),
            _ => (Units::Utf16, 2000),
        };
        Self { units, budget }
    }

    fn cost(self, ch: char) -> usize {
        match self.units {
            Units::Utf16 => ch.len_utf16(),
            Units::Scalars => 1,
            Units::JsonBytes => match ch {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
                '\u{00}'..='\u{1f}' => 6,
                _ => ch.len_utf8(),
            },
        }
    }

    /// Prefer paragraphs, then lines and whitespace. A token longer than the
    /// entire platform budget must be split at a Unicode scalar boundary.
    /// Concatenating all parts always recovers the input, including whitespace.
    pub fn chunks(self, text: &str) -> impl Iterator<Item = &str> {
        let mut rest = text;
        std::iter::from_fn(move || {
            if rest.is_empty() {
                return None;
            }
            let mut used = 0;
            let mut end = 0;
            for (index, ch) in rest.char_indices() {
                let cost = self.cost(ch);
                if used + cost > self.budget {
                    break;
                }
                used += cost;
                end = index + ch.len_utf8();
            }
            if end < rest.len() {
                let prefix = &rest[..end];
                end = prefix
                    .rfind("\n\n")
                    .map(|index| index + 2)
                    .or_else(|| prefix.rfind('\n').map(|index| index + 1))
                    .or_else(|| {
                        prefix
                            .char_indices()
                            .rev()
                            .find(|(_, ch)| ch.is_whitespace())
                            .map(|(index, ch)| index + ch.len_utf8())
                    })
                    .unwrap_or(end);
                // A backslash and the escaped scalar are one rendering atom.
                // An even run ends in an escaped backslash and is safe to cut.
                if rest[..end]
                    .bytes()
                    .rev()
                    .take_while(|b| *b == b'\\')
                    .count()
                    % 2
                    == 1
                {
                    end -= 1;
                }
            }
            let (part, tail) = rest.split_at(end);
            rest = tail;
            Some(part)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_markdown_escape_pairs_together_at_hard_boundaries() {
        let text = format!("{}\\*tail", "a".repeat(1999));
        let parts: Vec<_> = TextLimit::for_platform("discord").chunks(&text).collect();
        assert_eq!(parts.concat(), text);
        assert_eq!(parts[0], "a".repeat(1999));
        assert!(parts[1].starts_with("\\*"));
    }

    #[test]
    fn lark_keeps_chinese_report_and_link_in_one_message() {
        let text = format!(
            "{}\n\n[Event](https://example.com/event/long-url)",
            "活动安排。".repeat(1000)
        );
        for platform in ["lark", "feishu"] {
            assert_eq!(
                TextLimit::for_platform(platform)
                    .chunks(&text)
                    .collect::<Vec<_>>(),
                [&text]
            );
        }
    }

    #[test]
    fn prefers_paragraphs_and_keeps_url_with_following_text() {
        let url = "https://example.com/events/openagent-meetup-muse-instinct-2026-10-16?utm_source=openai";
        let text = format!(
            "{}\n\n[Event details]({url})\nNext paragraph.",
            "a".repeat(1950)
        );
        let parts = TextLimit::for_platform("discord")
            .chunks(&text)
            .collect::<Vec<_>>();
        assert_eq!(parts.len(), 2);
        assert!(parts[1].contains(url));
        assert_eq!(parts.concat(), text);
    }

    #[test]
    fn all_platforms_preserve_every_byte_and_respect_their_units() {
        let text = "第一段 🦉\\\"\n\nsecond\tline\u{01} https://example.com/a-b?c=d\n".repeat(2000);
        for platform in [
            "lark",
            "feishu",
            "telegram",
            "telegram-new",
            "discord",
            "slack",
            "whatsapp",
            "x",
        ] {
            let limit = TextLimit::for_platform(platform);
            let parts: Vec<_> = limit.chunks(&text).collect();
            assert_eq!(parts.concat(), text, "{platform}");
            assert!(
                parts.iter().all(|part| !part.is_empty()
                    && part.chars().map(|ch| limit.cost(ch)).sum::<usize>() <= limit.budget),
                "{platform}"
            );
        }
    }

    #[test]
    fn handles_exact_limits_empty_text_and_oversized_tokens() {
        for platform in ["lark", "telegram", "discord", "slack", "whatsapp", "x"] {
            let limit = TextLimit::for_platform(platform);
            assert_eq!(limit.chunks("").count(), 0);
            let exact = "x".repeat(limit.budget);
            assert_eq!(limit.chunks(&exact).count(), 1);
            let over = exact + "tail";
            assert_eq!(
                limit.chunks(&over).collect::<Vec<_>>(),
                [&over[..limit.budget], "tail"]
            );
            let emoji = "🦉".repeat(limit.budget);
            assert_eq!(limit.chunks(&emoji).collect::<String>(), emoji);
        }
    }
}
