//! Shared approval identity resolution for MCP-backed tool execution.

use crate::errors::{AppError, AppResult};
use crate::services::{mcp_service, proxy_service};

#[derive(Clone, Debug)]
pub(crate) struct McpApprovalTarget {
    pub(crate) service_id: String,
    pub(crate) service_name: String,
    pub(crate) service_slug: String,
    pub(crate) service_owner_user_id: String,
}

/// Resolve the approval policy identity for a loaded MCP service.
///
/// `UserService.id` selects the executable connection, while approval policy
/// rows for catalog-backed services are keyed by `catalog_service_id`. The
/// proxy hint resolvers preserve that distinction and re-check personal/org
/// access for the effective approval owner.
pub(crate) async fn approval_target_for_tool(
    db: &mongodb::Database,
    effective_approval_owner_user_id: &str,
    service: &mcp_service::McpToolService,
) -> AppResult<McpApprovalTarget> {
    let hint = match &service.source {
        mcp_service::McpToolSource::UserManaged {
            user_service_id, ..
        } => proxy_service::find_approval_resolution_hint_by_user_service_id(
            db,
            effective_approval_owner_user_id,
            user_service_id,
            Some(&service.service_slug),
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?,
        mcp_service::McpToolSource::Platform {
            downstream_service_id,
        } => proxy_service::find_approval_resolution_hint(
            db,
            effective_approval_owner_user_id,
            None,
            Some(downstream_service_id),
        )
        .await?
        .unwrap_or_else(|| proxy_service::ApprovalResolutionHint {
            service_id: downstream_service_id.clone(),
            service_owner_id: effective_approval_owner_user_id.to_string(),
        }),
    };

    Ok(McpApprovalTarget {
        service_id: hint.service_id,
        service_name: service.service_name.clone(),
        service_slug: service.service_slug.clone(),
        service_owner_user_id: hint.service_owner_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::service_approval_config::{
        ApprovalEffect, ApprovalMode, COLLECTION_NAME as SERVICE_APPROVAL_CONFIGS,
        ServiceApprovalConfig,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

    fn loaded_user_service(id: &str, owner_id: &str, slug: &str) -> mcp_service::McpToolService {
        mcp_service::McpToolService {
            service_id: id.to_string(),
            service_name: "Approval target".to_string(),
            service_slug: slug.to_string(),
            description: None,
            service_category: "user_service".to_string(),
            endpoints: Vec::new(),
            durable_endpoint_metadata: Default::default(),
            source: mcp_service::McpToolSource::UserManaged {
                user_service_id: id.to_string(),
                effective_owner_id: owner_id.to_string(),
                node_id: None,
                has_server_credential: true,
            },
            executable: true,
            is_generic_proxy: false,
            invalid_openapi_contract: false,
            recommended_skills: Vec::new(),
        }
    }

    async fn insert_deny_fixture(
        db: &mongodb::Database,
        actor_id: &str,
        owner_id: &str,
        catalog_id: &str,
        user_service_id: &str,
        slug: &str,
        org_owned: bool,
    ) {
        if org_owned {
            db.collection::<User>(USERS)
                .insert_one(crate::test_utils::test_user(owner_id, UserType::Org))
                .await
                .unwrap();
            db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                .insert_one(crate::test_utils::test_membership(
                    owner_id,
                    actor_id,
                    OrgRole::Member,
                    Some(vec![user_service_id.to_string()]),
                ))
                .await
                .unwrap();
        }
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(crate::test_utils::test_user_endpoint(
                &endpoint_id,
                owner_id,
                "Approval target",
                "https://approval.invalid",
                None,
                Some(catalog_id),
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(crate::test_utils::test_user_service(
                user_service_id,
                owner_id,
                slug,
                &endpoint_id,
                Some(catalog_id),
                None,
            ))
            .await
            .unwrap();
        let now = chrono::Utc::now();
        db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .insert_one(ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: owner_id.to_string(),
                service_id: catalog_id.to_string(),
                service_name: "Approval target".to_string(),
                approval_required: false,
                approval_mode: ApprovalMode::Grant,
                rules: Vec::new(),
                default_effect: Some(ApprovalEffect::Deny),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn poc_deny_target_uses_catalog_identity_and_personal_or_org_owner() {
        let Some(db) = crate::test_utils::connect_test_database("agent_poc_deny_parity").await
        else {
            eprintln!("skipping POC deny parity test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let operation =
            crate::services::operation_descriptor::build_mcp_descriptor("GET", "/health", None);

        for org_owned in [false, true] {
            let owner_id = if org_owned {
                uuid::Uuid::new_v4().to_string()
            } else {
                actor_id.clone()
            };
            let catalog_id = uuid::Uuid::new_v4().to_string();
            let user_service_id = uuid::Uuid::new_v4().to_string();
            let slug = format!("deny-{}", if org_owned { "org" } else { "personal" });
            insert_deny_fixture(
                &db,
                &actor_id,
                &owner_id,
                &catalog_id,
                &user_service_id,
                &slug,
                org_owned,
            )
            .await;
            let service = loaded_user_service(&user_service_id, &owner_id, &slug);
            let target = approval_target_for_tool(&db, &actor_id, &service)
                .await
                .unwrap();
            assert_eq!(target.service_id, catalog_id);
            assert_eq!(target.service_owner_user_id, owner_id);
            assert!(
                crate::services::approval_service::evaluate_deny_only(
                    &db,
                    &actor_id,
                    &target.service_owner_user_id,
                    &target.service_id,
                    &operation,
                )
                .await
                .unwrap(),
                "POC target must yield denied_by_policy for org_owned={org_owned}"
            );
            assert_eq!(
                crate::services::assistant_direct_agent_poc::enforce_deny_only(
                    &db, &actor_id, &actor_id, &service, &operation,
                )
                .await,
                Err("denied_by_policy"),
            );
        }
    }
}
