use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use mongodb::options::ReturnDocument;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::platform_vendor_template::{COLLECTION_NAME, PlatformVendorTemplate};
use crate::services::platform_operation_service::{
    DEFAULT_PLATFORM_VENDOR_TEMPLATES, SeededPlatformVendorTemplate,
    vendor_contract_for_operation_name,
};
use crate::services::url_validation::validate_base_url;

const MAX_VENDOR_KEY_LENGTH: usize = 64;
const MAX_VENDOR_SLUG_LENGTH: usize = 100;
const MAX_DISPLAY_NAME_LENGTH: usize = 200;
const MAX_CREDENTIAL_LABEL_LENGTH: usize = 120;
const MAX_HELP_LENGTH: usize = 4_096;

pub async fn list_templates(
    db: &mongodb::Database,
    include_inactive: bool,
) -> AppResult<Vec<PlatformVendorTemplate>> {
    let filter = if include_inactive {
        doc! {}
    } else {
        doc! { "is_active": true }
    };
    db.collection::<PlatformVendorTemplate>(COLLECTION_NAME)
        .find(filter)
        .await?
        .try_collect()
        .await
        .map_err(AppError::DatabaseError)
}

pub async fn seed_default_templates(db: &mongodb::Database, actor: &str) -> AppResult<()> {
    let collection = db.collection::<PlatformVendorTemplate>(COLLECTION_NAME);
    let now = Utc::now();
    for seed in DEFAULT_PLATFORM_VENDOR_TEMPLATES {
        let template = seeded_template(seed, now, actor);
        let document = mongodb::bson::to_document(&template).map_err(|error| {
            AppError::Internal(format!(
                "Failed to serialize platform vendor template: {error}"
            ))
        })?;
        collection
            .update_one(
                doc! { "vendor": seed.vendor },
                doc! { "$setOnInsert": document },
            )
            .upsert(true)
            .await?;
    }
    Ok(())
}

pub async fn create_template(
    db: &mongodb::Database,
    input: PlatformVendorTemplateInput,
    actor: &str,
) -> AppResult<PlatformVendorTemplate> {
    let input = normalize_input(input);
    validate_input(&input)?;
    let now = Utc::now();
    let template = PlatformVendorTemplate {
        id: Uuid::new_v4().to_string(),
        vendor: input.vendor,
        display_name: input.display_name,
        slug: input.slug,
        base_url: input.base_url,
        auth_method: input.auth_method,
        auth_key_name: input.auth_key_name,
        credential_label: input.credential_label,
        credential_note: input.credential_note,
        operation: input.operation,
        capability_summary: input.capability_summary,
        restriction_summary: input.restriction_summary,
        is_active: input.is_active,
        is_seeded: false,
        created_at: now,
        updated_at: now,
        updated_by: actor.to_string(),
    };
    db.collection::<PlatformVendorTemplate>(COLLECTION_NAME)
        .insert_one(&template)
        .await
        .map_err(|error| {
            if error.to_string().contains("duplicate key") {
                AppError::Conflict(
                    "A vendor template with this key or slug already exists".to_string(),
                )
            } else {
                AppError::DatabaseError(error)
            }
        })?;
    Ok(template)
}

pub async fn update_template(
    db: &mongodb::Database,
    id: &str,
    input: PlatformVendorTemplateInput,
    actor: &str,
) -> AppResult<PlatformVendorTemplate> {
    let input = normalize_input(input);
    validate_input(&input)?;
    let now = Utc::now();
    let set = doc! {
        "vendor": input.vendor,
        "display_name": input.display_name,
        "slug": input.slug,
        "base_url": input.base_url,
        "auth_method": input.auth_method,
        "auth_key_name": input.auth_key_name,
        "credential_label": input.credential_label,
        "credential_note": input.credential_note,
        "operation": input.operation,
        "capability_summary": input.capability_summary,
        "restriction_summary": input.restriction_summary,
        "is_active": input.is_active,
        "updated_at": bson::DateTime::from_chrono(now),
        "updated_by": actor,
    };
    db.collection::<PlatformVendorTemplate>(COLLECTION_NAME)
        .find_one_and_update(doc! { "_id": id }, doc! { "$set": set })
        .return_document(ReturnDocument::After)
        .await
        .map_err(|error| {
            if error.to_string().contains("duplicate key") {
                AppError::Conflict(
                    "A vendor template with this key or slug already exists".to_string(),
                )
            } else {
                AppError::DatabaseError(error)
            }
        })?
        .ok_or_else(|| AppError::NotFound("Vendor template not found".to_string()))
}

pub async fn disable_template(db: &mongodb::Database, id: &str, actor: &str) -> AppResult<()> {
    let result = db
        .collection::<PlatformVendorTemplate>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": id, "is_active": true },
            doc! {
                "$set": {
                    "is_active": false,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    "updated_by": actor,
                }
            },
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "Active vendor template not found".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct PlatformVendorTemplateInput {
    pub vendor: String,
    pub display_name: String,
    pub slug: String,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: Option<String>,
    pub credential_label: String,
    pub credential_note: String,
    pub operation: Option<String>,
    pub capability_summary: String,
    pub restriction_summary: String,
    pub is_active: bool,
}

fn normalize_input(mut input: PlatformVendorTemplateInput) -> PlatformVendorTemplateInput {
    input.vendor = input.vendor.trim().to_string();
    input.display_name = input.display_name.trim().to_string();
    input.slug = input.slug.trim().to_string();
    input.base_url = input.base_url.trim().to_string();
    input.auth_method = input.auth_method.trim().to_string();
    input.auth_key_name = input
        .auth_key_name
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));
    input.credential_label = input.credential_label.trim().to_string();
    input.credential_note = input.credential_note.trim().to_string();
    input.operation = input
        .operation
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));
    input.capability_summary = input.capability_summary.trim().to_string();
    input.restriction_summary = input.restriction_summary.trim().to_string();
    input
}

fn validate_input(input: &PlatformVendorTemplateInput) -> AppResult<()> {
    let vendor = input.vendor.trim();
    if vendor.is_empty()
        || vendor.len() > MAX_VENDOR_KEY_LENGTH
        || !vendor.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && (byte == b'_' || byte == b'-'))
        })
    {
        return Err(AppError::ValidationError(
            "vendor must use lowercase letters, digits, underscores, and hyphens".to_string(),
        ));
    }
    if input.display_name.trim().is_empty() || input.display_name.len() > MAX_DISPLAY_NAME_LENGTH {
        return Err(AppError::ValidationError(
            "display_name must be between 1 and 200 characters".to_string(),
        ));
    }
    let slug = input.slug.trim();
    if slug.len() < 9
        || slug.len() > MAX_VENDOR_SLUG_LENGTH
        || !slug.starts_with("platform-")
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(AppError::ValidationError(
            "slug must start with platform- and use lowercase letters, digits, and hyphens"
                .to_string(),
        ));
    }
    validate_base_url(input.base_url.trim())?;
    if !matches!(input.auth_method.as_str(), "header" | "bearer" | "basic") {
        return Err(AppError::ValidationError(
            "auth_method must be one of: header, bearer, basic".to_string(),
        ));
    }
    if input.auth_method == "header"
        && input
            .auth_key_name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
    {
        return Err(AppError::ValidationError(
            "auth_key_name is required for header auth_method".to_string(),
        ));
    }
    if input.credential_label.trim().is_empty()
        || input.credential_label.len() > MAX_CREDENTIAL_LABEL_LENGTH
    {
        return Err(AppError::ValidationError(
            "credential_label must be between 1 and 120 characters".to_string(),
        ));
    }
    for (label, value) in [
        ("credential_note", &input.credential_note),
        ("capability_summary", &input.capability_summary),
        ("restriction_summary", &input.restriction_summary),
    ] {
        if value.trim().is_empty() || value.len() > MAX_HELP_LENGTH {
            return Err(AppError::ValidationError(format!(
                "{label} must be between 1 and {MAX_HELP_LENGTH} characters"
            )));
        }
    }
    if let Some(operation) = input
        .operation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && let Some(contract) = vendor_contract_for_operation_name(operation)
    {
        if input.slug != contract.slug {
            return Err(AppError::ValidationError(format!(
                "operation '{operation}' requires canonical slug '{}'; template has '{}'",
                contract.slug, input.slug
            )));
        }
        if input.base_url.trim_end_matches('/') != contract.base_url {
            return Err(AppError::ValidationError(format!(
                "operation '{operation}' requires canonical base_url '{}'; template has '{}'",
                contract.base_url, input.base_url
            )));
        }
        if input.auth_method != contract.auth_method {
            return Err(AppError::ValidationError(format!(
                "operation '{operation}' requires auth_method '{}'; template has '{}'",
                contract.auth_method, input.auth_method
            )));
        }
        if contract.auth_key_name != input.auth_key_name.as_deref() {
            return Err(AppError::ValidationError(format!(
                "operation '{operation}' requires auth_key_name {:?}; template has {:?}",
                contract.auth_key_name, input.auth_key_name
            )));
        }
    }
    Ok(())
}

fn seeded_template(
    seed: SeededPlatformVendorTemplate,
    now: chrono::DateTime<Utc>,
    actor: &str,
) -> PlatformVendorTemplate {
    PlatformVendorTemplate {
        id: Uuid::new_v4().to_string(),
        vendor: seed.vendor.to_string(),
        display_name: seed.display_name.to_string(),
        slug: seed.slug.to_string(),
        base_url: seed.base_url.to_string(),
        auth_method: seed.auth_method.to_string(),
        auth_key_name: seed.auth_key_name.map(str::to_string),
        credential_label: seed.credential_label.to_string(),
        credential_note: seed.credential_note.to_string(),
        operation: seed.operation.map(str::to_string),
        capability_summary: seed.capability_summary.to_string(),
        restriction_summary: seed.restriction_summary.to_string(),
        is_active: true,
        is_seeded: true,
        created_at: now,
        updated_at: now,
        updated_by: actor.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::platform_operation_service::{
        DEFAULT_PLATFORM_VENDOR_TEMPLATES, vendor_requirement_for_operation,
    };

    #[test]
    fn seeded_templates_match_code_contracts() {
        for seed in DEFAULT_PLATFORM_VENDOR_TEMPLATES {
            let Some(operation) = seed.operation else {
                // Duffel is intentionally a credential template until a code
                // operation is shipped; there is no contract to cross-check.
                continue;
            };
            let operation =
                crate::services::platform_operation_service::parse_operation_name(operation)
                    .expect("seeded operation name");
            let contract = vendor_requirement_for_operation(operation);
            assert_eq!(seed.auth_method, contract.auth_method);
            assert_eq!(seed.auth_key_name, contract.auth_key_name);
        }
    }

    #[test]
    fn contradictory_operation_template_is_rejected() {
        let mut input = PlatformVendorTemplateInput {
            vendor: "bad-elevenlabs".to_string(),
            display_name: "Bad ElevenLabs".to_string(),
            slug: "platform-elevenlabs".to_string(),
            base_url: "https://api.elevenlabs.io".to_string(),
            auth_method: "bearer".to_string(),
            auth_key_name: None,
            credential_label: "API key".to_string(),
            credential_note: "note".to_string(),
            operation: Some("speak".to_string()),
            capability_summary: "capability".to_string(),
            restriction_summary: "restriction".to_string(),
            is_active: true,
        };
        let error = validate_input(&input).expect_err("contradictory template must be rejected");
        assert!(error.to_string().contains("requires auth_method 'header'"));
        input.auth_method = "header".to_string();
        input.auth_key_name = Some("xi-api-key".to_string());
        validate_input(&input).expect("matching template shape");
    }

    #[test]
    fn operation_template_cannot_override_canonical_identity() {
        let input = PlatformVendorTemplateInput {
            vendor: "alternate-elevenlabs".to_string(),
            display_name: "Alternate ElevenLabs".to_string(),
            slug: "platform-alternate-elevenlabs".to_string(),
            base_url: "https://api.elevenlabs.io".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: Some("xi-api-key".to_string()),
            credential_label: "API key".to_string(),
            credential_note: "note".to_string(),
            operation: Some("speak".to_string()),
            capability_summary: "capability".to_string(),
            restriction_summary: "restriction".to_string(),
            is_active: true,
        };
        let error = validate_input(&input).expect_err("operation identity must be canonical");
        assert!(
            error
                .to_string()
                .contains("requires canonical slug 'platform-elevenlabs'")
        );
    }
}
