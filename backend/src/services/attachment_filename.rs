//! Download names derived from known media types. This does not validate bytes.

/// Bounded download metadata only. Raster validation remains the extractor's job.
pub(crate) fn image_content_type(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(&bytes[..bytes.len().min(32)]) {
        Ok(
            format @ (image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP),
        ) => Some(format.to_mime_type()),
        _ => None,
    }
}

/// A source name is a bounded hint, never a path or a content-type authority.
/// Preserve a short original suffix when truncating its sanitized stem.
pub(crate) fn original_name(value: Option<&str>, max_chars: usize) -> Option<String> {
    let value = value.filter(|v| v.len() <= 1024 && !v.chars().any(char::is_control))?;
    sanitize_name(value, max_chars)
}

/// Shared display sanitization. Bound allocations and preserve a usable suffix
/// when shortening a stem; a removed stem must not turn .PDF into the name PDF.
pub(crate) fn sanitize_name(value: &str, max_chars: usize) -> Option<String> {
    fn allowed(c: char) -> bool {
        c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '(' | ')')
    }
    // Normalize the entire trailing run before choosing the suffix or checking
    // its stem. Filtering a trailing disallowed character must expose padding
    // here, rather than leave a different name for the next sanitization pass.
    let leaf = value
        .rsplit(['/', '\\'])
        .next()?
        .trim_end_matches(|c| !allowed(c) || matches!(c, ' ' | '.'));
    let parts = leaf.rsplit_once('.');
    if let Some((stem, _)) = parts
        && !stem.is_empty()
        && !stem.chars().any(|c| allowed(c) && c != '.' && c != ' ')
    {
        return None;
    }
    let prefix: String = leaf
        .chars()
        .filter(|c| allowed(*c))
        .skip_while(|c| matches!(c, ' ' | '.'))
        .take(max_chars + 1)
        .collect();
    let name = if prefix.chars().count() > max_chars {
        if let Some((stem, raw_suffix)) = parts {
            let suffix: String = raw_suffix
                .chars()
                .filter(|c| allowed(*c))
                .take(max_chars + 1)
                .collect();
            if !suffix.is_empty()
                && suffix.chars().all(char::is_alphanumeric)
                && suffix.chars().count() + 2 <= max_chars
            {
                let room = max_chars - suffix.chars().count() - 1;
                let stem: String = stem
                    .chars()
                    .filter(|c| allowed(*c))
                    .skip_while(|c| matches!(c, ' ' | '.'))
                    .take(room)
                    .collect();
                format!("{}.{}", stem.trim_end(), suffix)
            } else {
                prefix.chars().take(max_chars).collect()
            }
        } else {
            prefix.chars().take(max_chars).collect()
        }
    } else {
        prefix
    };
    let name = name.trim_matches([' ', '.']);
    (!name.is_empty()).then(|| name.to_owned())
}

/// Only missing or legacy generic labels need a type-derived default. Named
/// originals, including extensionless ones, keep their own identity.
pub(crate) fn default_name(value: Option<&str>, content_type: &str, max_chars: usize) -> String {
    let name = original_name(value, max_chars).unwrap_or_else(|| "attachment".into());
    if name != "attachment" {
        return name;
    }
    let extension = content_type.parse::<mime::Mime>().ok().and_then(|kind| {
        Some(match kind.essence_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "application/pdf" => "pdf",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
            "text/plain" => "txt",
            "text/markdown" => "md",
            "text/csv" => "csv",
            "application/json" => "json",
            "audio/mpeg" => "mp3",
            "audio/mp4" => "m4a",
            "audio/aac" => "aac",
            "audio/ogg" => "ogg",
            "audio/opus" => "opus",
            "audio/wav" | "audio/x-wav" => "wav",
            "audio/flac" => "flac",
            "audio/webm" => "webm",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            "video/quicktime" => "mov",
            "video/mpeg" => "mpeg",
            _ => return None,
        })
    });
    match extension {
        Some(ext) => format!(
            "{}.{}",
            name.chars()
                .take(max_chars.saturating_sub(ext.len() + 1))
                .collect::<String>(),
            ext
        ),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        assistant_upload_service::{MAX_FILENAME_CHARS as UPLOAD_MAX, safe_name},
        channel_media_service::{MAX_FILENAME_CHARS as RELAY_MAX, safe_filename},
    };

    #[test]
    fn fallback_names_cover_extractor_types_and_known_media_without_guessing_opaque_extensions() {
        for (kind, suffix) in [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/gif", "gif"),
            ("image/webp", "webp"),
            ("application/pdf", "pdf"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "docx",
            ),
            ("text/plain", "txt"),
            ("text/markdown", "md"),
            ("text/csv", "csv"),
            ("application/json", "json"),
            ("audio/mpeg", "mp3"),
            ("audio/mp4", "m4a"),
            ("audio/aac", "aac"),
            ("audio/ogg; codecs=opus", "ogg"),
            ("audio/opus", "opus"),
            ("audio/wav", "wav"),
            ("audio/x-wav", "wav"),
            ("audio/flac", "flac"),
            ("audio/webm", "webm"),
            ("video/mp4", "mp4"),
            ("video/webm", "webm"),
            ("video/quicktime", "mov"),
            ("video/mpeg", "mpeg"),
        ] {
            for label in [
                None,
                Some(""),
                Some("attachment"),
                Some("../.."),
                Some("bad\rname"),
            ] {
                let name = default_name(label, kind, UPLOAD_MAX);
                assert_eq!(name, format!("attachment.{suffix}"));
                assert_eq!(
                    default_name(Some(&safe_name(&name)), kind, UPLOAD_MAX),
                    name
                );
                assert_eq!(safe_filename(Some(&name)), name);
            }
            for label in [
                "receipt",
                "receipt.jpeg",
                "Receipt.JPG",
                "照片.png",
                "report.v2.PDF",
            ] {
                assert_eq!(default_name(Some(label), kind, UPLOAD_MAX), label);
            }
        }
        for kind in [
            "application/octet-stream",
            "image/unknown",
            "audio/unknown",
            "invalid",
        ] {
            assert_eq!(default_name(None, kind, UPLOAD_MAX), "attachment");
            assert_eq!(
                default_name(Some("invoice.PDF"), kind, UPLOAD_MAX),
                "invoice.PDF"
            );
        }
    }

    #[test]
    fn original_names_keep_stems_and_suffixes_under_caps_and_are_idempotent() {
        assert_eq!(safe_name(".gitignore"), "gitignore");
        for prefix in [
            " ".repeat(120),
            ".".repeat(120),
            format!("🙂{}", " ".repeat(120)),
            format!(" .{}", " .".repeat(120)),
        ] {
            assert_eq!(safe_name(&format!("{prefix}a.pdf")), "a.pdf");
            assert_eq!(
                safe_name(&format!("{prefix}{}.pdf", "a".repeat(121))),
                format!("{}.pdf", "a".repeat(UPLOAD_MAX - 4))
            );
        }
        for (name, kind, fallback) in [
            ("🧾.PDF", "application/pdf", "attachment.pdf"),
            ("📷.jpg", "image/jpeg", "attachment.jpg"),
        ] {
            assert!(original_name(Some(name), UPLOAD_MAX).is_none());
            assert_eq!(default_name(Some(name), kind, UPLOAD_MAX), fallback);
            assert_eq!(sanitize_name(name, UPLOAD_MAX), None);
        }
        let long_pdf = format!("{}.pdf", "a".repeat(121));
        assert_eq!(
            safe_name(&long_pdf),
            format!("{}.pdf", "a".repeat(UPLOAD_MAX - 4))
        );
        for limit in [UPLOAD_MAX, RELAY_MAX] {
            for label in [
                "照片".repeat(120),
                format!("{}.JPEG", "照".repeat(200)),
                format!("{}.pdf", "a".repeat(400)),
            ] {
                let name = original_name(Some(&label), limit).unwrap();
                assert!(name.chars().count() <= limit);
                assert_eq!(original_name(Some(&name), limit).as_ref(), Some(&name));
                if let Some((_, suffix)) = label.rsplit_once('.') {
                    assert!(name.ends_with(&format!(".{suffix}")));
                }
                if limit == UPLOAD_MAX {
                    assert_eq!(safe_name(&name), name);
                }
                assert!(safe_filename(Some(&name)).chars().count() <= RELAY_MAX);
            }
        }
        assert!(original_name(Some(&"a".repeat(1025)), UPLOAD_MAX).is_none());
    }

    #[test]
    fn trailing_padding_is_normalized_before_suffix_selection_and_is_idempotent() {
        for (input, expected) in [
            (
                format!("{}.v1.csv ", "a".repeat(117)),
                format!("{}.csv", "a".repeat(UPLOAD_MAX - 4)),
            ),
            ("Invoice.json. .".into(), "Invoice.json".into()),
            ("Invoice.json. .🔥".into(), "Invoice.json".into()),
            (".gitignore. .🔥".into(), "gitignore".into()),
            ("attachment. .".into(), "attachment".into()),
        ] {
            let name = sanitize_name(&input, UPLOAD_MAX).unwrap();
            assert_eq!(name, expected);
            assert_eq!(sanitize_name(&name, UPLOAD_MAX).as_ref(), Some(&name));
            assert_eq!(safe_name(&input), expected);
            assert!(name.chars().count() <= UPLOAD_MAX);
        }
        for input in ["🧾.PDF.", "🧾.PDF. .🔥", "📷.jpg. .", "🔥. ."] {
            assert!(sanitize_name(input, UPLOAD_MAX).is_none());
            assert_eq!(safe_name(input), "attachment");
            assert_eq!(
                default_name(Some(input), "application/pdf", UPLOAD_MAX),
                "attachment.pdf"
            );
        }
    }
}
