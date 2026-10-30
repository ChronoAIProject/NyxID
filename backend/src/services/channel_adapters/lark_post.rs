//! Lark/Feishu rich posts, adapted from CMA d2db9ee1 ingress/relay/lark_post.rs.
//! Text rendering follows CMA; NyxID retains at most ten provider handles.
use serde_json::Value;
const MAX_ATTACHMENTS: usize = 10;
fn forbidden_text_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}
/// The activity's bound on `Message.text`.
const MAX_TEXT_BYTES: usize = 32 * 1_024;
const MARKER: &str = "\n[post truncated]";
const OMITTED: &str = "[more embedded media omitted]";
const MAX_NAME_CHARS: usize = 64;
const MAX_HREF_BYTES: usize = 2_048;
const MAX_LANGUAGE_BYTES: usize = 32;
const MAX_EMOJI_TYPE_BYTES: usize = 64;

/// A flattened post: its text and one placeholder per embedded image or video.
pub struct PostContent {
    pub text: String,
    pub attachments: Vec<crate::models::channel_message::StoredAttachment>,
    pub omitted: usize,
}

impl std::fmt::Debug for PostContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PostContent([REDACTED])")
    }
}

/// Flatten a post's content JSON, an object with an optional string `title` and
/// `content`, an array of paragraphs that are arrays of element objects. Any
/// other shape, and a post with neither text nor embedded media, is `None`.
///
/// `mentions` is `event.message.mentions`. With a pinned `bot_id`, the bot's own
/// mention is removed; without one the gateway cannot tell which mention is the
/// bot, so every mention renders as a name.
pub fn parse_post(
    content: &Value,
    mentions: &[Value],
    bot_id: Option<&str>,
) -> Option<PostContent> {
    let post = content.as_object()?;
    let title = match post.get("title") {
        None | Some(Value::Null) => "",
        Some(Value::String(title)) => title,
        Some(_) => return None,
    };
    let paragraphs = post.get("content")?.as_array()?;
    if !paragraphs.iter().all(|paragraph| {
        paragraph
            .as_array()
            .is_some_and(|elements| elements.iter().all(Value::is_object))
    }) {
        return None;
    }
    let mut writer = Writer {
        mentions,
        bot_id,
        text: String::new(),
        truncated: false,
        attachments: Vec::new(),
        embedded: 0,
        omitted: Omitted::No,
    };
    writer.line(&inline(title));
    for paragraph in paragraphs.iter().filter_map(Value::as_array) {
        writer.paragraph(paragraph);
    }
    let Writer {
        text,
        attachments,
        embedded,
        ..
    } = writer;
    (!text.is_empty() || !attachments.is_empty()).then(|| {
        let omitted = embedded.saturating_sub(attachments.len());
        PostContent {
            text,
            attachments,
            omitted,
        }
    })
}

/// Whether the notice for embedded media beyond the attachment bound is owed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Omitted {
    No,
    Owed,
    Written,
}

struct Writer<'a> {
    mentions: &'a [Value],
    bot_id: Option<&'a str>,
    text: String,
    truncated: bool,
    attachments: Vec<crate::models::channel_message::StoredAttachment>,
    /// Embedded media elements seen so far, kept or not.
    embedded: usize,
    omitted: Omitted,
}

impl Writer<'_> {
    /// Inline elements concatenate; a code block or rule ends the run before it
    /// and stands on its own lines.
    fn paragraph(&mut self, elements: &[Value]) {
        let mut run = String::new();
        for element in elements {
            let tag = field(element, "tag");
            match tag {
                "text" => run.push_str(field(element, "text")),
                "a" => run.push_str(&link(element)),
                "at" => run.push_str(&self.mention(element)),
                "emotion" => run.push_str(&emotion(element)),
                "code_block" => {
                    self.line(&inline(&run));
                    run.clear();
                    self.line(&code_block(element));
                }
                "hr" => {
                    self.line(&inline(&run));
                    run.clear();
                    self.line("---");
                }
                "img" => self.embed(element, "image_key", "image"),
                "media" => self.embed(element, "file_key", "file"),
                _ => {}
            }
        }
        self.line(&inline(&run));
        if self.omitted == Omitted::Owed {
            self.line(OMITTED);
            self.omitted = Omitted::Written;
        }
    }

    /// Append `line` as its own line. The first line that does not fit is cut
    /// on a char boundary and the marker ends the text; nothing follows it.
    fn line(&mut self, line: &str) {
        if self.truncated || line.is_empty() {
            return;
        }
        let line = clean(line);
        let separator = if self.text.is_empty() { "" } else { "\n" };
        let room = (MAX_TEXT_BYTES - MARKER.len()).saturating_sub(self.text.len());
        if separator.len() + line.len() <= room {
            self.text.push_str(separator);
            self.text.push_str(&line);
            return;
        }
        let piece = format!("{separator}{line}");
        self.text
            .push_str(&piece[..piece.floor_char_boundary(room)]);
        self.text.truncate(self.text.trim_end().len());
        self.text.push_str(MARKER);
        self.truncated = true;
    }

    /// A placeholder for an embedded image or video, which renders no text.
    fn embed(&mut self, element: &Value, key: &str, kind: &str) {
        self.embedded += 1;
        if self.attachments.len() == MAX_ATTACHMENTS {
            if self.omitted == Omitted::No {
                self.omitted = Omitted::Owed;
            }
            return;
        }
        let handle = element
            .get(key)
            .and_then(Value::as_str)
            .filter(|key| {
                (1..=1024).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic())
            })
            .map(str::to_owned);
        self.attachments
            .push(crate::models::channel_message::StoredAttachment {
                content_type: kind.into(),
                provider_ref: String::new(),
                platform_message_id: None,
                file_key: (kind == "file").then(|| handle.clone()).flatten(),
                image_key: (kind == "image").then_some(handle).flatten(),
                filename: None,
                mime_type: None,
                size_bytes: None,
            });
    }

    /// An `at` element resolves its placeholder key, such as `@_user_1`, through
    /// the event's `mentions`. The pinned bot's own mention addresses the bot
    /// and carries no meaning for the agent, so it renders nothing.
    fn mention(&self, element: &Value) -> String {
        let user_id = field(element, "user_id");
        let entry = self.mentions.iter().find(|mention| {
            !user_id.is_empty() && mention.get("key").and_then(Value::as_str) == Some(user_id)
        });
        let open_id = entry.and_then(|entry| entry.pointer("/id/open_id"));
        if self
            .bot_id
            .is_some_and(|bot_id| open_id.and_then(Value::as_str) == Some(bot_id))
        {
            return String::new();
        }
        if let Some(name) = entry
            .map(|entry| name(field(entry, "name")))
            .filter(|name| !name.is_empty())
        {
            return format!("@{name}");
        }
        if matches!(user_id, "@_all" | "all") {
            return "@all".to_owned();
        }
        match name(field(element, "user_name")) {
            name if entry.is_none() && !name.is_empty() => format!("@{name}"),
            _ => String::new(),
        }
    }
}

/// A string field, or empty when it is absent or not a string.
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A link's text, followed by its target when that is an absolute http or https
/// URL. The URL parser silently drops tabs and newlines, so whitespace and
/// control characters are refused first rather than smuggled into the text.
fn link(element: &Value) -> String {
    let text = field(element, "text");
    let href = field(element, "href");
    let web = !href.is_empty()
        && href.len() <= MAX_HREF_BYTES
        && !href
            .chars()
            .any(|c| c.is_whitespace() || forbidden_text_char(c))
        && url::Url::parse(href).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
    if web {
        format!("{text} ({href})")
    } else {
        text.to_owned()
    }
}

fn emotion(element: &Value) -> String {
    let emoji = field(element, "emoji_type");
    let valid = (1..=MAX_EMOJI_TYPE_BYTES).contains(&emoji.len())
        && emoji
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid {
        format!(":{emoji}:")
    } else {
        String::new()
    }
}

/// A fenced block whose language is kept only when it is a plain identifier,
/// so it cannot break the fence line.
fn code_block(element: &Value) -> String {
    let language = field(element, "language").to_ascii_lowercase();
    let language = if language.len() <= MAX_LANGUAGE_BYTES
        && language.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+#_-".contains(&byte)
        }) {
        language
    } else {
        String::new()
    };
    format!("```{language}\n{}\n```", field(element, "text"))
}

/// A display name: without control or bidirectional formatting characters,
/// whitespace-collapsed and at most 64 characters.
fn name(value: &str) -> String {
    let cleaned: String = value.chars().filter(|c| !forbidden_text_char(*c)).collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(MAX_NAME_CHARS).collect();
    cut.trim_end().to_owned()
}

/// Remove C0 and C1 controls other than newline and tab, and the bidirectional
/// formatting characters that could reorder what the model reads.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(c, '\n' | '\t') || !forbidden_text_char(*c))
        .collect()
}

/// An inline run: cleaned, with each run of horizontal whitespace collapsed to
/// one space, and trimmed.
fn inline(run: &str) -> String {
    let mut output = String::with_capacity(run.len());
    let mut space = false;
    for c in clean(run).chars() {
        if c != '\n' && c.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            output.push(' ');
            space = false;
        }
        output.push(c);
    }
    output.trim().to_owned()
}

#[cfg(test)]
#[path = "lark_post_tests.rs"]
mod tests;
