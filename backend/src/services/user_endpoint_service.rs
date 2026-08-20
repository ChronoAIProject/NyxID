use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::user_endpoint::{COLLECTION_NAME, UserEndpoint};
use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
use crate::services::url_validation::{
    reject_url_userinfo, validate_base_url, validate_optional_spec_url,
};

fn validate_endpoint_url(url: &str) -> AppResult<()> {
    // Node-resolved endpoints carry an empty URL.
    if url.is_empty() {
        return Ok(());
    }

    // `UserEndpoint.url` is echoed back in ordinary list and detail responses,
    // so a credential embedded in userinfo is a disclosure path. Reject
    // userinfo and fragments for EVERY accepted scheme. Previously `ssh://`
    // skipped validation entirely, and `validate_base_url` never inspected
    // userinfo at all, so `https://user:pass@host` was accepted too.
    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError("Invalid endpoint URL format".to_string()))?;
    reject_url_userinfo(&parsed)?;
    if parsed.fragment().is_some() {
        return Err(AppError::ValidationError(
            "Endpoint URL must not contain a fragment".to_string(),
        ));
    }

    // SSH endpoints are otherwise exempt from the HTTP base-URL rules.
    if url.starts_with("ssh://") {
        return Ok(());
    }

    validate_base_url(url)
}

fn validate_openapi_spec_url(url: &str) -> AppResult<()> {
    // Empty string is not accepted -- callers should pass None to clear.
    // `validate_optional_spec_url` enforces 2048-char ceiling + scheme +
    // cloud-metadata blocks. Deeper SSRF hardening happens at fetch time
    // in `api_docs_service::fetch_spec_json`.
    validate_optional_spec_url(url)
}

/// List all endpoints for a user, sorted by created_at descending.
pub async fn list_endpoints(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<UserEndpoint>> {
    let endpoints: Vec<UserEndpoint> = db
        .collection::<UserEndpoint>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    Ok(endpoints)
}

/// Get single endpoint by ID, verifying ownership.
pub async fn get_endpoint(
    db: &mongodb::Database,
    user_id: &str,
    endpoint_id: &str,
) -> AppResult<UserEndpoint> {
    db.collection::<UserEndpoint>(COLLECTION_NAME)
        .find_one(doc! { "_id": endpoint_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))
}

/// Create a new endpoint.
pub async fn create_endpoint(
    db: &mongodb::Database,
    user_id: &str,
    label: &str,
    url: &str,
    catalog_service_id: Option<&str>,
    openapi_spec_url: Option<&str>,
) -> AppResult<UserEndpoint> {
    if label.is_empty() || label.len() > 200 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 200 characters".to_string(),
        ));
    }
    validate_endpoint_url(url)?;
    let openapi_spec_url = match openapi_spec_url {
        Some(s) if !s.trim().is_empty() => {
            validate_openapi_spec_url(s.trim())?;
            Some(s.trim().to_string())
        }
        _ => None,
    };

    let now = Utc::now();
    let endpoint = UserEndpoint {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        label: label.to_string(),
        url: url.to_string(),
        catalog_service_id: catalog_service_id.map(|s| s.to_string()),
        openapi_spec_url,
        recommended_skills: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserEndpoint>(COLLECTION_NAME)
        .insert_one(&endpoint)
        .await?;

    Ok(endpoint)
}

/// Instance-level recommended-skill list limits (mirrors the catalog's
/// `required_permissions`-style caps).
const MAX_RECOMMENDED_SKILLS: usize = 20;
const MAX_RECOMMENDED_SKILL_LEN: usize = 128;

fn validate_recommended_skills(skills: &[String]) -> AppResult<()> {
    if skills.len() > MAX_RECOMMENDED_SKILLS {
        return Err(AppError::ValidationError(format!(
            "recommended_skills accepts at most {MAX_RECOMMENDED_SKILLS} entries"
        )));
    }
    for skill in skills {
        let trimmed = skill.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_RECOMMENDED_SKILL_LEN {
            return Err(AppError::ValidationError(format!(
                "recommended_skills entries must be 1-{MAX_RECOMMENDED_SKILL_LEN} characters"
            )));
        }
    }
    Ok(())
}

/// How the caller wants to treat `recommended_skills` on update.
#[derive(Debug, Default)]
pub enum RecommendedSkillsUpdate {
    /// Leave existing value untouched.
    #[default]
    Leave,
    /// Replace with a new list.
    Set(Vec<String>),
    /// Remove the field (an empty list from the client maps here).
    Clear,
}

/// How the caller wants to treat the `openapi_spec_url` field on update.
#[derive(Debug, Default)]
pub enum OpenApiSpecUrlUpdate<'a> {
    /// Leave existing value untouched.
    #[default]
    Leave,
    /// Replace with a new value.
    Set(&'a str),
    /// Remove the field (e.g. `""` from the client maps here).
    Clear,
}

/// Update endpoint URL, label, and/or OpenAPI spec URL.
pub async fn update_endpoint(
    db: &mongodb::Database,
    user_id: &str,
    endpoint_id: &str,
    url: Option<&str>,
    label: Option<&str>,
    openapi_spec_url: OpenApiSpecUrlUpdate<'_>,
    recommended_skills: RecommendedSkillsUpdate,
) -> AppResult<()> {
    let spec_update = match openapi_spec_url {
        OpenApiSpecUrlUpdate::Leave => None,
        OpenApiSpecUrlUpdate::Clear => Some(None),
        OpenApiSpecUrlUpdate::Set(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Some(None)
            } else {
                validate_openapi_spec_url(trimmed)?;
                Some(Some(trimmed.to_string()))
            }
        }
    };

    let skills_update = match recommended_skills {
        RecommendedSkillsUpdate::Leave => None,
        RecommendedSkillsUpdate::Clear => Some(None),
        RecommendedSkillsUpdate::Set(skills) => {
            let trimmed: Vec<String> = skills
                .iter()
                .map(|skill| skill.trim().to_string())
                .filter(|skill| !skill.is_empty())
                .collect();
            if trimmed.is_empty() {
                Some(None)
            } else {
                validate_recommended_skills(&trimmed)?;
                Some(Some(trimmed))
            }
        }
    };

    if url.is_none() && label.is_none() && spec_update.is_none() && skills_update.is_none() {
        return Err(AppError::BadRequest(
            "At least one field must be provided".to_string(),
        ));
    }

    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    let mut unset_doc = doc! {};

    if let Some(u) = url {
        validate_endpoint_url(u)?;
        set_doc.insert("url", u);
    }
    if let Some(l) = label {
        if l.is_empty() || l.len() > 200 {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 200 characters".to_string(),
            ));
        }
        set_doc.insert("label", l);
    }
    match spec_update {
        None => {}
        Some(Some(value)) => {
            set_doc.insert("openapi_spec_url", value);
        }
        Some(None) => {
            unset_doc.insert("openapi_spec_url", "");
        }
    }
    match skills_update {
        None => {}
        Some(Some(skills)) => {
            set_doc.insert("recommended_skills", skills);
        }
        Some(None) => {
            unset_doc.insert("recommended_skills", "");
        }
    }

    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    let result = db
        .collection::<UserEndpoint>(COLLECTION_NAME)
        .update_one(doc! { "_id": endpoint_id, "user_id": user_id }, update_doc)
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }

    Ok(())
}

/// Delete endpoint. Fails if any active UserService references it.
pub async fn delete_endpoint(
    db: &mongodb::Database,
    user_id: &str,
    endpoint_id: &str,
) -> AppResult<()> {
    // Verify ownership
    let _ = get_endpoint(db, user_id, endpoint_id).await?;

    // Check for active references
    let ref_count = db
        .collection::<mongodb::bson::Document>(USER_SERVICES)
        .count_documents(doc! {
            "endpoint_id": endpoint_id,
            "is_active": true,
        })
        .await?;

    if ref_count > 0 {
        return Err(AppError::Conflict(
            "Endpoint is in use by active services".to_string(),
        ));
    }

    db.collection::<UserEndpoint>(COLLECTION_NAME)
        .delete_one(doc! { "_id": endpoint_id, "user_id": user_id })
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_endpoint_url;

    #[test]
    fn validate_endpoint_url_accepts_empty_and_ssh_urls() {
        assert!(validate_endpoint_url("").is_ok());
        assert!(validate_endpoint_url("ssh://example.internal:22").is_ok());
    }

    #[test]
    fn validate_endpoint_url_accepts_http_urls() {
        assert!(validate_endpoint_url("https://api.example.com").is_ok());
        assert!(validate_endpoint_url("http://localhost:3000").is_ok());
    }

    #[test]
    fn validate_endpoint_url_rejects_non_http_non_ssh_urls() {
        assert!(validate_endpoint_url("ftp://example.com").is_err());
    }

    /// `UserEndpoint.url` is echoed back in ordinary list and detail
    /// responses, so a credential embedded in userinfo is a disclosure path.
    /// Every accepted scheme must reject it -- `ssh://` previously skipped
    /// validation entirely and `validate_base_url` never checked userinfo.
    #[test]
    fn validate_endpoint_url_rejects_userinfo_in_every_accepted_scheme() {
        assert!(validate_endpoint_url("https://user:pass@example.com").is_err());
        assert!(validate_endpoint_url("http://user:pass@example.com").is_err());
        assert!(validate_endpoint_url("https://user@example.com").is_err());
        assert!(validate_endpoint_url("ssh://user:pass@example.com:22").is_err());
        assert!(validate_endpoint_url("ssh://user@example.com:22").is_err());
    }

    #[test]
    fn validate_endpoint_url_rejects_percent_encoded_userinfo() {
        assert!(validate_endpoint_url("ssh://%75ser:%70ass@example.com:22").is_err());
        assert!(validate_endpoint_url("https://%75ser:%70ass@example.com").is_err());
    }

    #[test]
    fn validate_endpoint_url_rejects_fragments() {
        assert!(validate_endpoint_url("https://example.com/base#frag").is_err());
        assert!(validate_endpoint_url("ssh://example.internal:22#frag").is_err());
    }

    #[test]
    fn validate_endpoint_url_still_accepts_credential_free_ssh_and_http() {
        assert!(validate_endpoint_url("").is_ok());
        assert!(validate_endpoint_url("ssh://example.internal:22").is_ok());
        assert!(validate_endpoint_url("https://api.example.com").is_ok());
    }
}
