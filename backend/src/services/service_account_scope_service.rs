//! Suggestions for service-account scope fields; never an authorization allowlist.
use std::collections::BTreeSet;

use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Deserialize;

use crate::errors::{AppError, AppResult};
use crate::models::service_account::COLLECTION_NAME;
use crate::mw::auth::{LLM_PROXY_SCOPE, PROXY_SCOPE, WIDE_PROXY_SCOPE};
use crate::services::curation_grant_service::{READ_SCOPE, WRITE_SCOPE};

pub const DEFINITION_VERSION: &str = "service-account-suggestions-v6";
pub const MAX_OWNER_ACCOUNTS: i64 = 1_000;
pub const MAX_CONFIGURED_SCOPES: usize = 10_000;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;

pub struct ScopeDefinition {
    pub value: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

pub const DEFINITIONS: &[ScopeDefinition] = &[
    ScopeDefinition {
        value: PROXY_SCOPE,
        label: "All services",
        description: "Proxy access to services available to this account, including the LLM gateway. Existing resource and authorization checks still apply.",
    },
    ScopeDefinition {
        value: LLM_PROXY_SCOPE,
        label: "LLM gateway",
        description: "Access available LLM providers through the LLM gateway, including provider status.",
    },
    ScopeDefinition {
        value: "roles",
        label: "Role claims",
        description: "Include assigned roles and permissions in the account's OAuth userinfo response.",
    },
    ScopeDefinition {
        value: READ_SCOPE,
        label: "Read catalog skills",
        description: "When saved by a platform administrator, read metadata and skills for every catalog service, including GET /keys and /keys/{id}. Legacy account grants remain supported.",
    },
    ScopeDefinition {
        value: super::service_account_key_read_service::READ_SCOPE,
        label: "Read key metadata",
        description: "Read private connection metadata through a key read grant, or support legacy role-authorized catalog editors. Direct catalog:skills:read authority does not require this scope. Never delivers credentials.",
    },
    ScopeDefinition {
        value: WRITE_SCOPE,
        label: "Manage catalog skills",
        description: "When saved by a platform administrator, assign, replace, remove, and restore skills for every catalog service. Does not grant Ornn package editing or service execution.",
    },
    ScopeDefinition {
        value: WIDE_PROXY_SCOPE,
        label: "All services (proxy alias)",
        description: "Existing alias of proxy with the same service and LLM gateway access. Resource and authorization checks still apply; it adds no access beyond proxy.",
    },
    ScopeDefinition {
        value: "groups",
        label: "Group claims (empty for service accounts)",
        description: "Include groups in OAuth userinfo. Service accounts have no group memberships, so the group list is empty; this grants no access.",
    },
];

#[derive(Deserialize)]
struct ScopeProjection {
    allowed_scopes: String,
}

/// Includes disabled accounts because these are previously configured values,
/// not evidence of usable authority. The owner fallback matches ServiceAccount.
pub async fn configured_scopes(db: &Database, owner: &str) -> AppResult<BTreeSet<String>> {
    let mut cursor = db
        .collection::<ScopeProjection>(COLLECTION_NAME)
        .find(doc! { "$or": [
            { "owner_user_id": owner },
            { "owner_user_id": null, "created_by": owner },
        ] })
        .projection(doc! { "_id": 0, "allowed_scopes": 1 })
        .limit(MAX_OWNER_ACCOUNTS + 1)
        .batch_size(25)
        .await?;
    let mut values = BTreeSet::new();
    let mut records = 0;
    let mut bytes = 0;
    while let Some(row) = cursor.try_next().await? {
        records += 1;
        bytes += row.allowed_scopes.len();
        if records > MAX_OWNER_ACCOUNTS || bytes > MAX_SOURCE_BYTES {
            return Err(source_limit_error());
        }
        for value in row.allowed_scopes.split_whitespace() {
            values.insert(value.to_string());
            if values.len() > MAX_CONFIGURED_SCOPES {
                return Err(source_limit_error());
            }
        }
    }
    Ok(values)
}

fn source_limit_error() -> AppError {
    AppError::ValidationError("Configured scope suggestions exceed the source limit. Custom scope entry remains available.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_suggestions_match_existing_permission_checks() {
        assert!(crate::mw::auth::scope_allows_rest_proxy(
            DEFINITIONS[0].value
        ));
        assert!(crate::mw::auth::scope_allows_llm_proxy(
            DEFINITIONS[1].value
        ));
        assert!(!crate::mw::auth::scope_allows_rest_proxy(
            DEFINITIONS[1].value
        ));
        assert_eq!(DEFINITIONS[2].value, "roles");
    }

    #[test]
    fn editor_suggestions_explain_admin_scope_authority() {
        for scope in [READ_SCOPE, WRITE_SCOPE] {
            let entry = DEFINITIONS
                .iter()
                .find(|entry| entry.value == scope)
                .unwrap();
            assert!(
                entry
                    .description
                    .contains("saved by a platform administrator")
            );
        }
    }

    #[tokio::test]
    async fn configured_source_is_bounded_without_restricting_stored_values() {
        let db = crate::test_utils::connect_test_database("scope_suggestion_bounds")
            .await
            .expect("MongoDB required");
        db.collection::<mongodb::bson::Document>(COLLECTION_NAME)
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(), "created_by": "owner",
                "allowed_scopes": "x".repeat(MAX_SOURCE_BYTES + 1),
            })
            .await
            .unwrap();
        assert!(configured_scopes(&db, "owner").await.is_err());
        assert!(configured_scopes(&db, "other").await.unwrap().is_empty());
    }
}
