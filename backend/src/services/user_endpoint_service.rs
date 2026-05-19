use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::user_endpoint::{COLLECTION_NAME, UserEndpoint};
use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
use crate::services::url_validation::{validate_base_url, validate_optional_spec_url};

fn validate_endpoint_url(url: &str) -> AppResult<()> {
    // Skip URL validation for node-resolved endpoints (empty URL) and SSH endpoints.
    if url.is_empty() || url.starts_with("ssh://") {
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
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserEndpoint>(COLLECTION_NAME)
        .insert_one(&endpoint)
        .await?;

    Ok(endpoint)
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

    if url.is_none() && label.is_none() && spec_update.is_none() {
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
}
