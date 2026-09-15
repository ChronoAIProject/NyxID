use std::collections::{BTreeMap, HashSet};

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::app_requirement_manifest::{
    AppRequirementManifest, COLLECTION_NAME, CompiledManifest, Enforcement, ServiceRequirement,
    ValidatorSelection,
};
use crate::models::downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService};
use crate::models::oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient};

use super::{api_key_mutation_service, app_connect_rollout, validator_profiles};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishManifest {
    pub enforcement: Enforcement,
    pub requirements: Vec<ServiceRequirement>,
}

fn invalid(message: &str) -> AppError {
    AppError::AppRequirementsInvalid(message.into())
}

pub async fn list(
    db: &mongodb::Database,
    client_id: &str,
) -> AppResult<Vec<AppRequirementManifest>> {
    Ok(db
        .collection::<AppRequirementManifest>(COLLECTION_NAME)
        .find(doc! { "oauth_client_id": client_id })
        .sort(doc! { "version": -1 })
        .await?
        .try_collect()
        .await?)
}

pub async fn current(
    db: &mongodb::Database,
    client: &OauthClient,
) -> AppResult<AppRequirementManifest> {
    let version = client
        .current_manifest_version
        .ok_or_else(|| AppError::NotFound("No published requirements for this app".into()))?;
    db.collection::<AppRequirementManifest>(COLLECTION_NAME)
        .find_one(doc! { "oauth_client_id": &client.id, "version": i64::from(version) })
        .await?
        .ok_or_else(|| AppError::NotFound("No published requirements for this app".into()))
}

pub async fn compile(
    db: &mongodb::Database,
    client_id: &str,
    input: &mut PublishManifest,
) -> AppResult<CompiledManifest> {
    if input.requirements.len() > 25 {
        return Err(invalid("A manifest may contain at most 25 requirements"));
    }
    let mut ids = HashSet::new();
    let mut compiled = CompiledManifest::default();
    for requirement in &mut input.requirements {
        validate_fields(requirement)?;
        if !ids.insert(requirement.id.clone()) {
            return Err(invalid("Requirement ids must be unique"));
        }
        if let Some(prefix) = &requirement.any_of_catalog_prefix {
            // System is the durable provenance used by the catalog seed service.
            let seeded: Vec<DownstreamService> = db
                .collection::<DownstreamService>(CATALOG)
                .find(doc! { "created_by": "system", "is_active": true,
                "service_category": { "$ne": "provider" } })
                .await?
                .try_collect()
                .await?;
            requirement.any_of_catalog_slugs.extend(
                seeded
                    .into_iter()
                    .filter(|service| service.slug.starts_with(prefix))
                    .map(|service| service.slug),
            );
        }
        requirement.any_of_catalog_slugs.sort();
        requirement.any_of_catalog_slugs.dedup();
        if requirement.any_of_catalog_slugs.is_empty()
            || requirement.any_of_catalog_slugs.len() > 25
        {
            return Err(invalid(
                "Each requirement must resolve to 1 to 25 catalog slugs",
            ));
        }
        let catalog: Vec<DownstreamService> = db
            .collection::<DownstreamService>(CATALOG)
            .find(doc! { "slug": { "$in": &requirement.any_of_catalog_slugs } })
            .await?
            .try_collect()
            .await?;
        let by_slug: BTreeMap<_, _> = catalog
            .iter()
            .map(|service| (service.slug.as_str(), service))
            .collect();
        for slug in &requirement.any_of_catalog_slugs {
            let service = by_slug
                .get(slug.as_str())
                .ok_or_else(|| invalid("Unknown catalog slug"))?;
            if !service.is_active || service.service_category == "provider" {
                return Err(invalid(
                    "Requirements must name active, user-connectable catalog services",
                ));
            }
            if requirement.allow_no_credential
                && service.visibility != "public"
                && service
                    .developer_app_ids
                    .as_ref()
                    .is_some_and(|ids| ids.iter().any(|id| id == client_id))
            {
                return Err(invalid(
                    "Private no-credential services must have an independent prerequisite; they cannot depend on consent to this same app",
                ));
            }
            compiled
                .catalog_service_ids
                .insert(slug.clone(), service.id.clone());
        }
        if let ValidatorSelection::Profile { id } = &requirement.validator {
            let profile = validator_profiles::PROFILES
                .iter()
                .find(|profile| profile.id == id)
                .ok_or_else(|| invalid("Unknown validator profile"))?;
            if !requirement
                .any_of_catalog_slugs
                .iter()
                .any(|slug| profile.catalog_slugs.contains(&slug.as_str()))
            {
                return Err(invalid(
                    "The validator profile must apply to at least one selected catalog slug",
                ));
            }
            let mut profiles = BTreeMap::new();
            for slug in &requirement.any_of_catalog_slugs {
                let applicable = if profile.catalog_slugs.contains(&slug.as_str()) {
                    Some(profile)
                } else {
                    validator_profiles::for_slug(slug)
                }
                .ok_or_else(|| {
                    invalid("Every catalog alternative must have an applicable validator profile")
                })?;
                profiles.insert(slug.clone(), applicable.id.to_string());
                compiled
                    .validator_versions
                    .insert(applicable.id.to_string(), applicable.version);
            }
            compiled
                .validators_by_requirement
                .insert(requirement.id.clone(), profiles);
        }
    }
    Ok(compiled)
}

/// Only the profiles and versions frozen at publication can supply evidence.
pub fn compiled_profile(
    manifest: &AppRequirementManifest,
    requirement_id: &str,
    slug: &str,
) -> Option<&'static validator_profiles::ValidatorProfile> {
    let id = manifest
        .compiled
        .validators_by_requirement
        .get(requirement_id)?
        .get(slug)?;
    validator_profiles::PROFILES.iter().find(|profile| {
        profile.id == id
            && profile.catalog_slugs.contains(&slug)
            && manifest.compiled.validator_versions.get(id) == Some(&profile.version)
    })
}

fn validate_fields(requirement: &ServiceRequirement) -> AppResult<()> {
    if requirement.id.is_empty()
        || requirement.id.len() > 32
        || !requirement.id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(invalid("Requirement ids must match [a-z0-9_-]{1,32}"));
    }
    if requirement.label.trim().is_empty() || requirement.label.chars().count() > 160 {
        return Err(invalid(
            "Requirement labels must contain 1 to 160 characters",
        ));
    }
    if requirement.any_of_catalog_slugs.len() > 25
        || requirement
            .any_of_catalog_slugs
            .iter()
            .any(|slug| slug.is_empty() || slug.len() > 128)
    {
        return Err(invalid(
            "Catalog slugs must be nonempty and at most 128 bytes; maximum 25",
        ));
    }
    if requirement
        .any_of_catalog_prefix
        .as_ref()
        .is_some_and(|prefix| {
            prefix.is_empty()
                || prefix.len() > 128
                || !prefix.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                })
        })
    {
        return Err(invalid(
            "Catalog prefixes must contain lowercase letters, digits, hyphens, or underscores",
        ));
    }
    if requirement.accepted_credential_types.len() > 25
        || requirement.accepted_credential_types.iter().any(|kind| {
            !matches!(
                kind.as_str(),
                "api_key"
                    | "oauth2"
                    | "bearer"
                    | "basic"
                    | "node_managed"
                    | "ssh_certificate"
                    | "aws_sigv4"
                    | "gcp_service_account"
            )
        })
    {
        return Err(invalid("Unknown or excessive accepted credential types"));
    }
    if requirement.required_downstream_scopes.len() > 100
        || requirement.required_downstream_scopes.iter().any(|scope| {
            scope.is_empty() || scope.len() > 256 || scope.chars().any(char::is_whitespace)
        })
    {
        return Err(invalid(
            "Downstream scopes must be nonempty tokens of at most 256 bytes; maximum 100",
        ));
    }
    Ok(())
}

pub async fn publish(
    state: &AppState,
    client: &OauthClient,
    actor: &str,
    mut input: PublishManifest,
) -> AppResult<AppRequirementManifest> {
    if !app_connect_rollout::is_enabled_for(state, client).await? {
        return Err(AppError::AppConnectLinkNotFound);
    }
    let compiled = compile(&state.db, &client.id, &mut input).await?;
    let db = state.db.clone();
    let client_id = client.id.clone();
    let owner = client.created_by.clone();
    let published_by = actor.to_string();
    let id = Uuid::new_v4().to_string();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<AppRequirementManifest> = async {
                let live = db
                    .collection::<OauthClient>(CLIENTS)
                    .find_one(doc! {
                        "_id": &client_id, "created_by": &owner, "is_active": true,
                        "app_connect_capability_enabled": true,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound("No published requirements for this app".into())
                    })?;
                let version = live
                    .current_manifest_version
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| invalid("Manifest version limit reached"))?;
                let manifest = AppRequirementManifest {
                    id: id.clone(),
                    oauth_client_id: client_id.clone(),
                    version,
                    enforcement: input.enforcement,
                    requirements: input.requirements.clone(),
                    compiled: compiled.clone(),
                    published_by: published_by.clone(),
                    published_at: Utc::now(),
                };
                db.collection::<AppRequirementManifest>(COLLECTION_NAME)
                    .insert_one(&manifest)
                    .session(&mut *session)
                    .await?;
                db.collection::<OauthClient>(CLIENTS)
                    .update_one(
                        doc! { "_id": &client_id },
                        doc! {
                            "$set": { "current_manifest_version": i64::from(version),
                                "updated_at": bson::DateTime::from_chrono(manifest.published_at) }
                        },
                    )
                    .session(&mut *session)
                    .await?;
                Ok(manifest)
            }
            .await;
            api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(api_key_mutation_service::map_transaction_error)
}
