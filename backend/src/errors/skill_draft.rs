//! Caller-safe diagnostics: only fixed rules/schema paths, indices and counts.
//! Never include a supplied filename, JSON key, matched text or parser error.
use serde::Serialize;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("Skill draft failed validation ({rule})")]
pub struct SkillDraftValidation {
    pub rule: &'static str,
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    /// Ornn's stable format rule identifiers (never its messages, which can
    /// quote filenames or parser output), sanitized and bounded.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ornn_rules: Vec<String>,
}

impl From<SkillDraftValidation> for super::AppError {
    fn from(value: SkillDraftValidation) -> Self {
        Self::SkillDraftValidation(Box::new(value))
    }
}

/// At most this many Ornn rule identifiers, each at most this many bytes.
const MAX_ORNN_RULES: usize = 10;
const MAX_ORNN_RULE_LEN: usize = 64;

impl SkillDraftValidation {
    pub(crate) fn at(rule: &'static str, field: &str) -> Self {
        Self {
            rule,
            field: field.into(),
            line: None,
            limit: None,
            actual: None,
            unit: None,
            ornn_rules: Vec::new(),
        }
    }

    pub(crate) fn on_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub(crate) fn size(field: &str, unit: &'static str, limit: usize, actual: usize) -> Self {
        Self {
            limit: Some(limit),
            actual: Some(actual),
            unit: Some(unit),
            ..Self::at("too_large", field)
        }
    }

    /// Ornn's format validator refused the package NyxID built from the
    /// draft. `rules` are Ornn's rule identifiers; anything outside
    /// `[A-Za-z0-9._-]` is dropped so no supplied text can pass through.
    pub(crate) fn ornn_rejected<'a>(rules: impl IntoIterator<Item = &'a str>) -> Self {
        let mut ornn_rules: Vec<String> = Vec::new();
        for rule in rules {
            if ornn_rules.len() == MAX_ORNN_RULES {
                break;
            }
            if rule.is_empty()
                || rule.len() > MAX_ORNN_RULE_LEN
                || !rule
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                || ornn_rules.iter().any(|known| known == rule)
            {
                continue;
            }
            ornn_rules.push(rule.to_owned());
        }
        let field = ornn_rules
            .first()
            .map_or("package", |rule| ornn_rule_field(rule));
        Self {
            ornn_rules,
            ..Self::at("ornn_format_rejected", field)
        }
    }
}

/// The draft field an Ornn format rule points at.
fn ornn_rule_field(rule: &str) -> &'static str {
    match rule {
        "allowed-root-items" | "no-readme-md" => "files",
        "no-xml-brackets" | "description-no-xml" => "description",
        "name-no-forbidden-terms" | "folder-name-kebab-case" | "name-matches-folder" => "name",
        _ if rule.starts_with("frontmatter.description") => "description",
        _ if rule.starts_with("frontmatter.name") => "name",
        _ => "package",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ornn_rules_are_sanitized_bounded_and_mapped() {
        let diagnostic = SkillDraftValidation::ornn_rejected([
            "description-no-xml",
            "Root item \"faq.md\"",
            "",
            "description-no-xml",
            "allowed-root-items",
        ]);
        assert_eq!(diagnostic.rule, "ornn_format_rejected");
        assert_eq!(diagnostic.field, "description");
        assert_eq!(
            diagnostic.ornn_rules,
            ["description-no-xml", "allowed-root-items"]
        );
        let many: Vec<String> = (0..20).map(|i| format!("rule-{i}")).collect();
        let diagnostic = SkillDraftValidation::ornn_rejected(many.iter().map(String::as_str));
        assert_eq!(diagnostic.ornn_rules.len(), MAX_ORNN_RULES);
        assert_eq!(diagnostic.field, "package");
        assert_eq!(
            SkillDraftValidation::ornn_rejected(["frontmatter.name"]).field,
            "name"
        );
        let empty = SkillDraftValidation::ornn_rejected([]);
        assert_eq!(empty.field, "package");
        assert!(
            serde_json::to_value(&empty)
                .unwrap()
                .get("ornn_rules")
                .is_none()
        );
    }
}
