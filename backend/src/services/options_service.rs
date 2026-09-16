//! Explicit option-set registry. Each resolver owns its domain vocabulary.
use mongodb::Database;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::service_account_scope_service as scopes;
use crate::errors::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionSet {
    ServiceScope,
}

impl OptionSet {
    pub fn resolve(name: &str) -> AppResult<Self> {
        match name {
            "service-scope" => Ok(Self::ServiceScope),
            _ => Err(AppError::NotFound("Unknown option set".into())),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct OptionsQuery {
    pub principal_type: String,
    pub owner_id: String,
    pub service_account_id: Option<String>,
    pub search: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

impl OptionsQuery {
    pub fn validate(&self) -> AppResult<()> {
        if self.principal_type != "service_account" {
            return Err(AppError::ValidationError(
                "service-scope requires principal_type=service_account".into(),
            ));
        }
        for id in std::iter::once(self.owner_id.as_str()).chain(self.service_account_id.as_deref())
        {
            if Uuid::parse_str(id).is_err() || id.len() != 36 {
                return Err(AppError::ValidationError(
                    "Owner and service account IDs must be UUIDs".into(),
                ));
            }
        }
        if self.search.as_ref().is_some_and(|s| s.len() > 200)
            || !(1..=100).contains(&self.limit.unwrap_or(50))
            || self.offset.unwrap_or(0) > scopes::MAX_CONFIGURED_SCOPES + scopes::DEFINITIONS.len()
        {
            return Err(AppError::ValidationError("Search must be at most 200 bytes, limit 1–100, and offset within the option-set bound".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OptionItem {
    pub value: String,
    pub label: String,
    pub description: String,
    pub group: String,
    /// backend_definition or configured_scope.
    pub source: String,
    pub owner_id: Option<String>,
    pub resource_id: Option<String>,
    pub disabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OptionsResponse {
    pub option_set: String,
    pub principal_type: String,
    pub owner_id: String,
    pub service_account_id: Option<String>,
    pub items: Vec<OptionItem>,
    /// Current editable account values, independent of search/pagination.
    pub selected_items: Vec<OptionItem>,
    pub total: usize,
    pub next_offset: Option<usize>,
    /// Hash of the full authorized option content and context, before paging.
    pub version: String,
    pub freshness: OptionsFreshness,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OptionsFreshness {
    pub definitions_version: String,
    pub resources: String,
    pub evaluated_at: String,
    /// Zero means ACLs and local resources are re-evaluated on every request.
    pub max_age_seconds: u32,
}

pub async fn resolve(
    db: &Database,
    option_set: OptionSet,
    query: &OptionsQuery,
    existing_scopes: Option<&str>,
) -> AppResult<OptionsResponse> {
    query.validate()?;
    let items = match option_set {
        OptionSet::ServiceScope => service_scope_items(db, &query.owner_id).await?,
    };
    let selected_items = existing_scopes
        .unwrap_or_default()
        .split_whitespace()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|value| {
            items
                .iter()
                .find(|item| item.value == value)
                .cloned()
                .unwrap_or_else(|| configured_item(value, &query.owner_id))
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec(&(
        &query.owner_id,
        &query.service_account_id,
        &items,
        &selected_items,
    ))
    .map_err(|e| AppError::Internal(format!("Options version serialization failed: {e}")))?;
    let version = format!(
        "{}:{}",
        scopes::DEFINITION_VERSION,
        hex::encode(Sha256::digest(encoded))
    );
    let search = query
        .search
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let matching: Vec<_> = items
        .into_iter()
        .filter(|item| {
            search.is_empty()
                || [&item.label, &item.value, &item.description]
                    .iter()
                    .any(|s| s.to_lowercase().contains(&search))
        })
        .collect();
    let total = matching.len();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50);
    let items = matching.into_iter().skip(offset).take(limit).collect();
    Ok(OptionsResponse {
        option_set: "service-scope".into(),
        principal_type: query.principal_type.clone(),
        owner_id: query.owner_id.clone(),
        service_account_id: query.service_account_id.clone(),
        items,
        selected_items,
        total,
        next_offset: (offset + limit < total).then_some(offset + limit),
        version,
        freshness: OptionsFreshness {
            definitions_version: scopes::DEFINITION_VERSION.into(),
            resources: "live".into(),
            evaluated_at: chrono::Utc::now().to_rfc3339(),
            max_age_seconds: 0,
        },
    })
}

async fn service_scope_items(db: &Database, owner: &str) -> AppResult<Vec<OptionItem>> {
    let mut items: Vec<_> = scopes::DEFINITIONS
        .iter()
        .map(|d| OptionItem {
            value: d.value.into(),
            label: d.label.into(),
            description: d.description.into(),
            group: "Permissions".into(),
            source: "backend_definition".into(),
            owner_id: None,
            resource_id: None,
            disabled: false,
            disabled_reason: None,
        })
        .collect();
    for value in scopes::configured_scopes(db, owner).await? {
        if !scopes::DEFINITIONS.iter().any(|item| item.value == value) {
            items.push(configured_item(&value, owner));
        }
    }
    Ok(items)
}

fn configured_item(value: &str, owner: &str) -> OptionItem {
    let description = match value {
        "proxy:*" => {
            "Previously configured alias of proxy (All services). Existing authorization checks still apply."
        }
        "groups" => {
            "Previously configured scope. Service accounts have no group memberships; userinfo returns an empty group list."
        }
        _ => {
            "Custom scope previously configured for this owner. Its effect depends on the service handling it."
        }
    };
    OptionItem {
        value: value.into(),
        label: value.into(),
        description: description.into(),
        group: "Previously configured scopes".into(),
        source: "configured_scope".into(),
        owner_id: Some(owner.into()),
        resource_id: None,
        disabled: false,
        disabled_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pagination_accepts_every_possible_final_item_offset() {
        let mut query = OptionsQuery {
            principal_type: "service_account".into(),
            owner_id: Uuid::new_v4().to_string(),
            service_account_id: None,
            search: None,
            limit: Some(1),
            offset: None,
        };
        let bound = scopes::MAX_CONFIGURED_SCOPES + scopes::DEFINITIONS.len();
        for offset in [10_000, bound - 1, bound] {
            query.offset = Some(offset);
            query.validate().unwrap();
        }
        query.offset = Some(bound + 1);
        assert!(query.validate().is_err());
    }
}
