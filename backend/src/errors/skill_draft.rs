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
}

impl From<SkillDraftValidation> for super::AppError {
    fn from(value: SkillDraftValidation) -> Self {
        Self::SkillDraftValidation(Box::new(value))
    }
}

impl SkillDraftValidation {
    pub(crate) fn at(rule: &'static str, field: &str) -> Self {
        Self {
            rule,
            field: field.into(),
            line: None,
            limit: None,
            actual: None,
            unit: None,
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
}
