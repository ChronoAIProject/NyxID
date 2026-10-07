//! Validation for explicitly authored, human-reviewed packages only.
//! L1 transcript-derived proposals deliberately keep their stricter validator.
use super::GeneratedProposal;
use crate::errors::{AppError, AppResult, skill_draft::SkillDraftValidation as Diagnostic};
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

pub(super) const MAX_CHARS: usize = 7_500;
pub(super) const MAX_BYTES: usize = 4 * MAX_CHARS;

pub(super) fn invalid(rule: &'static str, field: &str) -> AppError {
    Diagnostic::at(rule, field).into()
}

pub(super) fn object(value: &serde_json::Value, field: &str, allowed: &[&str]) -> AppResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("invalid_shape", field))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid("unknown_field", field));
    }
    Ok(())
}

pub(super) fn string(value: &serde_json::Value, field: &str) -> AppResult<()> {
    if !value.is_string() {
        return Err(invalid("invalid_shape", field));
    }
    Ok(())
}

pub(super) fn files_shape(value: Option<&serde_json::Value>) -> AppResult<()> {
    if let Some(value) = value {
        let files = value
            .as_array()
            .ok_or_else(|| invalid("invalid_shape", "files"))?;
        limit("files", "items", files.len(), 8)?;
        for (index, file) in files.iter().enumerate() {
            object(file, &format!("files[{index}]"), &["path", "content"])?;
            for field in ["path", "content"] {
                string(&file[field], &format!("files[{index}].{field}"))?;
            }
        }
    }
    Ok(())
}

fn limit(field: &str, unit: &'static str, actual: usize, maximum: usize) -> AppResult<()> {
    if actual > maximum {
        return Err(Diagnostic::size(field, unit, maximum, actual).into());
    }
    Ok(())
}

// Credential-shaped subsets of looks_secret / learning::SENSITIVE. Do not reuse
// their blanket PEM/certificate or 32-character-run rules, or telemetry's PII
// scrubber: IDs, paths, certificates, URLs and prose are not inherently secrets.
// ASCII boundaries also catch a key adjacent to CJK characters.
static CREDENTIALS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)-----BEGIN [^-]*PRIVATE KEY-----",
        r#"(?i)"?(?:password|passwd|secret|api[_-]?key|access[_-]?token|refresh[_-]?token|private[_-]?key|cookie|set-cookie|authorization|x-api-key)"?\s*[:=]\s*(?:"[^"\r\n]+"|[^\s,}\"]+)"#,
        r"(?i)(?-u:\b)bearer\s+[A-Za-z0-9_+/=.-]{16,}",
        r"(?-u:\b)(?:AKIA[A-Z0-9]{16}|gh[opusr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+|xox[baprs]-[A-Za-z0-9-]+|AIza[A-Za-z0-9_-]+|ornn_[A-Za-z0-9_-]+|nyx_(?:[a-fA-F0-9]{64}|(?:nauth|nreg|owk)_[A-Za-z0-9_-]+)|nyxid_ag_[A-Za-z0-9_-]+|sk-[A-Za-z0-9_-]{16,}|phc_[A-Za-z0-9_-]+|ya29\.[A-Za-z0-9_.-]+)",
        r"(?-u:\b)eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
    ].into_iter().map(|pattern| Regex::new(pattern).expect("static authored credential rule")).collect()
});
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)[a-z][a-z0-9+.-]*://[^\s<>"`]+"#).expect("static authored URL rule")
});

fn credential_position(text: &str) -> Option<usize> {
    let shape = CREDENTIALS
        .iter()
        .filter_map(|rule| rule.find(text).map(|m| m.start()));
    let urls = URL.find_iter(text).filter_map(|matched| {
        let url = url::Url::parse(matched.as_str()).ok()?;
        // A username alone (e.g. ssh://git@github.com/...) is not a secret.
        // Recognizable token usernames are already caught by CREDENTIALS.
        let credentials = url.password().is_some()
            || url.query_pairs().any(|(key, value)| {
                !value.is_empty()
                    && matches!(
                        key.to_ascii_lowercase().as_str(),
                        "token"
                            | "access_token"
                            | "refresh_token"
                            | "api_key"
                            | "api-key"
                            | "apikey"
                            | "password"
                            | "passwd"
                            | "secret"
                            | "authorization"
                            | "signature"
                            | "sig"
                            | "x-amz-signature"
                            | "x-goog-signature"
                    )
            });
        credentials.then_some(matched.start())
    });
    shape.chain(urls).min()
}

fn text(field: &str, value: &str, maximum: usize, required: bool) -> AppResult<()> {
    limit(field, "characters", value.chars().count(), maximum)?;
    if required && value.trim().is_empty() {
        return Err(invalid("required", field));
    }
    if let Some(position) = credential_position(value) {
        let line = value[..position].bytes().filter(|b| *b == b'\n').count() + 1;
        return Err(Diagnostic::at("credential_shape", field)
            .on_line(line)
            .into());
    }
    if let Some(position) = value.find(|c: char| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        let line = value[..position].bytes().filter(|b| *b == b'\n').count() + 1;
        return Err(Diagnostic::at("text_only", field).on_line(line).into());
    }
    Ok(())
}

pub(crate) fn validate(body: &GeneratedProposal) -> AppResult<Vec<u8>> {
    if body.schema_version != 1 {
        return Err(invalid("unsupported_schema", "schema_version"));
    }
    if !matches!(body.kind.as_str(), "new" | "improve") {
        return Err(invalid("invalid_kind", "kind"));
    }
    text("name", &body.name, 64, true)?;
    text("description", &body.description, 400, true)?;
    text("skill_md", &body.skill_md, MAX_CHARS, true)?;
    text("rationale", &body.rationale, 1_000, false)?;
    text("safety_notes", &body.safety_notes, 1_000, false)?;
    if !body
        .name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || body.name.starts_with('-')
        || body.name.ends_with('-')
        || body.name.contains("--")
    {
        return Err(invalid("invalid_name", "name"));
    }
    if (body.kind == "improve") != body.base_skill.is_some() {
        return Err(invalid("base_skill_mismatch", "base_skill"));
    }
    limit("files", "items", body.files.len(), 8)?;
    let mut paths = HashSet::new();
    for (index, file) in body.files.iter().enumerate() {
        let field = format!("files[{index}].path");
        text(&field, &file.path, 160, true)?;
        text(
            &format!("files[{index}].content"),
            &file.content,
            2_000,
            false,
        )?;
        if file.path.starts_with('/')
            || file.path.contains(['\\', ':'])
            || file.path.chars().any(char::is_control)
            || file
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !file.path.ends_with(".md") && !file.path.ends_with(".txt")
        {
            return Err(invalid("unsafe_path", &field));
        }
        if !paths.insert(file.path.to_ascii_lowercase())
            || file.path.eq_ignore_ascii_case("skill.md")
        {
            return Err(invalid("duplicate_path", &field));
        }
    }
    // Keep the existing TOTAL serialized character cap (including metadata and
    // JSON escaping). Four UTF-8 bytes per character accommodates every script.
    let encoded = serde_json::to_string(body).map_err(|_| invalid("invalid_shape", "draft"))?;
    limit("draft", "characters", encoded.chars().count(), MAX_CHARS)?;
    limit("draft", "bytes", encoded.len(), MAX_BYTES)?;
    Ok(encoded.into_bytes())
}

pub(crate) fn decode(value: &str) -> AppResult<GeneratedProposal> {
    limit("draft", "bytes", value.len(), MAX_BYTES)?;
    let value: serde_json::Value =
        serde_json::from_str(value).map_err(|_| invalid("invalid_shape", "draft"))?;
    object(
        &value,
        "draft",
        &[
            "schema_version",
            "kind",
            "name",
            "description",
            "skill_md",
            "files",
            "base_skill",
            "rationale",
            "safety_notes",
        ],
    )?;
    for field in [
        "name",
        "description",
        "skill_md",
        "rationale",
        "safety_notes",
    ] {
        if let Some(value) = value.get(field) {
            string(value, field)?;
        }
    }
    files_shape(value.get("files"))?;
    let body = serde_json::from_value(value).map_err(|_| invalid("invalid_shape", "draft"))?;
    validate(&body)?;
    Ok(body)
}
