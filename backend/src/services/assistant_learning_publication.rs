//! The fixed Ornn publication protocol. No model-selected destinations,
//! permissions, scripts or owner fallback. Calls are made by an injected
//! adapter using the approving person's signed identity.
use super::{
    agent_skill_service::{self as skills, OrnnCode, OrnnOutcome, OrnnReader, Preview},
    assistant_agent_learning::GeneratedProposal,
};
use crate::{
    errors::{AppError, AppResult},
    models::{assistant_agent_learning::LearningPublication, catalog_skill_revision::SkillPin},
};
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

fn invalid() -> AppError {
    AppError::Conflict("publication_integrity_failed".into())
}

#[derive(Debug)]
pub(super) enum PublicationError {
    Integrity,
    Ambiguous(Option<u16>),
    BaseChanged,
    BaseInterfaceIncompatible,
    Validation(Option<u16>),
    AuthMissing(u16),
    ReadForbidden(u16),
    SkillNotFound(u16),
    Unavailable(Option<u16>),
    /// NyxID refused or failed the read before dispatching it.
    Refused,
}

impl PublicationError {
    pub(super) fn status(&self) -> Option<u16> {
        match self {
            Self::Ambiguous(status) | Self::Validation(status) | Self::Unavailable(status) => {
                *status
            }
            Self::AuthMissing(status)
            | Self::ReadForbidden(status)
            | Self::SkillNotFound(status) => Some(*status),
            _ => None,
        }
    }

    /// Provider-neutral code for this failure at `stage`. Integrity failures
    /// of a version NyxID may have written are conflicts, and an absent or
    /// unreadable version after dispatch is never proof of non-publication.
    pub(super) fn failure_code(&self, stage: PublicationStage) -> FailureCode {
        use FailureCode as Code;
        use PublicationStage as Stage;
        match (self, stage) {
            (Self::Integrity, Stage::BaseVerify) => Code::BaseVerifyFailed,
            (Self::Integrity, Stage::Reconcile | Stage::Verify) => Code::VersionConflict,
            (Self::Integrity, _) => Code::OrnnValidationFailed,
            (Self::BaseChanged, _) => Code::BaseChanged,
            (Self::BaseInterfaceIncompatible, _) => Code::BaseInterfaceIncompatible,
            (Self::Validation(_), _) => Code::OrnnValidationFailed,
            (Self::AuthMissing(_), _) => Code::OrnnAuthMissing,
            (Self::ReadForbidden(_), _) => Code::OrnnReadForbidden,
            (
                Self::SkillNotFound(_) | Self::Unavailable(_) | Self::Ambiguous(_) | Self::Refused,
                Stage::Reconcile,
            ) => Code::PublishUncertain,
            (
                Self::SkillNotFound(_) | Self::Unavailable(_) | Self::Ambiguous(_) | Self::Refused,
                Stage::Verify,
            ) => Code::VerifyFailed,
            (Self::Refused, _) => Code::NyxidRefused,
            (Self::SkillNotFound(_), _) => Code::OrnnSkillNotFound,
            (Self::Unavailable(_) | Self::Ambiguous(_), _) => Code::OrnnUnavailable,
        }
    }

    pub(super) fn into_app_error(self) -> AppError {
        match self {
            Self::Integrity => invalid(),
            Self::Ambiguous(_) => ambiguous(),
            Self::BaseChanged => AppError::Conflict("base_skill_changed".into()),
            Self::BaseInterfaceIncompatible => {
                AppError::Conflict("base_interface_incompatible".into())
            }
            Self::Validation(_) => AppError::Conflict("ornn_validation_failed".into()),
            Self::AuthMissing(_) => AppError::Conflict("ornn_auth_missing".into()),
            Self::ReadForbidden(_) => AppError::Conflict("ornn_read_forbidden".into()),
            Self::SkillNotFound(_) => AppError::Conflict("ornn_skill_not_found".into()),
            Self::Unavailable(Some(_)) => AppError::Conflict("ornn_unavailable".into()),
            Self::Unavailable(None) => AppError::ServicePoolInfrastructureUnavailable,
            Self::Refused => {
                AppError::Conflict("nyxid_refused: publication temporarily unavailable".into())
            }
        }
    }
}

type PublicationResult<T> = Result<T, PublicationError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureCode {
    BaseChanged,
    BaseVerifyFailed,
    BaseInterfaceIncompatible,
    OrnnValidationFailed,
    OrnnAuthMissing,
    OrnnReadForbidden,
    OrnnWriteForbidden,
    OrnnSkillNotFound,
    OrnnUnavailable,
    OrnnPackageRejected,
    OrnnDependencyRejected,
    OrnnNameTaken,
    OrnnInterfaceChangeRequiresMajor,
    NyxidRefused,
    PublishUncertain,
    VersionConflict,
    VerifyFailed,
    PinConflict,
    ApprovalExpired,
    TargetBusy,
    EvidenceUnavailable,
    OperatorReleased,
}

impl FailureCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BaseChanged => "base_changed",
            Self::BaseVerifyFailed => "base_verify_failed",
            Self::BaseInterfaceIncompatible => "base_interface_incompatible",
            Self::OrnnValidationFailed => "ornn_validation_failed",
            Self::OrnnAuthMissing => "ornn_auth_missing",
            Self::OrnnReadForbidden => "ornn_read_forbidden",
            Self::OrnnWriteForbidden => "ornn_write_forbidden",
            Self::OrnnSkillNotFound => "ornn_skill_not_found",
            Self::OrnnUnavailable => "ornn_unavailable",
            Self::OrnnPackageRejected => "ornn_package_rejected",
            Self::OrnnDependencyRejected => "ornn_dependency_rejected",
            Self::OrnnNameTaken => "ornn_name_taken",
            Self::OrnnInterfaceChangeRequiresMajor => "ornn_interface_change_requires_major",
            Self::NyxidRefused => "nyxid_refused",
            Self::PublishUncertain => "publish_uncertain",
            Self::VersionConflict => "version_conflict",
            Self::VerifyFailed => "verify_failed",
            Self::PinConflict => "pin_conflict",
            Self::ApprovalExpired => "approval_expired",
            Self::TargetBusy => "target_busy",
            Self::EvidenceUnavailable => "evidence_unavailable",
            Self::OperatorReleased => "operator_released",
        }
    }

    const ALL: [Self; 22] = [
        Self::BaseChanged,
        Self::BaseVerifyFailed,
        Self::BaseInterfaceIncompatible,
        Self::OrnnValidationFailed,
        Self::OrnnAuthMissing,
        Self::OrnnReadForbidden,
        Self::OrnnWriteForbidden,
        Self::OrnnSkillNotFound,
        Self::OrnnUnavailable,
        Self::OrnnPackageRejected,
        Self::OrnnDependencyRejected,
        Self::OrnnNameTaken,
        Self::OrnnInterfaceChangeRequiresMajor,
        Self::NyxidRefused,
        Self::PublishUncertain,
        Self::VersionConflict,
        Self::VerifyFailed,
        Self::PinConflict,
        Self::ApprovalExpired,
        Self::TargetBusy,
        Self::EvidenceUnavailable,
        Self::OperatorReleased,
    ];

    pub(crate) fn parse(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == code)
    }

    /// The package bytes cannot succeed unchanged; only a revised draft (new
    /// operation and card) can publish. `version_conflict` qualifies only
    /// while its operation never dispatched, which the caller decides.
    pub(crate) fn requires_new_package(self) -> bool {
        matches!(
            self,
            Self::BaseChanged
                | Self::BaseVerifyFailed
                | Self::OrnnInterfaceChangeRequiresMajor
                | Self::BaseInterfaceIncompatible
                | Self::OrnnValidationFailed
                | Self::OrnnPackageRejected
                | Self::OrnnDependencyRejected
                | Self::OrnnNameTaken
        )
    }

    /// Transient or confirmation-level outcomes that a later pre-claim
    /// refusal of the same attempt may replace.
    pub(crate) fn replaceable_before_claim(self) -> bool {
        matches!(
            self,
            Self::OrnnUnavailable
                | Self::NyxidRefused
                | Self::OrnnReadForbidden
                | Self::OperatorReleased
        )
    }

    pub(crate) fn codes(filter: fn(Self) -> bool) -> Vec<&'static str> {
        Self::ALL
            .into_iter()
            .filter(|code| filter(*code))
            .map(Self::as_str)
            .collect()
    }
}

/// Durable saga stage, stored and audited for recovery (never shown as copy).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicationStage {
    Preflight,
    FormatValidate,
    BaseVerify,
    Publish,
    Reconcile,
    Verify,
    Pin,
}

impl PublicationStage {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::FormatValidate => "format_validate",
            Self::BaseVerify => "base_verify",
            Self::Publish => "publish",
            Self::Reconcile => "reconcile",
            Self::Verify => "verify",
            Self::Pin => "pin",
        }
    }
}
pub(super) fn ambiguous() -> AppError {
    AppError::Conflict("publication_ambiguous: retry reconciles the existing operation; it never creates another skill".into())
}

pub(super) fn next_version(base: Option<&SkillPin>) -> AppResult<String> {
    let Some(base) = base else {
        return Ok("1.0".into());
    };
    // Ornn's literal format is major.minor. Never interpolate model strings.
    let (major, minor) = base.version.split_once('.').ok_or_else(invalid)?;
    let major: u32 = major.parse().map_err(|_| invalid())?;
    let minor: u32 = minor.parse().map_err(|_| invalid())?;
    Ok(format!(
        "{major}.{}",
        minor.checked_add(1).ok_or_else(invalid)?
    ))
}

pub(super) fn package_with_snapshot(
    draft: &GeneratedProposal,
    operation: &str,
    name: &str,
    version: &str,
    snapshot: Option<&InterfaceSnapshot>,
) -> AppResult<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    // JSON string literals are also safe YAML scalars (quotes/newlines cannot
    // introduce frontmatter keys). Every control field is server-selected.
    let front = skill_markdown_with_snapshot(draft, operation, name, version, snapshot)?;
    zip.start_file(format!("{name}/SKILL.md"), options)
        .map_err(|_| invalid())?;
    zip.write_all(front.as_bytes()).map_err(|_| invalid())?;
    // Source-specific validation runs before this function. All file names are safe,
    // unique UTF-8 text paths; the archive is never extracted on the API host.
    for file in &draft.files {
        zip.start_file(format!("{name}/{}", file.path), options)
            .map_err(|_| invalid())?;
        zip.write_all(file.content.as_bytes())
            .map_err(|_| invalid())?;
    }
    Ok(zip.finish().map_err(|_| invalid())?.into_inner())
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct InterfaceSnapshot {
    pub category: String,
    pub output_type: Option<String>,
    pub runtimes: Vec<String>,
    pub runtime_dependencies: Vec<String>,
    pub runtime_env_vars: Vec<String>,
    pub tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn bounded_names(
    values: Option<&Vec<Value>>,
    field: &str,
    max: usize,
    len: usize,
) -> AppResult<Vec<String>> {
    let values = values.ok_or_else(|| AppError::Conflict("base_interface_incompatible".into()))?;
    if values.len() > max {
        return Err(AppError::Conflict("base_interface_incompatible".into()));
    }
    values
        .iter()
        .map(|value| {
            value
                .get(field)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty() && text.len() <= len)
                .map(str::to_owned)
                .ok_or_else(|| AppError::Conflict("base_interface_incompatible".into()))
        })
        .collect()
}

impl InterfaceSnapshot {
    pub(super) fn from_metadata(meta: &Value) -> AppResult<Self> {
        let incompatible = || AppError::Conflict("base_interface_incompatible".into());
        let category = meta["category"].as_str().ok_or_else(incompatible)?;
        if !["plain", "tool-based", "runtime-based", "mixed"].contains(&category) {
            return Err(incompatible());
        }
        let output_type = meta["outputType"].as_str().map(str::to_owned);
        if output_type
            .as_deref()
            .is_some_and(|v| v != "text" && v != "file")
        {
            return Err(incompatible());
        }
        let runtimes = meta["runtimes"].as_array().cloned().unwrap_or_default();
        let tools = meta["tools"].as_array().cloned().unwrap_or_default();
        let runtime_names = bounded_names(Some(&runtimes), "runtime", 10, 50)?;
        let tool_names = bounded_names(Some(&tools), "tool", 50, 100)?;
        let tags = match meta.get("tags") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(tags)) => tags.clone(),
            _ => return Err(incompatible()),
        };
        if tags.len() > 10 {
            return Err(incompatible());
        }
        let tags: Vec<String> = tags
            .iter()
            .map(|tag| {
                tag.as_str()
                    .filter(|tag| {
                        !tag.is_empty()
                            && tag.len() <= 30
                            && tag
                                .bytes()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                    })
                    .map(str::to_owned)
                    .ok_or_else(incompatible)
            })
            .collect::<AppResult<_>>()?;
        let mut dependencies = Vec::new();
        let mut env_vars = Vec::new();
        for (index, runtime) in runtimes.iter().enumerate() {
            let deps = runtime["dependencies"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let envs = runtime["envs"].as_array().cloned().unwrap_or_default();
            let names = bounded_names(Some(&deps), "library", 50, 200)?;
            let env_names = bounded_names(Some(&envs), "var", 30, 100)?;
            if env_names.iter().any(|name| {
                !name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            }) {
                return Err(incompatible());
            }
            if index == 0 {
                dependencies = names;
                env_vars = env_names;
            } else if dependencies
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                != names.iter().collect::<std::collections::BTreeSet<_>>()
                || env_vars.iter().collect::<std::collections::BTreeSet<_>>()
                    != env_names.iter().collect::<std::collections::BTreeSet<_>>()
            {
                return Err(incompatible());
            }
        }
        let has_runtime = !runtime_names.is_empty();
        let has_tools = !tool_names.is_empty();
        let valid = match category {
            "plain" => !has_runtime && !has_tools && output_type.is_none(),
            "tool-based" => !has_runtime && has_tools && output_type.is_none(),
            "runtime-based" => has_runtime && !has_tools && output_type.is_some(),
            "mixed" => has_runtime && has_tools && output_type.is_some(),
            _ => false,
        };
        if !valid {
            return Err(incompatible());
        }
        Ok(Self {
            category: category.into(),
            output_type,
            runtimes: runtime_names,
            runtime_dependencies: dependencies,
            runtime_env_vars: env_vars,
            tools: tool_names,
            tags,
        })
    }
}

pub(super) async fn snapshot(
    reader: &impl OrnnReader,
    actor: &str,
    base: &SkillPin,
) -> PublicationResult<InterfaceSnapshot> {
    verify_base(reader, actor, base).await?;
    let meta = data(
        reader,
        &format!("/api/v1/skills/{}?version={}", base.skill_id, base.version),
    )
    .await?;
    InterfaceSnapshot::from_metadata(&meta["metadata"])
        .map_err(|_| PublicationError::BaseInterfaceIncompatible)
}

pub(super) fn skill_markdown_with_snapshot(
    draft: &GeneratedProposal,
    operation: &str,
    name: &str,
    version: &str,
    snapshot: Option<&InterfaceSnapshot>,
) -> AppResult<String> {
    let quote = |text: &str| serde_json::to_string(text).map_err(|_| invalid());
    let interface = if let Some(s) = snapshot {
        let mut lines = format!("  category: {}\n", quote(&s.category)?);
        if let Some(output) = &s.output_type {
            lines.push_str(&format!("  output-type: {}\n", quote(output)?));
        }
        for (key, values) in [
            ("runtime", &s.runtimes),
            ("runtime-dependency", &s.runtime_dependencies),
            ("runtime-env-var", &s.runtime_env_vars),
            ("tool-list", &s.tools),
            ("tag", &s.tags),
        ] {
            if !values.is_empty() {
                lines.push_str(&format!(
                    "  {key}: {}\n",
                    serde_json::to_string(values).map_err(|_| invalid())?
                ));
            }
        }
        lines
    } else {
        "  category: plain\n".into()
    };
    Ok(format!(
        "---\nname: {}\ndescription: {}\nversion: {}\nmetadata:\n{}  generated-by: nyxid-learning\n  operation-id: {}\n---\n\n{}",
        quote(name)?,
        quote(&draft.description)?,
        quote(version)?,
        interface,
        quote(operation)?,
        draft.skill_md
    ))
}

pub(super) fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn read_result(outcome: OrnnOutcome) -> PublicationResult<Vec<u8>> {
    match outcome {
        OrnnOutcome::Ok(bytes) => Ok(bytes),
        OrnnOutcome::Response { status: 401, .. } => Err(PublicationError::AuthMissing(401)),
        OrnnOutcome::Response { status: 403, .. } => Err(PublicationError::ReadForbidden(403)),
        OrnnOutcome::Response {
            code: OrnnCode::SkillNotFound | OrnnCode::SkillVersionNotFound,
            status: 404,
            ..
        } => Err(PublicationError::SkillNotFound(404)),
        OrnnOutcome::Response { status, .. } => Err(PublicationError::Unavailable(Some(status))),
        // Nothing was sent, as for a refused publication request.
        OrnnOutcome::LocalRefusal(_) | OrnnOutcome::LocalError(_) => Err(PublicationError::Refused),
        OrnnOutcome::ProxyError | OrnnOutcome::Interrupted | OrnnOutcome::Uncertain => {
            Err(PublicationError::Unavailable(None))
        }
    }
}
/// Only content that contradicts the reviewed pin is an integrity failure; a
/// read that did not complete or came back malformed can be retried.
fn preview_error(error: skills::PreviewFailure) -> PublicationError {
    match error {
        skills::PreviewFailure::Mismatch(_) => PublicationError::Integrity,
        skills::PreviewFailure::Read(_) => PublicationError::Unavailable(None),
    }
}
/// A malformed 2xx reply is a failed read, never evidence about the skill.
fn malformed() -> PublicationError {
    PublicationError::Unavailable(None)
}
async fn data(reader: &impl OrnnReader, path: &str) -> PublicationResult<Value> {
    let bytes = read_result(reader.classified(Method::GET, path, Vec::new()).await)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
    value.get("data").cloned().ok_or_else(malformed)
}
fn text<'a>(meta: &'a Value, key: &str) -> PublicationResult<&'a str> {
    meta[key].as_str().ok_or_else(malformed)
}
fn private_owner(meta: &Value, actor: &str) -> PublicationResult<()> {
    let private = meta["isPrivate"].as_bool().ok_or_else(malformed)?;
    let owner = text(meta, "createdBy")?;
    let mut shared = false;
    for key in ["sharedWithUsers", "sharedWithOrgs", "grants"] {
        shared |= !meta[key].as_array().ok_or_else(malformed)?.is_empty();
    }
    if !private || owner != actor || shared {
        return Err(PublicationError::Integrity);
    }
    Ok(())
}

pub(super) async fn verify_base(
    reader: &impl OrnnReader,
    actor: &str,
    base: &SkillPin,
) -> PublicationResult<()> {
    let meta = data(
        reader,
        &format!("/api/v1/skills/{}?version={}", base.skill_id, base.version),
    )
    .await?;
    private_owner(&meta, actor)?;
    if text(&meta, "skillHash")? != base.sha256 {
        return Err(PublicationError::Integrity);
    }
    let preview = skills::preview_checked(reader, &base.skill_id, &base.version)
        .await
        .map_err(preview_error)?;
    if preview.reference.sha256 != base.sha256
        || preview.reference.name != base.name
        || !preview.reference.dependencies.is_empty()
    {
        return Err(PublicationError::Integrity);
    }
    // Updating must extend this exact latest version, never an unrelated edit.
    let versions = data(
        reader,
        &format!("/api/v1/skills/{}/versions", base.skill_id),
    )
    .await?;
    let items = versions["items"].as_array().ok_or_else(malformed)?;
    if items.first().and_then(|v| v["version"].as_str()) != Some(&base.version) {
        return Err(PublicationError::BaseChanged);
    }
    Ok(())
}

pub(super) async fn validate(reader: &impl OrnnReader, bytes: &[u8]) -> PublicationResult<()> {
    let response = match reader
        .classified(
            Method::POST,
            "/api/v1/skill-format/validate",
            bytes.to_vec(),
        )
        .await
    {
        OrnnOutcome::Ok(bytes) => bytes,
        OrnnOutcome::Response {
            code: OrnnCode::ValidationFailed,
            status: status @ 400..=499,
            ..
        } => {
            return Err(PublicationError::Validation(Some(status)));
        }
        other => return read_result(other).map(|_| ()),
    };
    let response: Value = serde_json::from_slice(&response).map_err(|_| malformed())?;
    let valid = response["data"]["valid"].as_bool().ok_or_else(malformed)?;
    let violations = match &response["data"]["violations"] {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        _ => return Err(malformed()),
    };
    // Only an explicit verdict refuses the package.
    if !valid || violations {
        return Err(PublicationError::Validation(None));
    }
    Ok(())
}

pub(super) enum PublishOutcome {
    Published(String),
    Refused {
        code: FailureCode,
        status: Option<u16>,
    },
    VersionConflict {
        status: u16,
    },
    Uncertain {
        status: Option<u16>,
    },
}

pub(super) async fn publish_classified(
    reader: &impl OrnnReader,
    base: Option<&SkillPin>,
    bytes: Vec<u8>,
) -> PublishOutcome {
    let (method, path) = base.map_or_else(
        || (Method::POST, "/api/v1/skills".into()),
        |b| (Method::PUT, format!("/api/v1/skills/{}", b.skill_id)),
    );
    match reader.classified(method, &path, bytes).await {
        OrnnOutcome::Ok(body) => {
            let id = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|value| value["data"]["guid"].as_str().map(str::to_owned));
            match id {
                Some(id)
                    if uuid::Uuid::parse_str(&id).is_ok()
                        && base.is_none_or(|b| b.skill_id == id) =>
                {
                    PublishOutcome::Published(id)
                }
                _ => PublishOutcome::Uncertain { status: None },
            }
        }
        // Refused or failed before the proxy was called: nothing was sent.
        OrnnOutcome::LocalRefusal(_) | OrnnOutcome::LocalError(_) => PublishOutcome::Refused {
            code: FailureCode::NyxidRefused,
            status: None,
        },
        OrnnOutcome::Uncertain | OrnnOutcome::ProxyError | OrnnOutcome::Interrupted => {
            PublishOutcome::Uncertain { status: None }
        }
        OrnnOutcome::Response { status, code } => {
            if !(400..500).contains(&status) || status == 408 || status == 429 {
                return PublishOutcome::Uncertain {
                    status: Some(status),
                };
            }
            let reason = match code {
                OrnnCode::AuthMissing => Some(FailureCode::OrnnAuthMissing),
                OrnnCode::Forbidden => Some(FailureCode::OrnnWriteForbidden),
                OrnnCode::SkillNotFound if base.is_some() => Some(FailureCode::OrnnSkillNotFound),
                OrnnCode::PayloadTooLarge
                | OrnnCode::InvalidZip
                | OrnnCode::TooManyFiles
                | OrnnCode::UncompressedTooLarge
                | OrnnCode::NoUpdate
                | OrnnCode::InvalidBody => Some(FailureCode::OrnnPackageRejected),
                OrnnCode::ValidationFailed => Some(FailureCode::OrnnValidationFailed),
                OrnnCode::BreakingChangeWithoutMajorBump if base.is_some() => {
                    Some(FailureCode::OrnnInterfaceChangeRequiresMajor)
                }
                OrnnCode::SkillDependencyNotFound
                | OrnnCode::DependencyCycle
                | OrnnCode::DependencyConflict => Some(FailureCode::OrnnDependencyRejected),
                OrnnCode::ReservedName if base.is_none() => Some(FailureCode::OrnnNameTaken),
                OrnnCode::VersionNotIncremented | OrnnCode::SkillVersionExists
                    if base.is_some() =>
                {
                    return PublishOutcome::VersionConflict { status };
                }
                OrnnCode::SkillNameExists if base.is_none() => {
                    return PublishOutcome::VersionConflict { status };
                }
                _ => None,
            };
            reason.map_or(
                PublishOutcome::Uncertain {
                    status: Some(status),
                },
                |code| PublishOutcome::Refused {
                    code,
                    status: Some(status),
                },
            )
        }
    }
}

pub(super) async fn verify(
    reader: &impl OrnnReader,
    actor: &str,
    p: &LearningPublication,
    id: &str,
) -> PublicationResult<Preview> {
    uuid::Uuid::parse_str(id).map_err(|_| PublicationError::Integrity)?;
    let meta = data(
        reader,
        &format!("/api/v1/skills/{id}?version={}", p.version),
    )
    .await?;
    private_owner(&meta, actor)?;
    if text(&meta, "guid")? != id
        || text(&meta, "version")? != p.version
        || text(&meta, "name")? != p.name
        || text(&meta, "skillHash")? != p.sha256
    {
        return Err(PublicationError::Integrity);
    }
    let preview = skills::preview_checked(reader, id, &p.version)
        .await
        .map_err(preview_error)?;
    if preview.reference.sha256 != p.sha256 || !preview.reference.dependencies.is_empty() {
        return Err(PublicationError::Integrity);
    }
    Ok(preview)
}

pub(super) async fn reconcile(
    reader: &impl OrnnReader,
    actor: &str,
    p: &LearningPublication,
    base: Option<&SkillPin>,
) -> PublicationResult<String> {
    if let Some(id) = p
        .skill_id
        .as_deref()
        .or_else(|| base.map(|b| b.skill_id.as_str()))
    {
        verify(reader, actor, p, id).await?;
        return Ok(id.into());
    }
    // The unique server-generated name embeds the operation UUID. Ornn search
    // indexes names, descriptions and tags, not arbitrary package metadata.
    let result = data(
        reader,
        &format!(
            "/api/v1/skill-search?scope=private&mode=keyword&pageSize=20&page=1&q={}",
            urlencoding::encode(&p.name)
        ),
    )
    .await?;
    let items = result["items"]
        .as_array()
        .ok_or(PublicationError::Ambiguous(None))?;
    if items.len() > 20 || result["totalPages"].as_u64().is_some_and(|n| n > 1) {
        return Err(PublicationError::Ambiguous(None));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut mismatched = false;
    for item in items {
        if item["name"] != p.name {
            continue;
        }
        let id = item["guid"]
            .as_str()
            .ok_or(PublicationError::Ambiguous(None))?;
        // Search rows need not include versions or hashes. Resolve the exact
        // version chosen by NyxID and verify owner, ACL, closure and ZIP bytes.
        match verify(reader, actor, p, id).await {
            Ok(_) => {
                ids.insert(id.to_owned());
            }
            Err(PublicationError::Integrity) => {
                mismatched = true;
            }
            Err(error) => return Err(error),
        }
    }
    if ids.is_empty() && mismatched {
        return Err(PublicationError::Integrity);
    }
    if ids.len() != 1 {
        return Err(PublicationError::Ambiguous(None));
    }
    ids.into_iter()
        .next()
        .ok_or(PublicationError::Ambiguous(None))
}

pub(super) fn binding(
    row: &crate::models::assistant_agent_learning::AssistantAgentLearningProposal,
    p: &LearningPublication,
    skills_revision: i64,
) -> Value {
    let mut binding = json!({"agent_id":row.agent_id,"proposal_id":row.id,"owner_id":row.owner_id,"config_revision":row.config_revision,"skills_revision":skills_revision,"revision":row.revision,"fingerprint":row.fingerprint,"operation_id":p.operation_id,"package_sha256":p.sha256,"version":p.version});
    if row.source == crate::models::assistant_agent_learning::ProposalSource::Authored {
        binding["authored_skill"] = json!({"agent_id":row.agent_id,"proposal_id":row.id,"revision":row.revision,"skills_revision":skills_revision});
    }
    binding
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use uuid::Uuid;

    struct ClassifiedReader(OrnnCode, u16);
    #[async_trait::async_trait]
    impl OrnnReader for ClassifiedReader {
        async fn get(&self, _path: &str) -> AppResult<Vec<u8>> {
            Err(invalid())
        }
        async fn classified(&self, _method: Method, _path: &str, _body: Vec<u8>) -> OrnnOutcome {
            OrnnOutcome::Response {
                status: self.1,
                code: self.0,
            }
        }
    }
    struct TimedOutReader;
    #[async_trait::async_trait]
    impl OrnnReader for TimedOutReader {
        async fn get(&self, _path: &str) -> AppResult<Vec<u8>> {
            Err(AppError::ServicePoolInfrastructureUnavailable)
        }
        async fn classified(&self, _method: Method, _path: &str, _body: Vec<u8>) -> OrnnOutcome {
            OrnnOutcome::Uncertain
        }
    }

    #[tokio::test]
    async fn publication_refusals_distinguish_no_write_from_version_conflict() {
        let base = SkillPin {
            source: "ornn".into(),
            skill_id: Uuid::new_v4().to_string(),
            name: "base".into(),
            version: "1.0".into(),
            sha256: "a".repeat(64),
        };
        assert!(matches!(
            publish_classified(
                &ClassifiedReader(OrnnCode::BreakingChangeWithoutMajorBump, 409),
                Some(&base),
                vec![]
            )
            .await,
            PublishOutcome::Refused {
                code: FailureCode::OrnnInterfaceChangeRequiresMajor,
                status: Some(409)
            }
        ));
        assert!(matches!(
            publish_classified(
                &ClassifiedReader(OrnnCode::SkillVersionExists, 409),
                Some(&base),
                vec![]
            )
            .await,
            PublishOutcome::VersionConflict { status: 409 }
        ));
        assert!(matches!(
            publish_classified(
                &ClassifiedReader(OrnnCode::VersionNotIncremented, 409),
                Some(&base),
                vec![]
            )
            .await,
            PublishOutcome::VersionConflict { status: 409 }
        ));
        assert!(matches!(
            publish_classified(
                &ClassifiedReader(OrnnCode::Unknown, 400),
                Some(&base),
                vec![]
            )
            .await,
            PublishOutcome::Uncertain { status: Some(400) }
        ));
        assert!(matches!(
            publish_classified(
                &ClassifiedReader(OrnnCode::Unknown, 503),
                Some(&base),
                vec![]
            )
            .await,
            PublishOutcome::Uncertain { status: Some(503) }
        ));
        assert!(matches!(
            publish_classified(&TimedOutReader, Some(&base), vec![]).await,
            PublishOutcome::Uncertain { status: None }
        ));
    }

    #[tokio::test]
    async fn validator_distinguishes_package_errors_from_access_and_transport() {
        for (status, code, expected) in [
            (422, OrnnCode::ValidationFailed, "ornn_validation_failed"),
            (401, OrnnCode::AuthMissing, "ornn_auth_missing"),
            (403, OrnnCode::Forbidden, "ornn_read_forbidden"),
        ] {
            let error = validate(&ClassifiedReader(code, status), b"zip")
                .await
                .unwrap_err();
            assert_eq!(
                error.into_app_error().to_string(),
                AppError::Conflict(expected.into()).to_string()
            );
        }
        assert!(matches!(
            validate(&ClassifiedReader(OrnnCode::Unknown, 503), b"zip").await,
            Err(PublicationError::Unavailable(Some(503)))
        ));
        let anomalous = read_result(OrnnOutcome::Response {
            status: 451,
            code: OrnnCode::Forbidden,
        })
        .unwrap_err();
        assert_eq!(anomalous.status(), Some(451));
        assert!(matches!(
            anomalous,
            PublicationError::Unavailable(Some(451))
        ));
    }

    #[tokio::test]
    async fn malformed_or_interrupted_reads_retry_and_only_mismatches_are_integrity() {
        struct Scripted {
            meta: Value,
            garbled: bool,
            download: Option<Vec<u8>>,
            verdict: &'static [u8],
        }
        #[async_trait::async_trait]
        impl OrnnReader for Scripted {
            async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
                self.classified(Method::GET, path, Vec::new())
                    .await
                    .into_app_result()
            }
            async fn classified(&self, method: Method, path: &str, _body: Vec<u8>) -> OrnnOutcome {
                let body = |value: Value| OrnnOutcome::Ok(serde_json::to_vec(&value).unwrap());
                if method == Method::POST {
                    return OrnnOutcome::Ok(self.verdict.to_vec());
                }
                if path.ends_with("/download") {
                    return self
                        .download
                        .clone()
                        .map_or(OrnnOutcome::Interrupted, OrnnOutcome::Ok);
                }
                if path.contains("/closure") {
                    return body(json!({"data":{"items":[]}}));
                }
                if self.garbled {
                    return OrnnOutcome::Ok(b"<html>gateway</html>".to_vec());
                }
                body(json!({ "data": self.meta }))
            }
        }
        let id = Uuid::new_v4().to_string();
        let bytes = package_with_snapshot(&draft(), "op", "safe-helper", "1.0", None).unwrap();
        let p = LearningPublication {
            operation_id: "op".into(),
            name: "safe-helper".into(),
            version: "1.0".into(),
            sha256: hash(&bytes),
            started: true,
            ..Default::default()
        };
        let meta = json!({"guid":id,"name":"safe-helper","version":"1.0","skillHash":p.sha256,
            "isPrivate":true,"createdBy":"owner","sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]});
        let reader = |meta: Value, garbled: bool, download: Option<Vec<u8>>| Scripted {
            meta,
            garbled,
            download,
            verdict: br#"{"data":{"valid":true,"violations":[]}}"#,
        };
        let check = |reader: Scripted| {
            let p = p.clone();
            let id = id.clone();
            async move { verify(&reader, "owner", &p, &id).await.map(|_| ()) }
        };
        assert!(
            check(reader(meta.clone(), false, Some(bytes.clone())))
                .await
                .is_ok()
        );
        for retryable in [
            reader(meta.clone(), true, Some(bytes.clone())),
            reader(meta.clone(), false, None),
            reader(
                {
                    let mut m = meta.clone();
                    m.as_object_mut().unwrap().remove("createdBy");
                    m
                },
                false,
                Some(bytes.clone()),
            ),
        ] {
            assert!(matches!(
                check(retryable).await,
                Err(PublicationError::Unavailable(None))
            ));
        }
        for mismatch in [
            reader(meta.clone(), false, Some(b"other bytes".to_vec())),
            reader(
                {
                    let mut m = meta.clone();
                    m["createdBy"] = json!("someone-else");
                    m
                },
                false,
                Some(bytes.clone()),
            ),
            reader(
                {
                    let mut m = meta.clone();
                    m["sharedWithUsers"] = json!(["friend"]);
                    m
                },
                false,
                Some(bytes.clone()),
            ),
        ] {
            assert!(matches!(
                check(mismatch).await,
                Err(PublicationError::Integrity)
            ));
        }
        let verdict = |verdict: &'static [u8]| Scripted {
            meta: Value::Null,
            garbled: false,
            download: None,
            verdict,
        };
        assert!(
            validate(&verdict(br#"{"data":{"valid":true}}"#), b"zip")
                .await
                .is_ok()
        );
        assert!(matches!(
            validate(&verdict(b"<html>gateway</html>"), b"zip").await,
            Err(PublicationError::Unavailable(None))
        ));
        assert!(matches!(
            validate(&verdict(br#"{"data":{}}"#), b"zip").await,
            Err(PublicationError::Unavailable(None))
        ));
        assert!(matches!(
            validate(&verdict(br#"{"data":{"valid":false}}"#), b"zip").await,
            Err(PublicationError::Validation(None))
        ));
        assert!(matches!(
            validate(
                &verdict(br#"{"data":{"valid":true,"violations":[{"rule":"x"}]}}"#),
                b"zip"
            )
            .await,
            Err(PublicationError::Validation(None))
        ));
    }

    #[test]
    fn local_refusals_before_dispatch_are_neutral_nyxid_refusals() {
        for outcome in [
            OrnnOutcome::LocalRefusal(skills::OrnnLocalRefusal::ServiceUnavailable),
            OrnnOutcome::LocalError(AppError::NotFound("Conversation not found".into())),
        ] {
            let error = read_result(outcome).unwrap_err();
            assert!(matches!(error, PublicationError::Refused));
            assert_eq!(error.status(), None);
            for (stage, code) in [
                (PublicationStage::FormatValidate, FailureCode::NyxidRefused),
                (PublicationStage::BaseVerify, FailureCode::NyxidRefused),
                (PublicationStage::Reconcile, FailureCode::PublishUncertain),
                (PublicationStage::Verify, FailureCode::VerifyFailed),
            ] {
                assert_eq!(error.failure_code(stage), code);
            }
        }
        assert!(matches!(
            read_result(OrnnOutcome::Uncertain),
            Err(PublicationError::Unavailable(None))
        ));
    }

    #[tokio::test]
    async fn reconciliation_transport_failure_cannot_authorize_a_write() {
        let p = LearningPublication {
            operation_id: Uuid::new_v4().to_string(),
            name: "safe-helper".into(),
            version: "1.0".into(),
            sha256: "a".repeat(64),
            started: true,
            ..Default::default()
        };
        assert!(matches!(
            reconcile(&TimedOutReader, "owner", &p, None).await,
            Err(PublicationError::Unavailable(None))
        ));
    }

    #[tokio::test]
    async fn create_reconciliation_reports_an_exact_name_with_a_different_version_as_conflict() {
        struct Reader {
            id: String,
        }
        #[async_trait::async_trait]
        impl OrnnReader for Reader {
            async fn get(&self, _path: &str) -> AppResult<Vec<u8>> {
                Err(invalid())
            }
            async fn classified(&self, _method: Method, path: &str, _body: Vec<u8>) -> OrnnOutcome {
                let data = if path.starts_with("/api/v1/skill-search?") {
                    json!({"data":{"items":[{"guid":self.id,"name":"same-name"}],"totalPages":1}})
                } else {
                    json!({"data":{"guid":self.id,"name":"same-name","version":"1.0","skillHash":"different","isPrivate":true,"createdBy":"owner","sharedWithUsers":[],"sharedWithOrgs":[],"grants":[]}})
                };
                OrnnOutcome::Ok(serde_json::to_vec(&data).unwrap())
            }
        }
        let publication = LearningPublication {
            operation_id: Uuid::new_v4().to_string(),
            name: "same-name".into(),
            version: "1.0".into(),
            sha256: "a".repeat(64),
            ..Default::default()
        };
        let result = reconcile(
            &Reader {
                id: Uuid::new_v4().to_string(),
            },
            "owner",
            &publication,
            None,
        )
        .await;
        assert!(matches!(result, Err(PublicationError::Integrity)));
    }

    #[test]
    fn per_runtime_interface_mismatch_is_rejected() {
        let value = json!({"category":"runtime-based","outputType":"text","runtimes":[
            {"runtime":"python","dependencies":[{"library":"requests","version":"*"}],"envs":[{"var":"API_KEY","description":""}]},
            {"runtime":"node","dependencies":[],"envs":[{"var":"API_KEY","description":""}]}
        ]});
        assert!(
            matches!(InterfaceSnapshot::from_metadata(&value), Err(AppError::Conflict(code)) if code == "base_interface_incompatible")
        );
    }

    #[test]
    fn interface_snapshot_emits_flat_frontmatter_without_changing_legacy_bytes() {
        let meta = json!({"category":"mixed","outputType":"file","runtimes":[
            {"runtime":"python","dependencies":[{"library":"requests==2.31","version":"*"}],"envs":[{"var":"API_KEY","description":""}]},
            {"runtime":"node","dependencies":[{"library":"requests==2.31","version":"*"}],"envs":[{"var":"API_KEY","description":""}]}
        ],"tools":[{"tool":"Bash","type":"mcp"}]});
        let snapshot = InterfaceSnapshot::from_metadata(&meta).unwrap();
        let front =
            skill_markdown_with_snapshot(&draft(), "op", "safe-helper", "1.1", Some(&snapshot))
                .unwrap();
        assert!(front.contains("  category: \"mixed\""));
        assert!(front.contains("  runtime: [\"python\",\"node\"]"));
        assert!(front.contains("  runtime-dependency: [\"requests==2.31\"]"));
        assert!(front.contains("  runtime-env-var: [\"API_KEY\"]"));
        assert!(front.contains("  tool-list: [\"Bash\"]"));
        let legacy =
            skill_markdown_with_snapshot(&draft(), "op", "safe-helper", "1.1", None).unwrap();
        assert!(legacy.contains("  category: plain\n  generated-by"));
    }

    #[test]
    fn interface_snapshot_round_trips_ornn_interface_and_tags() {
        for (category, runtimes, tools, output) in [
            ("plain", json!([]), json!([]), Value::Null),
            (
                "tool-based",
                json!([]),
                json!([{"tool":"Bash"}]),
                Value::Null,
            ),
            (
                "runtime-based",
                json!([{"runtime":"python","dependencies":[{"library":"requests"}],"envs":[{"var":"API_KEY"}]}]),
                json!([]),
                json!("text"),
            ),
            (
                "mixed",
                json!([{"runtime":"python","dependencies":[{"library":"requests"}],"envs":[{"var":"API_KEY"}]}]),
                json!([{"tool":"Bash"}]),
                json!("file"),
            ),
        ] {
            let meta = json!({"category":category,"outputType":output,"runtimes":runtimes,"tools":tools,"tags":["review-tools","v2"]});
            let snapshot = InterfaceSnapshot::from_metadata(&meta).unwrap();
            let front =
                skill_markdown_with_snapshot(&draft(), "op", "safe-helper", "1.1", Some(&snapshot))
                    .unwrap();
            let field = |name: &str| -> Option<Value> {
                front
                    .lines()
                    .find_map(|line| line.strip_prefix(&format!("  {name}: ")))
                    .map(|text| serde_json::from_str(text).unwrap())
            };
            let extracted = InterfaceSnapshot {
                category: field("category").unwrap().as_str().unwrap().to_owned(),
                output_type: field("output-type").and_then(|v| v.as_str().map(str::to_owned)),
                runtimes: field("runtime")
                    .map_or_else(Vec::new, |v| serde_json::from_value(v).unwrap()),
                runtime_dependencies: field("runtime-dependency")
                    .map_or_else(Vec::new, |v| serde_json::from_value(v).unwrap()),
                runtime_env_vars: field("runtime-env-var")
                    .map_or_else(Vec::new, |v| serde_json::from_value(v).unwrap()),
                tools: field("tool-list")
                    .map_or_else(Vec::new, |v| serde_json::from_value(v).unwrap()),
                tags: field("tag").map_or_else(Vec::new, |v| serde_json::from_value(v).unwrap()),
            };
            assert!(snapshot == extracted);
        }
        assert!(
            InterfaceSnapshot::from_metadata(&json!({"category":"plain","tags":["Invalid"]}))
                .is_err()
        );
        assert!(
            InterfaceSnapshot::from_metadata(&json!({"category":"plain","tags":vec!["valid";11]}))
                .is_err()
        );
    }

    #[test]
    fn emitted_frontmatter_preserves_ornn_interface_independently() {
        fn interface(meta: &Value) -> Value {
            let runtimes: std::collections::BTreeMap<String, Value> = meta["runtimes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|runtime| {
                    let names = |key: &str, field: &str| -> std::collections::BTreeSet<String> {
                        runtime[key]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|entry| entry[field].as_str().map(str::to_owned))
                            .collect()
                    };
                    (
                        runtime["runtime"].as_str().unwrap().to_owned(),
                        json!({"dependencies":names("dependencies", "library"),
                            "envs":names("envs", "var")}),
                    )
                })
                .collect();
            let tools: std::collections::BTreeSet<String> = meta["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|tool| tool["tool"].as_str().map(str::to_owned))
                .collect();
            json!({"category":meta["category"],"outputType":meta["outputType"],
                "runtimes":runtimes,"tools":tools})
        }
        for (category, runtimes, tools, output) in [
            ("plain", json!([]), json!([]), Value::Null),
            (
                "tool-based",
                json!([]),
                json!([{"tool":"Bash"},{"tool":"Browser"}]),
                Value::Null,
            ),
            (
                "runtime-based",
                json!([
                    {"runtime":"python","dependencies":[{"library":"requests","version":"*"},{"library":"pydantic","version":"*"}],"envs":[{"var":"API_KEY"},{"var":"REGION"}]},
                    {"runtime":"node","dependencies":[{"library":"pydantic","version":"*"},{"library":"requests","version":"*"}],"envs":[{"var":"REGION"},{"var":"API_KEY"}]}
                ]),
                json!([]),
                json!("text"),
            ),
            (
                "mixed",
                json!([
                    {"runtime":"python","dependencies":[{"library":"requests"}],"envs":[{"var":"API_KEY"}]},
                    {"runtime":"node","dependencies":[{"library":"requests"}],"envs":[{"var":"API_KEY"}]}
                ]),
                json!([{"tool":"Bash"},{"tool":"Browser"}]),
                json!("file"),
            ),
        ] {
            let original = json!({"category":category,"outputType":output,
                "runtimes":runtimes,"tools":tools});
            let snapshot = InterfaceSnapshot::from_metadata(&original).unwrap();
            let front =
                skill_markdown_with_snapshot(&draft(), "op", "safe-helper", "1.1", Some(&snapshot))
                    .unwrap();
            let field = |name: &str| -> Option<Value> {
                front
                    .lines()
                    .find_map(|line| line.strip_prefix(&format!("  {name}: ")))
                    .map(|value| serde_json::from_str(value).unwrap())
            };
            let names = |key: &str| -> Vec<String> {
                field(key).map_or_else(Vec::new, |value| serde_json::from_value(value).unwrap())
            };
            let dependencies = names("runtime-dependency");
            let envs = names("runtime-env-var");
            let extracted = json!({
                "category":field("category").unwrap(),
                "outputType":field("output-type"),
                "runtimes":names("runtime").into_iter().map(|runtime| json!({
                    "runtime":runtime,
                    "dependencies":dependencies.iter().map(|library| json!({"library":library,"version":"*"})).collect::<Vec<_>>(),
                    "envs":envs.iter().map(|var| json!({"var":var,"description":""})).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "tools":names("tool-list").into_iter().map(|tool| json!({"tool":tool,"type":"mcp"})).collect::<Vec<_>>(),
            });
            assert_eq!(interface(&original), interface(&extracted), "{category}");
        }
    }

    fn draft() -> GeneratedProposal {
        GeneratedProposal {
            schema_version: 1,
            kind: "new".into(),
            name: "Safe helper".into(),
            description: "quoted: value".into(),
            skill_md: "# Guidance\nUse the reviewed steps.".into(),
            files: vec![],
            base_skill: None,
            rationale: "bounded".into(),
            safety_notes: "review".into(),
        }
    }

    #[test]
    fn package_quotes_frontmatter_and_is_deterministic() {
        let a = package_with_snapshot(&draft(), "op-1", "safe-helper", "1.0", None).unwrap();
        let b = package_with_snapshot(&draft(), "op-1", "safe-helper", "1.0", None).unwrap();
        assert_eq!(a, b);
        let mut zip = zip::ZipArchive::new(Cursor::new(a)).unwrap();
        let mut body = String::new();
        zip.by_name("safe-helper/SKILL.md")
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert!(body.contains("description: \"quoted: value\""));
        assert!(body.contains("operation-id: \"op-1\""));
    }

    #[test]
    fn improvement_versions_are_strictly_incremented_and_bounded() {
        let pin = SkillPin {
            source: "ornn".into(),
            skill_id: Uuid::new_v4().to_string(),
            name: "x".into(),
            version: "2.9".into(),
            sha256: "a".repeat(64),
        };
        assert_eq!(next_version(Some(&pin)).unwrap(), "2.10");
        let mut bad = pin;
        bad.version = "2".into();
        assert!(next_version(Some(&bad)).is_err());
    }
}
