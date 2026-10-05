//! The fixed Ornn publication protocol. No model-selected destinations,
//! permissions, scripts or owner fallback. Calls are made by an injected
//! adapter using the approving person's signed identity.
use super::{
    agent_skill_service::{self as skills, OrnnReader, Preview},
    assistant_agent_learning::GeneratedProposal,
};
use crate::{
    errors::{AppError, AppResult},
    models::{assistant_agent_learning::LearningPublication, catalog_skill_revision::SkillPin},
};
use http::Method;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

fn invalid() -> AppError {
    AppError::Conflict("publication_integrity_failed".into())
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

pub(super) fn package(
    draft: &GeneratedProposal,
    operation: &str,
    name: &str,
    version: &str,
) -> AppResult<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    // JSON string literals are also safe YAML scalars (quotes/newlines cannot
    // introduce frontmatter keys). Every control field is server-selected.
    let quote = |text: &str| serde_json::to_string(text).map_err(|_| invalid());
    let front = format!(
        "---\nname: {}\ndescription: {}\nversion: {}\nmetadata:\n  category: plain\n  generated-by: nyxid-learning\n  operation-id: {}\n---\n\n",
        quote(name)?,
        quote(&draft.description)?,
        quote(version)?,
        quote(operation)?
    );
    zip.start_file(format!("{name}/SKILL.md"), options)
        .map_err(|_| invalid())?;
    zip.write_all(front.as_bytes())
        .and_then(|_| zip.write_all(draft.skill_md.as_bytes()))
        .map_err(|_| invalid())?;
    // validate_generated runs before this function. All file names are safe,
    // unique UTF-8 text paths; the archive is never extracted on the API host.
    for file in &draft.files {
        zip.start_file(format!("{name}/{}", file.path), options)
            .map_err(|_| invalid())?;
        zip.write_all(file.content.as_bytes())
            .map_err(|_| invalid())?;
    }
    Ok(zip.finish().map_err(|_| invalid())?.into_inner())
}

pub(super) fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
async fn data(reader: &impl OrnnReader, path: &str) -> AppResult<Value> {
    let bytes = reader.get(path).await?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    value.get("data").cloned().ok_or_else(invalid)
}
fn private_owner(meta: &Value, actor: &str) -> AppResult<()> {
    if meta["isPrivate"] != true
        || meta["createdBy"].as_str() != Some(actor)
        || ["sharedWithUsers", "sharedWithOrgs", "grants"]
            .iter()
            .any(|key| meta[*key].as_array().is_none_or(|v| !v.is_empty()))
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) async fn verify_base(
    reader: &impl OrnnReader,
    actor: &str,
    base: &SkillPin,
) -> AppResult<()> {
    let meta = data(
        reader,
        &format!("/api/v1/skills/{}?version={}", base.skill_id, base.version),
    )
    .await?;
    private_owner(&meta, actor)?;
    let preview = skills::preview(reader, &base.skill_id, &base.version).await?;
    if preview.reference.sha256 != base.sha256
        || preview.reference.name != base.name
        || !preview.reference.dependencies.is_empty()
    {
        return Err(invalid());
    }
    // Updating must extend this exact latest version, never an unrelated edit.
    let versions = data(
        reader,
        &format!("/api/v1/skills/{}/versions", base.skill_id),
    )
    .await?;
    let items = versions["items"].as_array().ok_or_else(invalid)?;
    if items.first().and_then(|v| v["version"].as_str()) != Some(&base.version) {
        return Err(AppError::Conflict("base_skill_changed".into()));
    }
    Ok(())
}

pub(super) async fn validate(reader: &impl OrnnReader, bytes: &[u8]) -> AppResult<()> {
    let response = reader
        .request(
            Method::POST,
            "/api/v1/skill-format/validate",
            bytes.to_vec(),
        )
        .await?;
    let response: Value = serde_json::from_slice(&response).map_err(|_| invalid())?;
    if response["data"]["valid"] != true
        || response["data"]["violations"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
    {
        return Err(AppError::ValidationError("ornn_validation_failed".into()));
    }
    Ok(())
}

pub(super) async fn publish(
    reader: &impl OrnnReader,
    base: Option<&SkillPin>,
    bytes: Vec<u8>,
) -> AppResult<String> {
    let (method, path) = base.map_or_else(
        || (Method::POST, "/api/v1/skills".into()),
        |b| (Method::PUT, format!("/api/v1/skills/{}", b.skill_id)),
    );
    // Once invoked, every failure is ambiguous, including malformed JSON and
    // an interrupted success body. The caller has durably marked started.
    let response = reader
        .request(method, &path, bytes)
        .await
        .map_err(|_| ambiguous())?;
    let response: Value = serde_json::from_slice(&response).map_err(|_| ambiguous())?;
    let id = response["data"]["guid"].as_str().ok_or_else(ambiguous)?;
    uuid::Uuid::parse_str(id).map_err(|_| ambiguous())?;
    if base.is_some_and(|b| b.skill_id != id) {
        return Err(ambiguous());
    }
    Ok(id.into())
}

pub(super) async fn verify(
    reader: &impl OrnnReader,
    actor: &str,
    p: &LearningPublication,
    id: &str,
) -> AppResult<Preview> {
    uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
    let meta = data(
        reader,
        &format!("/api/v1/skills/{id}?version={}", p.version),
    )
    .await?;
    private_owner(&meta, actor)?;
    if meta["guid"] != id
        || meta["version"] != p.version
        || meta["name"] != p.name
        || meta["skillHash"] != p.sha256
    {
        return Err(invalid());
    }
    let preview = skills::preview(reader, id, &p.version).await?;
    if preview.reference.sha256 != p.sha256 || !preview.reference.dependencies.is_empty() {
        return Err(invalid());
    }
    Ok(preview)
}

pub(super) async fn reconcile(
    reader: &impl OrnnReader,
    actor: &str,
    p: &LearningPublication,
    base: Option<&SkillPin>,
) -> AppResult<String> {
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
    let items = result["items"].as_array().ok_or_else(ambiguous)?;
    if items.len() > 20 || result["totalPages"].as_u64().is_some_and(|n| n > 1) {
        return Err(ambiguous());
    }
    let mut ids = std::collections::BTreeSet::new();
    for item in items {
        if item["name"] != p.name {
            continue;
        }
        let id = item["guid"].as_str().ok_or_else(ambiguous)?;
        // Search rows need not include versions or hashes. Resolve the exact
        // version chosen by NyxID and verify owner, ACL, closure and ZIP bytes.
        if verify(reader, actor, p, id).await.is_ok() {
            ids.insert(id.to_owned());
        }
    }
    if ids.len() != 1 {
        return Err(ambiguous());
    }
    ids.into_iter().next().ok_or_else(ambiguous)
}

pub(super) fn binding(
    row: &crate::models::assistant_agent_learning::AssistantAgentLearningProposal,
    p: &LearningPublication,
    skills_revision: i64,
) -> Value {
    json!({"agent_id":row.agent_id,"proposal_id":row.id,"owner_id":row.owner_id,"config_revision":row.config_revision,"skills_revision":skills_revision,"revision":row.revision,"fingerprint":row.fingerprint,"operation_id":p.operation_id,"package_sha256":p.sha256,"version":p.version})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use uuid::Uuid;

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
        let a = package(&draft(), "op-1", "safe-helper", "1.0").unwrap();
        let b = package(&draft(), "op-1", "safe-helper", "1.0").unwrap();
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
